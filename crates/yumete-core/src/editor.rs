//! The [`Editor`]: top-level state owning the open buffers, the active one, and
//! the modal editing state (mode, cursor, command line).
//!
//! [`Editor::execute`] runs a parsed `:` command, and [`Editor::on_key`] drives
//! the modal state machine (Normal / Insert / Command) from backend-agnostic
//! [`Key`] presses, so the whole interaction can be unit-tested without a
//! terminal.

use crate::say;
use std::cell::{Cell, RefCell};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};

use regex::Regex;
use ropey::Rope;
use yumete_cjk::{is_han, CategorySegmenter, NoReader, Reader, Segmenter};

use crate::buffer::Buffer;
use crate::command::{self, Command, CommandError};
use crate::input::{Key, Mode};
use crate::lookfor;
use crate::motion;
use crate::ruby::{Dialect, Dialects};
use crate::text_store::TextStore;
use crate::zong::{self, Grid, Layout, DEFAULT_ZONG_LENGTH};

/// The two squares a Chinese paragraph opens with — what a level that draws an
/// indent draws when the reader has not named a width.
const DEFAULT_INDENT: usize = 2;

/// A paragraph's word ranges, kept against a hash of the paragraph's text.
type SegmentCache = HashMap<usize, (u64, Vec<(usize, usize)>)>;

/// A paragraph's 平仄, kept the same way and for the same reason (Feature #247).
type MeterCache = HashMap<usize, (u64, Vec<crate::meter::Mark>)>;

/// A line's inline notes, kept the same way and for the same reason (#248).
type NoteCache = HashMap<usize, (u64, Vec<crate::drawn::Run>)>;

/// The other work area: a buffer, a place in it, and what to look at there.
///
/// **The editor has one cursor** (Feature #176). A split does not give it a
/// second one: one pane holds the keys and this holds the place the *other*
/// pane was left at — written when it loses the keys, read when it gets them
/// back. It is the same act `Buffer::cursor` performs when you leave a file,
/// one level up, because two panes can hold one buffer and a buffer has room
/// for one place.
#[derive(Debug, Clone)]
pub struct Pane {
    /// The buffer's **id**, never its index: closing a file shifts every
    /// index, and a pane that kept one would show a different chapter.
    pub buffer: u64,
    cursor: usize,
    anchor: usize,
    goal_column: usize,
    goal_slot: usize,
    extend: bool,
    /// What to mark in it while it is only being read — a search hit, say.
    pub highlight: Option<(usize, usize)>,
    /// One line saying what this pane is showing.
    pub caption: String,
}

impl Pane {
    /// Where the pane is looking.
    pub fn cursor(&self) -> usize {
        self.cursor
    }
}

/// What a read-only draw is looking at instead of the live work area (#281).
///
/// The renderer asks the editor forty questions a frame — what is on line 12,
/// what is hidden there, where the caret is — and every one of them answers
/// about the **current** buffer. The other half of a split is a different
/// buffer, so all forty came back about the wrong file while the divider above
/// them printed the right file's name.
#[derive(Debug, Clone, Copy)]
struct Viewing {
    /// Into `buffers`, resolved once at the door: the pane keeps an id, and
    /// resolving it per question would be a scan per line of every frame.
    buffer: usize,
    /// The pane's place in that buffer, **clamped there**. The live cursor is
    /// an offset into another file and would index off the end of this one.
    at: usize,
}

/// Holds [`Editor::view_pane`] open, and closes it however the draw ends.
///
/// A guard rather than a closure because the renderer draws through a
/// `&mut Frame` it already holds, and because an override left standing after
/// a panic would put the keys in the wrong file.
///
/// It holds a **shared** borrow of the editor, so nothing can take a mutable
/// one while the override is open: 「reading only」 is not a rule anybody has
/// to remember here, it is the only program that compiles.
#[must_use = "the override lasts as long as this guard"]
pub struct Viewed<'a> {
    editor: &'a Editor,
    previous: Option<Viewing>,
}

impl Drop for Viewed<'_> {
    fn drop(&mut self) {
        self.editor.viewing.set(self.previous);
    }
}

/// The places one search found, in the buffer and revision it found them in.
#[derive(Debug, Clone)]
struct Hits {
    /// The buffer's id — never its index.
    buffer: u64,
    /// The revision it was searched at. An edit does not move the offsets
    /// *usefully*, so a stale list is dropped rather than adjusted.
    revision: u64,
    spans: Vec<(usize, usize)>,
    at: usize,
}

/// Which lines are off the page, against the buffer and revision they were
/// worked out for — and the span of lines where folding is unsafe because
/// something other than prose is written there.
type FoldMap = ((u64, u64), Vec<bool>, (usize, usize));

/// One line's Markdown runs, against the hash of the text they were read from,
/// keyed by the buffer that line is in and its number.
type MarkupCache = HashMap<(u64, usize), (u64, Vec<crate::markdown::Span>)>;

/// Every line's block and every merge conflict in the document, against the
/// buffer they were worked out for and that buffer's revision — the two things
/// that decide whether they are still true.
///
/// The conflicts ride along because they come out of the same walk: the block
/// scan already reads every line's opening, and the four markers are settled
/// by exactly those characters (#249).
type BlockCache = ((u64, u64), Vec<crate::markdown::Block>, Vec<crate::conflict::Conflict>);

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
    /// `M` awaiting the letter to name this place by.
    Mark,
    /// `'` awaiting the letter of a place to go back to.
    Recall,
    /// `]` or `[` — 「the next one of these」, and which way. Helix keeps the
    /// walks between things a document *has* on the brackets, and #249's
    /// `]c` is the first of them here.
    Hop { forward: bool },
    /// `空格 c` — what to keep of the merge conflict under the cursor.
    Conflict,
    /// `` ` `` — 「不改它說什麼，只改它長什麼樣」 (§5.2.3 ②).
    ///
    /// Helix spends three top-level keys here (`` ` `` 小寫, `` A-` `` 大寫,
    /// `~` 互換) on an operation that is the **identity on 漢字** — only
    /// full-width Ａ↔ａ actually maps. By the level law they can wait for a
    /// second key, so they became a group and the three keys are unbound.
    Case,
}

/// Which way an in-line character search runs.
///
/// **Two, not four** — `t`/`T` (till) were retired when `t` became the table
/// group (§14, 2026-09-04), and the `…To` in the old names was the till half
/// saying so. 「走到下一個某字母的前面」 has nothing to land on in a Chinese
/// manuscript; `f`/`F` stayed because 「走到下一個某字母」 still does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FindKind {
    Forward,
    Backward,
}

/// How many hits `:grep` gathers before it stops looking.
///
/// A listing longer than this is not an answer, it is the manuscript again;
/// the writer wants a narrower pattern, and being told so beats waiting.
/// How much bigger a `:write` has to be than the file on disk before it stops
/// to ask (Feature #295).
///
/// **Both bounds, not either.** 翻倍 alone fires on a 3 KB draft that grew to
/// 7 KB, which is a morning's writing; +256 KB alone fires on a long book
/// gaining a chapter. Together they describe the accident this exists for —
/// an alignment, a paste, a generated block — where the file *multiplies* and
/// the amount is more than a person types in a day. 2026-09-08 measured the
/// case that prompted it: 425,694 bytes to 2,945,642, on one keystroke.
const OVERSIZE_JUMP: u64 = 256 * 1024;

const GREP_LIMIT: usize = 500;

/// The largest file `:grep` will read. A manuscript chapter is kilobytes;
/// anything above this is data that happens to live in the same directory.
const GREP_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// How many mined words `:word discover` writes into the list (Feature #239).
///
/// Two hundred. What is being written is not a report but a *file the writer
/// then reads line by line*, and a thousand lines of it would be deleted
/// unread — which is worse than not offering them, because the ninety good
/// ones go with the rest. Ranked by count, so the two hundred kept are the
/// names that are on every page.
const DISCOVER_LIMIT: usize = 200;

/// How much of a project `:word discover` reads before it stops.
///
/// The three signals are ratios, so more text only sharpens them; this bound
/// is about the seconds a writer waits, not about the statistics. A novel is
/// one or two megabytes and never reaches it.
const DISCOVER_MAX_BYTES: usize = 16 * 1024 * 1024;

/// How many files the picker offers.
///
/// A project with more than this is not one a writer is choosing a chapter
/// from, and gathering all of it would make `Space f` pause before it drew.
const PICKER_LIMIT: usize = 4000;

/// A byte count the way a person says it: `425 KB`, `2.9 MB`.
///
/// Rounded on purpose. The question this feeds (#295) is 「did the file just
/// multiply」, and eight digits of a number nobody counts in makes that harder
/// to see, not easier — the multiple is said beside it and carries the answer.
fn human_size(bytes: u64) -> String {
    const K: f64 = 1024.0;
    let n = bytes as f64;
    match bytes {
        0..=1023 => format!("{bytes} B"),
        1024..=1_048_575 => format!("{:.0} KB", n / K),
        _ => format!("{:.1} MB", n / (K * K)),
    }
}

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

/// The scheme a link's destination opens with, lowercased, if it has one.
///
/// `https://…` has one, `fu.md` and `第三章` do not, and `mailto:` does — which
/// is the whole point: what is not named here is not handed to the machine.
/// Spelled the way RFC 3986 spells it, except that a single letter is never a
/// scheme, so a Windows path keeps its drive.
fn link_scheme(target: &str) -> Option<String> {
    let (before, _) = target.split_once(':')?;
    let mut chars = before.chars();
    let first = chars.next()?;
    (before.len() > 1
        && first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
    .then(|| before.to_ascii_lowercase())
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
    of: (u64, u64, usize),
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
    ///
    /// **`None` is not the same as `Some("")`**: an empty field is a finding in
    /// a 拆分表, and a field this row does not have at all is a *different*
    /// finding — the row is short, and the panel that drew both as a blank was
    /// the one place a person fixing cells by eye could not tell them apart.
    pub rows: Vec<(String, Option<String>)>,
    /// The rows this cell points at, and whether each one exists.
    pub links: Vec<(char, Option<usize>)>,
}

/// What `:shot` asked the front end to do once this frame is on the screen.
///
/// The editor decides *what* — which file, and whether the picture keeps its
/// colours — and refuses before the frame is drawn if it cannot. The front end
/// is the only half that holds the cells, so it does the writing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShotJob {
    /// Hand the screen to the platform's own screenshot program, which leaves
    /// the picture on the clipboard.
    Screen,
    /// The same program, told to write the picture here instead.
    Png { target: PathBuf },
    /// Write the frame here — coloured HTML unless `text`.
    Page { target: PathBuf, text: bool },
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
    ///
    /// The *keys* are `&'static str` — `hjkl` is `hjkl` in any language — and
    /// what they mean is a `String`, because it is said in the reader's.
    Keys(String, Vec<(&'static str, String)>),
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
    /// Feed it the **whole buffer** and rewrite the buffer with the answer
    /// (`:convert`).
    ///
    /// Not `Pipe`: that one replaces a selection and holds the result to a
    /// table's shape, both of which are wrong here. 簡繁 is a property of the
    /// manuscript, not of a passage — converting the paragraph the cursor
    /// happens to be in leaves a book in two scripts.
    Convert(String),
}

/// A file to hand to the typesetter, or a typesetter to stop.
///
/// `:render full` is how much of the result *this* page shows; this is the
/// other kind of preview — the real one, made by the tool that makes the book,
/// shown where a book can be shown. They are different questions and they get
/// different words.
/// A language's own command, which the front end runs (Feature #197).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanguageRun {
    /// The verb the reader asked for: `format`, `preview`, or a name of their
    /// own.
    pub verb: String,
    /// Which language's table to look in — the buffer's syntax, by name.
    pub language: String,
    /// The file it is about.
    pub path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preview {
    Start {
        path: PathBuf,
        syntax: crate::syntax::Syntax,
    },
    /// A typesetter is already running: open its page again rather than start a
    /// second one. `:preview` twice used to kill the server and start another,
    /// which is a fresh compile of the whole book to answer 「where was that
    /// page again?」.
    Show,
    Stop,
}

/// How much of the result the page shows.
///
/// One axis, not two switches: each step shows more of what the file *means*
/// and less of how it is written — and the same three names the other three
/// dimensions take, because it is the one that assigns them (#283).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Render {
    /// The file exactly as it is, in one colour.
    Off,
    /// Coloured, with every marker still on the page. The default: this is a
    /// manuscript, and you have to be able to see what is in the file.
    ///
    /// **It adds and it does not take away** — style over the characters the
    /// writer typed, and never a character fewer. That is the law the other
    /// three dimensions are held to at this level as well.
    ///
    /// It was called `On` until #283, and `:render` with no argument used to
    /// mean it. It reports now: there are three levels, so the bare word is
    /// better spent saying which one you are on.
    #[default]
    Basic,
    /// The markers come off the page — except the ones the cursor is inside,
    /// so the cursor is never in text that is not on the screen (所見即所得).
    Full,
}

/// How the editor says, beside the caret, what you have typed (#284).
///
/// **Not a fifth dimension of [`Render`].** `:render` writes the levels of the
/// four things that decide how the *file* is drawn; this one is how the editor
/// talks about *itself*, and a reader who turns the markup off has said
/// nothing about whether the count in front of `d` should be visible. So
/// `:render` never writes it, and `render.is` keeps its four fields.
///
/// The three names are the shared ones, and the factory level is `Basic` for
/// §5.7's reason: the level a window opens at must be identical to `Off` in
/// the worst case, and 醒目 is bought with style rather than with hiding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hud {
    /// Nothing beside the caret. The status line's right edge still says it —
    /// that is #193's floor, and no level takes it away.
    Off,
    /// A 藥丸 in the nearest margin: gold on the band, one row high, and
    /// **not one character of the manuscript hidden**. Where it lands is
    /// scored against the caret (#269), so it moves as the page fills.
    #[default]
    Basic,
    /// A bordered panel, pinned under the caret, over whatever is there.
    ///
    /// The frame and the covering are one decision, not two: a panel wants a
    /// rectangle that a page of prose does not have, so a frame forces
    /// covering — and once the mark sits on the same paper as the writing,
    /// the frame is the only thing saying which characters are not yours.
    Full,
}

/// What `:view hud` says about a level, whether it was just set or only asked.
fn hud_says(how: Hud) -> String {
    match how {
        Hud::Off => say!("hud.off"),
        Hud::Basic => say!("hud.basic"),
        Hud::Full => say!("hud.full"),
    }
}

impl Hud {
    /// The name it is written with, on the command line and in a report.
    pub fn tag(self) -> &'static str {
        match self {
            Hud::Off => "off",
            Hud::Basic => "basic",
            Hud::Full => "full",
        }
    }
}

/// One line of a register's contents, for a list to show.
///
/// A yank is often a paragraph and sometimes a chapter; what tells two of them
/// apart is the first line and the size, so that is what is shown.
fn one_line(text: &str) -> String {
    let first: String = text.lines().next().unwrap_or("").chars().take(40).collect();
    let lines = text.lines().count();
    if lines > 1 {
        say!("register.several-lines", first, lines)
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
    /// Which buffer (by id), which revision of it, which line the cursor was
    /// on — and which **kind** of region was asked for, because since #216
    /// two of them are walked and one cache answers both.
    asked: (u64, u64, usize, Bounds),
    region: Option<crate::mdtable::Region>,
}

/// Every `|` table in the file, and which document that was true of.
#[derive(Debug, Clone)]
struct MdTables {
    /// Buffer id and revision — the same key `blocks_through` uses.
    asked: (u64, u64),
    /// `(first, last)` of every table that parses as one, in file order.
    rows: Vec<(usize, usize)>,
}

/// What one table's drawn padding was worked out from (Feature #212).
///
/// Everything the answer depends on, so that a hit is really a hit: which
/// document and which revision of it, which lines the table occupies, and the
/// state that decides what comes *off* the page on the way to the screen —
/// 所見即所得, the readings being laid out, and the cursor and selection,
/// since the construct they are inside is never hidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PadKey {
    buffer: u64,
    revision: u64,
    first: usize,
    last: usize,
    /// **Where the caret is, but only when it can change the answer.**
    ///
    /// 所見即所得 puts a run of markup back on the page under the selection,
    /// so what comes off a row — and therefore how wide its cells draw —
    /// moves with the caret. Under `:render basic` it does not: nothing in
    /// [`Editor::hidden_on_line`] asks where the caret is unless
    /// [`Editor::wysiwyg`] is true.
    ///
    /// Carrying it unconditionally threw the whole table's padding away on
    /// every `j`, and a padding is a walk down every row of the table: one
    /// flick of a trackpad over the 223-row table in this project's own
    /// `development.md` took 8.3 s. Asking only when the answer depends on
    /// it: 41 ms.
    caret: Option<(usize, usize)>,
    render: Render,
    ruby: Dialects,
    /// **Whether cells are folded** (#283). `t f` and `t w` move neither the
    /// revision nor `:render`, and a folded cell is a *narrower* cell — so
    /// without this the level a table was first drawn at was the level it kept
    /// until somebody typed in it, and `t w` did nothing at all.
    folds: bool,
    /// What comes off the page depends on how the file is being read, and
    /// `:syntax` changes that without touching a byte of it — so a table drawn
    /// with the backticks hidden stayed drawn that way after `:syntax text`
    /// put them back, one cell ragged per hidden character.
    syntax: crate::syntax::Syntax,
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
    /// What splits one cell from the next.
    pub separator: Separator,
    /// Whether the grid widget has the whole window — `t t` (#283).
    ///
    /// **Orthogonal to the level**, which is why `t q` needs nothing written
    /// down to find its way back: it puts this to `false` and whatever
    /// [`Editor::table_level`] was before the takeover is still there. The
    /// field this replaced remembered the way back by hand, and had to be
    /// cleared correctly at every exit or `t q` walked you into prose.
    ///
    /// [`Editor::table_level`]: crate::editor::Editor::table_level
    pub pane: bool,
    /// Where the table starts and stops.
    pub bounds: Bounds,
    /// How far the mode reaches — this table, or every table in the file.
    pub reach: Reach,
}

/// What splits one cell of a row from the next (#261).
///
/// **The splitter, and nothing else.** It used to be half of `Shape`, whose
/// other half was which renderer draws the thing — so every one of the twenty
/// tests against it had to be reread to find out which of the two questions it
/// was really asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Separator {
    /// One character between cells: `,` in a CSV, a tab in a TSV, and — when
    /// the third tier of #261 lands — `&` in LaTeX or Typst. Every line of the
    /// file is a row.
    Delimiter(char),
    /// Markdown's own spelling: a `|` on both sides of every cell. **Only** a
    /// line that opens with one is a row, which is what lets a table live in a
    /// chapter without the paragraphs around it becoming cells.
    Pipe,
}

/// How much of a table is drawn (#283).
///
/// One of the four dimensions — `render`, `table`, `ruby`, `indent` — and they
/// all take the same three values, so that a reader who learns one has learnt
/// the others. `:render <level>` assigns this one; `t o` / `t b` / `t f` set
/// it directly, and the next `:render` washes that override away.
///
/// **The law that decides which level a feature belongs to**: `Basic` does not
/// hide, does not fold, and does not replace — it may only *add*, and what it
/// adds is never in the file. `Full` may do all three. That is why the walls
/// are `Full`: drawing the writer's `|` as `┆` is replacing a character.
///
/// The full-window grid is **not** a level. It is [`TableView::pane`], because
/// it is a different question: how much of a table is drawn, and whether it
/// has the screen to itself, are answered separately and `t q` only touches
/// the second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellWidth {
    /// The cap bites: what is past it comes off, and a `>` says so.
    Fold,
    /// Every cell as wide as it is, running off the side of the window.
    Whole,
    /// The cap bites and **the rest is drawn underneath**, inside the cell's
    /// own column — the grid's answer only, since the prose page's rows come
    /// from the shared wrap layer and it has no hanging indent to give.
    Wrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableLevel {
    /// 源碼模式 `t o` — the file as it is written. `|` and commas and all,
    /// and `hjkl` are letters: the grid is not there to take them.
    Off,
    /// `t b` — the columns line up because the text itself is padded (#212),
    /// and the **keys** belong to the grid where a table is: `hjkl` walk
    /// cells, `t d` drops a row. **Nothing is hidden, folded or replaced** —
    /// every character the writer typed is still on the page, so a 縱書
    /// chapter with no table in it is drawn exactly as `Off` draws it.
    #[default]
    Basic,
    /// `t f` — everything `Basic` draws, and then the parts are optimised:
    /// the `|` the writer typed is drawn as the wall `┆` it means, and the
    /// column ruler goes above the table.
    ///
    /// **The columns are the table's**, not the screen's: the widest cell in
    /// each column decides, once, for the whole table. That is what makes the
    /// padding stable while you scroll — and it is the one thing the pane does
    /// differently.
    Full,
}

/// The numbers a `t` or `g` sequence has been given, and how they were joined.
///
/// **`-` is a range, `,` is a list or a pair** (§5.7). One key had been doing
/// both jobs, and the day `t20,20g` was typed the two readings collided: row
/// 20 *and* column 20 is two kinds of thing, where `t2-10/` is a span of one
/// kind. They are different keys now, and one sequence is one of them or the
/// other — never both, because nothing has been agreed about what a mixture
/// would mean.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Sequence {
    /// In the order typed, never empty — the one being typed is the last.
    numbers: Vec<usize>,
    /// `None` until a second number has been asked for.
    joint: Option<Joint>,
}

/// What the key between two of a [`Sequence`]'s numbers meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Joint {
    /// `2-10` — the first, the last, and everything between them.
    Span,
    /// `1,5,9` — these and no others; with exactly two of them, a pair.
    List,
}

impl Sequence {
    /// A sequence just begun: one number, still being typed.
    fn started() -> Self {
        Self { numbers: vec![0], joint: None }
    }

    /// The number being typed — always the last one.
    fn last(&mut self) -> &mut usize {
        self.numbers.last_mut().expect("never empty")
    }

    /// The single number this is, if it is a single number.
    ///
    /// What `a`/`d` close over: `t1a` is one column, and `t2-5a` is not a
    /// sort key at all.
    fn one(&self) -> Option<usize> {
        match self.numbers.as_slice() {
            [n] => Some(*n),
            _ => None,
        }
    }

    /// The two numbers this names, if it names exactly two of them.
    ///
    /// `t20,20g` — row and column, and the comma is what says so.
    fn pair(&self) -> Option<(usize, usize)> {
        match (self.joint, self.numbers.as_slice()) {
            (Some(Joint::List), [a, b]) => Some((*a, *b)),
            _ => None,
        }
    }

    /// The span this names, for the keys that read one — `g2-5d`.
    fn span(&self) -> Option<(usize, usize)> {
        match (self.joint, self.numbers.as_slice()) {
            (None, [n]) => Some((*n, *n)),
            (Some(Joint::Span), [a, b]) => Some((*a, *b)),
            _ => None,
        }
    }

    /// Every column this names, 1-based and in the reader's order.
    ///
    /// The one reading both joints answer: `t3/` is one column, `t2-10/` is
    /// nine, `t1,5,9s` is three.
    fn columns(&self) -> Vec<usize> {
        match (self.joint, self.numbers.as_slice()) {
            (Some(Joint::Span), [a, b]) => {
                let (a, b) = (a.min(b), a.max(b));
                (*a..=*b).collect()
            }
            _ => self.numbers.clone(),
        }
    }

    /// What has been typed, read back for the HUD — `2-10`, `1,5,9`, `20`.
    fn spelled(&self) -> String {
        let joint = match self.joint {
            Some(Joint::Span) => "-",
            _ => ",",
        };
        // A number still at zero because only the joint has been typed is not
        // written out: `t2-` reads as `t2-`, not as `t2-0`.
        let mut out = String::new();
        for (at, n) in self.numbers.iter().enumerate() {
            if at > 0 {
                out.push_str(joint);
                if *n == 0 && at + 1 == self.numbers.len() {
                    break;
                }
            }
            out.push_str(&n.to_string());
        }
        out
    }
}

/// Where the table starts and stops (#261).
///
/// **The rule, not the answer.** The lines are walked from the cursor every
/// time they are wanted — a document is edited while the table is open, so a
/// stored pair of line numbers is wrong one keystroke later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bounds {
    /// The first line to the last. A schema, or a name like `.csv`, is a person
    /// saying「this whole file is one table」, and the boundary is the one thing
    /// that used to have no value at all for such a file — it was the *absence*
    /// of `md_region()`, which is why a CSV and a `|` table could not share a
    /// code path.
    WholeFile,
    /// Walked out from the cursor by [`crate::mdtable::region`]: up and down
    /// while the line is still a `|` row, and never into a fenced block.
    Md,
    /// A run of delimited lines **recognised where it stands** (#216).
    ///
    /// A 碼表 pasted into a chapter, a `dict.yaml` whose table begins under a
    /// `---` preamble, a LaTeX `tabular`: the file is not a table and never
    /// becomes one, but these few lines of it are. Walked out from the cursor
    /// the same way `Md` is — up and down while the line still holds the
    /// separator — and stopping at a blank line, which is *a* boundary and not
    /// the boundary, because a table often sits directly under its heading.
    ///
    /// **Recognised, not converted.** `:table pipe` rewrites a block so that
    /// the file says what it is on every one of its own lines; this is the
    /// other answer, for the block that must stay byte for byte as it is.
    Block,
}

/// How far a table mode reaches (#275).
///
/// **The author's rule, 2026-09-05**, and the reason it is safe:
///
/// > 有明確表格語法定義的文檔（csv tsv markdown），可以在文件任何位置通過 `ti`
/// > `tt` 進入表格視圖…對於這個文件中所有的表格都生效。如果一個文件沒有確切的
/// > 表格語法，比如 txt、yaml 用空格制表符隔開，那麼我們就…在制表符上按 `ti`
/// > `tt` 將這一段進入表格模式，離開表格立刻回到 prose 狀態。
///
/// A file whose syntax *says*「table」can be turned on with confidence. A file
/// where the editor is **guessing** from a run of tab characters should never
/// leave that guess standing on the screen after the reader has walked away
/// from it.
///
/// This is not [`Bounds`]. `Bounds` says which lines *this* table occupies —
/// asked freshly every time, because the document is being edited. `Reach`
/// says whether the mode survives the cursor walking out of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// The mode belongs to **the file**. Every table in it is read as a table,
    /// and walking into the prose between two of them leaves both drawn.
    File,
    /// The mode belongs to **this block**. Walk out of it and the file is prose
    /// again; to see it as a table once more, press again.
    Cursor,
}

impl TableView {
    /// Whether it is drawn as part of the document it sits in.
    pub fn in_prose(&self) -> bool {
        !self.pane
    }

    /// Whether this table takes the whole pane — the grid widget's own case.
    pub fn takes_the_pane(&self) -> bool {
        self.pane
    }

    /// Whether the mode belongs to the file rather than to one block (#275).
    pub fn is_file_wide(&self) -> bool {
        self.reach == Reach::File
    }

    /// Where the cells of one line are — what an edit takes.
    pub fn cells(&self, line: &str) -> Vec<(usize, usize)> {
        match self.separator {
            Separator::Pipe => crate::mdtable::cells(line),
            Separator::Delimiter(d) => crate::table::cells(line, d),
        }
    }

    /// Where the **boxes** of one line are — what a reader sees as one cell.
    ///
    /// The two differ only for a `|` table, where the spaces around the content
    /// are the column's own width. See [`Editor::row_cell_boxes`].
    pub fn boxes(&self, line: &str) -> Vec<(usize, usize)> {
        match self.separator {
            Separator::Pipe => crate::mdtable::boxes(line),
            Separator::Delimiter(d) => crate::table::cells(line, d),
        }
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
    pub fn label(self) -> String {
        match self {
            Grain::Cell => say!("label.cell"),
            Grain::Char => say!("label.character"),
        }
    }
}

/// The depth of a Chinese chapter heading, if this line is one.
///
/// 「第三章」, 「第四百一十二卷」, 「楔子」, 「後記」 — how a manuscript with no
/// markup says where a chapter begins, and the only thing `:toc` can go on in a
/// `.txt`. Deliberately narrow: a *short* line that begins with 第 and a number
/// and a chapter word, or that is one of a dozen names a book uses for its
/// front and back matter. A line of prose that happens to open with 第一章的
/// 那天 is 20 characters into a sentence and is not caught by this.
fn chapter_heading(line: &str) -> Option<(usize, String)> {
    const NAMED: &[&str] = &[
        "序", "序章", "序言", "自序", "前言", "引子", "楔子", "小引", "凡例",
        "尾聲", "尾声", "終章", "终章", "後記", "后记", "跋", "附錄", "附录",
        "番外", "外傳", "外传", "目錄", "目录",
    ];
    const DIGITS: &str = "一二三四五六七八九十百千萬万零〇兩两0123456789０１２３４５６７８９";
    let text = line.trim();
    let chars: Vec<char> = text.chars().collect();
    // A heading is a line by itself, and a short one. The longest real chapter
    // title in the corpora this was written against is well under this.
    if chars.is_empty() || chars.len() > 40 {
        return None;
    }
    // 序、楔子、後記: no number, so the whole line has to be the name.
    if NAMED.contains(&text) {
        return Some((2, text.to_string()));
    }
    // A 卷 holds 章 the way a part holds chapters, so it sits above them.
    let depth = |unit: char| match unit {
        '卷' | '部' | '篇' | '集' => Some(1),
        '章' | '回' | '節' | '节' | '折' | '幕' | '話' | '话' => Some(2),
        _ => None,
    };
    let run = |from: usize| chars[from..].iter().take_while(|c| DIGITS.contains(**c)).count();
    // **Both spellings.** 第三章 and 第一卷 put the number between 第 and the
    // unit; 卷002 and 卷二十 put the unit first and drop 第 — which is how
    // 資治通鑑 writes all 294 of its 卷, and the book this feature was written
    // for. A unit with no number after it is prose: 「話說天下大勢」 opens 三國
    // 演義 and is not a chapter heading.
    let (unit, after, number) = match chars[0] {
        '第' => {
            let digits = run(1);
            if digits == 0 {
                return None;
            }
            let number: String = chars[1..1 + digits].iter().collect();
            (*chars.get(1 + digits)?, 2 + digits, number)
        }
        first if depth(first).is_some() => {
            let digits = run(1);
            if digits == 0 {
                return None;
            }
            let number: String = chars[1..1 + digits].iter().collect();
            (first, 1 + digits, number)
        }
        _ => return None,
    };
    // What comes after the unit is the title, and it has to be separated from
    // it — 第三章 or 第三章　風雪, but not 第三章魚 (which is prose).
    match chars.get(after) {
        None => {}
        Some(c) if c.is_whitespace() || matches!(c, '、' | '：' | ':' | '.' | '·' | '，') => {}
        Some(_) => return None,
    }
    // **The number, not the line**, so a book that writes the same chapter twice
    // running — 「第一卷　周紀一」 and then 「巻一 ◄ 資治通鑑」 at its end — is
    // one chapter in the outline. 巻 and 卷 are the same 卷.
    let same = |c: char| match c {
        '巻' => '卷',
        '节' => '節',
        '话' => '話',
        other => other,
    };
    depth(unit).map(|d| (d, format!("{}{}", same(unit), number)))
}

/// Where a picture goes when nobody said where (#189).
///
/// **Not beside the manuscript.** A picture is made to be sent to somebody and
/// then forgotten about; a chapter folder that fills up with them is a folder
/// the writer has to tidy. The downloads folder is where the rest of the
/// machine already puts things of that kind, and every status line names the
/// full path, so 「它到哪去了」 is answered before it is asked.
///
/// `XDG_DOWNLOAD_DIR` first, because a Linux desktop that has been told the
/// folder is called something else has been told in that one place. Then
/// `~/Downloads` if it is really there — and if it is not, home rather than a
/// folder invented in somebody's home directory, because a picture in the
/// wrong place can be moved and a folder that appeared by itself cannot be
/// explained. Failing even that, wherever the editor was started.
fn downloads_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_DOWNLOAD_DIR") {
        if !dir.trim().is_empty() {
            return PathBuf::from(dir);
        }
    }
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()));
    if let Some(home) = home {
        let home = PathBuf::from(home);
        let downloads = home.join("Downloads");
        if downloads.is_dir() {
            return downloads;
        }
        if home.is_dir() {
            return home;
        }
    }
    PathBuf::from(".")
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
        // Not what this editor just wrote. `:export html` puts the book's own
        // words into a `.html` beside it, and `:grep` then found every one of
        // them twice — the second time in a file the writer cannot edit.
        if entry.file_type().is_ok_and(|t| t.is_file()) && is_build_output(&name) {
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

/// How often the disk is asked whether the file moved, under `:reload auto on`
/// (Feature #214).
///
/// Two seconds, not five: this one answers a question the writer is *waiting*
/// on — they alt-tabbed away, ran a script, and came back to see whether the
/// page caught up. The cheap path is one `stat`.
const DISK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

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
    /// Which row of the `::` search is under the highlight (Feature #224).
    ///
    /// Reset to the top by every keystroke that changes the query: the row
    /// that was third for `竖` is not the row that is third for `竖排`, and
    /// keeping the number would leave the highlight pointing at something
    /// nobody chose.
    lookfor_focus: usize,
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
    /// Where readings come from for `:ruby auto` (Feature #234). Defaults to
    /// [`NoReader`], which knows nothing; the front end installs a reader over
    /// 宇浩's 字料層 once the data is loaded, exactly as it does the segmenter.
    reader: Box<dyn Reader>,
    /// The project's own words, shared with the segmenter wrapped around the
    /// one in force — so reloading the list reaches a segmenter already handed
    /// out.
    project_words: std::rc::Rc<RefCell<yumete_cjk::WordList>>,
    /// Whether the segmentation overlay (word background tint) is shown.
    show_segmentation: bool,
    word_mark: yumete_cjk::WordMark,
    /// How loudly the editor says what you have typed, beside the caret
    /// (Feature #284).
    hud: Hud,
    /// How readily characters join into words (`:word level`), kept so a
    /// segmenter installed later arrives at the level the reader chose.
    word_level: yumete_cjk::WordLevel,
    /// `:word list reload` asking the front end to build the dictionary again
    /// — it owns the IME and the data directory; the editor owns neither.
    words_request: bool,
    /// How a table's columns are told apart (Feature #157).
    table_rules: crate::table::Rules,
    /// **疏排 on the horizontal page** (Feature #181): a row of air above every
    /// row, which is what 密排's opposite means when the writing runs across
    /// rather than down. Its own field rather than `!dense`, because `dense`
    /// starts out false on a fresh editor and 疏排 must be something the reader
    /// asked for.
    loose_rows: bool,
    /// **Typewriter mode** (Feature #166): the row being written stays in the
    /// middle of the screen and the paper moves under it, the way a typewriter
    /// works and the way every focus mode since has.
    typewriter: bool,
    /// 焦點模式: everything but the 段 being written stands back a rung
    /// (Feature #246). A drawing setting — motion and wrapping never see it.
    focus: bool,
    /// How far one notch of the mouse wheel moves (Feature #222), counted in
    /// whichever unit the page is set in — 縱 vertically, rows horizontally.
    ///
    /// Kept here rather than read out of the config at the front end, because
    /// `:wheel` has to be able to change it while the editor is running, and
    /// the config is not written back.
    wheel_step: usize,
    /// The detail panel's width, when the reader has said one (Feature #187).
    /// `None` follows the config.
    detail_width: Option<usize>,
    /// Whether a row of column numbers is drawn above the header.
    ///
    /// **The keys need it.** `3gd`, `t20,20g`, `t1a2d8as` all name a column by
    /// number, and a 28-column 拆分表 gives no way to count to 17 except by
    /// counting. One row, and the numeric keys become usable.
    table_numbers: bool,
    /// Whether a 碼表 is loaded, as last reported by the front end.
    ime_available: bool,
    /// Whether the answer is being **shown in the other work area** (`gw`,
    /// `g?`, `t?`) or **gone to** (`gd`, `g/`, `t/`). Set as the key is pressed
    /// and read wherever the landing happens — including a page later, when a
    /// search's `n` walks to the next hit.
    definition_preview: bool,
    /// A `:shot` waiting for the frame it is a picture of.
    screenshot_request: Option<ShotJob>,
    /// A character whose 字料 the front end has not looked up yet (#215).
    dictionary_query: Option<char>,
    /// The character the 字典 panel is about, and the answer if one has come.
    ///
    /// Three states, because three things can be true. `None`: nobody has
    /// asked. `Some(ch, None)`: asked, and the front end has not answered yet
    /// — the panel shows the character alone. `Some(ch, Some([]))`: answered,
    /// and the 拆分表 has nothing for it, which the panel has to say out loud
    /// rather than draw as an empty box.
    dictionary: Option<(char, Option<Vec<(String, String)>>)>,
    /// The other work area, when the page is split (Feature #176).
    other: Option<Pane>,
    /// Which half of the screen holds the keys — **screen order**, so
    /// switching panes never makes the top one jump to the bottom.
    live_pane: usize,
    /// Whether the line-number band carries a ground of its own.
    number_fill: bool,
    /// What is drawn in a paragraph's opening squares, if anything.
    indent_hint: crate::zong::IndentHint,
    /// The character `IndentHint::Symbol` draws there.
    indent_symbol: String,
    /// A pending count prefix, so `3w` moves three words (Helix counts).
    count: Option<usize>,
    /// The second half of a **span** count — `2-5gd` is columns two through
    /// five. `Some(None)` means the `-` has been typed and the number after it
    /// has not; `Some(Some(n))` is that number.
    ///
    /// A span is not a repetition, so it is not `count`: 「do this five times」
    /// and 「do this to columns two through five」 are different things, and one
    /// number cannot say both.
    count_to: Option<Option<usize>>,
    /// The text typed during the last Insert session, replayed by `.`.
    /// The keys of the command being watched, and the revision it started at.
    ///
    /// `.` repeats the last **change**, and a change here is not one shape: it
    /// is `r` plus a character, `d` on a selection, `ms(`, `mr\"'`, a whole
    /// typing session. Rather than enumerate them, the editor watches: a
    /// command that leaves the buffer different from how it found it *was* a
    /// change, and its keys are what `.` plays back.
    edit_keys: Vec<Key>,
    /// Which buffer the command started in, and that buffer's revision.
    ///
    /// The buffer too: after `gn` the revision belongs to a *different* file,
    /// so a pure motion looked like a change and `.` came to mean 「switch
    /// buffer」 — which for someone walking a hundred chapters is the common
    /// case.
    edit_revision: (u64, u64),
    /// The last change's keys.
    last_edit_keys: Vec<Key>,
    /// What the IME last committed into a `r` (§5.2.3 ②), so `.` can repeat it.
    ///
    /// The code letters are eaten by the IME and never reach [`Editor::on_key`],
    /// so replaying the keys of an IME replace replays `r` and nothing else.
    /// The text is remembered here and [`Editor::repeat_edit`] finishes it.
    last_replacement: String,
    /// Places named by a letter, and reachable from any file (`M a`, `' a`).
    marks: HashMap<char, Spot>,
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
    /// A pending `:theme`, waiting for the front end that owns the palette.
    theme_request: Option<(Option<String>, Option<crate::command::Mood>)>,
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
    /// The columns `gd` was asked about this time: `(first, last)`, 1-based.
    column_span: Option<(usize, usize)>,
    /// The numeric argument of the sequence being typed — `g3d`'s 3, `g2-5d`'s
    /// 2 and 5, `t1,5,9s`'s three columns. See [`Sequence`].
    sequence: Option<Sequence>,
    /// The columns a sort has been told about so far, 1-based, `true` for
    /// descending — `t1a2d8a` is three of them, waiting for its `s`.
    ///
    /// **A prefix-free grammar** (2026-09-05): the old spelling was
    /// `t1s2S8s`, where the very first `s` is already a whole command, so a
    /// multi-column sort could not be typed at all. `a`／`d` close a column
    /// without asking for anything to happen, and `s` is the one key that acts.
    sort_keys: Vec<(usize, bool)>,
    /// Whether the move that just happened was a **jump** — a search hit, a
    /// mark, `gg`, `:42` — rather than a step. The page centres a jump.
    jumped: bool,
    /// A pending `:format` or `:run <name>`, waiting for the front end — the
    /// editor knows *what* was asked for and the front end knows how to run a
    /// program.
    language_run: Option<LanguageRun>,
    /// A pending `:view preview`, waiting for the front end — starting a typesetter
    /// is running a program, which only the front end can do.
    preview_request: Option<Preview>,
    /// Where the typesetter that **is running** put its page.
    ///
    /// A preview server is a thing with a life of its own: it holds a port and
    /// a few hundred megabytes for as long as it runs. The editor knows it is
    /// there so it can say so on the status bar, hand the address back when
    /// asked, and refuse to start a second one.
    preview_at: Option<String>,
    /// A link the reader followed to a page on the web (`gx`), waiting for
    /// the front end — only it can hand a URL to the machine, and only ever as
    /// one argument to `open`/`xdg-open`.
    open_request: Option<String>,
    /// A pending `:sh` or `:!`, waiting for the front end.
    shell_request: Option<Shell>,
    /// The 字形 table waiting for opencc to come back (Feature #241).
    convert_patch: Option<crate::convert::Side>,
    /// The grid this file is being read as, when a schema says it is a table.
    ///
    /// A view, never a copy: the text stays the truth, and this only says how
    /// to find the cells in it.
    ///
    /// **Which table, not which level** (#283). It answers 「where are the
    /// cells around the cursor」, and that is discovered per buffer — every
    /// open clears it, and walking out of a guessed block clears it. What the
    /// *reader asked for* is [`Editor::table_level`], which survives all of
    /// that because a preference that clears itself is not a preference.
    table: Option<TableView>,
    /// How much of a table is drawn — the reader's standing answer (#283).
    ///
    /// Untouched by opening a file, by walking out of a table, by there being
    /// no table at all. `TableLevel::Basic` in a file with no table draws the
    /// same page `Off` does, to the character, because every gate below asks
    /// `table_here()` first — which is what makes it safe as the factory
    /// value, and what lets `:render` assign it without knowing anything about
    /// the file.
    table_level: TableLevel,
    /// Whether the detail panel is wanted. It only appears where there is
    /// something to say, so this is "show it when there is", not "show it".
    show_detail: bool,
    /// Whether the grid's edit guard is lifted for the operation in hand.
    table_bypass: std::cell::Cell<bool>,
    /// Where buffers with no file keep their recovery copies.
    drafts_dir: Option<PathBuf>,
    /// The data directory, for the global word list. Set by the front end.
    data_dir: Option<PathBuf>,
    /// Where this project's session is remembered — which files were open and
    /// where the cursor was in each.
    session_file: Option<PathBuf>,
    /// The layout a grid turned the page away from, so leaving gives it back.
    turned_for_table: Option<Layout>,
    /// Where the cursor was before each far jump, and how far back we have
    /// walked through them.
    /// A gap between 縱 set at runtime, overriding the config's.
    ///
    /// The one piece of the dense arrangement that was a startup-only setting
    /// while the other three were live toggles — which is why `:view dense` had to
    /// exist rather than being three keys anybody could find.
    zong_gap: Option<usize>,
    /// Whether the dense arrangement is on, so the ticks know to stay away.
    dense: bool,
    /// Whether every 句 opens a 縱 of its own (`:view sentence`, Feature #237).
    sentences: bool,
    /// How many squares open a paragraph (首行縮進), as configured.
    indent: usize,
    /// Whether the blank line between two indented paragraphs comes off the
    /// page — 全 only (#283).
    ///
    /// The indent itself is 中階: two squares *added* at the head of a
    /// paragraph, and every character the writer typed still there. Folding
    /// the blank line the indent stands in for is the 全 half of the same
    /// idea, and 中階 does not fold.
    indent_folds: bool,
    /// Whether a cell wider than [`crate::mdtable::MAX_COLUMN`] has its tail
    /// folded away — `t w` is the switch (#283).
    ///
    /// Off, the columns go to their natural width and run off the side of the
    /// window, which is what the reader asks for when a cell is the thing
    /// being read rather than scanned — the reason this is a switch and not a
    /// constant.
    ///
    /// **`None` is 「nobody has said」**, and the place answers for itself:
    /// the 全 page folds, 基本 does not (「`basic` 不藏、不摺、不替換」), and
    /// the `t t` grid caps at its own width wherever it is opened from. `t w`
    /// writes a `Some`, and from then on the reader's answer travels with them
    /// — a bare `bool` could not hold both 「基本 folds nothing unasked」 and
    /// 「基本 folds when asked」, which is the whole of the question
    /// (2026-09-07: 「虽然 tb 在默认状态下不折叠，但能不能在按下 tw 之后折叠？」).
    cell_folds: Option<CellWidth>,
    /// Whether this key **meant** to go to another table — `t ]` and `t [`.
    ///
    /// 全窗表格 holds the cursor to the table it is showing
    /// ([`Editor::hold_the_pane`]), and the two keys whose whole job is to
    /// leave for the next one would otherwise be held with everything else.
    /// A one-shot, cleared at the end of every key: 「this move was asked
    /// for」 is about the key that is being handled, not about the editor.
    crossed_tables: bool,
    /// How many bands the vertical page is divided into (段組).
    bands: usize,
    /// What the last `Enter` search found, **and which document it found it
    /// in** (Feature #191).
    ///
    /// A bare `Vec<(usize, usize)>` of char offsets is the most dangerous
    /// thing this editor can hold: it survived a buffer switch and an edit,
    /// while `n` and `N` belonged to it, so 「第 3/78 處」 could be said about a
    /// character in a file that was never searched — and the next `d` deleted
    /// it. The rule the marks and the jump list already follow, applied here:
    /// **a stored position names the buffer it is in**, and a revision that
    /// has moved means the answer is gone rather than wrong.
    hits: Option<Hits>,

    /// Where a jump came from: the buffer's **id** and the cursor.
    ///
    /// By id, not by index: closing a buffer shifts every later one down, and
    /// a jump list that kept indices would walk back into a different chapter.
    jumps: Vec<(u64, usize)>,
    jump_at: usize,
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
    /// The paper `:export html` writes its print stylesheet for. Not a screen
    /// setting and not a command: the trim a book is printed at is decided once
    /// for the book, so it is read from the configuration and left alone.
    paper: crate::export::Paper,
    /// The other work area, while it is being drawn (#281). See [`Viewing`].
    viewing: Cell<Option<Viewing>>,
    /// The question the editor has stopped to ask, if it has (#295).
    query: Option<Query>,
    /// Word ranges already worked out, per line, against a hash of that line.
    segment_cache: RefCell<SegmentCache>,
    /// 平仄 in the margin (Feature #247), and the answers already worked out.
    ///
    /// A drawing setting, like [`Self::focus`]: it changes what is in the
    /// margin beside the writing and never what the writing is.
    meter: bool,
    meter_cache: RefCell<MeterCache>,
    /// Inline notes on the marks a Chinese manuscript got wrong (#248), and
    /// the answers already worked out.
    ///
    /// The first producer of [virtual text](crate::drawn) that is neither the
    /// writer's own typing nor a table's geometry: the editor saying something
    /// *about* the text, in the text's own place, as it is written.
    notes: bool,
    note_cache: RefCell<NoteCache>,
    /// Which lines are folded away, against the buffer they were worked out
    /// for. One pass over the file per edit — the answer is not line-local (a
    /// blank line inside a fence is code, not a paragraph break), and asking
    /// per line would walk the document once per line.
    fold_cache: RefCell<Option<FoldMap>>,
    /// The Markdown runs of each paragraph, cached the same way and for the
    /// same reason: the renderer asks for every paragraph on screen, every
    /// frame, and the answer only changes when the paragraph does.
    /// Keyed by the buffer's **id** and the line — see `blocks_through`.
    /// A `HashMap<line, …>` said that line 3 of every file was the same line,
    /// so `*強調*` in a Markdown chapter came back as emphasis in a `:syntax
    /// text` manuscript that happened to hold the same words.
    markup_cache: RefCell<MarkupCache>,
    /// The block of every line, against the buffer it was worked out for and
    /// that buffer's revision.
    block_cache: RefCell<Option<BlockCache>>,
    /// The drawn padding of the table last asked about (Feature #212).
    ///
    /// One table at a time: the page asks per line, every line of a table
    /// needs the widths of all the others, and a document has at most a
    /// screenful of table on it at once.
    pad_cache: RefCell<Option<(PadKey, Vec<Vec<(usize, String)>>)>>,
    /// Which `|` table the cursor is in, against the buffer, its revision and
    /// the line the answer was worked out for.
    ///
    /// The region is asked for several times a frame — the hint row, the
    /// status line, and every key that has to know whether the grid's rules
    /// apply here. Walking out from the cursor is cheap for a table of ten
    /// rows and is not cheap for a table of ten thousand, and the answer is
    /// the same all three times.
    md_cache: RefCell<Option<MdCache>>,
    /// Every `|` table in the file, against the buffer and its revision.
    ///
    /// **Per document, not per line** (#275). `table_lines_at` is asked once
    /// per visible row per frame — five or six times, through
    /// `wrap::Measure::with_unwrapped` — and walking out from that row costs
    /// the length of the table it lands in: a 20 000-row table took 1.9
    /// seconds a frame, and 500 rows 55 ms, which is every keystroke. Walking
    /// the file once per edit is O(file) and answers every row of the frame.
    md_tables: RefCell<Option<MdTables>>,
    /// Whether Markdown is coloured at all (Feature #96).
    /// 所見即所得 (Feature #104): the markup comes off the page, except on the
    /// construct the cursor is in.
    /// **The inline candidate** (Feature #211), as `(line, column, what is
    /// drawn there)` — the one piece of [virtual text](crate::drawn) the core
    /// is *told* rather than works out.
    ///
    /// It comes from outside: the input method is offering it and the core has
    /// no way to know. Everything else on the page that the file has no bytes
    /// for is derived here — a table's padding (#212), an inline note (#248) —
    /// and all of it goes out through [`Editor::drawn_on_line`], so that
    /// everything which asks where a character is (the wrap, the caret, `j`,
    /// the mouse) asks about the same page.
    candidate: Vec<(usize, usize, String)>,
    /// The command-line completion in progress: the prefix Tab started from, and
    /// which match is selected. The prefix is kept because the typed text is
    /// replaced by each candidate in turn, so the line itself can no longer say
    /// what was being completed.
    completion: Option<(String, usize)>,
    /// Which spellings of a reading **count as one** (Feature #65) — what the
    /// word count subtracts, what `:ruby` edits, what `:ruby auto` writes.
    ///
    /// Separate from [`Editor::ruby_drawn`] since #283, because the middle
    /// level needs both answers at once: a reading is *known* at 中階 and it is
    /// not *drawn*, so the tags stay on the page and the count is still of the
    /// text a reader sees. One field could only say one of those.
    ruby: Dialects,
    /// Whether a known reading is laid out beside its base — 全 only.
    ///
    /// Drawing it means taking the tags off the page, and that is replacing,
    /// which 中階 does not do.
    ruby_drawn: bool,
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
    /// Whether every file opened from here on is locked (Feature #213).
    ///
    /// What `--readonly` sets. Kept on the editor rather than handed to each
    /// buffer at birth because `:open` opens buffers too, and a session started
    /// to *read* a directory of chapters should not go writable at the second
    /// file.
    readonly_default: bool,
    /// Whether a clean buffer re-reads itself when the file changes on disk
    /// (Feature #214). `:reload auto on`.
    reload_auto: bool,
    /// When the disk was last asked about it, so a held-down `j` does not
    /// `stat` the file a thousand times.
    last_disk_check: Option<std::time::Instant>,
    /// Whether the writer has already been told that the file underneath them
    /// changed and their own edits are in the way — said once per change, not
    /// once per keystroke.
    reload_warned: bool,
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
    /// The reader's own 用字 groups, from `[editor] usage_groups` (#233).
    ///
    /// The built-in table cannot hold a novel's own names, and a novel's own
    /// names are what a manuscript slips on: 阿嬌 in chapter two and 阿姣 in
    /// chapter nineteen is invisible to every checker there is.
    usage_groups: Vec<String>,
    /// The last `:grep`: its pattern and the files it hit.
    ///
    /// What `:replace` acts on — so a project-wide change can only be made to
    /// something the writer has **already looked at**.
    grep_found: Option<(String, Vec<PathBuf>)>,
    /// 字 each open file held when this session opened it (Feature #244).
    ///
    /// What a day's writing is counted *from* when the log has no row for
    /// today yet: a chapter first saved at four in the afternoon did not have
    /// all of its 字 written since four. Keyed by path, because that is what a
    /// row of the log names, and a buffer's index moves.
    opened_with: HashMap<PathBuf, usize>,
    /// Seconds east of UTC, asked of the system once and kept (Feature #244).
    time_offset: Option<i64>,
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

/// What a write actually did — the three are different things, and `:wq` in
/// particular has to be able to tell「the chapter is on disk」from「a copy of it
/// is somewhere else」.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Wrote {
    /// The buffer's own file is on disk.
    Saved,
    /// A copy went elsewhere; the buffer is still where it was, still modified.
    Copied(PathBuf),
}

/// A question the editor has stopped to ask before doing something it cannot
/// take back, and the answers it will take (Feature #295).
///
/// **The editor is stopped while one stands**: every key goes to the answer
/// until one of the choices is taken, so there is no state in which the
/// question is on the screen and the keys are still editing the manuscript
/// behind it. That is the whole safety of it — a modal question that can be
/// typed past is a status line with a border.
///
/// The body says **numbers**, never 「幅度較大」: what is being weighed is how
/// much bigger the file gets, and an adjective is exactly the part the writer
/// cannot check. One question at a time; there is no queue, because a second
/// question would be about a command the first one has not answered yet.
pub struct Query {
    /// The name in the panel's top-left corner.
    pub title: String,
    /// What is being asked, in numbers.
    pub body: String,
    /// The answers, in the order they are drawn.
    pub choices: Vec<Answer>,
    /// What the question is about, and so what each answer does.
    what: Asking,
}

/// One answer to a [`Query`]: the key that takes it, and what it says.
pub struct Answer {
    /// The key, lowercase. `Esc` is always the last choice as well.
    pub key: char,
    /// What it does, as the panel draws it.
    pub label: String,
}

/// What a [`Query`] is about.
///
/// One arm today. It is an enum and not a `bool` because the interface is the
/// point: the next thing that needs to stop and ask adds an arm and a match
/// branch, and inherits the panel, the key routing and the 「Esc is no」 rule
/// without touching any of them.
enum Asking {
    /// `:write` about to make the file on disk very much bigger.
    OversizeWrite {
        /// The path `:write` was given, if it was given one.
        path: Option<String>,
    },
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
                write!(f, "{}", say!("buffer.unsaved-changes"))
            }
            // A dead end otherwise: this is the first thing a new file says
            // back, and 「沒有檔名」 alone does not tell you how to give it one.
            EditorError::NoFileName => {
                write!(f, "{}", say!("buffer.no-name-yet"))
            }
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
            lookfor_focus: 0,
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
            reader: Box::new(NoReader),
            project_words: std::rc::Rc::new(RefCell::new(yumete_cjk::WordList::default())),
            show_segmentation: false,
            word_mark: yumete_cjk::WordMark::default(),
            hud: Hud::default(),
            word_level: yumete_cjk::WordLevel::default(),
            words_request: false,
            table_rules: crate::table::Rules::default(),
            table_numbers: true,
            detail_width: None,
            typewriter: false,
            focus: false,
            wheel_step: 3,
            ime_available: false,
            definition_preview: false,
            screenshot_request: None,
            dictionary_query: None,
            dictionary: None,
            other: None,
            live_pane: 0,
            number_fill: false,
            indent_hint: crate::zong::IndentHint::default(),
            indent_symbol: "↵".to_string(),
            count: None,
            edit_keys: Vec::new(),
            edit_revision: (0, 0),
            last_edit_keys: Vec::new(),
            last_replacement: String::new(),
            marks: HashMap::new(),
            repeating_edit: false,
            insert_recording: String::new(),
            last_find: None,
            indent_width: 4,
            chaifen_request: None,
            scheme_request: None,
            theme_request: None,
            clipboard_request: None,
            clipboard_read: None,
            render: Render::Basic,
            count_to: None,
            column_span: None,
            sequence: None,
            sort_keys: Vec::new(),
            jumped: false,
            language_run: None,
            preview_request: None,
            open_request: None,
            preview_at: None,
            shell_request: None,
            convert_patch: None,
            table: None,
            table_level: TableLevel::default(),
            show_detail: true,
            table_bypass: std::cell::Cell::new(false),
            drafts_dir: None,
            data_dir: None,
            session_file: None,
            turned_for_table: None,
            zong_gap: None,
            dense: false,
            sentences: false,
            loose_rows: false,
            indent: 0,
            indent_folds: true,
            cell_folds: None,
            crossed_tables: false,
            bands: 1,
            hits: None,
            jumps: Vec::new(),
            jump_at: 0,
            key_index: RefCell::new(None),
            chaifen: false,
            ruby_target: None,
            completion: None,
            tatechuyoko: false,
            hanging: false,
            paper: crate::export::Paper::A5,
            viewing: Cell::new(None),
            query: None,
            segment_cache: RefCell::new(SegmentCache::new()),
            meter: false,
            meter_cache: RefCell::new(MeterCache::new()),
            notes: false,
            note_cache: RefCell::new(NoteCache::new()),
            fold_cache: RefCell::new(None),
            markup_cache: RefCell::new(HashMap::new()),
            block_cache: RefCell::new(None),
            pad_cache: RefCell::new(None),
            md_cache: RefCell::new(None),
            md_tables: RefCell::new(None),
            candidate: Vec::new(),
            ruby: Dialects::only(crate::ruby::Dialect::Html),
            ruby_drawn: true,
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
            readonly_default: false,
            reload_auto: false,
            last_disk_check: None,
            reload_warned: false,
            compiled: RefCell::new(None),
            picker: None,
            sidebar: None,
            sidebar_focus: false,
            default_syntax: None,
            syntax_by_name: HashMap::new(),
            grep_root: None,
            usage_groups: Vec::new(),
            grep_found: None,
            opened_with: HashMap::new(),
            time_offset: None,
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
                out.extend_from_slice(group.base_text(&chars));
                at = group.end;
            }
            out.extend_from_slice(&chars[at..]);
        }
        out
    }

    /// 字 in `text` — the publisher's count, ruby markup reduced to its base.
    /// The same rule [`Editor::count_report`] answers with, so 進度 and `:count`
    /// can never disagree about how long a chapter is.
    fn han_in(&self, text: &str) -> usize {
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

    /// This book's log as it stands on disk. Missing is empty, not an error.
    fn progress_log(&self, path: &Path) -> crate::progress::Log {
        std::fs::read_to_string(path)
            .map(|text| crate::progress::Log::from_text(&text))
            .unwrap_or_default()
    }

    /// Write the log back, answering with what went wrong.
    fn write_progress_log(&self, path: &Path, log: &crate::progress::Log) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, log.to_text())
    }

    /// Record what this file holds now, after a save (Feature #244).
    ///
    /// **It is silent, and it makes nothing.** A save must not fail, or even
    /// say anything, because a progress log could not be written — and a
    /// manuscript is not the only thing an editor saves. A ledger appears when
    /// the writer asks for one (`:count target`, `:count progress`), never because a
    /// config file was edited in a directory that had never heard of yumete;
    /// after that every save keeps it up to date.
    fn note_progress(&mut self) {
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

    /// `:count progress` — 寫作進度: today against the target, and every day before.
    fn progress_report(&mut self) {
        let Some(path) = self.progress_path() else {
            self.status = say!("progress.no-file");
            return;
        };
        let mut log = self.progress_log(&path);
        // Asking is enough to open the ledger: `:count progress` on a book that has
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
        // reader: `:count progress` opens a listing, and a second one read from
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

    /// `:count target <字>` — how many 字 a day, or `off`.
    fn set_target(&mut self, target: Option<usize>) {
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
    fn caret(&self) -> usize {
        self.viewing.get().map_or(self.cursor, |v| v.at)
    }

    /// The other end of the selection, by the same rule. A peeked pane has no
    /// selection of its own — what it marks is the hit it was opened to show —
    /// so both ends are its caret and [`Self::has_selection`] is false there.
    fn mark(&self) -> usize {
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
    fn forget_the_document(&mut self) {
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
    fn forget_the_text(&mut self) {
        self.segment_cache.borrow_mut().clear();
        self.markup_cache.borrow_mut().clear();
        *self.md_cache.borrow_mut() = None;
        *self.md_tables.borrow_mut() = None;
        *self.block_cache.borrow_mut() = None;
        *self.fold_cache.borrow_mut() = None;
        *self.key_index.borrow_mut() = None;
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
            self.status = say!("find.grep-no-hits", files, pattern);
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
            say!("find.grep-too-many", found)
        } else {
            say!("find.grep-hits", found, files)
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
    /// and `:write all` is the moment a person says yes. Writing 120 files from a
    /// command line with no undo is the kind of thing an editor should not
    /// make easy.
    fn replace_found(&mut self, text: &str, reshape: bool) {
        let Some((pattern, files)) = self.grep_found.clone() else {
            self.status = say!("find.replace-needs-grep");
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
        let mut refused: Vec<String> = Vec::new();
        for path in &files {
            if self.open_file(path).is_err() {
                continue;
            }
            if self.current_buffer().is_readonly() {
                refused.push(say!("readonly.replace-refused", self.buffer_name()));
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
            // **The same check `:s` makes.** A whole-buffer rewrite lifts the
            // cell guard, so the one thing table mode promises — a row's
            // delimiter count never changes — was checked for `:s` and not for
            // the command that reaches *every file in the project*.
            if let Some(why) = (!reshape)
                .then(|| self.substitution_breaks_the_grid(&rebuilt))
                .flatten()
            {
                refused.push(say!("find.file-and-message", self.buffer_name(), why));
                continue;
            }
            self.snapshot();
            let len = self.current_buffer().char_count();
            let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(0..len, &rebuilt));
            if done.is_err() {
                refused.push(say!("readonly.replace-refused", self.buffer_name()));
                continue;
            }
            self.clamp_cursor();
            self.anchor = self.cursor;
            hits += here;
            changed += 1;
        }
        self.current = was.min(self.buffers.len().saturating_sub(1));
        self.set_cursor(self.current_buffer().saved_cursor());
        if changed == 0 {
            self.status = match refused.is_empty() {
                true => say!("find.replace-nothing-replaced", pattern),
                false => listed(&refused),
            };
            return;
        }
        self.status = match refused.is_empty() {
            true => say!("find.replaced-across-files", changed, hits),
            // The files it would have broken are named: a rename across a book
            // that quietly skipped the 拆分表 would be worse than one that
            // says which files it did not touch.
            false => say!(
                "find.files-hits-and-message",
                changed,
                hits,
                listed(&refused)
            ),
        };
    }

    /// Save every buffer that has changed (`:write all`).
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
        let full = match (&self.grep_root, path.is_absolute()) {
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
    fn follow_link(&mut self) {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor);
        let at = self.cursor - rope.line_to_char(line);
        let text = rope.line(line).to_string();
        let text = text.trim_end_matches(['\n', '\r']);
        // Only Markdown writes links this way; Typst spells them `#link(…)`,
        // which is code and is read as code.
        let found = match self.current_buffer().syntax() {
            crate::syntax::Syntax::Markdown => crate::markdown::link_at(text, at),
            _ => None,
        };
        let Some(link) = found else {
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
    fn buffer_holding(&self, target: &Path) -> Option<usize> {
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
    fn export(
        &mut self,
        format: &str,
        path: Option<&str>,
        force: bool,
    ) -> Result<CommandOutcome, EditorError> {
        // **`csv` is the one export that is a region, not the document.** A
        // manuscript has no rows; the table under the cursor does. So it is
        // answered here, from the same machinery `:table csv` uses, rather than
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
    fn take_a_picture(
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
    /// The buffer is not touched: this is the difference between `:table csv`,
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
                let mut bad = None;
                for (r, source) in text.lines().enumerate() {
                    let mut row = Vec::new();
                    for (c, span) in crate::table::cells(source, from).into_iter().enumerate() {
                        let cell = crate::table::cell_text(source, span);
                        if from != delimiter && cell.contains(delimiter) {
                            bad = bad.or(Some((r, c)));
                        }
                        row.push(cell);
                    }
                    out.push(row.join(&delimiter.to_string()));
                }
                match bad {
                    Some(at) => Err(at),
                    None => Ok(out),
                }
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
        self.render = how;
        // **The link is made here, once** (#283). Every other dimension is
        // *assigned* the matching level and then left alone: nothing anywhere
        // else re-derives 「is a table drawn」 from 「is markup shown」, which
        // is what kept growing holes. An override afterwards stands until the
        // next `:render`, and there is nothing to un-pin because there is
        // nothing pinned.
        self.set_ruby_level(how);
        self.table_level = match how {
            Render::Off => TableLevel::Off,
            Render::Basic => TableLevel::Basic,
            Render::Full => TableLevel::Full,
        };
        if self.table_level == TableLevel::Off {
            self.leave_table_quietly();
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
        // **One line's answer is one lookup.** This used to go through
        // `blocks_through`, which hands back a copy of every line above it —
        // so a question the 縱書 page asks per line cost 11 ns near the top of
        // 資治通鑑 and 6.9 µs at line 19,883, and a page anchored down there
        // spent 364 µs a frame copying blocks nobody looked at.
        if !self.markup_visible() {
            return crate::markdown::Block::Prose;
        }
        self.scan_blocks();
        let key = (
            self.current_buffer().id(),
            self.current_buffer().revision(),
        );
        match self.block_cache.borrow().as_ref() {
            Some((cached, blocks, _)) if *cached == key => {
                blocks.get(line).copied().unwrap_or_default()
            }
            _ => crate::markdown::Block::default(),
        }
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
        // By **id**, never by index: closing a buffer shifts every later one
        // down, and a fresh buffer opens at revision 0 — so an index-keyed
        // entry could be handed to a different document that happens to sit
        // where the old one did, and answer for it.
        self.scan_blocks();
        let key = (buffer.id(), buffer.revision());
        if let Some((cached, blocks, _)) = self.block_cache.borrow().as_ref() {
            if *cached == key {
                return blocks[..=last.min(blocks.len() - 1)].to_vec();
            }
        }
        vec![crate::markdown::Block::Prose; last + 1]
    }

    /// Read every line's block, once per edit, into the cache.
    ///
    /// Blocks are the part of Markdown that is *not* line-local — a fence
    /// opened three paragraphs ago decides whether this line is code — so the
    /// scan is from the top, and the answer only changes when the text does.
    ///
    /// By **id**, never by index: closing a buffer shifts every later one down,
    /// and a fresh buffer opens at revision 0 — so an index-keyed entry could
    /// be handed to a different document that happens to sit where the old one
    /// did, and answer for it.
    fn scan_blocks(&self) {
        let buffer = self.current_buffer();
        let rope = buffer.rope();
        let lines = rope.len_lines();
        let key = (buffer.id(), buffer.revision());
        if let Some((cached, _, _)) = self.block_cache.borrow().as_ref() {
            if *cached == key {
                return;
            }
        }
        let typst = buffer.syntax() == crate::syntax::Syntax::Typst;
        let mut markdown = crate::markdown::BlockScanner::new();
        let mut typst_scanner = crate::markdown::typst::BlockScanner::new();
        let mut blocks = Vec::with_capacity(lines);
        // The four markers are settled by the same opening characters, so a
        // merge conflict costs this walk nothing but the rare line it finds.
        let mut marks = Vec::new();
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
            if let Some(kind) = crate::conflict::marker(&prefix) {
                marks.push((line, kind, crate::conflict::label(&prefix)));
            }
            blocks.push(if typst {
                typst_scanner.feed(&prefix, end - start)
            } else {
                markdown.feed(&prefix, end - start)
            });
        }
        // **Laid over the answer, not woven into it.** A forward scan cannot
        // know whether a `<<<<<<<` ever closes, and a paragraph *about* merges
        // must not turn the rest of the chapter into somebody's side of an
        // argument. So the conflicts are assembled first — which throws the
        // unclosed ones away — and only then do their lines take the label.
        let conflicts = crate::conflict::assemble(marks);
        for found in &conflicts {
            for line in found.lines() {
                blocks[line] = crate::markdown::Block::Conflict(found.side_of(line));
            }
        }
        *self.block_cache.borrow_mut() = Some((key, blocks, conflicts));
    }

    /// Every merge conflict in this buffer, in the order they are written
    /// (Feature #249).
    ///
    /// Not gated on whether the markup is drawn: `:render off` says how the
    /// page is *coloured*, and `]c` is a motion. A file with seven angle
    /// brackets in it is in a state the writer needs to get out of either way.
    pub fn conflicts(&self) -> Vec<crate::conflict::Conflict> {
        self.scan_blocks();
        let key = (
            self.current_buffer().id(),
            self.current_buffer().revision(),
        );
        match self.block_cache.borrow().as_ref() {
            Some((cached, _, found)) if *cached == key => found.clone(),
            _ => Vec::new(),
        }
    }

    /// The conflict `line` stands in, markers included.
    pub fn conflict_at(&self, line: usize) -> Option<crate::conflict::Conflict> {
        self.conflicts()
            .into_iter()
            .find(|c| c.lines().contains(&line))
    }

    /// `:check merge` — every conflict in this file, as a buffer to walk (#249).
    ///
    /// The same `路徑:行:` shape `:grep` writes, so `gf` follows a row back to
    /// the line it names and every motion works in the list. **This file
    /// only**: a merge conflict is a state a file is in, and the file the
    /// writer is looking at is the one they are about to resolve.
    fn list_conflicts(&mut self) {
        let found = self.conflicts();
        if found.is_empty() {
            self.status = say!("conflict.none");
            return;
        }
        let shown = self
            .current_buffer()
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| self.current_buffer().display_name());
        let mut listing = String::new();
        for c in &found {
            // Wordless on purpose: the two labels are the branches git wrote
            // into the file, and they say more than any sentence here could.
            listing.push_str(&format!(
                "{shown}:{}: {} ⇄ {}\n",
                c.head + 1,
                c.ours,
                c.theirs
            ));
        }
        let count = found.len();
        let mut buffer = Buffer::from_text(&listing);
        buffer.name_as("[conflicts]");
        self.add_buffer(buffer);
        self.set_cursor(0);
        self.status = say!("conflict.some", count);
    }

    /// Whether the markup is taken off the page (所見即所得).
    pub fn wysiwyg(&self) -> bool {
        self.render == Render::Full
    }

    /// Lay readings out at the level `:render` was just set to (#283).
    ///
    /// A reading is markup like any other — though only the vertical page can
    /// show one, since that is the only layout with a column to put it in.
    ///
    /// **Assigned, not stashed.** This used to put the reader's dialects aside
    /// on the way into 所見即所得 and hand them back on the way out, and the
    /// stash went stale the moment `:ruby` was typed while 所見即所得 was on:
    /// leaving it then gave back a set the reader had already replaced. With
    /// the level assigned outright there is nothing to go stale, and `:ruby`
    /// afterwards is an override that stands until the next `:render`.
    fn set_ruby_level(&mut self, how: Render) {
        // 源碼模式: the tags are text, and nothing reads them — a word count
        // counts what is written, because that is what is on the page.
        self.ruby = match how {
            Render::Off => Dialects::NONE,
            Render::Basic | Render::Full => {
                let mut all = Dialects::NONE;
                for dialect in crate::ruby::Dialect::ALL {
                    all.insert(dialect);
                }
                all
            }
        };
        // **中階 knows the reading and does not draw it** (settled 2026-09-06).
        // Laying the reading out beside the base means taking the tags off the
        // page — 「正文不许摘 ruby 标签」 — and taking something off the page is
        // 全's business. So 中階 keeps both: the tags where the writer typed
        // them, and a word count that knows 「錢塘」 is two 字 and `qián táng`
        // is none.
        //
        // This is the level that used to lie. `:render basic` set every dialect
        // *and* drew them, so the tags came off while the status line said
        // 「標記留在畫面上」.
        self.ruby_drawn = how == Render::Full;
    }

    /// Which of the three levels the reading dimension is on (#283).
    ///
    /// **Computed, not stored.** The state is the pair 「is a reading known」
    /// and 「is it drawn」; a fourth field naming the sum of those could only
    /// go stale, and every dimension in #283 exists to stop exactly that.
    pub fn ruby_level(&self) -> Render {
        match (self.ruby.is_empty(), self.ruby_drawn) {
            (true, _) => Render::Off,
            (false, false) => Render::Basic,
            (false, true) => Render::Full,
        }
    }

    /// `:indent off|basic|full` — how much of a paragraph's opening is drawn.
    ///
    /// **The three words, and not `:render`'s to write** (settled 2026-09-06).
    /// The other three dimensions are all one question — how much of the
    /// *markup* is resolved — and a master switch over them is a switch over
    /// one idea. An indent is not markup: it is how a Chinese paragraph opens,
    /// it belongs to 縱書, and Markdown is read across. So `:render` writes
    /// three and this one stands on its own, saying the same three words.
    ///
    /// [`DEFAULT_INDENT`] is what a level that draws comes to when the reader
    /// has not named a width; `:indent <數字>` is the width and leaves the
    /// level alone, because those are two questions.
    fn set_indent_level(&mut self, how: Render) {
        self.indent = match how {
            Render::Off => 0,
            _ if self.indent > 0 => self.indent,
            _ => DEFAULT_INDENT,
        };
        self.indent_folds = how == Render::Full;
    }

    /// Which of the three levels the paragraph dimension is on (#283).
    pub fn indent_level(&self) -> Render {
        match (self.indent == 0, self.indent_folds) {
            (true, _) => Render::Off,
            (false, false) => Render::Basic,
            (false, true) => Render::Full,
        }
    }

    /// The markup to take off `line`, as char ranges within it.
    ///
    /// Empty unless 所見即所得 is on. The construct the cursor is in is never
    /// hidden, so the cursor is never inside text that is not on the screen —
    /// which is what makes every motion and every edit act on what can be seen.
    pub fn hidden_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        let mut off = self.markup_off_line(line);
        // **A folded cell tail is off the page by the same door** (#283), and
        // it has to be: the width the padding squares up, the columns the wrap
        // counts and the cell the mouse lands in all read this one list. Give
        // the fold its own channel and the three would disagree — the page
        // would draw a short cell and the caret would walk a long one.
        //
        // The markup is handed on rather than asked for again: this is called
        // once per line of every frame, and a fold is measured against exactly
        // the list that was just built.
        let folded = self.cell_folds_against(line, &off);
        off.extend(folded);
        off.sort_unstable();
        off
    }

    /// [`Self::hidden_on_line`] without the folds — the markup alone.
    ///
    /// Separate because a fold is *measured* against this: how wide a cell is
    /// drawn is how wide it is with its markup already off.
    fn markup_off_line(&self, line: usize) -> Vec<(usize, usize)> {
        // A reading that is being *laid out* is drawn beside the base, so its
        // markup comes off the page whatever `:render` says — leaving the tags
        // on would be showing the same reading twice. This is what the 縱書
        // page has always done; the horizontal one now does it too.
        let mut off: Vec<(usize, usize)> = self
            .readings_on_line(line)
            .into_iter()
            .flat_map(|g| [(g.start, g.base.0), (g.base.1, g.end)])
            .filter(|(a, b)| b > a)
            .collect();
        if self.wysiwyg() {
            let block = self.block_of(line);
            // **A conflict marker is markup too** (#249): seven brackets and
            // the space after them come off, exactly as a heading's hashes do,
            // and what is left is the one part a reader wants — whose side this
            // is. `=======` has no label, so its row goes empty, which is what
            // a divider between two halves should look like.
            if block == crate::markdown::Block::Conflict(None) {
                let text = self.current_buffer().rope().line(line).to_string();
                let brackets = text.chars().take(7).count();
                let take = brackets + usize::from(text.chars().nth(7) == Some(' '));
                off.push((0, take));
            }
            // Inside a fence nothing is markup, so nothing comes off.
            let spans = self.markup_line_in(line, block);
            off.extend(crate::markdown::hidden(&spans, self.selected_columns(line)));
            off.sort_unstable();
        }
        off
    }

    /// **Everything drawn on `line` that the file has no bytes for** (#248),
    /// in the order it is drawn.
    ///
    /// The mirror of [`Self::hidden_on_line`], and the general form of #210's
    /// drawn text. Three producers stand behind it and more can: the inline
    /// candidate the writer typed, the padding that squares a table up, and
    /// the notes `:view punct` puts beside a mark that is wrong. Each run is
    /// anchored *before* one of the file's own characters and none of them is
    /// addressable — see [`crate::drawn`] for the invariant that makes that
    /// safe.
    pub fn drawn_runs_on_line(&self, line: usize) -> Vec<crate::drawn::Run> {
        use crate::drawn::{Ink, Run};
        let mut runs: Vec<Run> = self
            .typed_on_line(line)
            .into_iter()
            .map(|(at, text)| Run::new(at, text, Ink::Typed))
            .collect();
        runs.extend(
            self.table_padding_on_line(line)
                .into_iter()
                .map(|(at, text)| Run::new(at, text, Ink::Padding)),
        );
        runs.extend(self.fold_marks_on_line(line));
        runs.extend(self.notes_on_line(line));
        crate::drawn::compose(runs)
    }

    /// The same page as one answer per anchor — what the wrap, the caret and
    /// the click map ask.
    ///
    /// Ordered by column, so the renderer, the wrap and the mouse walk it the
    /// same way.
    pub fn drawn_on_line(&self, line: usize) -> Vec<(usize, String)> {
        crate::drawn::flat(&self.drawn_runs_on_line(line))
    }

    /// The part of what is drawn on `line` that the writer **typed**: the
    /// inline candidate, and nothing derived.
    ///
    /// The caret's own page. A run is drawn before the character it is
    /// anchored at, and a caret resting on that character stands after the
    /// candidate — you typed it — but *before* the padding that reaches from
    /// the same anchor to the pipe, and before a note about the mark there.
    /// Told apart here so [`crate::wrap`] can put the caret between them;
    /// everything else wants them as one page and asks [`Self::drawn_on_line`].
    pub fn typed_on_line(&self, line: usize) -> Vec<(usize, String)> {
        self.candidate
            .iter()
            .filter(|&&(l, _, _)| l == line)
            .map(|(_, at, text)| (*at, text.clone()))
            .collect()
    }

    /// Whether the marks that are wrong are named on the page (#248).
    pub fn notes(&self) -> bool {
        self.notes
    }

    /// The notes drawn on `line`: what each mark there should have been.
    ///
    /// **Only what one line can answer for.** A half-width `,` among 漢字 and
    /// an English `...` are wrong wherever they stand; an unclosed 「 may be
    /// perfectly correct — Chinese typesetting opens it again at the head of
    /// each paragraph of a long quotation — and no line can see that on its
    /// own. Those stay with `:check punct`, which reads the whole manuscript.
    /// See [`crate::punct::check_line`].
    ///
    /// **Nothing inside a fence**, where a `,` is code and right; and nothing
    /// while `:render off` asks for the file exactly as it is, which is the
    /// same rule #212's padding keeps.
    ///
    /// Kept against a hash of the line's own text, like the 平仄 beside it: a
    /// note is worked out from that line and nothing else, so it can never go
    /// stale the way a finding stored from a walk of the file does.
    fn notes_on_line(&self, line: usize) -> Vec<crate::drawn::Run> {
        use crate::drawn::{Ink, Run};
        if !self.notes || !self.markup_visible() {
            return Vec::new();
        }
        if self.block_of(line).is_literal() {
            return Vec::new();
        }
        let Some(text) = self.line_text(line) else {
            return Vec::new();
        };
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        let mut cache = self.note_cache.borrow_mut();
        if let Some((cached, runs)) = cache.get(&line) {
            if *cached == hash {
                return runs.clone();
            }
        }
        // The note stands **after** the mark it is about, so the page reads
        // 「what you wrote, then what it should be」 — `他說,，` — and the mark
        // itself keeps the column the cursor goes to.
        let runs: Vec<Run> = crate::punct::check_line(&text)
            .into_iter()
            .map(|slip| {
                let after = slip.column + slip.written.chars().count();
                Run::new(after, slip.wanted, Ink::Note)
            })
            .collect();
        if cache.len() >= SEGMENT_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(line, (hash, runs.clone()));
        runs
    }

    /// Whether `|` tables are squared up as the page draws them (Feature #212).
    ///
    /// **Horizontal only.** Down a 縱 a row is one column and every character
    /// takes one cell of it, wide or narrow — so padding measured in display
    /// width, which is what squares a table up across a page, aligns nothing
    /// there.
    ///
    /// **One field, asked once** (#283). It used to ask `markup_visible()` —
    /// a question about `:render`, answered by reaching across into another
    /// subsystem's state, and every proposal to extend it grew a new hole:
    /// the `layout` term dropped, `Reach::Cursor` silently widening to the
    /// file, `PadKey` going stale across a mode switch that moves neither
    /// revision nor render. The link is made where the command runs instead —
    /// `:render off` **assigns** `TableLevel::Off` — so nothing here has to
    /// re-derive it, and nothing here can get it wrong.
    ///
    /// Whether or not `:table` was typed, though: a table in a manuscript is a
    /// table because of what it is, and the writer who most needs to see one
    /// squared up is the one editing their own documentation. That is why the
    /// question is the *level* and not [`Editor::table`] — the level is the
    /// file's, and holds in the paragraph between two tables.
    fn table_padding_on(&self) -> bool {
        self.layout == Layout::Horizontal && self.table_level != TableLevel::Off
    }

    /// `t w` — fold the over-wide cells away, or give them back (#283).
    ///
    /// **It asks for a table that has been squared up, not for 全.** The law
    /// 「`basic` 不藏、不摺、不替換」 says what a *level* does on its own, and
    /// `t w` is the reader's own key — asking for it at 基本 is not the editor
    /// hiding anything behind anybody's back (author, 2026-09-07: 「虽然 tb 在
    /// 默认状态下不折叠，但能不能在按下 tw 之后折叠？」). What folding really
    /// needs is a column to fold *against*, and 基本 squares one up exactly as
    /// 全 does. Only 源碼 has none, and there the answer is still a refusal
    /// rather than a level quietly raised: a width key that also drew walls
    /// and a ruler would be a second way to change the level, and the reader
    /// could not tell which of the two they had asked for. The switch is
    /// still *set* — walk up a level and the answer is the one asked for.
    ///
    /// **The pane is not below 全 — it is beside it.** `t t` draws its own
    /// grid at its own cap, so the switch means there exactly what it means
    /// in prose, and refusing it there left over-wide cells
    /// that could not be opened by any key at all (author, 2026-09-07:
    /// 「tw 功能无法在 tt 模式下使用……长单元格被折叠的信息永远无法读取」).
    fn toggle_cell_folds(&mut self) {
        // **Two toggles over one axis, and they cannot both be on** (author,
        // 2026-09-07). A three-way cycle on `t w` was the other way to spell
        // this, and it would have made the same key mean a toggle in prose
        // and a cycle in the window; a reader learns 「`t w` 摺不摺」 once and
        // it has to hold everywhere. So `t w` answers 摺／不摺, and either
        // key pulls the table out of whatever the other one had done.
        //
        // From 折行, `t w` **opens the table out** rather than folding it
        // (author, 2026-09-08). Both keys name a way of not showing a cell
        // whole, so the way back from either of them is the whole cell: from
        // 折行 the reader who presses the other key is asking to stop wrapping,
        // and answering with 摺起 hands them the one state they did not name.
        let want = match self.cell_width_now() {
            CellWidth::Fold | CellWidth::Wrap => CellWidth::Whole,
            CellWidth::Whole => CellWidth::Fold,
        };
        self.set_cell_width(want);
        let cap = crate::mdtable::MAX_COLUMN.to_string();
        self.status = match (want, self.folds_can_bite()) {
            (_, false) => say!("table.folds-need-a-drawn-table"),
            (CellWidth::Fold, _) => say!("table.folds-on", cap, crate::mdtable::FOLD_MARK),
            _ => say!("table.folds-off", cap),
        };
    }

    /// `t a` — 格內折行: the tail is drawn **under** the cell, in its own
    /// column, rather than taken off the page (author, 2026-09-07: 「把所有超
    /// 长的单元格都在单元格下方的空行中 soft wrap」).
    ///
    /// **The grid's answer, and only the grid's.** The prose page's rows come
    /// from [`crate::wrap`], which every motion, the mouse, 縱書 and the split
    /// panes read, and it has no hanging indent: a row wrapped there would
    /// carry on at the left margin with the rest of its cells trailing after
    /// the wrapped text, which is not what anybody means by 折行. So in prose
    /// the key says where it works — and still **sets the switch**, so the
    /// answer is waiting when the reader presses `t t`.
    fn toggle_cell_wrap(&mut self) {
        let want = match self.cell_width_now() {
            CellWidth::Wrap => CellWidth::Whole,
            CellWidth::Fold | CellWidth::Whole => CellWidth::Wrap,
        };
        self.set_cell_width(want);
        let pane = self.table.as_ref().is_some_and(|view| view.takes_the_pane());
        let cap = crate::mdtable::MAX_COLUMN.to_string();
        self.status = match (want, pane) {
            (CellWidth::Wrap, false) => say!("table.wrap-needs-the-window"),
            (CellWidth::Wrap, true) => say!("table.wrap-on", cap),
            (_, _) => say!("table.folds-off", cap),
        };
    }

    /// Write the reader's own answer to 「太寬的格子怎麼辦」 and forget what
    /// was drawn under the old one.
    fn set_cell_width(&mut self, want: CellWidth) {
        self.cell_folds = Some(want);
        self.pad_cache.borrow_mut().take();
    }

    /// Whether `t w` has anywhere to bite from where the reader is standing —
    /// a page that squares its tables up, or the grid the pane draws for
    /// itself. 源碼 is the one place with nothing to fold against.
    fn folds_can_bite(&self) -> bool {
        self.table_level != TableLevel::Off
            || self.table.as_ref().is_some_and(|view| view.takes_the_pane())
    }

    /// The answer that holds where the reader is standing — theirs if they
    /// have given one, and otherwise the place's own (#283).
    ///
    /// The two places have different defaults and both are right: the `t t`
    /// grid draws its own columns and caps them however it was opened, while
    /// the prose page folds only at 全, because below 全 hiding is something
    /// the level may not do unasked.
    fn cell_width_now(&self) -> CellWidth {
        let pane = self.table.as_ref().is_some_and(|view| view.takes_the_pane());
        self.cell_folds.unwrap_or(match pane || self.table_level == TableLevel::Full {
            true => CellWidth::Fold,
            false => CellWidth::Whole,
        })
    }

    /// Whether the cap bites where the reader is standing.
    ///
    /// **折行 folds too.** 「Wrap」 is 「fold, and draw the tail underneath」,
    /// and only the grid can draw the second half — so on the prose page, and
    /// everywhere else that asks this question, it is the cap that answers.
    fn folds_now(&self) -> bool {
        self.cell_width_now() != CellWidth::Whole
    }

    /// Whether over-wide cells are folded (#283) — the switch, with no
    /// question about *where* the table is drawn.
    ///
    /// The prose page asks [`Self::cells_fold_here`], which adds the terms
    /// that only prose has; the grid — which caps and scrolls by its own
    /// rules — asks this.
    pub fn cell_folds(&self) -> bool {
        self.folds_now()
    }

    /// Whether an over-wide cell is drawn **wrapped under itself** — `t a`.
    ///
    /// The grid asks; nothing else can answer it. See [`Self::toggle_cell_wrap`].
    pub fn cell_wrap(&self) -> bool {
        self.cell_width_now() == CellWidth::Wrap
    }

    /// Whether an over-wide cell has its tail folded away on this page (#283).
    ///
    /// Two terms, and each one is the law rather than a preference:
    ///
    /// * **Wherever the table is squared up.** A fold is measured in display
    ///   width against a squared-up column, so `table_padding_on` is the whole
    ///   of the question — 基本 squares up and 源碼 does not. 「`basic` 不藏、
    ///   不摺、不替換」 governs what the *level* does unasked; `t w` is asked.
    /// * **In prose only.** The pane draws its own grid, and folds it by
    ///   drawing its columns narrow rather than by hiding characters of a
    ///   line — [`Self::cell_folds`] is the switch it reads.
    ///
    /// [`Editor::cell_folds`] is the writer's switch over the top — `t w`.
    fn cells_fold_here(&self) -> bool {
        self.folds_now()
            && self.table_padding_on()
            && !self.table.as_ref().is_some_and(|view| view.pane)
    }

    /// The cell tails folded away on `line`, as char ranges within it (#283).
    ///
    /// Empty on the rule row: `---|:---:|---` is not writing, it is the shape
    /// of the table, and folding it would hide the alignment the row exists to
    /// declare.
    fn cell_folds_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        if !self.cells_fold_here() {
            return Vec::new();
        }
        let markup = self.markup_off_line(line);
        self.cell_tails_against(line, &markup, self.folds_open_at(line))
    }

    /// The tails as the **measure** reads them: every cell folded, the one
    /// being typed in included — see [`Self::cell_folds_measured`]. The mark
    /// is one cell of its column, and the column is measured closed, so the
    /// mark is counted on the open row too.
    fn cell_folds_measured_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        if !self.cells_fold_here() {
            return Vec::new();
        }
        let markup = self.markup_off_line(line);
        self.cell_tails_against(line, &markup, None)
    }

    /// Everything a row keeps off the page for the table's sake: the tails,
    /// and **the padding the file itself holds** with them.
    ///
    /// The two are one list here because one list is what the width, the wrap
    /// and the mouse all read — and they are two functions because only the
    /// tail leaves a mark. A `>` over the spaces between a cell and its pipe
    /// would say something was folded away there, and nothing was.
    fn cell_folds_against(&self, line: usize, markup: &[(usize, usize)]) -> Vec<(usize, usize)> {
        let open = self.folds_open_at(line);
        let mut out = self.cell_tails_against(line, markup, open);
        out.extend(self.cell_slack_against(line, markup, open));
        out.sort_unstable();
        out
    }

    /// The same list, **as the table is measured** rather than as it is drawn.
    ///
    /// The two differ on one row at most, and only while somebody is typing
    /// in it: the cell being edited has its tail back on the page, and a
    /// column that grew to hold it would swell and shrink under the reader's
    /// hands — every other row shifting sideways because one cell is open.
    /// So the column is measured as though every cell were folded, the open
    /// cell juts out past its own wall, and the table's geometry stops
    /// depending on the caret altogether. That is what makes the layout
    /// worth remembering across a keystroke.
    fn cell_folds_measured(&self, line: usize, markup: &[(usize, usize)]) -> Vec<(usize, usize)> {
        let mut out = self.cell_tails_against(line, markup, None);
        out.extend(self.cell_slack_against(line, markup, None));
        out.sort_unstable();
        out
    }

    /// [`Self::hidden_on_line`] as the **measure** reads it — see
    /// [`Self::cell_folds_measured`].
    fn hidden_measured_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        let mut off = self.markup_off_line(line);
        let folded = self.cell_folds_measured(line, &off);
        off.extend(folded);
        off.sort_unstable();
        off
    }

    /// The padding the file holds in this row, when folding is on.
    ///
    /// **Without this the cap buys nothing on a squared-up table.** The width
    /// a column is drawn to is the widest *box* in it, pipe to pipe, and a
    /// table that has been formatted in the file — `:table rules`, and every
    /// table in this project's own docs — pads every cell out to the column's
    /// natural width. Fold the writing to 32 and the spaces behind it still
    /// vote 43: the mark lands where the writing stopped and a field of empty
    /// cells follows it to the pipe. The file's padding is the page's to
    /// spend, so under `t f` it comes off — **down to the cap and no
    /// further**, which is what keeps #212's law (a drawn can only add) true
    /// everywhere the cap does not bite: a table that fits is drawn exactly
    /// as the file wrote it.
    fn cell_slack_against(
        &self,
        line: usize,
        markup: &[(usize, usize)],
        open: Option<(usize, usize)>,
    ) -> Vec<(usize, usize)> {
        if !self.cells_fold_here() {
            return Vec::new();
        }
        if !self.opens_a_row(line) || self.block_of(line).is_literal() {
            return Vec::new();
        }
        let Some(text) = self.line_text(line) else {
            return Vec::new();
        };
        crate::mdtable::slack(
            &text,
            markup,
            crate::mdtable::MAX_COLUMN,
            open,
        )
    }

    /// **Where the caret opens a folded cell — and it is not by standing in
    /// it** (#283, remade 2026-09-07).
    ///
    /// It used to be `selected_columns`: walk into a cell and its tail came
    /// back. That is one line of code and it cost the table its whole layout
    /// on **every** keystroke, because a caret in the key is a caret in the
    /// key whether or not the cell it moved to was ever folded — a `j` down
    /// the `#` column of this project's own `development.md` rebuilt 286 rows
    /// and took 15 ms, in a column two characters wide (author, 2026-09-07:
    /// 「不是说撑开的时候卡，而是不撑开的单元格也卡」).
    ///
    /// So reading and editing are told apart. **Reading** does not need the
    /// tail on the page — `t i`'s panel holds the whole cell, wrapped, which
    /// is what that panel is for — and in exchange the page's layout stops
    /// depending on where the caret is at all: it is worked out once per
    /// edit, and a cursor moving over a folded table costs nothing.
    /// **Editing** does need it, and there is no argument: the caret may
    /// never sit inside characters the page does not draw, or every motion,
    /// the mouse and the caret's own column disagree with what is on screen.
    /// So the cell opens on `i`/`a`/`c` and closes again on `Esc`.
    fn folds_open_at(&self, line: usize) -> Option<(usize, usize)> {
        match self.mode {
            Mode::Insert => self.selected_columns(line),
            _ => None,
        }
    }

    /// The cell tails folded away on `line`, with the markup already worked
    /// out — the spans that get a mark.
    fn cell_tails_against(
        &self,
        line: usize,
        markup: &[(usize, usize)],
        open: Option<(usize, usize)>,
    ) -> Vec<(usize, usize)> {
        if !self.cells_fold_here() {
            return Vec::new();
        }
        if !self.opens_a_row(line) || self.block_of(line).is_literal() {
            return Vec::new();
        }
        let Some(text) = self.line_text(line) else {
            return Vec::new();
        };
        if crate::mdtable::rule_of(&text).is_some() {
            return Vec::new();
        }
        crate::mdtable::folds(
            &text,
            markup,
            crate::mdtable::MAX_COLUMN,
            open,
        )
    }

    /// The fold marks drawn on `line` — one per cell whose tail came off.
    ///
    /// [`crate::drawn::Ink::Fold`], which is its own ink and not the note's:
    /// a note is *about* what is written and takes the markup's grey, and a
    /// grey `>` beside grey writing is a `>` the reader takes for the
    /// writer's own. It is anchored at the first character of the folded
    /// tail, so it stands exactly where the writing stopped.
    fn fold_marks_on_line(&self, line: usize) -> Vec<crate::drawn::Run> {
        use crate::drawn::{Ink, Run};
        self.cell_folds_on_line(line)
            .into_iter()
            .map(|(at, _)| Run::new(at, crate::mdtable::FOLD_MARK.to_string(), Ink::Fold))
            .collect()
    }

    /// The padding drawn on `line` so its table lines up (Feature #212).
    ///
    /// Empty unless the line really is a row of a `|` table — a quoted one
    /// inside a fence is writing *about* a table, and the whole editor already
    /// agrees about that.
    ///
    /// **The file is not touched.** [`crate::mdtable::format`] pads the source
    /// by display width, which is right for every other reader of it, and
    /// still leaves the page ragged: 所見即所得 takes `**` off one cell and
    /// `(url)` off another, a different number of columns from every row. So
    /// the padding that squares the *page* up is drawn rather than written,
    /// and an unformatted `|a|b|` lines up too, with the file left as it was
    /// typed.
    fn table_padding_on_line(&self, line: usize) -> Vec<(usize, String)> {
        if !self.table_padding_on() {
            return Vec::new();
        }
        if !self.opens_a_row(line) || self.block_of(line).is_literal() {
            return Vec::new();
        }
        let buffer = self.current_buffer();
        let key = |first, last| PadKey {
            buffer: buffer.id(),
            revision: buffer.revision(),
            first,
            last,
            // **Folding asks where the caret is too**, whatever `:render`
            // says: the cell it stands in is left whole, so the answer moves
            // when it moves.
            caret: (self.wysiwyg() || (self.cells_fold_here() && self.mode == Mode::Insert))
                .then(|| self.selection()),
            render: self.render,
            ruby: self.ruby(),
            syntax: buffer.syntax(),
            folds: self.cells_fold_here(),
        };
        // **The memo answers before the region is worked out.** Finding where
        // the table starts and ends is a walk to both ends of it, and this is
        // asked of every line the page touches, several times a frame: on the
        // 223-row table in this project's own `development.md` that walk alone
        // was 15 ms a frame. A line inside the remembered region needs no walk
        // — that is what the region *is*.
        if let Some((cached, runs)) = self.pad_cache.borrow().as_ref() {
            if cached.first <= line && line <= cached.last && *cached == key(cached.first, cached.last)
            {
                return runs.get(line - cached.first).cloned().unwrap_or_default();
            }
        }
        let Some(region) = crate::mdtable::region(|i| self.line_text(i), line) else {
            return Vec::new();
        };
        let key = key(region.first, region.last);
        // The whole table at once: every row's padding is decided by the
        // widest cell in each column, so there is no such thing as one row's
        // answer on its own.
        let rows: Vec<(String, Vec<(usize, usize)>)> = (region.first..=region.last)
            .map(|i| {
                (
                    self.line_text(i).unwrap_or_default(),
                    self.hidden_measured_on_line(i),
                )
            })
            .collect();
        // What the page really hides, which is the same list on every row but
        // the one being typed in — see [`Self::cell_folds_measured`].
        let shown: Vec<Vec<(usize, usize)>> = match self.mode == Mode::Insert {
            false => Vec::new(),
            true => (region.first..=region.last)
                .map(|i| self.hidden_on_line(i))
                .collect(),
        };
        // **The fold mark is one cell of its column.** It is drawn, not
        // written, so `visible_width` cannot see it — and a column padded as
        // though it were not there comes out one cell narrow on every row that
        // folds, which is every row the cap bites.
        let width = yumete_cjk::str_width(crate::mdtable::FOLD_MARK);
        let marks: Vec<Vec<(usize, usize)>> = (region.first..=region.last)
            .map(|i| {
                self.cell_folds_measured_on_line(i)
                    .into_iter()
                    .map(|(at, _)| (at, width))
                    .collect()
            })
            .collect();
        let runs = crate::mdtable::padding(
            &rows,
            region.rule.map(|at| at - region.first),
            &marks,
            &shown,
        );
        let answer = runs.get(line - region.first).cloned().unwrap_or_default();
        *self.pad_cache.borrow_mut() = Some((key, runs));
        answer
    }

    /// Whether an inline candidate is standing on the page.
    ///
    /// Only the candidate. It used to be "anything the file does not contain",
    /// which stopped being a useful question the day the padding that squares
    /// a table up became drawn too (#212): that padding is **derived**, it is
    /// on nearly every page of documentation, and no caller ever meant it.
    pub fn has_candidate(&self) -> bool {
        !self.candidate.is_empty()
    }

    /// Put `runs` on the page in place of whatever was there.
    ///
    /// Wholesale, never appended: the caller says what the page holds now, so
    /// a candidate that has been committed leaves nothing behind.
    pub fn set_candidate(&mut self, runs: Vec<(usize, usize, String)>) {
        self.candidate = runs;
    }

    /// The ruby groups on `line` that are being laid out as readings.
    ///
    /// Empty when no dialect is being rendered, which is also what makes the
    /// markup show as the text it is: nothing is hidden and nothing is drawn
    /// above it.
    pub fn readings_on_line(&self, line: usize) -> Vec<crate::ruby::Ruby> {
        let dialects = self.ruby();
        if dialects.is_empty() {
            return Vec::new();
        }
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return Vec::new();
        }
        let chars = crate::zong::line_chars(rope, line);
        crate::ruby::groups(&chars, dialects)
    }

    /// The 平仄 of `line`, for the margin (Feature #247).
    ///
    /// Empty unless `:view meter` is on **and** a reader is installed: without the
    /// 拆分表 there are no tones to read, and a margin of guesses beside a poem
    /// is worse than an empty one.
    ///
    /// The segmenter's own ranges, not [`Self::segment_line`]'s: that one hands
    /// back only the words a reader cannot already see the edges of, which is
    /// right for the overlay and wrong here — 「春眠」 on a line of its own is
    /// bounded on both sides and still has two tones. The answers are cached
    /// against a hash of the line, the way the overlay's are: a page is asked
    /// for every visible paragraph every frame, and a reading costs a walk
    /// through the 拆分表 per character.
    pub fn meter_on_line(&self, line: usize) -> Vec<crate::meter::Mark> {
        if !self.meter || !self.reader.available() {
            return Vec::new();
        }
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return Vec::new();
        }
        let chars = crate::zong::line_chars(rope, line);
        let text: String = chars.iter().collect();
        let mut hasher = DefaultHasher::new();
        text.hash(&mut hasher);
        let hash = hasher.finish();
        let mut cache = self.meter_cache.borrow_mut();
        if let Some((cached, marks)) = cache.get(&line) {
            if *cached == hash {
                return marks.clone();
            }
        }
        let words = self.segmenter.segment(&text);
        let marks = crate::meter::marks(&chars, &words, &|word| self.reader.read(word));
        if cache.len() >= SEGMENT_CACHE_LIMIT {
            cache.clear();
        }
        cache.insert(line, (hash, marks.clone()));
        marks
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
        // Asked once per line of every 縱書 frame, so it does not materialise
        // the line to measure it: a chapter of 資治通鑑 is one paragraph, and
        // copying it out to count its characters cost more than laying it out.
        let start = rope.line_to_char(line);
        let mut end = match line + 1 < rope.len_lines() {
            true => rope.line_to_char(line + 1),
            false => rope.len_chars(),
        };
        while end > start && matches!(rope.char(end - 1), '\n' | '\r') {
            end -= 1;
        }
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
        // **Which version of the document, not what it says.** Reading the
        // paragraph to find out whether it had changed made the cache cost more
        // than it saved: a chapter written as one 500,000-character paragraph
        // was hashed once per 縱 of every frame. A revision moves on every
        // edit, so an edit anywhere costs the visible lines one pass — which is
        // what they would have cost anyway.
        //
        // The syntax is in it too: the same characters mean different things in
        // different syntaxes, and `:syntax text` is one keystroke away.
        let mut hasher = DefaultHasher::new();
        (
            self.current_buffer().revision(),
            self.current_buffer().syntax() as u8,
        )
            .hash(&mut hasher);
        let hash = hasher.finish();

        let key = (self.current_buffer().id(), line);
        let mut cache = self.markup_cache.borrow_mut();
        if let Some((cached, spans)) = cache.get(&key) {
            if *cached == hash {
                return spans.clone();
            }
        }
        let mut text = rope.line(line).to_string();
        while text.ends_with('\n') || text.ends_with('\r') {
            text.pop();
        }
        let spans = match self.current_buffer().syntax() {
            crate::syntax::Syntax::Markdown => crate::markdown::spans(&text),
            crate::syntax::Syntax::Typst => crate::markdown::typst::spans(&text),
            // Nothing in the file means anything but itself.
            crate::syntax::Syntax::Text => Vec::new(),
        };
        cache.insert(key, (hash, spans.clone()));
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
                // A file with no markup has its chapters found below instead.
                crate::syntax::Syntax::Text => continue,
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
        // **A novel is a text file with chapters in it and no markup at all.**
        // 資治通鑑 is 700 chapters and not one `#` — exactly the file where
        // 「go to chapter 412」 is worth a key, and the one this said had no
        // outline. The chapters are written 第四百一十二卷, which is a heading
        // whether or not anybody marked it up.
        //
        // Only when nothing else was found: a manuscript that *does* use `#`
        // has said how it marks a chapter, and a stray 第三章 line in its prose
        // is not a second opinion.
        if out.is_empty() {
            // Every line that *reads* as a chapter heading…
            let mut found: Vec<(usize, usize, String, String)> = Vec::new();
            for line in 0..rope.len_lines() {
                let text = rope.line(line).to_string();
                let trimmed = text.trim_end_matches(['\n', '\r']);
                if let Some((depth, key)) = chapter_heading(trimmed) {
                    found.push((line, depth, trimmed.trim().to_string(), key));
                }
            }
            // …minus the **table of contents**. 資治通鑑 opens with 294 lines
            // reading 卷002, 卷003, 卷004 — every chapter named, none of them
            // *at* its chapter, and together they are the whole of the outline
            // panel before a reader can reach the first page of writing.
            //
            // What tells them apart is not how they are written but what is
            // under them: **a chapter has writing under it.** A listing has the
            // next listing — and, in this book, the odd 「秦紀」 label between
            // two of them, which is why the bar is three lines and not one.
            // Measured on the corpora: at three lines 資治通鑑's outline is 295
            // for its 294 卷 and 紅樓夢's is 121 for its 120 回; at one line
            // they are 310 and 122, the extras all listing lines.
            const WRITING_UNDER_A_CHAPTER: usize = 3;
            let mut last: Option<String> = None;
            for (n, (line, depth, title, key)) in found.iter().enumerate() {
                let next = found.get(n + 1).map(|&(l, ..)| l).unwrap_or(rope.len_lines());
                let writing = (line + 1..next)
                    .filter(|&l| !rope.line(l).to_string().trim().is_empty())
                    .take(WRITING_UNDER_A_CHAPTER)
                    .count();
                if writing < WRITING_UNDER_A_CHAPTER {
                    continue;
                }
                // …and a chapter marked twice running is one chapter.
                if last.as_deref() == Some(key.as_str()) {
                    continue;
                }
                last = Some(key.clone());
                out.push((*line, *depth, title.clone()));
            }
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
    /// Create a new, empty scratch buffer and make it active.
    pub fn new_buffer(&mut self) {
        self.add_buffer(Buffer::scratch());
    }

    /// Run a `:` command line.
    ///
    /// Returns [`CommandOutcome::Quit`] when a `:q` / `:q!` should end the
    /// session, and [`CommandOutcome::Continue`] otherwise.
    pub fn execute(&mut self, line: &str) -> Result<CommandOutcome, EditorError> {
        // **The line belongs to this command.** A command that fails says so
        // through its error, and the status is where the *last* command's
        // answer was: a failed `:export` used to leave 「存了 ch1.md」 standing,
        // which reads as an export that worked.
        self.status.clear();
        // What this command needs before it can mean anything (Feature #170).
        // A setting whose prerequisite is missing used to be *set* and then
        // read by nobody: `:view hanging on` on a horizontal page turned a flag on,
        // changed nothing, and said 「標點旁置：開」, which is three kinds of
        // wrong at once.
        let (line, force) = match line.trim_end().strip_suffix(" force") {
            Some(rest) => (rest.trim_end(), true),
            None => (line, false),
        };
        let unmet: Vec<command::Need> = command::needs_of(line)
            .iter()
            .copied()
            .filter(|need| !self.meets(*need))
            .collect();
        if !unmet.is_empty() {
            if !force {
                let what: Vec<String> = unmet.iter().map(|n| n.says()).collect();
                self.status = say!(
                    "cmd.needs-these-first",
                    what.join(&say!("label.comma"))
                );
                return Ok(CommandOutcome::Continue);
            }
            // Satisfied until they stay satisfied: turning the page 縱書 can
            // make a second prerequisite start mattering, and a `force` that
            // half-worked would be worse than one that did not.
            for _ in 0..3 {
                let left: Vec<command::Need> = command::needs_of(line)
                    .iter()
                    .copied()
                    .filter(|need| !self.meets(*need))
                    .collect();
                if left.is_empty() {
                    break;
                }
                for need in left {
                    self.satisfy(need);
                }
            }
        }
        let mut asked = command::parse(line)?;
        // `force` is stripped above, where it means 「do it anyway」 for a
        // command whose prerequisites are missing. `:convert … force` spells
        // the same word for the same kind of reason — do the *bigger* edit,
        // the one that changes words and not just characters — and the strip
        // above ate it before [`command::parse`] ever saw it. Handing it back
        // is the whole fix: every other reader of that word (the `:` menu,
        // [`command::parse`] called directly) sees it in place.
        if force {
            if let Command::Convert(command::ConvertAsk::Run { force, .. }) = &mut asked {
                *force = true;
            }
        }
        match asked {
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
                // **`:write` alone.** `:w!`, `:wq` and `:wa` are deliberately
                // not gated yet (2026-09-08): `:w!` already spells out 「over
                // whatever is there」, and the other two are one line each when
                // the shape of the question has been lived with. What is built
                // here is the interface, not the one caller.
                if let Some(ask) = self.oversize_query(path.as_deref()) {
                    self.query = Some(ask);
                    return Ok(CommandOutcome::Continue);
                }
                self.write_current(path.as_deref())?;
                Ok(CommandOutcome::Continue)
            }
            Command::SaveAs { path, force } => {
                let target = PathBuf::from(&path);
                // The same one rule: a file another buffer is holding may only
                // be written by that buffer.
                if let Some(which) = self.buffer_holding(&target) {
                    if which != self.current {
                        let name = self.buffers[which].display_name();
                        self.status = say!("buffer.already-open-elsewhere", name);
                        return Ok(CommandOutcome::Continue);
                    }
                }
                self.current_buffer_mut()
                    .save_as(target, force)
                    .map_err(EditorError::Io)?;
                self.status = say!("buffer.saved", self.current_buffer().display_name());
                Ok(CommandOutcome::Continue)
            }
            Command::WriteForce(path) => {
                self.write_forcing(path.as_deref(), true)?;
                Ok(CommandOutcome::Continue)
            }
            Command::Reload { force } => {
                self.reload(force)?;
                Ok(CommandOutcome::Continue)
            }
            Command::ReloadAuto(on) => {
                match on {
                    Some(on) => {
                        self.reload_auto = on;
                        // Asked now, not at the next keystroke: turning it on
                        // is usually a writer who already suspects the file
                        // moved.
                        self.last_disk_check = None;
                        self.reload_warned = false;
                        let word = if on { "on" } else { "off" };
                        self.status = say!("autoreload.set", word);
                    }
                    None => {
                        self.status = say!(
                            "autoreload.set",
                            if self.reload_auto { "on" } else { "off" }
                        )
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            Command::SetReadonly(on) => {
                match on {
                    Some(on) => {
                        self.current_buffer_mut().set_readonly(on);
                        // **`off` lifts `-R` too.** The flag locks every file
                        // the session opens, and a writer who has just said
                        // 「no, I do mean to edit this」 should not find the
                        // next `:open` locked again with no way to say it.
                        if !on {
                            self.readonly_default = false;
                        }
                        let word = if on { "on" } else { "off" };
                        // Locking a buffer that has unsaved changes takes `u`
                        // away with everything else, so say so once rather
                        // than let it be discovered at the worst moment.
                        self.status = match on && self.current_buffer().is_modified() {
                            true => say!("readonly.on-with-unsaved"),
                            false => say!("readonly.set", word),
                        };
                    }
                    None => {
                        self.status = say!(
                            "readonly.set",
                            if self.current_buffer().is_readonly() { "on" } else { "off" }
                        )
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            Command::Quit { force } => self.quit(force),
            Command::QuitAll { force } => self.quit_all(force),
            Command::Substitute {
                pattern,
                replacement,
                global,
                ignore_case,
                count_only,
                reshape,
                rows,
            } => {
                self.substitute(Substitution {
                    pattern: &pattern,
                    replacement: &replacement,
                    global,
                    ignore_case,
                    count_only,
                    reshape,
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
                if self.grid_is_drawn() && wants_vertical {
                    self.status = say!("table.vertical-not-allowed");
                    return Ok(CommandOutcome::Continue);
                }
                let layout = match direction {
                    Some(l) => {
                        self.set_layout(l);
                        l
                    }
                    None => self.toggle_layout(),
                };
                // Not `layout.label()`: that is the config's spelling, in
                // English, and it was being read out inside a Chinese
                // sentence to a reader who had just switched to 竪排.
                self.status = match layout {
                    Layout::Vertical => say!("layout.vertical"),
                    Layout::Horizontal => say!("layout.horizontal"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::Ruby => {
                self.enter_ruby_mode();
                Ok(CommandOutcome::Continue)
            }
            Command::RenderRuby { dialect, on } => {
                self.render_ruby(dialect, on);
                let listed_names: Vec<String> =
                    self.ruby.iter().map(|d| d.name().to_string()).collect();
                self.status = if listed_names.is_empty() {
                    say!("ruby.layout-off")
                } else {
                    say!("ruby.layout-on", listed(&listed_names))
                };
                Ok(CommandOutcome::Continue)
            }
            Command::AutoRuby { rare } => {
                self.auto_ruby(rare);
                Ok(CommandOutcome::Continue)
            }
            Command::FormatRuby(dialect) => {
                self.format_ruby(dialect);
                Ok(CommandOutcome::Continue)
            }
            Command::WriteQuit(path) => {
                // **`:wq <名字>` means 「save it as this, I am done」.** Left as
                // `:w <path>` it wrote a *copy* and then refused to quit,
                // because the chapter itself was still unsaved — and a writer
                // with vi's muscle memory reads that refusal and reaches for
                // `:q!`. So it rebinds, exactly as `:write as` does, and the file
                // that is saved is the one the name says.
                match path.as_deref() {
                    Some(path) => {
                        let target = PathBuf::from(path);
                        if let Some(which) = self.buffer_holding(&target) {
                            if which != self.current {
                                let name = self.buffers[which].display_name();
                                self.status =
                                    say!("buffer.already-open-elsewhere", name);
                                return Ok(CommandOutcome::Continue);
                            }
                        }
                        self.current_buffer_mut()
                            .save_as(target, false)
                            .map_err(EditorError::Io)?;
                        self.status = say!("buffer.saved", self.current_buffer().display_name());
                    }
                    None => {
                        self.write_current(None)?;
                    }
                }
                // Saving *this* buffer is not saving the session: another open
                // file may still be dirty, and `:wq` reads as "everything is
                // safe now", so it is held to the same check `:q` is.
                self.quit(false)
            }
            Command::ReplaceFound(text, reshape) => {
                self.replace_found(&text, reshape);
                Ok(CommandOutcome::Continue)
            }
            Command::WriteAll => self.write_all(),
            Command::Search { pattern, by } => {
                match by {
                    crate::command::Axis::Row => {
                        self.last_search = pattern;
                        // A row search takes `n` back from a column search's
                        // answers, the way `/` does.
                        self.hits = None;
                        let forward = self.search_forward;
                        self.repeat_search(forward);
                    }
                    // A column search is a table's; a document has no columns
                    // to run down.
                    crate::command::Axis::Column if self.table_here() => {
                        // Shown in the other work area, which is what a column
                        // search is for: 卵's own row and a row that uses 卵
                        // are two places, and the question is about both.
                        self.definition_preview = true;
                        self.search_columns(&pattern)
                    }
                    crate::command::Axis::Column => {
                        self.status = say!("table.not-in-a-table")
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            Command::Word(what) => self.word_command(what),
            Command::SetBands(n) => {
                self.set_bands(n);
                Ok(CommandOutcome::Continue)
            }
            Command::SetNumberFill(on) => {
                self.number_fill = on.unwrap_or(!self.number_fill);
                self.status = match self.number_fill {
                    true => say!("layout.line-numbers-on-a-band"),
                    false => say!("layout.line-numbers-on-the-page"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetIndentHint(hint) => {
                self.indent_hint = hint;
                self.status = say!("layout.indent-hint", hint.name());
                Ok(CommandOutcome::Continue)
            }
            Command::SetIndent(n) => {
                self.set_indent(n);
                Ok(CommandOutcome::Continue)
            }
            Command::CheckTable => {
                self.check_table();
                Ok(CommandOutcome::Continue)
            }
            Command::CheckPunct => {
                self.check_punct();
                Ok(CommandOutcome::Continue)
            }
            Command::CheckCharset => {
                self.check_charset();
                Ok(CommandOutcome::Continue)
            }
            Command::CheckUsage => {
                self.check_usage();
                Ok(CommandOutcome::Continue)
            }
            Command::Progress => {
                self.progress_report();
                Ok(CommandOutcome::Continue)
            }
            Command::Target(target) => {
                self.set_target(target);
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
            Command::Export {
                format,
                path,
                force,
            } => self.export(&format, path.as_deref(), force),
            Command::Grep(pattern) => {
                let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                self.grep(&pattern, &root)
            }
            Command::Conflicts => {
                self.list_conflicts();
                Ok(CommandOutcome::Continue)
            }
            Command::Diff(against) => {
                self.diff_against(against.as_deref());
                Ok(CommandOutcome::Continue)
            }
            Command::Outline(nth) => {
                let headings = self.outline();
                if headings.is_empty() {
                    self.status = say!("goto.no-headings");
                    return Ok(CommandOutcome::Continue);
                }
                match nth {
                    // `:toc <n>` goes to the nth heading…
                    Some(n) => match headings.get(n.saturating_sub(1)) {
                        Some(&(line, _, _)) => self.goto_line(line + 1),
                        None => self.status = say!("goto.only-n-headings", headings.len()),
                    },
                    // …and a bare `:toc` opens the outline, which is a *list*
                    // — one heading a line, scrollable, with the keys. It used
                    // to join all of them into the status line with three
                    // spaces between, which for a novel is 700 chapters on one
                    // row, of which the reader can see four.
                    None => {
                        self.show_sidebar(crate::sidebar::View::Outline);
                        self.status = say!("toc.headings-found", headings.len());
                    }
                }
                Ok(CommandOutcome::Continue)
            }
            Command::Tutor => {
                self.open_tutor();
                Ok(CommandOutcome::Continue)
            }
            Command::Help(topic) => {
                self.open_help(topic.as_deref());
                Ok(CommandOutcome::Continue)
            }
            Command::ListBuffers => {
                // The picker, not the status line: with 122 chapters open the
                // list is 1,783 characters and the status line is one row. A
                // list you cannot read is not a list — and the picker is the
                // same list, searchable, which is what you wanted anyway.
                self.open_buffer_picker();
                Ok(CommandOutcome::Continue)
            }
            Command::SetHanging(want) => {
                let on = want.unwrap_or(!self.hanging);
                self.set_hanging_punctuation(on);
                self.status = if on {
                    say!("layout.hung-punctuation-on")
                } else {
                    say!("layout.hung-punctuation-off")
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
                            self.status = say!("render.markup-is", syntax.name());
                        }
                        None => {
                            self.status = say!("render.no-such-markup", name)
                        }
                    },
                    None => {
                        self.status = say!("render.markup-is", self.syntax().name());
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
            // 上屏方式 is the engine's, so it rides the same channel: a word
            // rather than a sigil, because there is no scheme called `commit:`
            // and a request one can read is worth the four characters.
            Command::YumeCommit(mode) => {
                self.scheme_request = Some(format!("commit:{}", mode.unwrap_or_default()));
                Ok(CommandOutcome::Continue)
            }
            // 候選面板 rides it too, for the same reason: the front end is the
            // one that draws a panel, so it is the one that can be asked not
            // to (Feature #211).
            Command::YumePanel(mode) => {
                self.scheme_request = Some(format!("panel:{}", mode.unwrap_or_default()));
                Ok(CommandOutcome::Continue)
            }
            // Taken by the front end **after the next frame**: the command
            // line is still open on this one, and a picture of the thing you
            // are debugging with the debugger's own prompt across it is not a
            // picture of the thing.
            Command::Screenshot { shot, force } => self.take_a_picture(shot, force),
            Command::InstalledScheme => {
                self.scheme_request = Some(String::from("~"));
                Ok(CommandOutcome::Continue)
            }
            Command::YumeLanguage(want) => {
                // `lang:` for the same reason `commit:` and `panel:` have a
                // prefix: the front end holds the session, and this is one
                // more question about it — asked by name now that there are
                // three answers and not two (#290).
                self.scheme_request = Some(format!(
                    "lang:{}",
                    match want {
                        command::Engagement::Chinese => "chinese",
                        command::Engagement::Ascii => "abc",
                        command::Engagement::Off => "off",
                    }
                ));
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
            Command::Convert(ask) => {
                self.convert(ask);
                Ok(CommandOutcome::Continue)
            }
            Command::SetPreview(on) => {
                // Already running: the question 「where is it?」 is the one a
                // writer actually asks, and the address was said once and lost.
                if on && self.preview_at.is_some() {
                    self.preview_request = Some(Preview::Show);
                    return Ok(CommandOutcome::Continue);
                }
                self.preview_request = Some(if on {
                    match self.current_buffer().path() {
                        Some(path) => Preview::Start {
                            path: path.to_path_buf(),
                            syntax: self.current_buffer().syntax(),
                        },
                        None => {
                            self.status = say!("preview.save-first");
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
                    Render::Off => say!("render.source"),
                    Render::Basic => say!("render.full"),
                    Render::Full => say!("render.wysiwyg"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::ReportRender => {
                let word = |level: TableLevel| match level {
                    TableLevel::Off => say!("level.off"),
                    TableLevel::Basic => say!("level.basic"),
                    TableLevel::Full => say!("level.full"),
                };
                // **The three it writes**, not four. `:indent` reports itself
                // — it is not `:render`'s to set, so saying its level here
                // would read as a claim that it is. All three have three
                // levels since 2026-09-06: ruby used to have two states and an
                // N-to-1 mapping onto the three names, and the state it was
                // missing is the one the law asks for — know a reading without
                // drawing it.
                let level = |how: Render| match how {
                    Render::Off => TableLevel::Off,
                    Render::Basic => TableLevel::Basic,
                    Render::Full => TableLevel::Full,
                };
                self.status = say!(
                    "render.is",
                    word(level(self.render)),
                    word(self.table_level),
                    word(level(self.ruby_level()))
                );
                Ok(CommandOutcome::Continue)
            }
            // `:view hud` sets and reports with the same three sentences: what the
            // level *is* and what it was just changed to are the same fact,
            // and two wordings of it would be two things to keep true.
            Command::SetHud(how) => {
                self.hud = how;
                self.status = hud_says(how);
                Ok(CommandOutcome::Continue)
            }
            Command::ReportHud => {
                self.status = hud_says(self.hud);
                Ok(CommandOutcome::Continue)
            }
            // A measure is only a measure if the rows honour it, so setting
            // one turns wrapping on: `:view wrap 50` says "write to fifty", and
            // fifty columns of text running off the edge is not that.
            Command::SetDense(on) => {
                self.set_dense(on);
                Ok(CommandOutcome::Continue)
            }
            Command::SetSentences(on) => {
                self.set_sentences(on);
                Ok(CommandOutcome::Continue)
            }
            // The palette lives in the front end — the core does not know a
            // colour exists — so the request is left here and answered there,
            // the same way `:yume scheme` reaches the input method.
            Command::Theme { name, mood } => {
                self.theme_request = Some((name, mood));
                Ok(CommandOutcome::Continue)
            }
            Command::TableToPipe(delimiter) => {
                self.table_to_pipe(delimiter);
                Ok(CommandOutcome::Continue)
            }
            Command::TableToDelimited(delimiter) => {
                self.table_to_delimited(delimiter);
                Ok(CommandOutcome::Continue)
            }
            Command::SortTable(keys) => {
                self.sort_table(&keys);
                Ok(CommandOutcome::Continue)
            }
            Command::ShowDetail(want) => {
                self.show_detail = want.unwrap_or(!self.show_detail);
                self.status = match self.show_detail {
                    true => say!("ui.detail-panel-on"),
                    false => say!("ui.detail-panel-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetDetailWidth(n) => {
                self.detail_width = Some(n.clamp(12, 80));
                self.show_detail = true;
                self.status = say!("ui.detail-panel-width", self.detail_width.unwrap_or(n));
                Ok(CommandOutcome::Continue)
            }
            Command::Language(verb) => {
                let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
                    self.status = say!("language.save-first");
                    return Ok(CommandOutcome::Continue);
                };
                self.language_run = Some(LanguageRun {
                    verb,
                    language: self.current_buffer().syntax().name().to_string(),
                    path,
                });
                Ok(CommandOutcome::Continue)
            }
            Command::Markdown(bit) => {
                self.write_markdown(bit);
                Ok(CommandOutcome::Continue)
            }
            Command::SetTypewriter(want) => {
                self.typewriter = want.unwrap_or(!self.typewriter);
                self.status = match self.typewriter {
                    true => say!("layout.typewriter-on"),
                    false => say!("layout.typewriter-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetFocus(want) => {
                self.focus = want.unwrap_or(!self.focus);
                self.status = match self.focus {
                    true => say!("layout.focus-on"),
                    false => say!("layout.focus-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetMeter(want) => {
                self.meter = want.unwrap_or(!self.meter);
                self.status = match (self.meter, self.reader.available()) {
                    // 平仄 come out of the 拆分表, and an editor without one
                    // would turn the mode on and draw an empty margin. Say
                    // which of the two it is, rather than letting the writer
                    // conclude their poem has no tones in it.
                    (true, false) => say!("layout.meter-no-readings"),
                    (true, true) => say!("layout.meter-on"),
                    (false, _) => say!("layout.meter-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetNote(want) => {
                self.notes = want.unwrap_or(!self.notes);
                self.status = match (self.notes, self.markup_visible()) {
                    // A note is drawn on the page, and `:render off` is the one
                    // setting that says 「draw nothing the file does not
                    // contain」. Turning notes on under it would leave the
                    // writer waiting for a mark that is never coming.
                    (true, false) => say!("layout.note-no-render"),
                    (true, true) => say!("layout.note-on"),
                    (false, _) => say!("layout.note-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetTableNumbers(on) => {
                self.table_numbers = on;
                self.status = match on {
                    true => say!("table.column-numbers-on"),
                    false => say!("table.column-numbers-off"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetTableHeader(want) => {
                self.set_table_header(want);
                Ok(CommandOutcome::Continue)
            }
            Command::OpenTableSchema => {
                self.open_schema();
                Ok(CommandOutcome::Continue)
            }
            Command::NewTable { rows, columns } => {
                self.new_table(rows, columns);
                Ok(CommandOutcome::Continue)
            }
            Command::SetTableRules(rules) => {
                if let Some(rules) = rules {
                    self.table_rules = rules;
                }
                self.status = say!("table.column-rules", self.table_rules.name());
                Ok(CommandOutcome::Continue)
            }
            Command::EnterTable => {
                // **`:table` is the door, not a surface.** Typed while the
                // grid had the window it went in again as 畫成表格 — a silent
                // demotion that also threw away `t q`'s way back.
                match self.table.as_ref().map(|v| v.pane) {
                    None => {
                        self.enter_table();
                    }
                    Some(true) => self.status = say!("table.already-the-window"),
                    Some(false) => match self.table_level {
                        TableLevel::Full => self.status = say!("table.already-drawn"),
                        _ => self.status = say!("table.already-operated"),
                    },
                }
                Ok(CommandOutcome::Continue)
            }
            Command::SetTableLevel(level) => {
                self.set_table_level(level);
                Ok(CommandOutcome::Continue)
            }
            Command::ReportIndent => {
                self.status = self.indent_report();
                Ok(CommandOutcome::Continue)
            }
            Command::SetIndentLevel(how) => {
                self.set_indent_level(how);
                self.status = self.indent_report();
                Ok(CommandOutcome::Continue)
            }
            Command::SetRubyLevel(how) => {
                self.set_ruby_level(how);
                self.status = match self.ruby_level() {
                    Render::Off => say!("ruby.level-off"),
                    Render::Basic => say!("ruby.level-basic"),
                    Render::Full => say!("ruby.level-full"),
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetWheelStep(step) => {
                match step {
                    // Asking is a use of its own: the number is in a config
                    // file the reader may never have written.
                    None => self.status = say!("wheel.is", self.wheel_step),
                    Some(step) => {
                        self.set_wheel_step(step);
                        self.status = say!("wheel.set", self.wheel_step);
                    }
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
                // **縱書 has nothing to turn off.** A 縱 is broken by the
                // height of the window, and that is not the writer's to set —
                // `:view wrap 40` does set the 縱 length, in either layout, but
                // 開／關 does not reach it. Taking it anyway and answering
                // 「長段落跑出右邊」 named a right edge this page does not
                // have, and left the reader looking for a change that had not
                // been made.
                if self.layout == Layout::Vertical {
                    self.status = say!("wrap.vertical-has-no-wrap");
                    return Ok(CommandOutcome::Continue);
                }
                self.set_soft_wrap(on);
                self.refresh_goal_column();
                self.status = if on {
                    say!("wrap.on")
                } else {
                    say!("wrap.off")
                };
                Ok(CommandOutcome::Continue)
            }
            Command::SetScheme(tag) => {
                self.scheme_request = Some(tag);
                Ok(CommandOutcome::Continue)
            }
            Command::SetChaifen(want) => {
                self.chaifen = want.unwrap_or(!self.chaifen);
                self.chaifen_request = Some(self.chaifen);
                Ok(CommandOutcome::Continue)
            }

        }
    }

    /// The question the editor has stopped to ask, if it has (#295).
    ///
    /// The front end draws it and stops drawing everything that answers keys.
    pub fn query(&self) -> Option<&Query> {
        self.query.as_ref()
    }

    /// Answer the open question. Every key comes here while one stands.
    ///
    /// A key that is not one of the choices **leaves the question standing**
    /// rather than falling through to the manuscript: the one thing a modal
    /// question may never do is let a stray keystroke edit the file behind it.
    /// `Esc` is 「no」 — the same answer the last choice spells out, because a
    /// reader who wants out of a dialog reaches for `Esc` before reading it.
    fn answer_query(&mut self, key: Key) {
        let Some(asked) = self.query.take() else { return };
        let answer = match key {
            Key::Esc => 'n',
            Key::Char(c) => c.to_ascii_lowercase(),
            _ => {
                self.query = Some(asked);
                return;
            }
        };
        if !asked.choices.iter().any(|a| a.key == answer) {
            self.query = Some(asked);
            return;
        }
        match asked.what {
            Asking::OversizeWrite { path } => match answer {
                // Yes: the same save, with the gate already answered.
                'y' => {
                    let _ = self.write_forcing(path.as_deref(), false);
                }
                // 檢視區別 **abandons the save**. Nothing is written, and the
                // buffer is left exactly as it was — which is the point: the
                // reader is going to look at what changed and decide again.
                'd' => self.diff_against(None),
                _ => self.status = say!("write.oversize-stopped"),
            },
        }
    }

    /// Whether this `:write` would make the file on disk very much bigger, and
    /// the two sizes if it would (Feature #295).
    ///
    /// **Never on a first save.** A file that is not there yet has no size to
    /// have multiplied, and a writer saving a new chapter is the one person
    /// this must not stop. The buffer is measured in bytes off the rope rather
    /// than by rendering it: the question is about an order of magnitude, and
    /// a line-ending pass would cost a copy of the manuscript to sharpen a
    /// number that is about to be rounded to 「MB」 anyway.
    fn oversize_write(&self, path: Option<&str>) -> Option<(u64, u64)> {
        let target = match path {
            Some(p) => PathBuf::from(p),
            None => self.current_buffer().path()?.to_path_buf(),
        };
        let was = std::fs::metadata(&target).ok()?.len();
        let now = self.current_buffer().rope().len_bytes() as u64;
        let doubled = was > 0 && now / 2 >= was;
        let jumped = now >= was.saturating_add(OVERSIZE_JUMP);
        (doubled && jumped).then_some((was, now))
    }

    /// The question `:write` asks when [`Self::oversize_write`] says to.
    fn oversize_query(&self, path: Option<&str>) -> Option<Query> {
        let (was, now) = self.oversize_write(path)?;
        let times = now as f64 / was as f64;
        Some(Query {
            title: say!("write.oversize-title"),
            body: say!(
                "write.oversize-what",
                self.current_buffer().display_name(),
                human_size(was),
                human_size(now),
                format!("{times:.1}")
            ),
            choices: vec![
                Answer { key: 'y', label: say!("write.oversize-go") },
                Answer { key: 'd', label: say!("write.oversize-look") },
                Answer { key: 'n', label: say!("write.oversize-no") },
            ],
            what: Asking::OversizeWrite { path: path.map(str::to_string) },
        })
    }

    /// Save the active buffer, optionally to a new `path` (save-as).
    fn write_current(&mut self, path: Option<&str>) -> Result<(), EditorError> {
        self.write_forcing(path, false).map(|_| ())
    }

    /// The same, and `force` writes over a file that changed on disk (`:w!`).
    ///
    /// Returns **what it did**, because the three are different things and the
    /// caller has to be able to tell them apart. It used to return `()` and the
    /// caller sniffed the rendered status line for a Chinese character to find
    /// out — which in English said 「saved ch1.md」 about a chapter that had not
    /// been saved, and in Chinese left a stale 「抄了一份」 standing over a save
    /// that had happened.
    fn write_forcing(&mut self, path: Option<&str>, force: bool) -> Result<Wrote, EditorError> {
        let saved: Result<Wrote, EditorError> = match path {
            // `:w path` writes a **copy** and stays here; `:w! path` writes it
            // over whatever is already there. Rebinding this buffer to another
            // name is `:write as`, which says so — `:w chapter-copy.md` used to
            // rebind silently, and every save after it went to the copy while
            // the chapter itself stayed at the version before.
            Some(p) => {
                let target = PathBuf::from(p);
                match self.buffer_holding(&target) {
                    // Its own file, spelled another way: an ordinary save.
                    Some(which) if which == self.current => {
                        return self.write_forcing(None, force)
                    }
                    // Somebody else's file, and that somebody is holding it in
                    // memory: a copy written here is text they will overwrite
                    // from a buffer that still believes it is clean.
                    Some(which) => {
                        let name = self.buffers[which].display_name();
                        self.status = say!("buffer.already-open-elsewhere", name);
                        return Err(EditorError::Io(std::io::Error::other(
                            self.status.clone(),
                        )));
                    }
                    // A buffer with no name of its own takes this one, as vi
                    // does: there is no manuscript here for the copy to be a
                    // copy *of*, and a scratch buffer that stayed nameless
                    // after `:w 第一章.md` would ask again at the next save.
                    None if self.current_buffer().path().is_none() => self
                        .current_buffer_mut()
                        .save_as(target, force)
                        .map(|()| Wrote::Saved)
                        .map_err(EditorError::Io),
                    None => self
                        .current_buffer()
                        .write_copy(&target, force)
                        .map(|()| Wrote::Copied(target))
                        .map_err(EditorError::Io),
                }
            }
            None => {
                if self.current_buffer().path().is_none() {
                    return Err(EditorError::NoFileName);
                }
                self.current_buffer_mut()
                    .save_forcing(force)
                    .map(|()| Wrote::Saved)
                    .map_err(EditorError::Io)
            }
        };
        // A save that said nothing was a save you could not tell from a save
        // that did not happen — and the manual has been quoting this line as
        // its example of the hint row all along.
        // 寫作進度 is kept from the save, not from the keystroke: what a day
        // holds is what the writer committed to disk that day (Feature #244).
        let word_list = matches!(saved, Ok(Wrote::Saved)) && self.note_word_list_saved();
        if matches!(saved, Ok(Wrote::Saved)) {
            self.note_progress();
        }
        match &saved {
            // A saved word list says so itself, and says how many words are in
            // force now — 「存了 words.txt」 alone would leave the reader
            // wondering whether the weeding took effect.
            Ok(Wrote::Saved) if word_list => {}
            Ok(Wrote::Saved) => {
                self.status = say!("buffer.saved", self.current_buffer().display_name())
            }
            Ok(Wrote::Copied(to)) => self.status = say!("buffer.copied-to", to.display()),
            Err(_) => {}
        }
        saved
    }

    // ---- Modal editing (Feature #5) ---------------------------------------

    /// The current editing mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether `r` is waiting for the character it will write (§5.2.3 ②).
    ///
    /// The front end asks so the IME may run for it: `r` then 中文 opens the
    /// candidate panel, and the choice is the replacement.
    pub fn replacing(&self) -> bool {
        self.pending == Pending::Replace
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
        self.caret()
    }

    /// The current selection as a character range `(start, end)` with
    /// `start <= end`. When `start == end` the selection is collapsed (just the
    /// cursor). Helix treats the cursor as a one-wide selection, so `d` still
    /// deletes the grapheme under a collapsed cursor.
    pub fn selection(&self) -> (usize, usize) {
        let (start, end) = (self.mark().min(self.caret()), self.mark().max(self.caret()));
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
        (self.mark().min(self.caret()), self.mark().max(self.caret()))
    }

    /// Whether the writer has actually selected a range, rather than merely
    /// standing on a character.
    ///
    /// [`Self::selection`] is never empty — the cursor's own grapheme is always
    /// in it — so it cannot answer this. The renderer needs the difference: a
    /// bare cursor is drawn as a cursor, not as a one-character highlight.
    pub fn has_selection(&self) -> bool {
        self.mark() != self.caret()
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

    /// The active prompt (Command, Search, Ruby or `::`): what is written
    /// before it and the text typed so far, or `None` when no prompt is open.
    ///
    /// A **string** rather than a character, because `::` is two of them
    /// (#224) and a prompt that drew itself as `:` would be lying about which
    /// of the two lines the next Enter belongs to.
    pub fn prompt(&self) -> Option<(&'static str, &str)> {
        match self.mode {
            Mode::Command => Some((":", &self.command_line)),
            Mode::Lookfor => Some(("::", &self.command_line)),
            Mode::Search => Some((
                if self.search_forward { "/" } else { "?" },
                &self.command_line,
            )),
            Mode::Ruby => Some(("注", &self.command_line)),
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
        // **An empty search prompt already guesses** (#274). The author,
        // 2026-09-05: 「`/` 搜索，enter 確認，再次按下 `/` 搜索，這個時候是不是
        // 應該預填寫（灰色）上次搜索過內容？」 — `Enter` on an empty line has
        // always repeated the last pattern, and the only thing missing was
        // *saying so*: the guess is the whole of it from the first keystroke,
        // so `/⏎` reads as「再找一次這個」rather than as a prompt you have to
        // remember what you last put in. Typing narrows it the way it always
        // did, and the first character that does not match takes it away.
        if self.mode == Mode::Search && self.command_line.is_empty() {
            return self.last_search.clone();
        }
        // The guess completes the *word* being typed, so a line with arguments
        // on it can still be guessed at: `:yume sch` guesses `eme`.
        let (start, _) = command::complete_at(&self.command_line);
        let typed = &self.command_line[start.min(self.command_line.len())..];
        if typed.is_empty() {
            return String::new();
        }
        let whole = match self.mode {
            // `written()`, the same as Tab writes (`cycle_completion`). A deep
            // match carries its parent — `:sch` is answered with `yume scheme`
            // — and offering the bare `name` guessed `:scheme`, a line that
            // does not parse, while Tab on the same keystroke wrote
            // `:yume scheme`. Where the parent is not what was typed the guess
            // is now simply not offered, and Tab still says the whole thing.
            Mode::Command => command::complete(&self.command_line)
                .first()
                .map(|e| e.written()),
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
        let drawn = self.prompt_ghost();
        self.command_line.push_str(&drawn);
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
        self.current_buffer().rope().char_to_line(self.caret())
    }

    /// The 0-based **character** column the cursor is at within its line.
    ///
    /// Not [`Self::cursor_visual_column`], which is cells: this is the column
    /// a drawn run is anchored at, and those are counted in characters the way
    /// `hidden` is (Feature #211).
    pub fn cursor_column(&self) -> usize {
        let rope = self.current_buffer().rope();
        let at = self.caret().min(rope.len_chars());
        at - rope.line_to_char(rope.char_to_line(at))
    }

    /// The cursor's visual column (summed display width within its line).
    pub fn cursor_visual_column(&self) -> usize {
        motion::visual_column(self.current_buffer().rope(), self.caret())
    }

    /// The character under the cursor, for the status line to name.
    ///
    /// At the end of a line — where Insert mode spends most of its time —
    /// there is nothing under the cursor, so the character *before* it is the
    /// answer instead: what a writer wants named is the 字 they are looking at,
    /// and having just typed it counts as looking at it.
    pub fn char_at_cursor(&self) -> Option<char> {
        let rope = self.current_buffer().rope();
        let here = (self.caret() < rope.len_chars()).then(|| rope.char(self.caret()));
        match here {
            Some(c) if c != '\n' && c != '\r' => Some(c),
            _ => (self.caret() > 0)
                .then(|| rope.char(self.caret() - 1))
                .filter(|&c| c != '\n' && c != '\r'),
        }
    }

    // Grids — table mode, `|` tables and delimited text — are in
    // `editor/tables.rs` (#296).

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
        self.status = say!("edit.pipe-replaced-characters", text.chars().count());
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
        // **The table the cursor is in, not the one it was entered in** (#283):
        // a document holds as many tables as somebody typed, and filtering the
        // five-column one through `sort` while the view still carried the
        // two-column one's schema refused every row it was handed.
        let want = self.table_column_count_at(self.cursor_line());
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let got = view.cells(line).len();
            if got != want {
                return Some(say!("table.filter-changed-shape", i + 1, got, want));
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
        self.status = say!("shell.done", line);
    }

    /// **`:markdown …`** — write a piece of Markdown at the cursor.
    ///
    /// The things a manuscript keeps needing and nobody wants to type: a
    /// footnote with the next free number *and* its note at the foot, an inline
    /// note, a table of a given size. The cursor is left where the typing goes,
    /// in Insert, because that is the next thing that happens every time.
    fn write_markdown(&mut self, bit: crate::command::MarkdownBit) {
        use crate::command::MarkdownBit;
        match bit {
            MarkdownBit::Footnote => {
                // A footnote is Markdown. In a Typst book `[^1]` is four
                // characters of nothing, and in a plain manuscript it is four
                // characters the reader did not ask for.
                if self.current_buffer().syntax() != crate::syntax::Syntax::Markdown {
                    self.status = say!("note.not-markdown");
                    return;
                }
                // **The next free number**, from the file itself: a footnote
                // whose number is already taken is a footnote pointing at
                // somebody else's note.
                let text = self.current_buffer().text();
                let taken: Vec<usize> = crate::markdown::footnote_numbers(&text);
                let n = (1..).find(|n| !taken.contains(n)).unwrap_or(1);
                let tag = format!("[^{n}]");
                self.snapshot();
                let at = self.cursor;
                if !self.edit_insert(at, &tag) {
                    return;
                }
                self.set_cursor(at + tag.chars().count());
                // …and the note it points at, written and stood in. `gd` does
                // exactly this when it cannot find a note; this is the same
                // path, asked for rather than stumbled into.
                self.definition_preview = false;
                let rope = self.current_buffer().rope();
                let end = rope.len_chars();
                let text = self.current_buffer().text();
                let lead = match text.ends_with("\n\n") {
                    true => String::new(),
                    false => match text.ends_with('\n') {
                        true => "\n".to_string(),
                        false => "\n\n".to_string(),
                    },
                };
                let note = format!("{lead}{tag}: ");
                self.write_the_note(end, &note, &tag);
                self.mode = Mode::Insert;
            }
            MarkdownBit::InlineNote => {
                let at = self.cursor;
                self.snapshot();
                if !self.edit_insert(at, "^[]") {
                    return;
                }
                // Between the brackets, where the note goes.
                self.set_cursor(at + 2);
                self.mode = Mode::Insert;
                self.status = say!("md.inline-note-inserted");
            }
        }
    }

    /// **`:tutor`** — the lesson, copied into a file of the reader's own.
    ///
    /// A **real file**, not a scratch buffer: `:w` works, `u` is part of lesson
    /// one, and every destructive key in it is safe because it is a copy. A
    /// second `:tutor` numbers a fresh one rather than overwriting the first,
    /// which may have a week's notes in it by then.
    fn open_tutor(&mut self) {
        let Some(dir) = self.drafts_dir.as_ref().and_then(|d| d.parent()) else {
            // No data directory — the front end never said where it is. The
            // lesson still opens; it just has nowhere to live.
            let mut buffer = crate::Buffer::from_text(crate::tutor::LESSON);
            buffer.name_as("[tutor]");
            self.add_buffer(buffer);
            self.status = say!("tutor.open-unsaved");
            return;
        };
        let dir = dir.to_path_buf();
        let _ = std::fs::create_dir_all(&dir);
        // The first name that is free: a lesson from last week is somebody's
        // notes now.
        let path = (0..100)
            .map(|n| dir.join(crate::tutor::file_name(n)))
            .find(|p| !p.exists());
        let Some(path) = path else {
            self.status = say!("tutor.no-name-left");
            return;
        };
        if let Err(err) = crate::buffer::write_file_atomically(&path, crate::tutor::LESSON) {
            self.status = say!("tutor.cannot-write", err);
            return;
        }
        match self.open_file(&path) {
            Ok(()) => self.status = say!("tutor.this-copy-is-yours"),
            Err(err) => self.status = say!("tutor.cannot-open", err),
        }
    }

    /// **`:help`** — the keys and the commands, in a buffer you can read with
    /// the editor itself.
    ///
    /// Written from the same declarations the editor runs on — `COMMANDS`,
    /// `SPACE_KEYS`, the which-key lists — so it cannot drift from what the
    /// keys actually do. A manual can be out of date; this cannot be, because
    /// there is nothing here to keep up to date.
    ///
    /// It opens as an ordinary buffer, so `/`, `n`, `空格 f` and every motion
    /// work in it: the way to learn an editor is to use it on something, and
    /// this is something.
    fn open_help(&mut self, topic: Option<&str>) {
        let (title, text) = match topic.map(str::trim).unwrap_or_default() {
            "" => (say!("help.common.title"), self.help_common()),
            t if "chinese".starts_with(t) || "中文".starts_with(t) => {
                (say!("help.chinese.title"), Self::help_chinese())
            }
            t if "vertical".starts_with(t) || "竪排".starts_with(t) => {
                (say!("help.vertical.title"), Self::help_vertical())
            }
            t if "table".starts_with(t) || "表格".starts_with(t) => {
                (say!("label.table"), self.help_table())
            }
            t if "commands".starts_with(t) || "命令".starts_with(t) => {
                (say!("help.commands.title"), Self::help_commands())
            }
            other => {
                self.status = say!(
                    "help.no-such-section",
                    other
                );
                return;
            }
        };
        let mut buffer = crate::Buffer::from_text(&text);
        buffer.name_as(&format!("[help · {title}]"));
        self.add_buffer(buffer);
        self.status = say!("help.title", title);
    }

    /// The keys a writer uses in the first hour, drawn from what is bound.
    fn help_common(&self) -> String {
        let mut out = String::new();
        out.push_str("# yumete

");
        out.push_str(&format!("{}\n\n", say!("help.self-reported")));
        out.push_str(&format!("## {}\n\n", say!("help.common.motion-title")));
        for (keys, what) in [
            ("h j k l", say!("help.common.hjkl")),
            ("w b e", say!("help.common.word-motions")),
            ("3w 5j", say!("help.common.count-first")),
            ("g30g / 30G", say!("help.common.go-to-line")),
            ("gg ge", say!("help.common.start-end-of-file")),
            ("gh gl gs", say!("help.common.line-start-end")),
            ("J K", say!("help.common.half-page")),
            ("gd gw", say!("help.common.follow-what-it-points-at")),
            ("g/ g?", say!("help.common.word-elsewhere")),
            ("/ n N", say!("help.common.search")),
            ("C-o C-i", say!("help.common.jump-list")),
            ("M a  'a", say!("help.common.marks")),
        ] {
            out.push_str(&format!("- `{keys}` — {what}
"));
        }
        out.push_str(&format!("\n## {}\n\n", say!("help.common.editing-title")));
        for (keys, what) in [
            ("i a", say!("help.common.insert-before-after")),
            ("o O", say!("help.common.open-line")),
            ("d c", say!("help.common.delete-change")),
            ("x", say!("help.common.select-line")),
            ("v ;", say!("help.common.extend-collapse")),
            (") (", say!("help.common.sentence-motions")),
            ("}} {{", say!("help.common.paragraph-motions")),
            ("y p", say!("help.common.yank-put")),
            ("u U", say!("help.common.undo-redo")),
            (".", say!("help.common.repeat-edit")),
            ("q Q", say!("help.common.macros")),
            ("> <", say!("help.common.indent")),
        ] {
            out.push_str(&format!("- `{keys}` — {what}
"));
        }
        out.push_str(&format!("\n## {}\n\n", say!("help.common.space-title")));
        for (key, what) in Self::SPACE_KEYS {
            out.push_str(&format!(
                "- `空格 {key}` — {}
",
                crate::messages::say(what, &[])
            ));
        }
        out.push_str(&format!("\n## {}\n\n", say!("help.common.other-sections")));
        for (topic, what) in [
            ("chinese", say!("help.chinese.section-summary")),
            ("vertical", say!("help.vertical.title")),
            ("table", say!("help.table.section-summary")),
            ("commands", say!("help.common.every-command")),
        ] {
            out.push_str(&format!("- `:help {topic}` — {what}\n"));
        }
        out
    }

    fn help_chinese() -> String {
        let mut out = format!("# {}\n\n", say!("help.chinese.title"));
        for (keys, what) in [
            (":yume on", say!("help.chinese.toggle-ime")),
            (":yume scheme", say!("help.chinese.switch-scheme")),
            (":yume chaifen on", say!("help.chinese.chaifen-under-candidates")),
            ("w b e", say!("help.chinese.word-boundaries")),
            (":word show on", say!("help.chinese.word-tint")),
            (":word list reload", say!("help.chinese.reload-project-words")),
            (":word habit", say!("help.chinese.habit-words")),
            (":ruby", say!("help.chinese.annotate-reading")),
            (":ruby format html", say!("help.chinese.unify-reading-spelling")),
            (":render full", say!("render.wysiwyg")),
            (":indent 2", say!("help.chinese.first-line-indent")),
            (":view hanging on", say!("help.chinese.hung-punctuation")),
            (":count", say!("help.chinese.count")),
        ] {
            out.push_str(&format!("- `{keys}` — {what}
"));
        }
        out
    }

    fn help_vertical() -> String {
        let mut out = format!("# {}\n\n", say!("help.vertical.title"));
        for (keys, what) in [
            (":layout vertical", say!("help.vertical.turn-vertical")),
            ("h l", say!("help.vertical.previous-next-column")),
            ("j k", say!("help.vertical.down-up-column")),
            (":view wrap 24", say!("help.vertical.column-length")),
            (":view bands 2", say!("help.vertical.bands")),
            (":view hanging on", say!("help.vertical.hung-punctuation")),
            (":view dense off", say!("help.vertical.loose")),
            (":view sentence", say!("help.vertical.sentence")),
            (":indent 2", say!("help.vertical.first-line-indent")),
            (":render full", say!("help.vertical.wysiwyg")),
        ] {
            out.push_str(&format!("- `{keys}` — {what}
"));
        }
        out
    }

    fn help_table(&self) -> String {
        let mut out = format!("# {}\n\n", say!("label.table"));
        for (keys, what) in [
            (":table", say!("help.table.enter-table")),
            ("h j k l", say!("help.table.step-by-cell")),
            ("Tab S-Tab", say!("help.table.next-previous-cell")),
            ("i a c d", say!("help.table.type-in-cell")),
            ("gd gw", say!("help.table.which-row-this-names")),
            ("3gd g2-5d", say!("help.table.search-in-columns")),
            ("t/ t?", say!("help.table.who-uses-this")),
            ("t o t b t f t t", say!("help.table.four-surfaces")),
            ("t i t w", say!("help.table.detail-and-folds")),
            ("t r t d", say!("help.table.add-or-drop-row")),
            ("t s t S", say!("help.table.sort-by-column")),
            ("t y t p", say!("help.table.yank-or-put-column")),
            (":table rules off", say!("help.table.no-column-rules")),
        ] {
            out.push_str(&format!("- `{keys}` — {what}
"));
        }
        out
    }

    /// Every `:` command, from the table the parser itself reads.
    fn help_commands() -> String {
        let mut out = format!("# {}\n\n", say!("help.commands.title"));
        for entry in crate::command::COMMANDS {
            let aliases = match entry.aliases.is_empty() {
                true => String::new(),
                false => format!("（{}）", entry.aliases.join(" ")),
            };
            out.push_str(&format!(
                "- `:{}`{} {} — {}
",
                entry.name,
                aliases,
                entry.args.hint(),
                crate::messages::say(entry.help, &[])
            ));
        }
        out
    }

    /// A typesetter the front end should start or stop.
    /// Whether the move that just happened was a jump, so the page can centre
    /// what it landed on rather than nudge it in from an edge.
    pub fn jumped(&self) -> bool {
        self.jumped
    }

    /// **Replace the whole document**, as one undoable edit.
    ///
    /// What a formatter does: the buffer went out, this came back, and `u`
    /// takes it back like any other change. Returns whether anything moved —
    /// an edit that changes nothing must not earn an undo point.
    ///
    /// The cursor keeps its place by character index, clamped: a formatter
    /// moves text about and there is no honest way to follow it, but landing
    /// near where you were beats landing at the top.
    pub fn replace_everything(&mut self, text: &str) -> bool {
        if self.current_buffer().text() == text {
            return false;
        }
        let at = self.cursor;
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(0..len, text));
        if !self.applied(done) {
            return false;
        }
        self.set_cursor(at.min(self.current_buffer().char_count()));
        self.clamp_cursor();
        self.forget_the_document();
        true
    }

    /// Take the language command the reader asked for, if any.
    pub fn take_language_run(&mut self) -> Option<LanguageRun> {
        self.language_run.take()
    }

    /// Say where the running typesetter's page is — or that there is none.
    ///
    /// The front end owns the process, so it is the front end that knows.
    pub fn set_preview_at(&mut self, url: Option<String>) {
        self.preview_at = url;
    }

    /// The running typesetter's address, for the status bar and for `:view preview`.
    pub fn preview_at(&self) -> Option<&str> {
        self.preview_at.as_deref()
    }

    pub fn take_preview_request(&mut self) -> Option<Preview> {
        self.preview_request.take()
    }

    /// The web page `gx` was pressed on, once.
    pub fn take_open_request(&mut self) -> Option<String> {
        self.open_request.take()
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
            return Hint::Keys(say!("hint.sidebar"), vec![
                    ("j k", say!("hint.move")),
                    ("l", say!("hint.enter")),
                    ("h", say!("hint.sidebar.collapse")),
                    ("Tab", say!("hint.sidebar.other-view")),
                    ("w", say!("hint.sidebar.width")),
                    ("R", say!("hint.sidebar.re-read")),
                    ("C-w", say!("hint.sidebar.back-to-text")),
                    ("q", say!("hint.close")),
                ]);
        }
        // Standing on a footnote reference, the key that shows the note is
        // worth saying: it is the one place `gd` has an answer that the reader
        // could not guess from the page.
        if self.mode == Mode::Normal && self.note_tag_at_cursor().is_some() {
            return Hint::Keys(say!("hint.footnote"), vec![
                ("gd", say!("hint.footnote.show-or-write")),
                ("g/ g?", say!("hint.word-elsewhere")),
            ]);
        }
        match self.mode {
            Mode::Ruby if self.ruby_target.is_some() => {
                Hint::Keys(say!("hint.reading"), vec![("Enter", say!("hint.keep-it")), ("Esc", say!("hint.cancel"))])
            }
            // The one key worth saying inside a cell — without it a person
            // types a value, presses Esc, walks right and types the next.
            Mode::Insert if self.insert_bounds().is_some() => Hint::Keys(say!("hint.table.in-a-cell"), vec![("Tab", say!("hint.table.next-cell")), ("S-Tab", say!("hint.table.previous-cell")), ("Esc", say!("hint.back-to-normal"))]),
            Mode::Normal if self.table_here() => {
                let grain = self.table.as_ref().map(|v| v.grain).unwrap_or(Grain::Cell);
                let markdown = self.md_region().is_some();
                match grain {
                    Grain::Cell if markdown => Hint::Keys(say!("label.table"), vec![
                            ("hjkl", say!("hint.table.by-cell")),
                            ("c d", say!("hint.table.change-or-clear-cell")),
                            ("y Y", say!("hint.table.yank-cell-or-row")),
                            ("p", say!("hint.paste")),
                            ("t", say!("hint.table.operations")),
                            ("t/ t?", say!("hint.table.who-uses-this")),
                            ("Tab", say!("hint.table.by-character-instead")),
                        ]),
                    Grain::Cell => Hint::Keys(say!("label.table"), vec![
                            ("hjkl", say!("hint.table.by-cell")),
                            ("c d", say!("hint.table.change-or-clear-cell")),
                            ("y Y", say!("hint.table.yank-cell-or-row")),
                            ("p", say!("hint.paste")),
                            ("t", say!("hint.table.operations")),
                            ("t/ t?", say!("hint.table.who-uses-this")),
                            ("gd gw", say!("hint.table.row-this-is-about")),
                            ("Tab", say!("hint.table.by-character-instead")),
                        ]),
                    Grain::Char => Hint::Keys(say!("hint.table.character-mode"), vec![
                            ("hjkl", say!("hint.table.by-character")),
                            ("t/ t?", say!("hint.table.who-uses-this-character")),
                            ("gd gw", say!("hint.table.row-this-character-is-about")),
                            ("Tab", say!("hint.table.by-cell-instead")),
                        ]),
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
                    .map(|n| Hint::Says(say!("hint.count-pending", n)));
            }
            Pending::Space => (
                say!("hint.space.title"),
                Self::SPACE_KEYS
                    .iter()
                    .map(|(key, what)| {
                        // Leaked once each, at most a dozen: the panel wants
                        // `&'static str` keys like every other row here, and a
                        // `char` is not one.
                        let key: &'static str = Box::leak(key.to_string().into_boxed_str());
                        (key, crate::messages::say(what, &[]))
                    })
                    .collect(),
            ),
            Pending::Goto => (say!("hint.goto.title"), Self::said(Self::GOTO_KEYS.iter().copied())),
            Pending::Find(_) => (say!("hint.find"), vec![("", say!("hint.type-a-character"))]),
            Pending::Replace => (say!("hint.overwrite"), vec![("", say!("hint.type-a-character-to-overwrite"))]),
            Pending::Case => (say!("hint.case.title"), Self::said(Self::CASE_KEYS.iter().copied())),
            Pending::Register => (say!("hint.register.title"), vec![("a–z", say!("hint.register.which-one"))]),
            Pending::Match => (say!("hint.match.title"), Self::said(Self::MATCH_KEYS.iter().copied())),
            Pending::MatchPair { .. } => (say!("hint.bracket"), vec![("", say!("hint.type-a-bracket-or-quote"))]),
            Pending::Surround => (say!("hint.match.surround"), vec![("", say!("hint.type-a-bracket"))]),
            Pending::SurroundFrom => (say!("hint.match.take-off"), vec![("", say!("hint.type-the-one-to-take-off"))]),
            Pending::SurroundTo(_) => (say!("hint.change-to"), vec![("", say!("hint.type-the-one-to-change-to"))]),
            Pending::Hop { forward } => (
                match forward {
                    true => say!("hint.hop.next"),
                    false => say!("hint.hop.previous"),
                },
                Self::said(Self::HOP_KEYS.iter().copied()),
            ),
            Pending::Conflict => (
                say!("hint.conflict.title"),
                Self::said(Self::CONFLICT_KEYS.iter().copied()),
            ),
            Pending::Mark => (say!("hint.mark.set-here"), vec![("a–z", say!("hint.mark.name-it"))]),
            Pending::Recall => (say!("hint.mark.go-back"), vec![("a–z", say!("hint.register.which-one"))]),
            // **Which list is a question about the cursor, not the mode.** It
            // used to be `md_region().is_none()`, which is *also* true of a
            // Markdown table nobody has opened yet — so standing in one of
            // 手冊's own tables offered the delimited file's keys.
            Pending::Table => {
                let inside = match self.table.as_ref().map(|v| v.bounds) {
                    Some(Bounds::Block) if self.block_region().is_some() => Some(Bounds::Block),
                    Some(Bounds::Md) if self.md_region().is_some() => Some(Bounds::Md),
                    Some(Bounds::WholeFile) => Some(Bounds::WholeFile),
                    _ => None,
                };
                (say!("hint.table.title"), Self::said(Self::table_keys(inside)))
            }
        };
        Some(Hint::Keys(keys.0, keys.1))
    }

    /// **What the half-pressed key can be finished with** — the which-key
    /// panel's whole content: a title, and each key with what it does.
    ///
    /// The same answer the hint row has always had; it is a panel now because a
    /// row holds four of these and `空格` has fourteen.
    pub fn pending_menu(&self) -> Option<(String, Vec<(&'static str, String)>)> {
        match self.pending_keys()? {
            Hint::Keys(title, keys) => Some((title, keys)),
            _ => None,
        }
    }

    /// **Where the cursor should sit on the page**, in rows from its top.
    ///
    /// One answer, asked by all three drawing surfaces — the horizontal page,
    /// the vertical one and the grid — because 「where does the cursor sit」 is
    /// one question and three copies of it is how two of them came to miss
    /// typewriter mode entirely.
    ///
    /// `distance` is how far the cursor is from the page's top, if it is on the
    /// page at all; `last` is the page's last row. `None` means 「leave the
    /// page where it is」.
    pub fn page_inset(&self, distance: Option<usize>, last: usize, scrolloff: usize) -> Option<usize> {
        // Typewriter: the row being written stays in the middle and the paper
        // moves under it. Every move is a jump, which is what that means.
        if self.typewriter || self.jumped {
            return Some(last / 2);
        }
        match distance {
            // On the page with room to spare: leave it alone.
            Some(d) if d >= scrolloff && d + scrolloff <= last => None,
            Some(d) if d < scrolloff => Some(scrolloff),
            Some(_) => Some(last.saturating_sub(scrolloff)),
            // Off the page altogether is a jump, and a jump lands in the
            // middle.
            None => Some(last / 2),
        }
    }

    /// Whether everything but the 段 being written stands back (焦點模式).
    pub fn focus(&self) -> bool {
        self.focus
    }

    /// Whether `:view meter` is on — the setting, which the status line reports.
    ///
    /// **Not the question the page asks.** A page wants
    /// [`Self::meter_drawn`]: with the 平仄 asked for and no 拆分表 to read
    /// them out of, the setting is on and every mark is empty.
    pub fn meter(&self) -> bool {
        self.meter
    }

    /// Whether the 平仄 will actually be drawn (Feature #247).
    ///
    /// The margin they go in is a **cell off every 縱 on the page**, bought
    /// before a line is asked for its marks. Bought on the setting alone, a
    /// `:view meter on` with no 拆分表 installed reflowed the whole page to make
    /// room for a column that can never hold anything — while the status line
    /// was busy saying there is no reading table. The command says so
    /// *instead* of drawing an empty margin, which is what it always claimed.
    pub fn meter_drawn(&self) -> bool {
        self.meter && self.reader.available()
    }

    /// Whether the cursor's row is kept in the middle of the page.
    pub fn typewriter(&self) -> bool {
        self.typewriter
    }

    /// How far one notch of the mouse wheel moves (Feature #222).
    pub fn wheel_step(&self) -> usize {
        self.wheel_step
    }

    /// Set how far one notch of the wheel moves. Zero is the terminal's own
    /// step — one unit a notch — not 「do not scroll」.
    pub fn set_wheel_step(&mut self, step: usize) {
        self.wheel_step = step.max(1);
    }

    /// How wide the detail panel should be, when it has been said.
    pub fn detail_width(&self) -> Option<usize> {
        self.detail_width
    }

    /// Whether the row of column numbers is drawn.
    pub fn table_numbers(&self) -> bool {
        self.table_numbers
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
        // **This** table's headings, not the schema's: standing in the second
        // table of a document, the status line named the first table's columns.
        let headings = self.table_headings();
        let name = headings
            .get(cell)
            .cloned()
            .unwrap_or_else(|| format!("+{}", cell + 1 - headings.len()));
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
        self.status = say!("table.yanked-cell", n);
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
        self.status = say!("table.yanked-row");
    }

    /// Put the register into the cell — or, if it is a whole row, below this one.
    ///
    /// Two things are worth pasting in a grid and they are told apart by what
    /// is in the register, not by a second key: a cell's worth of text replaces
    /// the cell, and a row's worth becomes a new row. Anything else — half a
    /// row, two cells — is refused, because there is no honest place to put it.
    fn put_cell(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let text = self.recall();
        if text.is_empty() {
            self.status = say!("edit.nothing-yanked-yet");
            return;
        }
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        let Some(view) = &self.table else { return };
        let (separator, columns) = (view.separator, view.schema.columns.len());
        let body = text.trim_end_matches(['\n', '\r']);
        // A block of cells — what a spreadsheet puts on the clipboard. It goes
        // in **at the cursor's cell**, filling right and down from there, which
        // is what every grid does with a pasted block and what a writer means
        // by it.
        if let Some(grid) = sniff_grid(body) {
            self.paste_grid(grid);
            return;
        }
        // A row: the right number of cells, and no line break left inside it.
        // A Markdown row says what it is by its own pipes, so it is recognised
        // by the same test that finds a table in the first place.
        let is_row = !body.contains(['\n', '\r'])
            && match separator {
                Separator::Pipe => crate::mdtable::is_row(body),
                Separator::Delimiter(d) => {
                    body.chars().filter(|&c| c == d).count() + 1 == columns && columns > 1
                }
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
                self.status = say!("table.pasted-as-new-row");
                return;
            }
            self.snapshot();
            let rope = self.current_buffer().rope();
            let at = motion::line_end(rope, self.cursor);
            let done =
                self.without_cell_guard(|e| e.current_buffer_mut().insert(at, &format!("\n{body}")));
            if !self.applied(done) {
                return;
            }
            self.set_cursor(at + 1);
            self.status = say!("table.pasted-as-new-row");
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
            self.status = say!("table.cell-replaced");
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
    /// Find every row whose 拆分 uses what is under the cursor.
    ///
    /// A search rather than a jump, because the answer is usually many rows:
    /// 卵 is a component of dozens of characters, and which of them you wanted
    /// is not a question the editor can answer. `n` and `N` walk the answers,
    /// as they walk the answers to `/`.
    fn search_columns_in(&mut self, columns: Option<Vec<usize>>) {
        let needle = self.what_is_here();
        if needle.trim().is_empty() {
            self.status = say!("table.cell-is-empty");
            return;
        }
        // The text, not a pattern — the same rule a search of the selection
        // follows.
        self.search_columns_within(&regex::escape(&needle), columns);
    }

    /// The question a table key is asking: the selection, or what the cursor is
    /// on by whichever unit `Tab` last chose.
    fn what_is_here(&self) -> String {
        let (from, to) = self.selection();
        if to > from + 1 {
            let rope = self.current_buffer().rope();
            return rope.slice(from..to.min(rope.len_chars())).to_string();
        }
        match self.table.as_ref().map(|v| v.grain) {
            Some(Grain::Char) => self.char_at_cursor().map(String::from).unwrap_or_default(),
            _ => self
                .cell_position()
                .map(|(line, cell)| self.cell_text(line, cell))
                .unwrap_or_default(),
        }
    }
    /// Search **down one column, then the next** (`:table find column`, `Enter`).
    ///
    /// The other axis of the same verb, and *only* the axis: a hit is a match,
    /// the match becomes the selection, `n` and `N` walk them, the pattern is a
    /// regular expression, and it wraps at the end. All of that is what `/`
    /// does. The one thing that differs is the order the page is read in —
    /// across a line and down, or down a column and across.
    ///
    /// Which columns: the ones a `[table.link] from` names, in the order it
    /// names them — that is what a schema is *for*, and on a 28-column table it
    /// is two columns instead of twenty-eight. With none named, all of them,
    /// from the first.
    fn search_columns(&mut self, pattern: &str) {
        self.search_columns_within(pattern, None)
    }

    /// The same, over the columns the sequence named — `t2-10?`.
    fn search_columns_within(&mut self, pattern: &str, named: Option<Vec<usize>>) {
        if !self.table_here() {
            self.status = say!("table.not-in-a-table");
            return;
        }
        let re = match self.compile(pattern) {
            Ok(re) => re,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        // **A column that is not there is said, not ignored** — `sort_table`'s
        // rule, and this key had the other habit: the span was clamped to the
        // width, so `t99/` searched the last column and answered as though
        // that were what had been asked for.
        let width = self.table_columns();
        if let Some(&n) = named.iter().flatten().find(|&&n| n == 0 || n > width) {
            self.status = say!("table.no-such-column", &n.to_string(), &width.to_string());
            return;
        }
        let asked = named.clone();
        let view = self.table.as_ref().expect("table_here");
        let declared: Option<Vec<usize>> = view.schema.link.as_ref().map(|link| {
            link.from
                .iter()
                .filter_map(|name| view.schema.index_of(name))
                .collect()
        });
        let total = view.schema.columns.len();
        let columns: Vec<usize> = match named {
            // Said outright: 1-based, as the reader counts them.
            Some(named) => {
                let mut columns: Vec<usize> = named.into_iter().map(|n| n - 1).collect();
                columns.dedup();
                columns
            }
            None => match &declared {
                Some(named) if !named.is_empty() => named.clone(),
                _ => (0..total).collect(),
            },
        };
        let anchored = pattern.contains('^') || pattern.contains('$');
        let separator = view.separator;
        let first = usize::from(view.schema.header);
        let region = self.prose_region();
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        // Down the first column, then down the second: the order is the whole
        // point. It is **not** the loop order, though — reading the file once
        // per column meant splitting all 123,380 rows of a 拆分表 twenty-eight
        // times over, which is three seconds of the same work. The rows are
        // read once and the hits are filed by column, which is where the order
        // actually comes from.
        let mut by_column: Vec<Vec<(usize, usize)>> = vec![Vec::new(); columns.len()];
        // Walked with the rope's own iterator, carrying the character offset
        // along: `line(n)` and `line_to_char(n)` are each a descent of the
        // tree, and a search that asks them 123,380 times has read the file
        // twice before it looks at anything.
        let mut line_start = rope.line_to_char(first);
        for (nth_line, slice) in rope.lines_at(first).enumerate() {
            let line = first + nth_line;
            if line > last {
                break;
            }
            let here = line_start;
            line_start += slice.len_chars();
            if region.as_ref().is_some_and(|r| !r.holds(line) || r.is_rule(line)) {
                continue;
            }
            // Borrowed while the rope keeps the row in one piece, which is the
            // ordinary case; copied only when it straddles a chunk boundary.
            let owned;
            let text: &str = match slice.as_str() {
                Some(text) => text,
                None => {
                    owned = slice.to_string();
                    &owned
                }
            };
            // A row with nothing in it anywhere has nothing in any of its
            // cells, and that is almost every row — so the row is only cut
            // into cells when it might pay. Not when the pattern is anchored:
            // `^木` asks about the start of a *cell*, and the row it sits in
            // need not start with it.
            if !anchored && !re.is_match(text) {
                continue;
            }
            let spans = match separator {
                Separator::Pipe => crate::mdtable::cells(text),
                Separator::Delimiter(d) => crate::table::cells(text, d),
            };
            let line_start = here;
            // The row's characters, once. `cell_text` walks the row from the
            // start for each cell it cuts, which over twenty-eight columns is
            // the row read twenty-eight times.
            let chars: Vec<char> = text.trim_end_matches(['\n', '\r']).chars().collect();
            for (nth, &column) in columns.iter().enumerate() {
                let Some(&span) = spans.get(column) else {
                    continue;
                };
                let cell: String = chars[span.0.min(chars.len())..span.1.min(chars.len())]
                    .iter()
                    .collect();
                let start = line_start + span.0;
                // Every match inside the cell, not one per cell: two hits on
                // one line are two hits for `/` too.
                for m in re.find_iter(&cell) {
                    let before = cell[..m.start()].chars().count();
                    let length = cell[m.start()..m.end()].chars().count();
                    by_column[nth].push((start + before, start + before + length));
                }
            }
        }
        let spans: Vec<(usize, usize)> = by_column.into_iter().flatten().collect();
        if spans.is_empty() {
            self.status = say!("find.not-found", pattern);
            self.hits = None;
            return;
        }
        self.last_search = pattern.to_string();
        let found = spans.len();
        // From the first column's first hit, whatever column you were standing
        // in: `Enter` on 卵 gives the same route through the table every time,
        // which is what 「把所有用到它的地方過一遍」 means.
        self.remember_hits(spans, 0);
        self.show_table_hit();
        // Only when nobody said which columns: a schema's `[table.link] from`,
        // or the number the reader just typed. `t2-10/` *is* saying so, and
        // being told 「你沒說範圍，從第一欄找起」 about the range you named is
        // the editor disagreeing with what it just did.
        match (declared.is_none(), asked.as_deref()) {
            (true, None) => {
                self.status = say!(
                    "search.no-jump-scope",
                    found
                );
            }
            (_, Some([n])) => self.status = say!("search.hit-in-column", *n, found),
            // A run and a handful of columns are different things and are said
            // differently: `t2-10/` names a range, `t1,5,9/` names three.
            (_, Some(named)) if !named.is_empty() && named.windows(2).all(|w| w[1] == w[0] + 1) => {
                let (a, b) = (named[0], named[named.len() - 1]);
                self.status = say!("search.hit-in-column-range", a, b, found);
            }
            (_, Some(named)) => {
                let listed: Vec<String> = named.iter().map(usize::to_string).collect();
                self.status = say!("search.hit-in-columns", listed.join(&say!("label.comma")), found);
            }
            (false, None) => {}
        }
    }

    /// Remember what a search found, and which document it found it in.
    fn remember_hits(&mut self, spans: Vec<(usize, usize)>, at: usize) {
        let buffer = self.current_buffer();
        self.hits = Some(Hits {
            buffer: buffer.id(),
            revision: buffer.revision(),
            spans,
            at,
        });
    }

    /// The hits, if they are still about the document in front of you.
    ///
    /// **The one gate.** A list found in another file, or before an edit, is
    /// not a shorter answer — it is a wrong one, and it used to be given
    /// confidently: 「第 3/78 處」 about a character that matched nothing, in a
    /// chapter that was never searched.
    fn live_hits(&self) -> Option<&Hits> {
        let buffer = self.current_buffer();
        self.hits
            .as_ref()
            .filter(|h| h.buffer == buffer.id() && h.revision == buffer.revision())
    }

    /// Whether `n` and `N` belong to a hit list rather than to `/`.
    fn walking_hits(&self) -> bool {
        self.live_hits().is_some_and(|h| !h.spans.is_empty())
    }

    /// Step to the next or previous match the column search found.
    fn walk_table_hits(&mut self, forward: bool) -> bool {
        let Some(hits) = self.live_hits() else {
            return false;
        };
        let n = hits.spans.len();
        if n == 0 {
            return false;
        }
        let at = match forward {
            true => (hits.at + 1) % n,
            false => (hits.at + n - 1) % n,
        };
        if let Some(hits) = self.hits.as_mut() {
            hits.at = at;
        }
        self.show_table_hit();
        true
    }

    /// Select the match the column search is pointing at, and say which it is.
    fn show_table_hit(&mut self) {
        let Some(hits) = self.live_hits() else {
            return;
        };
        let (at, found) = (hits.at, hits.spans.len());
        let Some(&(from, to)) = hits.spans.get(at) else {
            return;
        };
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();
        let to = to.min(len);
        let from = from.min(len);
        let head = motion::prev_grapheme(rope, to).max(from);
        let which = say!("search.hit-n-of-m", at + 1, found);
        // **Shown, not jumped to** (Feature #176). 卵's own row and a row that
        // uses 卵 are two places, and the question 「誰用了卵」 is about both
        // of them at once — so the hit opens in the other work area and the
        // cursor stays where it was standing. Nothing is remembered in the
        // jump list, because nothing was left.
        //
        // Except when the keys are already in the other pane: there the hits
        // are being walked by hand, and「給你看」 means moving the cursor.
        //
        // …and except when the reader asked for the other answer: `/` finds it
        // **here**, `?` shows it over there. One pair of letters, in the goto
        // family and the table family alike.
        if self.definition_preview && self.live_pane == 0 {
            let line = rope.char_to_line(from);
            let caption = say!(
                "show.table-hit",
                self.current_buffer().display_name(),
                line + 1,
                which
            );
            self.show_in_split(from, Some((from, to)), caption);
            self.status = which;
            return;
        }
        // The match itself becomes the selection, exactly as `/` leaves it —
        // on its last grapheme, not one past it.
        self.anchor = from;
        self.cursor = head;
        self.extend = false;
        self.refresh_goal_column();
        self.status = which;
    }

    /// Land on `line` — in the other work area, or here.
    ///
    /// **`gd` goes and `gw` shows**, which is the pair every editor has: `gd`
    /// is *go to definition* everywhere, and the peek is the second command
    /// (VS Code's Peek Definition, vim's `C-w }`). Going leaves a jump behind,
    /// so `C-o` comes back; showing leaves nothing, because nothing was left.
    fn land_on_row(&mut self, line: usize, preview: bool) {
        match preview {
            true => self.show_row(line),
            false => {
                self.remember_jump();
                self.goto_line(line + 1);
            }
        }
    }

    /// Show `line` in the other work area.
    fn show_row(&mut self, line: usize) {
        let rope = self.current_buffer().rope();
        let at = rope.line_to_char(line.min(rope.len_lines().saturating_sub(1)));
        let end = at + crate::zong::line_chars(rope, line).len();
        let caption = say!(
            "show.row-in-file",
            self.current_buffer().display_name(),
            line + 1
        );
        self.show_in_split(at, Some((at, end)), caption);
        self.status = say!("show.row", line + 1);
    }

    /// The hit the search is standing on, if there is one.
    ///
    /// What the renderer marks louder than the rest: on a long line a hit in
    /// the ordinary selection ground is easy to miss, and every editor's
    /// answer to that is to give the **current** match a mark of its own.
    pub fn current_hit(&self) -> Option<(usize, usize)> {
        let hits = self.live_hits()?;
        hits.spans.get(hits.at).copied()
    }

    /// Which line the other work area is showing, for tests and for the
    /// status line.
    pub fn peeked_line(&self) -> Option<usize> {
        let pane = self.other.as_ref()?;
        Some(self.current_buffer().rope().char_to_line(pane.cursor))
    }

    /// Whether the cursor sits at the first character of its cell.
    fn at_cell_start(&self) -> bool {
        match self.cell_position() {
            Some((line, cell)) => self.cell_span(line, cell).map(|(a, _)| a) == Some(self.caret()),
            None => false,
        }
    }

    /// **What divides this file into cells, whatever mode it is in.**
    ///
    /// A `Separator`: `Pipe` says that only the lines which are `|` table rows
    /// are cells, as in a document; a `Delimiter` says every line is a row, as
    /// in a `.csv`.
    ///
    /// The one answer every gate asks for. The gates used to open with
    /// `self.table.as_ref()?` and so were off whenever `:table` was — which is
    /// the state a table in a manuscript is normally edited in, and the state
    /// the 拆分表 is in whenever the project has no schema file. A `.csv` is a
    /// grid because of its own name; a `|` table is a grid because of what is
    /// written there.
    fn grid_shape_here(&self) -> Option<Separator> {
        if let Some(view) = self.table.as_ref() {
            return Some(view.separator);
        }
        let extension = self
            .current_buffer()
            .path()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "csv" => Some(Separator::Delimiter(',')),
            "tsv" | "tab" => Some(Separator::Delimiter('\t')),
            _ => Some(Separator::Pipe),
        }
    }

    /// Whether putting `text` where `span` is would change how many cells that
    /// row has.
    ///
    /// For the writers that replace a range outright rather than typing into
    /// it — a ruby reading is the one that reaches the rope past every gate —
    /// and, like every bulk check, it does not ask whether `:table` is on.
    fn replacement_reshapes_the_grid(
        &self,
        span: (usize, usize),
        text: &str,
    ) -> Option<String> {
        let separator = self.grid_shape_here()?;
        let rows_only = separator == Separator::Pipe;
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(span.0.min(rope.len_chars()));
        if rows_only {
            let here = self.line_text(line).unwrap_or_default();
            // A row inside a fence is writing *about* a table.
            if !crate::mdtable::is_row(&here) || self.block_of(line).is_literal() {
                return None;
            }
        }
        let was = rope
            .slice(span.0.min(rope.len_chars())..span.1.min(rope.len_chars()))
            .to_string();
        let cells = |s: &str| -> usize {
            match separator {
                Separator::Pipe => crate::mdtable::pipes_from(s, false).len(),
                Separator::Delimiter(d) => s.chars().filter(|&c| c == d).count(),
            }
        };
        let (before, after) = (cells(&was), cells(text));
        (before != after).then(|| {
            say!(
                "table.row-would-change-width",
                line + 1,
                before + 1,
                after + 1
            )
        })
    }

    /// Whether typing `c` into a cell would break the file.
    ///
    /// With no quoting, a delimiter inside a cell is not a delimiter inside a
    /// cell — it is one more column, and every column right of it shifts. The
    /// generator that reads this file back would take the damage silently, so
    /// the key is refused here, where it can still be explained.
    fn cell_refuses(&self, c: char) -> Option<String> {
        // The **view**, deliberately: typing a `|` is how a table is written in
        // the first place, so a document is a grid to this gate only once the
        // writer has said so. What is *rewritten in bulk* — `:s`, `:replace`,
        // `:ruby format`, `gJ` — asks [`Self::grid_shape_here`] instead, which
        // does not care whether `:table` is on.
        let view = self.table.as_ref()?;
        if !self.table_here() {
            return None;
        }
        if c == view.schema.delimiter {
            return Some(say!("cell.refuses-delimiter", c));
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
        region.is_rule(rope.char_to_line(self.caret().min(rope.len_chars())))
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
        if view.separator == Separator::Pipe {
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
            '\n' | '\r' => Some(say!("cell.refuses-newline")),
            '\t' => Some(say!("cell.refuses-tab")),
            _ => None,
        }
    }

    /// Whether a cut over `range` reaches into a grid at all.
    ///
    /// **Not `self.table.is_some()`.** A `.md` holding one table anywhere is
    /// opened with a grid view so its columns are drawn straight away
    /// (`table_on_open`), and asking only whether that view exists made every
    /// paragraph in the file refuse to give up its line break: `d` at the end
    /// of a sentence answered 「格與格之間的分隔符刪不掉」 about prose that has
    /// no cells in it. **A grid guards the lines it occupies and no others** —
    /// the same law `table_here()` states for the keys and
    /// `cell_refuses_text_at` already kept for the other half of the edit.
    ///
    /// The two ends are what is asked, not every line between them: only a row
    /// cut *part* way can lose a delimiter, and a cut that swallows whole rows
    /// takes their delimiters with them and leaves every surviving row intact.
    fn cut_reaches_a_grid(&self, range: &std::ops::Range<usize>) -> bool {
        match self.table.as_ref().map(|v| v.bounds) {
            None => false,
            Some(Bounds::WholeFile) => true,
            // The level is one word for two halves — see `table_here()`.
            Some(Bounds::Md) if !self.table_padding_on() => false,
            Some(_) => {
                let rope = self.current_buffer().rope();
                let last = rope.len_chars();
                let head = rope.char_to_line(range.start.min(last));
                // The end is exclusive: a cut that stops at the head of a row
                // has not touched that row.
                let tail = rope
                    .char_to_line(range.end.saturating_sub(1).max(range.start).min(last));
                self.prose_region_at(head).is_some() || self.prose_region_at(tail).is_some()
            }
        }
    }

    /// The reason this range may not be cut out, if there is one.
    fn cell_refuses_cut(&self, range: std::ops::Range<usize>) -> Option<String> {
        if self.table_bypass.get() || !self.cut_reaches_a_grid(&range) {
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
        if self.table.as_ref().map(|v| v.separator) == Some(Separator::Pipe) {
            let escaped = self.backslash_before(range.start);
            let text = rope.slice(range).to_string();
            return (crate::mdtable::has_bare_pipe(&text, escaped)
                || text.contains(['\n', '\r']))
            .then(|| "格與格之間的分隔符刪不掉".to_string());
        }
        rope.slice(range)
            .chars()
            .find_map(|c| self.cell_refuses(c))
            .map(|_| "格與格之間的分隔符刪不掉".to_string())
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
        // The width is **this** table's (#283) — `o` in the second table of a
        // document was opening a row as wide as the first one.
        let columns = self.table_column_count_at(self.cursor_line());
        match &self.table {
            Some(view) => match view.separator {
                Separator::Pipe => crate::mdtable::blank_row(columns),
                Separator::Delimiter(d) => d.to_string().repeat(columns.saturating_sub(1)),
            },
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
            say!("ui.detail-panel-on")
        } else {
            say!("ui.detail-panel-off")
        };
    }

    /// What the detail panel should show, if anything.
    ///
    /// One panel, one question — "what is here?" — asked of whatever the
    /// cursor is in. A table row answers with its fields; other things will
    /// answer with theirs. The editor works out *what* to say; the front end
    /// decides where to put it.
    pub fn detail(&self) -> Option<Detail> {
        // **Whatever the cursor is standing in answers** (#283). It used to
        // be whatever the *file* was: a `Bounds::Md` view sent every question
        // to the note panel, so `t i` inside a Markdown table — a table key,
        // pressed in a table — answered 「這裏沒有註」 and the row panel could
        // only ever be reached by opening a `.csv`.
        //
        // The old reasoning was that a Markdown table is a page of a document
        // and the document's question is the one worth asking. That is right
        // in the *paragraph*, which is why the note panel still answers there
        // — and wrong in the row, the more so since a cell wide enough to be
        // folded away is one this panel is now the way to read whole.
        match self.in_a_table_row() {
            true => self.row_detail().or_else(|| self.note_detail()),
            false => self.note_detail(),
        }
    }

    /// Whether the panel is about to show a **row** rather than a note.
    ///
    /// The two want different shapes, and the shape used to be picked from
    /// whether the table had taken the window (#283): a row in a Markdown
    /// document got the note's four-line strip along the bottom, so a
    /// five-column row showed two of its fields and a 拆分表 row showed two
    /// of twenty-eight. What decides is what the panel *holds* — a row is a
    /// tall thing wherever it is written.
    pub fn detail_shows_a_row(&self) -> bool {
        self.in_a_table_row() && self.row_detail().is_some()
    }

    /// Whether the cursor is standing in a row a table panel can read.
    ///
    /// Not the header and not the `|---|` — neither is a row, and both would
    /// otherwise be shown as one with every field empty.
    fn in_a_table_row(&self) -> bool {
        let Some(view) = self.table.as_ref() else {
            return false;
        };
        let line = self.cursor_line();
        match view.bounds {
            Bounds::WholeFile => true,
            _ => match self.prose_region() {
                Some(region) => {
                    region.holds(line) && line != region.first && Some(line) != region.rule
                }
                None => false,
            },
        }
    }

    /// The schema of the table the cursor is **in**, not the file's (#283).
    ///
    /// A `.csv` has one schema and it is the view's. A Markdown document has
    /// as many tables as somebody typed, each with its own header, and the
    /// view carries the *first* one's — that is what made `t y` in the second
    /// table of a file report the first table's column name. Every question
    /// about columns asks this instead, and it is worked out from the header
    /// row that is actually above the cursor.
    fn schema_here(&self) -> Option<std::borrow::Cow<'_, crate::table::Schema>> {
        use std::borrow::Cow;
        let view = self.table.as_ref()?;
        match view.bounds {
            // **Only a `|` table has a header to read back.** A guessed block
            // is walked out afresh every time the cursor enters one
            // (`Reach::Cursor`), so the view's numbered schema is already this
            // block's — while reading its first line as a Markdown header
            // split a tab-delimited row on pipes and called the whole thing one
            // column, which is what emptied the panel over a 碼表.
            Bounds::Md => {
                let region = self.prose_region()?;
                let header = self.line_text(region.first)?;
                Some(Cow::Owned(crate::mdtable::schema(&header)))
            }
            _ => Some(Cow::Borrowed(&view.schema)),
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
        let within = self.caret() - rope.line_to_char(line);
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
                title: say!("detail.comment"),
                here: String::new(),
                rows: vec![(String::new(), Some(text.trim_matches('%').trim().to_string()))],
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
                    rows: vec![(String::new(), Some(body))],
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
        // A reference with no note is the ordinary way a note gets written:
        // you type `[^1]` in the sentence and then need somewhere to put it.
        if let Some(tag) = self.note_tag_at_cursor() {
            if self.footnote_body(&tag).is_none() {
                self.write_note(&tag);
                return;
            }
        }
        let Some(detail) = self.note_detail() else {
            // Not on a note, so `Enter` means what it means everywhere else:
            // 「這個詞還在哪裏」 — the same previewing search a table's key
            // column answers, with the word under the cursor as the question.
            self.search_the_page();
            return;
        };
        let Some(&(_, Some(at))) = detail.links.first() else {
            self.status = say!("note.points-nowhere");
            return;
        };
        if at == self.cursor_line() {
            self.status = say!("note.already-on-this-line");
            return;
        }
        let preview = self.definition_preview;
        self.land_on_row(at, preview);
    }

    /// `gd`: **what is this?** — the note, or the row a component names.
    ///
    /// The other half of the pair `Enter` is one half of. Shown in the other
    /// work area like everything else, and on a footnote reference that has no
    /// note yet it **writes the note** and shows that: following a link to a
    /// page that does not exist is how one gets written, which is what every
    /// wiki-shaped editor does and what a writer typing `[^1]` means.
    fn show_definition(&mut self, preview: bool) {
        self.definition_preview = preview;
        // **In a grid, `gd` is one question with one answer**: which row has
        // *this* in the column that names rows. Standing on 木 in a 拆分 cell,
        // 木's own row; standing on 木 in the key column, the same row, which
        // is where you already are — and that is not a disappointment, it is
        // the question answering itself.
        //
        // It used to be two questions decided by which column the cursor was
        // in — follow the link here, search for who uses it there — and 「誰用
        // 了它」 is what `Enter` is for. One key, one meaning.
        if self.table_here() {
            let span = self.column_span.take();
            self.go_to_the_row_named(span, preview);
            return;
        }
        self.follow_note();
    }

    /// The row whose cell in the named column is **exactly** what is here.
    ///
    /// `gd` searches the key column — the one a schema names as what its rows
    /// are *about* — or the first, if none is named. `3gd` searches column
    /// three; `2-5gd` searches columns two through five, which is how a 拆分表
    /// with four spellings of the same decomposition is asked one question.
    fn go_to_the_row_named(&mut self, span: Option<(usize, usize)>, preview: bool) {
        let Some(view) = self.table.as_ref() else {
            return;
        };
        // **How many columns this table has** (#283): `3gd` in the wider of two
        // tables was clamped to the narrower one's count.
        let columns = self
            .schema_here()
            .map(|s| s.columns.len())
            .unwrap_or_default()
            .max(1);
        // What is being looked up: the selection when there is one, else what
        // the cursor is on — by character or by cell, following `Tab`, which is
        // the same unit `hjkl` move by.
        // **More than the caret's own character.** Every motion here leaves a
        // selection — that is the editing model — so 「is something selected」
        // is not `to > from`, which is true of standing still.
        let (from, to) = self.selection();
        let needle = match to > from + 1 {
            true => self
                .current_buffer()
                .rope()
                .slice(from..to.min(self.current_buffer().rope().len_chars()))
                .to_string(),
            false => match view.grain {
                Grain::Char => self.char_at_cursor().map(String::from).unwrap_or_default(),
                _ => self
                    .cell_position()
                    .map(|(line, cell)| self.cell_text(line, cell))
                    .unwrap_or_default(),
            },
        };
        let needle = needle.trim().to_string();
        if needle.is_empty() {
            self.status = say!("table.cell-is-empty");
            return;
        }
        // ⿰⿱⿲ say how the components are arranged. There is nowhere to go
        // from one, and 「表裏沒有⿰」 is the wrong thing to say about it — no
        // table has a row for a piece of grammar.
        if let Some(c) = needle.chars().next() {
            if needle.chars().count() == 1 && is_ids_operator(c) {
                self.status = say!("chaifen.descriptor-not-component", c);
                return;
            }
        }
        // The columns to look in, 1-based as the reader counts them.
        let (first, last) = match span {
            Some((a, b)) => (a.min(b), a.max(b)),
            None => {
                let key = view
                    .schema
                    .link
                    .as_ref()
                    .and_then(|link| view.schema.index_of(&link.to))
                    .unwrap_or(0)
                    + 1;
                (key, key)
            }
        };
        let (first, last) = (first.clamp(1, columns) - 1, last.clamp(1, columns) - 1);
        let here = self.schema_here();
        let named: Vec<String> = (first..=last)
            .filter_map(|c| {
                here.as_ref()
                    .and_then(|s| s.columns.get(c))
                    .map(|col| col.name.clone())
            })
            .collect();
        let rows = self.current_buffer().line_count();
        let mut found = Vec::new();
        for line in 0..rows {
            for cell in first..=last {
                if self.cell_text(line, cell).trim() == needle {
                    found.push((line, cell));
                    break;
                }
            }
        }
        let which = match named.len() {
            1 => named.first().cloned().unwrap_or_default(),
            _ => say!("chaifen.column-range", first + 1, last + 1),
        };
        match found.len() {
            0 => self.status = say!("chaifen.no-such-row-in", which, needle),
            _ => {
                let (line, cell) = found[0];
                match preview {
                    true => self.show_row(line),
                    false => {
                        self.remember_jump();
                        self.goto_line(line + 1);
                        self.go_to_cell(line, cell);
                    }
                }
                self.status = match found.len() {
                    1 => say!("find.file-and-message", which, needle),
                    n => say!("chaifen.row-found", which, needle, n),
                };
            }
        }
    }

    /// `Enter` on prose: **who else says this?**
    ///
    /// One key, one meaning, in a table and out of it: 「在另一個工作區給我看
    /// 這個詞還出現在哪裏」. The selection is the question when there is one —
    /// so a phrase is asked about by selecting it — and the word under the
    /// cursor when there is not, which is what `w` would have taken.
    fn search_the_page(&mut self) {
        let rope = self.current_buffer().rope();
        let (from, to) = self.selection();
        let needle = match to > from {
            true => rope.slice(from..to.min(rope.len_chars())).to_string(),
            false => {
                let line = rope.char_to_line(self.cursor);
                let start = rope.line_to_char(line);
                let chars = crate::zong::line_chars(rope, line);
                let at = self.cursor - start;
                let words = self.segment_line(line);
                match words.iter().find(|&&(a, b)| at >= a && at < b) {
                    Some(&(a, b)) => chars[a..b.min(chars.len())].iter().collect(),
                    None => chars.get(at).map(|c| c.to_string()).unwrap_or_default(),
                }
            }
        };
        let needle = needle.trim().to_string();
        if needle.is_empty() {
            self.status = say!("find.nothing-here-to-look-for");
            return;
        }
        let spans = self.every_match(&regex::escape(&needle));
        if spans.len() <= 1 {
            self.status = say!("find.only-here", needle);
            self.hits = None;
            return;
        }
        self.last_search = regex::escape(&needle);
        // The first one *after* where you are standing: the useful answer to
        // 「還在哪裏」 is the next place, not the first page of the book.
        let here = self.cursor;
        let at = spans
            .iter()
            .position(|&(from, _)| from > here)
            .unwrap_or(0);
        self.remember_hits(spans, at);
        self.show_table_hit();
    }

    /// Every match of `pattern` in the buffer, as character ranges.
    fn every_match(&self, pattern: &str) -> Vec<(usize, usize)> {
        let Ok(re) = self.compile(pattern) else {
            return Vec::new();
        };
        let rope = self.current_buffer().rope();
        let mut hits = Vec::new();
        let mut at = 0usize;
        for line in 0..rope.len_lines() {
            let text = rope.line(line).to_string();
            let start = at;
            at += rope.line(line).len_chars();
            for m in re.find_iter(&text) {
                let before = text[..m.start()].chars().count();
                let length = text[m.start()..m.end()].chars().count();
                hits.push((start + before, start + before + length));
            }
        }
        hits
    }

    /// The footnote reference the cursor is standing in, if it is in one.
    fn note_tag_at_cursor(&self) -> Option<String> {
        if self.current_buffer().syntax() != crate::syntax::Syntax::Markdown {
            return None;
        }
        let rope = self.current_buffer().rope();
        let line = self.cursor_line();
        let within = self.caret() - rope.line_to_char(line);
        let block = self.blocks_through(line).get(line).copied().unwrap_or_default();
        let runs = self.markup_line_in(line, block);
        let span = runs.iter().find(|s| {
            s.kind == crate::markdown::Kind::Footnote && within >= s.start && within < s.end
        })?;
        let text: String = rope
            .line(line)
            .chars()
            .skip(span.start)
            .take(span.end - span.start)
            .collect();
        let tag = text.trim_end_matches(':').to_string();
        // The *definition* is not a reference: standing on `[^1]:` there is
        // nothing to go to — you are already there.
        match text.ends_with(':') {
            true => None,
            false => Some(tag),
        }
    }

    /// Write the note for `tag` at the foot of the file, and show it.
    fn write_note(&mut self, tag: &str) {
        let rope = self.current_buffer().rope();
        let end = rope.len_chars();
        let text = rope.to_string();
        // One blank line between the manuscript and its notes, and none added
        // when the file already ends with one.
        let lead = match text.ends_with("\n\n") {
            true => String::new(),
            false => match text.ends_with('\n') {
                true => "\n".to_string(),
                false => "\n\n".to_string(),
            },
        };
        let note = format!("{lead}{tag}: ");
        // The same gate every other writer passes: a note appended to a grid
        // gives it two one-column rows. `:markdown footnote` reaches this from
        // a key now, so「表格裏不寫註」 has to be said here rather than assumed.
        if let Some(why) = self.replacement_reshapes_the_grid((end, end), &note) {
            self.status = why;
            return;
        }
        if self.table_here() {
            self.status = say!("note.not-in-a-grid");
            return;
        }
        self.snapshot();
        self.write_the_note(end, &note, tag);
    }

    /// The note itself, with the undo point already taken.
    ///
    /// **One edit, one `u`.** `:markdown footnote` writes the tag *and* the
    /// note, and two snapshots left a `[^1]` pointing at nothing after a single
    /// undo — so the caller takes the one snapshot that covers both.
    fn write_the_note(&mut self, end: usize, note: &str, tag: &str) {
        if self.refuse_readonly() {
            return;
        }
        let at = end;
        let done = self.current_buffer_mut().insert(at, note);
        if !self.applied(done) {
            return;
        }
        // The other area is opened **at the end of the stub**, not at the head
        // of its line: 空格 w lands where the note is going to be typed, which
        // is the only place anybody is going next.
        let caret = at + note.chars().count();
        let line = self.current_buffer().rope().char_to_line(caret);
        match self.definition_preview {
            true => {
                let caption = say!(
                    "show.row-in-file",
                    self.current_buffer().display_name(),
                    line + 1
                );
                self.show_in_split(caret, None, caption);
                self.status = say!("note.written-other-pane", tag);
            }
            // `gd` goes, and a stub is written to be typed into, so it lands
            // at the end of it with Insert one keystroke away.
            false => {
                self.remember_jump();
                self.set_cursor(caret);
                self.status = say!("note.written-same-pane", tag);
            }
        }
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
        let schema = self.schema_here()?;
        let (line, cell) = self.cell_position()?;
        // The header names the columns; it is not a row and has no fields.
        // In prose the header is wherever the table starts, and the `|---|`
        // under it is not a row either — [`Self::in_a_table_row`] knows both,
        // and it is the same question.
        if view.bounds == Bounds::WholeFile {
            if schema.header && line == 0 {
                return None;
            }
        } else if !self.in_a_table_row() {
            return None;
        }
        // Nor is the empty line a file ending in a newline leaves behind — the
        // same thing `row_is_ragged` already knows not to complain about.
        let rope = self.current_buffer().rope();
        if line + 1 == rope.len_lines() && rope.line(line).len_chars() == 0 {
            return None;
        }
        let text = self.current_buffer().rope().line(line).to_string();
        // **The view splits the row, not the delimiter** (#283). A `|` row
        // begins and ends with the separator, so splitting it on the character
        // gives an empty cell at each end — and the panel then answered every
        // question one column to the left, while `cell_position`, which asks
        // the view, was pointing one column to the right.
        let spans = self.row_cells(line);
        let value = |name: &str| -> String {
            schema
                .index_of(name)
                .and_then(|i| spans.get(i))
                .map(|&s| crate::table::cell_text(&text, s))
                .unwrap_or_default()
        };
        // Titled by the row's key, since that is what a person calls the row.
        let title = match &schema.key {
            Some(key) => value(key),
            None => format!("{}", line + 1),
        };
        // The field the cursor is in is shown even when it is empty: that it
        // *is* empty is the answer to "what is in this cell".
        // **Numbered the same way the rows are**, or the panel can never find
        // the field the cursor is in: it compared 「unicode」 against 「 9
        // unicode」, never matched, and so never scrolled to it and never lit
        // it — both of the things it promises.
        let here_name = schema
            .columns
            .get(cell)
            .map(|c| format!("{:>2} {}", cell + 1, c.heading()))
            .unwrap_or_default();
        let mut rows: Vec<(String, Option<String>)> = schema
            .columns
            .iter()
            .enumerate()
            .filter(|(i, column)| !column.hidden || spans.get(*i).is_some())
            .map(|(i, column)| {
                // `None` when the row has no such field — a short row, which
                // the grid already marks as ragged. An empty field is
                // `Some("")`, and they are different answers to 「這一格有什麼」.
                let text = spans.get(i).map(|&s| crate::table::cell_text(&text, s));
                // **Numbered**, because the keys count columns: `3gd` looks in
                // the third, `t20,20g` goes to a cell by number, and the panel
                // is where a reader finds out which number a field is without
                // counting along the header.
                (format!("{:>2} {}", i + 1, column.heading()), text)
            })
            // **Every column, empty ones included** — an empty field *is* a
            // finding in a 拆分表, and a panel that leaves it out is a panel
            // that cannot answer 「這一格是不是空的」. They were hidden because
            // twenty-three blanks pushed the 部件 list off the bottom; the
            // panel scrolls to the field the cursor is in, so there is
            // somewhere for them to go — and `t20,20g` reaches any of them by
            // number, which is what the numbers are for.
            .collect();
        // Worked out, not stored — and marked as such, so nobody goes looking
        // for a column that is not in the file.
        for detail in &schema.details {
            let from = value(detail.compute.column());
            rows.push((
                format!("{}*", detail.name),
                Some(detail.compute.apply(&from, &schema.ranges)),
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
        let Some(link) = &view.schema.link else {
            return Vec::new();
        };
        let Some((line, cell)) = self.cell_position() else {
            return Vec::new();
        };
        let Some(column) = view.schema.columns.get(cell) else {
            return Vec::new();
        };
        if !link.from.contains(&column.name) {
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

    // ---- 簡繁, handed to opencc (Feature #241) ----------------------------

    /// `:convert` — every shape of it.
    fn convert(&mut self, ask: crate::command::ConvertAsk) {
        use crate::command::ConvertAsk;
        use crate::convert::{self, Snag};
        match ask {
            ConvertAsk::Explain => self.explain_convert(),
            ConvertAsk::Opencc { update } => {
                let line = match update {
                    true => convert::install_line().replace("install", "upgrade"),
                    false => convert::install_line().to_string(),
                };
                // Only where one command does it without a password. Elsewhere
                // the line is *said*, because a full-screen terminal
                // application is the wrong place to be asked for a sudo
                // password — and because which package manager this machine
                // has is not something the editor should guess at and then run.
                if !convert::install_runs_here() {
                    self.status = say!("convert.install-it-yourself", line);
                    return;
                }
                self.status = say!("convert.installing", line);
                self.shell_request = Some(Shell {
                    line,
                    how: How::Terminal,
                });
            }
            ConvertAsk::Run { from, to, force } => {
                let plan = match convert::plan(from, to, force) {
                    Ok(plan) => plan,
                    Err(Snag::Same) => {
                        self.status = say!("convert.same-side", from.word(), to.word());
                        return;
                    }
                    Err(Snag::NoRoute) => {
                        self.status = say!("convert.no-route", from.word(), to.word());
                        return;
                    }
                    Err(Snag::NoWords) => {
                        self.status = say!("convert.no-force", from.word(), to.word());
                        return;
                    }
                };
                // The way out of 通規 happens here rather than in the child:
                // opencc's configs cannot read those 字形, so what it is handed
                // has to be 字形 it knows.
                let mut text = self.current_buffer().rope().to_string();
                if let Some(side) = plan.unpatch {
                    text = convert::unrespell(&text, side);
                }
                let Some(config) = plan.config else {
                    // 繁體 to 繁體: 字形 tables and nothing else, so there is
                    // no program to be missing and no process to spawn.
                    if let Some(side) = plan.patch {
                        text = convert::respell(&text, side);
                    }
                    self.rewrite_with_conversion(&text);
                    return;
                };
                let Some(program) = convert::opencc() else {
                    self.status = say!("convert.opencc-needed", convert::install_line());
                    return;
                };
                self.convert_patch = plan.patch;
                self.shell_request = Some(Shell {
                    line: convert::command_line(&program, config),
                    how: How::Convert(text),
                });
            }
        }
    }

    /// What `:convert` on its own answers: what it can do, and whether the
    /// program it needs is here.
    fn explain_convert(&mut self) {
        use crate::convert::{self, Side};
        let mut listing = String::new();
        listing.push_str(&match convert::opencc() {
            Some(path) => say!("convert.opencc-at", path.display()),
            None => say!("convert.opencc-needed", convert::install_line()),
        });
        listing.push_str("\n\n");
        for side in Side::ALL {
            listing.push_str(&match side {
                Side::S => say!("convert.side-s"),
                Side::T => say!("convert.side-t"),
                Side::Tw => say!("convert.side-tw"),
                Side::Hk => say!("convert.side-hk"),
                Side::Jp => say!("convert.side-jp"),
                Side::C => say!("convert.side-c"),
                Side::G => say!("convert.side-g"),
            });
            listing.push('\n');
        }
        listing.push('\n');
        for side in Side::ALL {
            let there = convert::destinations(side);
            if there.is_empty() {
                continue;
            }
            let words: Vec<String> = there
                .iter()
                .map(|(to, force)| match force {
                    true => format!("{} [force]", to.word()),
                    false => to.word().to_string(),
                })
                .collect();
            // The tags are one and two letters wide, so the arrows line up
            // only if the short ones are padded — a column that wobbles by one
            // cell reads as a mistake in a page whose whole job is a table.
            listing.push_str(&say!(
                "convert.can-go",
                format!("{:<2}", side.word()),
                words.join("  ")
            ));
            listing.push('\n');
        }
        listing.push('\n');
        listing.push_str(&say!("convert.force-means"));
        listing.push('\n');
        listing.push_str(&say!("convert.undo-with-u"));
        listing.push('\n');
        self.show_listing(listing, say!("convert.what-it-can-do"));
        self.status = say!("convert.pick-two");
    }

    /// What opencc said, patched and put in the buffer.
    pub fn provide_conversion(&mut self, output: &str) {
        let patched = match self.convert_patch.take() {
            Some(side) => crate::convert::respell(output, side),
            None => output.to_string(),
        };
        // opencc answers an empty file with an empty file and exit 0, so
        // 「nothing came back」 is only a finding when something went in.
        if patched.is_empty() && self.current_buffer().char_count() > 0 {
            self.status = say!("convert.nothing-came-back");
            return;
        }
        self.rewrite_with_conversion(&patched);
    }

    /// Put a converted manuscript in place of the one that was there.
    ///
    /// One `snapshot`, so one `u` takes the whole book back. That matters more
    /// here than in any other command: the reverse direction is **not** the way
    /// back — `s2tw` then `tw2s` returns 头发 for 頭髮 and 里 for both 裡 and
    /// 裏 — so undo is the only thing that undoes this.
    fn rewrite_with_conversion(&mut self, text: &str) {
        let before = self.current_buffer().rope().to_string();
        if before == text {
            self.status = say!("convert.not-a-character");
            return;
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(0..len, text));
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
        self.forget_the_text();
        let (was, now) = (before.chars().count(), text.chars().count());
        self.status = match was == now {
            // The ordinary case, and the one worth a number: every 簡繁 step
            // but `force` is one character for one, so 「how many changed」 is
            // exactly how much of the book moved.
            true => say!(
                "convert.characters-changed",
                before
                    .chars()
                    .zip(text.chars())
                    .filter(|(a, b)| a != b)
                    .count()
            ),
            false => say!("convert.length-changed", was, now),
        };
    }

    /// `:diff [檔名]` — what changed, by 詞 (Feature #235).
    ///
    /// **The grain is the point.** Every diff a writer can reach is a line
    /// diff, and a Chinese paragraph is one line: move one 的 in a 五百字 段落
    /// and `git diff` paints the whole paragraph away and back again. The
    /// answer is true and useless, because the one thing being asked — *which
    /// word* — is buried in five hundred characters. So this one runs over the
    /// 詞 the editor's own segmenter finds, the same ones `w` steps over, and
    /// answers with the line as it stands now and `[-走了-]{+來了+}` in it.
    ///
    /// **What it compares against.** With no argument, the file on disk: 「這
    /// 一坐下來我改了什麼」, which is the question with the shortest half-life.
    /// With a path, that file — yesterday's chapter, a copy kept before a
    /// rewrite. Not the recovery copy: [`crate::buffer::Buffer::write_swap`]
    /// overwrites one path with the text as it stands, so it is never a base
    /// to compare against.
    fn diff_against(&mut self, other: Option<&str>) {
        let here = self
            .current_buffer()
            .path()
            .map(Path::to_path_buf);
        let path = match other {
            Some(given) => {
                let given = Path::new(given.trim());
                match (given.is_absolute(), here.as_ref().and_then(|p| p.parent())) {
                    (false, Some(dir)) => dir.join(given),
                    _ => given.to_path_buf(),
                }
            }
            None => match here {
                Some(path) => path,
                None => {
                    self.status = say!("diff.no-file");
                    return;
                }
            },
        };
        let old = match std::fs::read_to_string(&path) {
            // Same byte-order mark `Buffer::decode` drops on the way in. Left
            // on, it is a token of its own on line 1 and every file a Windows
            // editor has touched reports a change it does not have.
            Ok(text) => text.strip_prefix('\u{feff}').unwrap_or(&text).to_string(),
            Err(err) => {
                self.status = say!("buffer.cannot-open", path.display(), err.to_string());
                return;
            }
        };
        let new = self.current_buffer().rope().to_string();
        // The editor's segmenter, handed one line at a time — which is how its
        // own cache is keyed, so most of these lines are already answered.
        let segment = |line: &str| self.segmenter.segment(line);
        let (a, b) = (
            crate::diff::tokens(&old, &segment),
            crate::diff::tokens(&new, &segment),
        );
        let Some(ops) = crate::diff::script(&a, &b) else {
            self.status = say!("diff.too-far", path.display());
            return;
        };
        let changes = crate::diff::line_changes(&ops);
        let against = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        if changes.is_empty() {
            self.status = say!("diff.same", against);
            return;
        }
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name().to_string());
        let n = changes.len();
        let mut listing = String::new();
        for change in changes.iter().take(GREP_LIMIT) {
            listing.push_str(&say!("diff.line", name, change.line + 1, change.marked));
            listing.push('\n');
        }
        self.show_listing(listing, say!("diff.results", name, against));
        // The count and the listing have to agree: saying "1,200 lines differ"
        // over a buffer holding 500 of them sends the reader looking for rows
        // that were never written.
        self.status = match n > GREP_LIMIT {
            true => say!("diff.too-many", GREP_LIMIT, against),
            false => say!("diff.changed", n, against),
        };
    }

    /// Go to the row this table names by `key` (`:table jump 木`).
    ///
    /// The index behind it has always been built and has always answered in
    /// about 300 ns; until now nothing let a person ask it. Finding 木 in a
    /// 123,380-row table meant `/^木,` and hoping no other row started that
    /// way.
    fn goto_row(&mut self, key: &str) {
        let Some(view) = self.table.as_ref() else {
            self.status = say!("table.not-in-a-table");
            return;
        };
        if view.schema.link.is_none() && view.schema.key.is_none() {
            self.status = say!("table.schema-has-no-key-column");
            return;
        }
        let mut chars = key.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            self.status = say!("table.row-name-not-one-char", key);
            return;
        };
        match self.row_named(c) {
            Some(line) => {
                self.remember_jump();
                self.move_to_line(line + 1);
                self.snap_to_cell();
                self.status = say!("table.row-found-at-line", c, line + 1);
            }
            None => self.status = say!("table.no-such-row-name", c),
        }
    }

    /// Which buffer holds this id, if any is still open.
    fn buffer_with(&self, id: u64) -> Option<usize> {
        self.buffers.iter().position(|b| b.id() == id)
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
        let link = view.schema.link.as_ref()?;
        let at = view.schema.index_of(&link.to)?;
        let rope_lines = self.current_buffer().line_count();
        let want = (
            self.current_buffer().id(),
            self.current_buffer().revision(),
            rope_lines,
        );
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
        let Some(link) = &view.schema.link else {
            return false;
        };
        match (self.cell_position(), view.schema.index_of(&link.to)) {
            (Some((_, cell)), Some(key)) => cell == key,
            _ => false,
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
        //
        // **Whichever surface it is drawn on** (#275): 真表格顯示 turns the
        // page for a table in the middle of a chapter too — 「照舊把整頁轉橫」
        // — so while a grid is on the screen, vertical is refused wherever the
        // grid sits. 表格操作 (`t n`) leaves the pipes on the page and does not
        // ask for the turn, so it is not this case; `t n` and `t q` are what
        // give the manuscript back, and both restore the layout the grid took.
        if layout == Layout::Vertical && self.grid_is_drawn() {
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

    /// The paper a printed copy is set on, in whole millimetres.
    pub fn paper(&self) -> crate::export::Paper {
        self.paper
    }

    /// Set the paper `:export html` writes its print stylesheet for.
    ///
    /// A degenerate trim is refused rather than clamped: it can only come from
    /// a configuration line, and silently printing a book on paper the writer
    /// did not ask for is worse than printing it on A5.
    pub fn set_paper(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.paper = crate::export::Paper { width, height };
        }
    }

    /// Set the 縱 wrap length. The renderer calls this once the terminal size is
    /// known, so motion and drawing agree on where the 縱 break.
    pub fn set_zong_length(&mut self, length: usize) {
        self.zong_length = length.max(1);
    }

    /// How this buffer is gridded into 縱 — the wrap length plus whether ruby is
    /// laid out. Every 縱 question takes this, so the cursor and the page can
    /// never disagree about where a row begins.
    /// The 縱 grid, **handed the same page the horizontal side is handed**.
    ///
    /// The two closures are the whole point of the shape: `hidden` is
    /// [`Self::markup_hidden_on_line`] — which knows the file's syntax, which
    /// block each line is in, and what the selection is holding open — and
    /// `folded` is [`Self::line_is_folded`], the one fold rule. They are
    /// passed in by the caller exactly as [`crate::wrap::Measure`]'s are,
    /// because a borrow cannot outlive the call that made it.
    pub fn grid_with<'a>(
        &self,
        hidden: &'a dyn Fn(usize) -> Vec<(usize, usize)>,
        folded: &'a dyn Fn(usize) -> bool,
        drawn: &'a dyn Fn(usize) -> Vec<crate::drawn::Run>,
    ) -> Grid<'a> {
        // Through `ruby()` and `hanging_punctuation()`, not the fields: a page
        // packed tight lays out neither, and a grid that disagreed with what is
        // drawn would put the cursor somewhere the writer cannot see.
        // The stamp is what lets a page of forty 縱 lay a paragraph out once:
        // which document, and which version of it. Two numbers, hashed —
        // never the paragraph's own text, which on a chapter written as one
        // paragraph costs more to hash than to lay out.
        let mut stamp = DefaultHasher::new();
        (
            self.current_buffer().id(),
            self.current_buffer().revision(),
            // A revision does not move when the *selection* does, and a
            // selection holds a construct open — which changes the page.
            self.selection(),
            self.render as u8,
            // …and the syntax, which decides what counts as markup and does
            // not move the revision when `:syntax text` changes it.
            self.current_buffer().syntax() as u8,
        )
            .hash(&mut stamp);
        Grid::plain(self.zong_length, self.ruby())
            .with_stamp(stamp.finish().max(1))
            .with_tatechuyoko(self.tatechuyoko)
            .with_indent(self.paragraph_indent())
            .with_hidden(hidden)
            .with_folds(folded)
            .with_open_line(self.open_line())
            .with_hanging(self.hanging_punctuation())
            .with_sentences(self.sentences)
            .with_drawn(drawn)
    }

    /// The markup that is off the page on `line`, as columns within it.
    ///
    /// **Markup only** — the ruby markup is not in it, because the 縱書 page
    /// lays a reading out itself and hides the tags as part of doing so. The
    /// horizontal page, which draws the reading above the row, asks
    /// [`Self::hidden_on_line`], which is this plus the ruby tags.
    pub fn markup_hidden_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        if !self.wysiwyg() {
            return Vec::new();
        }
        let spans = self.markup_line_in(line, self.block_of(line));
        crate::markdown::hidden(&spans, self.selected_columns(line))
    }

    /// Whether `line` is left off the page altogether (Feature #159).
    ///
    /// **The blank line between two indented paragraphs.** A Chinese paragraph
    /// is marked one way or the other — a blank line, or an indent — and never
    /// both; but the *file* is Markdown, where the blank line is what makes it
    /// a paragraph at all. So the file keeps it and the page leaves it out,
    /// which is the same bargain 所見即所得 makes with `**`.
    ///
    /// Three things are never folded: the line the cursor is on (or you could
    /// not see what you were typing into), a run of two or more blank lines (a
    /// writer who typed two meant something by the second — it is a scene
    /// break), and anything inside a fence or a page's metadata, where a blank
    /// line is content.
    pub fn line_is_folded(&self, line: usize) -> bool {
        // 中階 draws the indent and keeps the blank line: adding two squares
        // takes nothing away, folding a line does (#283).
        if self.indent == 0 || !self.indent_folds {
            return false;
        }
        if line == self.cursor_line() {
            return false;
        }
        // The blank line above the paragraph the cursor is in comes back with
        // it: that paragraph is shown as the file has it.
        if self.open_line() == Some(line + 1) {
            return false;
        }
        self.remember_folds();
        let cache = self.fold_cache.borrow();
        cache
            .as_ref()
            .and_then(|(_, map, _)| map.get(line).copied())
            .unwrap_or(false)
    }

    /// The span of lines a fold must not touch, because what is written there
    /// is not prose: a fence, a page's metadata, a table.
    ///
    /// One span rather than a set, because it travels in the [`Grid`], which
    /// is a `Copy` value every 縱 question is handed. A manuscript has none of
    /// these at all and the span is empty; a file with one fence loses folding
    /// only around it.
    pub fn fold_free_span(&self) -> (usize, usize) {
        self.remember_folds();
        self.fold_cache
            .borrow()
            .as_ref()
            .map(|(_, _, span)| *span)
            .unwrap_or((usize::MAX, 0))
    }

    /// The paragraph shown **as the file has it**: no indent, and its blank
    /// line back (Feature #159).
    ///
    /// **Wherever the cursor is**, not only in Insert. The paragraph you are
    /// standing in is shown as the file has it — no opening squares, and the
    /// blank line above it back — so there is never a question about what is
    /// really there. It costs almost no movement: exactly one blank line is
    /// open at a time, so crossing from one paragraph to the next closes one
    /// and opens another and the page below does not shift.
    pub fn open_line(&self) -> Option<usize> {
        match self.indent > 0 && self.indent_folds {
            true => Some(self.cursor_line()),
            false => None,
        }
    }

    /// Work out the fold map for the buffer as it stands, once per edit.
    fn remember_folds(&self) {
        let buffer = self.current_buffer();
        let key = (buffer.id(), buffer.revision());
        if let Some((cached, _, _)) = self.fold_cache.borrow().as_ref() {
            if *cached == key {
                return;
            }
        }
        let rope = buffer.rope();
        let lines = rope.len_lines();
        let blank = |l: usize| {
            l < lines && rope.line(l).chars().all(char::is_whitespace)
        };
        let blocks = self.blocks_through(lines.saturating_sub(1));
        let prose = |l: usize| {
            matches!(
                blocks.get(l).copied().unwrap_or_default(),
                crate::markdown::Block::Prose | crate::markdown::Block::Quote
            )
        };
        let map: Vec<bool> = (0..lines)
            .map(|l| {
                l > 0
                    && l + 1 < lines
                    && blank(l)
                    && !blank(l - 1)
                    && !blank(l + 1)
                    && prose(l)
            })
            .collect();
        // …and where the vertical page, which cannot carry the map, must not
        // fold: the span holding the two blocks where a **blank line is
        // content**, which is a fence and a page's metadata. Not every block
        // that is not prose — a novel's chapter headings are not prose either,
        // and taking the span from the first heading to the last would be the
        // whole book, which is how 縱書 came to fold nothing at all.
        let span = (0..lines)
            .filter(|&l| {
                matches!(
                    blocks.get(l).copied().unwrap_or_default(),
                    crate::markdown::Block::Code | crate::markdown::Block::FrontMatter
                )
            })
            .fold((usize::MAX, 0usize), |(first, last), l| {
                (first.min(l), last.max(l))
            });
        *self.fold_cache.borrow_mut() = Some((key, map, span));
    }

    /// How many squares open a paragraph, as the page is drawn.
    ///
    /// **Not** masked by `:view dense`, unlike the readings, the hung 句讀 and the
    /// ticks. Those three each cost a *column* — the width `:view dense` exists to
    /// win back. The indent costs two squares at the head of a paragraph, and
    /// it is the one thing on a packed page that says where a paragraph
    /// begins: it is what replaces the blank line, which costs a whole 縱.
    /// Masking it made the feature invisible on the default page.
    pub fn paragraph_indent(&self) -> usize {
        self.indent
    }

    /// How many bands the vertical page is divided into (段組).
    pub fn bands(&self) -> usize {
        self.bands.max(1)
    }

    /// Divide the vertical page into `n` bands.
    pub fn set_bands(&mut self, n: usize) {
        self.bands = n.clamp(1, 4);
        self.status = match self.bands {
            1 => say!("layout.bands-one"),
            n => say!("layout.bands", n),
        };
    }

    /// Set the first-line indent, in squares.
    pub fn set_indent(&mut self, n: usize) {
        self.indent = n.min(8);
        self.status = self.indent_report();
    }

    /// What the reader is told about the indent — the width **and** whether the
    /// blank line between paragraphs is folded, because those are two questions
    /// (#283) and a report that answers one of them leaves the other to guess.
    fn indent_report(&self) -> String {
        match self.indent_level() {
            Render::Off => say!("layout.first-line-indent-off"),
            Render::Basic => say!("layout.first-line-indent-kept", self.indent),
            Render::Full => say!("layout.first-line-indent-folded", self.indent),
        }
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
    /// Masked by `:view dense`, the same way [`Self::hanging_punctuation`] is and
    /// for the same reason: packing the page *suppresses* the reading column,
    /// it does not turn readings off. `:view dense` said it dropped the column in
    /// its own doc comment and in the manual's table, and did not — so a
    /// packed page kept paying two cells a 縱 for readings it was not drawing.
    ///
    /// The configured set — what `:ruby` reports and what `:view dense off` gives
    /// back — is [`Self::ruby_configured`].
    pub fn ruby(&self) -> Dialects {
        // …and only where 密排 costs anything. It packs the *縱書* page: the
        // reading column is a column off every 縱's width. A horizontal page
        // pays no width for a reading — the row above is only taken where
        // there is one — so there is nothing for packing to win there, and
        // masking it would mean 橫排 could never show a reading at all, since
        // 密排 is the default page.
        if !self.ruby_drawn {
            return Dialects::NONE;
        }
        match self.dense && self.layout == Layout::Vertical {
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
            // Naming a spelling is asking to see it. `:ruby typst` on a page
            // at 中階 that then drew nothing would be a command with no effect
            // and no complaint — the shape of bug #283 is about.
            self.ruby_drawn = true;
        } else {
            self.ruby.remove(dialect);
            self.ruby_drawn &= !self.ruby.is_empty();
        }
    }

    /// Where the cursor sits in the 縱 grid (for the status line).
    pub fn zong_position(&self) -> zong::Position {
        let hidden = |line: usize| self.markup_hidden_on_line(line);
        let folded = |line: usize| self.line_is_folded(line);
        let drawn = |line: usize| self.drawn_runs_on_line(line);
        let grid = self.grid_with(&hidden, &folded, &drawn);
        zong::position(self.current_buffer().rope(), self.caret(), grid)
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

    /// The other work area, if the page is split.
    pub fn other_pane(&self) -> Option<&Pane> {
        self.other.as_ref()
    }

    /// Which half of the screen holds the keys (0 = the first drawn).
    pub fn live_pane(&self) -> usize {
        self.live_pane
    }

    /// Open the other work area, showing `at` in the current buffer.
    ///
    /// It opens **where you are standing**: nothing moves, which is the whole
    /// point — the second area is for reading a place without leaving the one
    /// you are in.
    pub fn open_split(&mut self, at: usize, highlight: Option<(usize, usize)>, caption: String) {
        let buffer = self.current_buffer().id();
        self.other = Some(Pane {
            buffer,
            cursor: at.min(self.current_buffer().rope().len_chars()),
            anchor: at.min(self.current_buffer().rope().len_chars()),
            goal_column: 0,
            goal_slot: 0,
            extend: false,
            highlight,
            caption,
        });
    }

    /// Show something else in the other work area, opening it if need be.
    pub fn show_in_split(&mut self, at: usize, highlight: Option<(usize, usize)>, caption: String) {
        // **Which buffer, every time.** A pane that kept the id it was opened
        // with while being handed another file's offsets is a pane that names
        // one document and shows another — and `空格 w` then goes to the one it
        // names.
        let buffer = self.current_buffer().id();
        match self.other.as_mut() {
            Some(pane) => {
                pane.buffer = buffer;
                pane.cursor = at;
                pane.anchor = at;
                pane.highlight = highlight;
                pane.caption = caption;
            }
            None => self.open_split(at, highlight, caption),
        }
    }

    /// Close the other work area, keeping the one the keys are in.
    ///
    /// Which is the same act as「關掉另一個」 when there are two of them, and
    /// it means the half you are standing in can never vanish under you.
    pub fn close_split(&mut self) -> bool {
        let had = self.other.take().is_some();
        self.live_pane = 0;
        had
    }

    /// Hand the keys to the other work area, and take back the place it held.
    ///
    /// The one place where the editor's single cursor moves between panes: the
    /// live pane's place is written into the pane it leaves, and the other's
    /// is installed. Nothing else in the editor learns that panes exist.
    pub fn switch_pane(&mut self) -> bool {
        let Some(mut pane) = self.other.take() else {
            return false;
        };
        let here = Pane {
            buffer: self.current_buffer().id(),
            cursor: self.cursor,
            anchor: self.anchor,
            goal_column: self.goal_column,
            goal_slot: self.goal_slot,
            extend: self.extend,
            highlight: None,
            caption: String::new(),
        };
        // The place is clamped rather than trusted: the other pane may have
        // been edited while this one was not looking — and the file it names
        // may have been closed, in which case there is nowhere to go and
        // saying so is the whole of the right answer.
        match self.buffer_with(pane.buffer) {
            // Through the same door `gn` uses: whether *this* file is a grid,
            // and every memo about the one being left, are re-asked there. Set
            // directly, the schema of a 拆分表 followed you into a chapter and
            // `o` wrote 「,,」 into your novel.
            Some(index) if index != self.current => {
                let at = self.cursor;
                self.buffers[self.current].save_cursor(at);
                self.current = index;
                self.forget_the_document();
            }
            Some(_) => {}
            None => {
                self.other = None;
                self.live_pane = 0;
                self.status = say!("pane.file-was-closed");
                return false;
            }
        }
        let len = self.current_buffer().rope().len_chars();
        pane.cursor = pane.cursor.min(len);
        pane.anchor = pane.anchor.min(len);
        self.cursor = pane.cursor;
        self.anchor = pane.anchor;
        self.goal_column = pane.goal_column;
        self.goal_slot = pane.goal_slot;
        self.extend = pane.extend;
        self.other = Some(here);
        self.live_pane = 1 - self.live_pane;
        self.refresh_goal_column();
        true
    }

    /// Whether the line-number band carries a ground of its own.
    pub fn number_fill(&self) -> bool {
        self.number_fill
    }

    /// Give the numbers a band, or leave them on the page.
    pub fn set_number_fill(&mut self, on: bool) {
        self.number_fill = on;
    }

    /// Take a pending `:shot`, if one is waiting for a frame.
    pub fn take_screenshot_request(&mut self) -> Option<ShotJob> {
        self.screenshot_request.take()
    }

    /// Take a pending `:theme`, if one is waiting for the front end.
    ///
    /// `None` in either half means「別動這一半」: `:theme dark` names no theme
    /// and `:theme moxiang` names no mood, and a bare `:theme` names neither,
    /// which is how it comes to be the way to *ask*.
    #[allow(clippy::type_complexity)]
    pub fn take_theme_request(
        &mut self,
    ) -> Option<(Option<String>, Option<crate::command::Mood>)> {
        self.theme_request.take()
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
        // **A flick does not leave 全窗表格 either** — it is a window onto one
        // table, and the wheel is a movement like any other (see
        // [`Self::hold_the_pane`], which cannot see this one: scrolling comes
        // in as a mouse event rather than as a key).
        let held = self
            .table
            .as_ref()
            .is_some_and(|view| view.takes_the_pane())
            .then(|| self.table_row_span())
            .flatten();
        for _ in 0..amount.max(1) {
            let before = self.cursor;
            if vertical {
                self.move_zong_from(!back, true);
            } else {
                self.move_vertical(back);
            }
            if let Some((first, last)) = held {
                if !(first..=last).contains(&self.cursor_line()) {
                    self.cursor = before;
                    break;
                }
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
                self.status = say!("recover.no-draft-kept", what);
            }
        } else {
            self.swap_warned = false;
        }
    }

    // ---- Reading the file again (Feature #214) ----------------------------

    /// Re-read the file from disk, throwing away what is in the buffer.
    ///
    /// `force` is the `!`: without it a buffer with unsaved changes is refused,
    /// because re-reading over them is losing them and the writer has to be the
    /// one who says so.
    fn reload(&mut self, force: bool) -> Result<(), EditorError> {
        if self.current_buffer().path().is_none() {
            return Err(EditorError::NoFileName);
        }
        if self.current_buffer().is_modified() && !force {
            return Err(EditorError::UnsavedChanges);
        }
        self.reread_now();
        self.status = say!("buffer.re-read", self.current_buffer().display_name());
        Ok(())
    }

    /// Take what is on disk, and put the editor back in step with it.
    ///
    /// Three caches answer questions *about the whole document* and every one
    /// of them is now about a document that is no longer here.
    fn reread_now(&mut self) {
        // A locked buffer is still re-readable: read-only is about **editing**
        // it, and taking a fresh copy of the file is the one thing a reader
        // does want.
        if let Err(err) = self.current_buffer_mut().reread() {
            self.status = say!("reload.failed", err.to_string());
            // **Latched, or it says it every two seconds.** A file that was
            // moved or deleted out from under a `:reload auto` session
            // answers `changed_underneath` yes for ever, and the failure
            // would stamp over whatever the reader is actually reading.
            self.reload_warned = true;
            return;
        }
        self.clamp_cursor();
        // The one list, not the three caches this used to clear: a re-read is
        // a new document, and `md_cache`, `fold_cache` and `key_index` all
        // described the old one. It also puts the warning latch back.
        self.forget_the_document();
    }

    /// Notice a file that changed underneath, if `:reload auto on` (Feature
    /// #214).
    ///
    /// Throttled the way [`Editor::autosave_tick`] is, so this is a clock check
    /// on most keys: `changed_underneath` costs a `stat` on the cheap path and
    /// a whole read on the expensive one, and a held-down `j` would pay it per
    /// row.
    pub fn disk_tick(&mut self) {
        if !self.reload_auto {
            return;
        }
        let now = std::time::Instant::now();
        if let Some(last) = self.last_disk_check {
            if now.duration_since(last) < DISK_INTERVAL {
                return;
            }
        }
        self.last_disk_check = Some(now);
        if !self.current_buffer().changed_underneath() {
            // All is well again — so the next divergence is worth saying out
            // loud, even though this one has been said.
            self.reload_warned = false;
            return;
        }
        // Said once already about this file, and nothing has changed since:
        // the writer knows, and the status line is theirs to use.
        if self.reload_warned {
            return;
        }
        // **A dirty buffer is never re-read behind the writer's back.** The
        // whole point of the setting is convenience, and there is no
        // convenience worth an afternoon's typing: this is the one case where
        // it stops and asks.
        if self.current_buffer().is_modified() {
            self.reload_warned = true;
            self.status = say!("autoreload.both-changed");
            return;
        }
        self.reread_now();
        self.status = say!("autoreload.reread", self.current_buffer().display_name());
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

    // ---- The session (Feature #43) ----------------------------------------

    /// Where to remember which files are open (`<data>/sessions/<key>.txt`).
    ///
    /// Kept in the data directory rather than in the project, because a
    /// session is a fact about *you* and this afternoon, not about the book —
    /// and because an editor should not leave a file in every directory it is
    /// ever run in.
    pub fn keep_session_in(&mut self, dir: PathBuf, project: &Path) {
        let mut hasher = DefaultHasher::new();
        project.hash(&mut hasher);
        let key = format!("{:016x}", hasher.finish());
        self.session_file = Some(dir.join(format!("{key}.txt")));
    }

    /// Write down which files are open and where the cursor is in each.
    ///
    /// One line per file: `path\tline`. A plain list rather than a format,
    /// because the only thing that reads it is the next hour of this editor,
    /// and a person looking at it should be able to see what it says.
    pub fn save_session(&mut self) {
        let Some(file) = self.session_file.clone() else {
            return;
        };
        let here = self.cursor;
        self.buffers[self.current].save_cursor(here);
        // The file you are in first, then the rest in order — and **capped**.
        // `:replace` opens every file it changes, so a rename across a book
        // leaves 120 buffers open, and a session that remembered all of them
        // would reopen 120 files tomorrow morning.
        let order = std::iter::once(self.current).chain(
            (0..self.buffers.len()).filter(|&i| i != self.current),
        );
        let mut out = String::new();
        for i in order.take(SESSION_FILES) {
            let buffer = &self.buffers[i];
            let Some(path) = buffer.path() else { continue };
            let line = buffer
                .rope()
                .char_to_line(buffer.saved_cursor().min(buffer.rope().len_chars()));
            out.push_str(&format!("{}\t{}\n", path.display(), line + 1));
        }
        if out.is_empty() {
            // Nothing was open, so there is nothing to come back to — and a
            // stale session is worse than none.
            let _ = std::fs::remove_file(&file);
            return;
        }
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&file, out);
    }

    /// Open again what was open last time, each at the line it was left on.
    ///
    /// Only when the editor was started with **no file named**: someone who
    /// said which file they wanted gets that file. Returns how many were
    /// opened, so the front end can say so — restoring five chapters silently
    /// would leave a person wondering what they were looking at.
    pub fn restore_session(&mut self) -> usize {
        let Some(file) = self.session_file.clone() else {
            return 0;
        };
        let Ok(text) = std::fs::read_to_string(&file) else {
            return 0;
        };
        let mut opened = 0usize;
        let mut first = None;
        for line in text.lines() {
            let (path, at) = match line.split_once('\t') {
                Some((p, n)) => (PathBuf::from(p), n.parse::<usize>().unwrap_or(1)),
                None => (PathBuf::from(line), 1),
            };
            // A file that has since been moved or deleted is simply not opened:
            // the session is a convenience, and a convenience does not get to
            // put an error on the screen every morning.
            if !path.is_file() || self.open_file(&path).is_err() {
                continue;
            }
            self.move_to_line(at);
            self.buffers[self.current].save_cursor(self.cursor);
            first.get_or_insert(self.current);
            opened += 1;
        }
        if let Some(i) = first {
            self.show_buffer(i);
        }
        self.status = String::new();
        opened
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
            buffer.name_as(&say!("recover.draft-name", name));
            // **It is text that exists nowhere else.** Recovered clean, it had
            // no file, no draft (the line below deletes it), and no dirty flag
            // — so `:q` threw away the crashed session's work without a word,
            // and the autosave never wrote it either.
            buffer.mark_modified();
            self.add_buffer(buffer);
            // The copy is now in a buffer the writer can see and save; leaving
            // the file behind would offer it again on the next launch — and it
            // is written again immediately, because the buffer is modified.
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
            self.status = say!("recover.drafts-waiting", orphans);
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
        self.status = say!("recover.drafts-newer-than-file", listed(&waiting));
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
                    0 => say!("recover.no-drafts"),
                    n => say!("recover.drafts-dropped", n),
                };
                return Ok(CommandOutcome::Continue);
            }
            let taken = self.take_orphan_drafts();
            self.status = match taken {
                0 => say!("recover.no-draft-for-this-file"),
                n => say!("recover.drafts-opened", n),
            };
            return Ok(CommandOutcome::Continue);
        };
        if discard {
            self.current_buffer_mut().discard_swap();
            self.status = say!("recover.draft-dropped");
            return Ok(CommandOutcome::Continue);
        }
        // An ordinary, undoable edit: `u` puts the file on disk back, so
        // recovering is a decision the writer can take back.
        //
        // **Which a locked buffer cannot do at all**, and must not pretend to:
        // `adopt_draft` below takes the swap file over, and on quit it is
        // deleted — so a `:recover` that quietly changed nothing would throw
        // away the crashed session's work while saying it had opened it.
        if self.refuse_readonly() {
            return Ok(CommandOutcome::Continue);
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.current_buffer_mut().replace(0..len, &draft);
        if !self.applied(done) {
            return Ok(CommandOutcome::Continue);
        }
        self.current_buffer_mut().adopt_draft();
        self.clamp_cursor();
        self.status = say!("recover.draft-opened");
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
    /// A measure the writer has set wins, but only downwards: `:view wrap 50` on a
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
        // the window. So `:view dense off` needs nothing remembered — what was
        // configured was never touched, and simply applies again.
        self.dense = on;
        // 橫排 has no columns to pack, so `:view dense` means the other axis there:
        // the row of air above every row.
        if self.layout == Layout::Horizontal {
            self.loose_rows = !on;
        }
        self.status = match (on, self.layout) {
            (true, Layout::Vertical) => say!("layout.dense-on"),
            (false, Layout::Vertical) => say!("layout.dense-off"),
            (true, Layout::Horizontal) => say!("layout.tight"),
            (false, Layout::Horizontal) => say!("layout.loose"),
        };
    }

    /// Whether the page is packed tight.
    pub fn dense(&self) -> bool {
        self.dense
    }

    /// One 句 to a 縱 (`:view sentence`, Feature #237).
    ///
    /// **A view, and the point is that it is one.** The manual has taught
    /// `:%s/。/。\n/g` for reading a draft back one sentence at a time since the
    /// first version — a substitution that edits the manuscript in order to
    /// read it, and has to be undone before anybody can write again. This is
    /// that, without touching the file: the page breaks a 縱 at the end of every
    /// 句 as well as at the measure, so a long sentence still wraps rather than
    /// running off the foot of the page.
    ///
    /// The boundaries are `motion::sentence_starts`', which is what `(` and `)`
    /// jump between — one answer, so the cursor cannot walk to a place the page
    /// does not break at.
    pub fn set_sentences(&mut self, on: bool) {
        self.sentences = on;
        self.status = match on {
            true => say!("sentence.on"),
            false => say!("sentence.off"),
        };
    }

    /// Whether every 句 opens a 縱 of its own.
    pub fn sentences(&self) -> bool {
        self.sentences
    }

    /// Whether the horizontal page keeps a row of air above every row (疏排).
    pub fn loose_rows(&self) -> bool {
        self.loose_rows
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
            Some(m) => say!("layout.measure-columns", m),
            None => say!("layout.measure-window"),
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

    /// Take one key of a sequence's numeric argument, if that is what it is.
    ///
    /// Returns whether the key was swallowed — in which case the sequence stays
    /// open, waiting for its verb.
    fn take_sequence_argument(&mut self, key: Key) -> bool {
        let Key::Char(c) = key else {
            return false;
        };
        if let Some(digit) = c.to_digit(10) {
            let at = self.sequence.get_or_insert_with(Sequence::started).last();
            *at = at.saturating_mul(10).saturating_add(digit as usize).min(1_000_000);
            return true;
        }
        // **`-` is a range and `,` is a list** (§5.7), and a sequence is one or
        // the other. Both only ever follow a number, so neither key is taken
        // away from anything: `t-` and `t,` are still whatever they were.
        let joint = match c {
            '-' => Joint::Span,
            ',' => Joint::List,
            _ => return false,
        };
        if let Some(sequence) = self.sequence.as_mut() {
            // A span has exactly two ends, so a second `-` is not part of it;
            // a list goes on as long as commas do.
            let room = match sequence.joint {
                None => true,
                Some(Joint::List) => joint == Joint::List,
                Some(Joint::Span) => false,
            };
            if room {
                sequence.joint = Some(joint);
                sequence.numbers.push(0);
                return true;
            }
        }
        false
    }

    /// Take one `<column><a|d>` of a sort, if that is what this key is.
    ///
    /// `t1a2d8as` — 「對第一列升序，第二列降序，第八列升序，最後的 s 發出動作指
    /// 令」. Returns whether the key was swallowed, in which case the sequence
    /// stays open for the next column.
    ///
    /// Only after a plain number, so `t d` is still 「delete this row」 and
    /// `t2-5d` — a *span*, which no sort key is — is still whatever it was.
    fn take_sort_key(&mut self, key: Key) -> bool {
        let Key::Char(c @ ('a' | 'd')) = key else {
            return false;
        };
        let Some(column) = self.sequence.as_ref().and_then(Sequence::one) else {
            return false;
        };
        self.sort_keys.push((column, c == 'd'));
        self.sequence = None;
        true
    }

    /// The sequence's argument as a span, if it was given one.
    fn sequence_span(&self) -> Option<(usize, usize)> {
        self.sequence.as_ref()?.span()
    }

    /// Every column the sequence names, 1-based — `t3`, `t2-10`, `t1,5,9`.
    fn sequence_columns(&self) -> Option<Vec<usize>> {
        Some(self.sequence.as_ref()?.columns())
    }

    /// **The command as far as it has been typed** — `3`, `3-5`, `g`, `2t`.
    ///
    /// A modal editor asks you to type a command a key at a time and then says
    /// nothing about what you have typed: press `3` and the editor looks
    /// exactly as it did, so `30d` and `3d` are told apart by memory alone.
    /// vi has answered this since 1976 (`showcmd`, in the bottom right) and so
    /// does Helix; this is that string, and the front end draws it in both
    /// places yumete draws it — the status line's right edge, and beside the
    /// caret, where the eyes already are.
    ///
    /// Empty when nothing is pending, which is most of the time.
    pub fn typed_so_far(&self) -> String {
        let word = match self.pending {
            Pending::None => "",
            Pending::Goto => "g",
            Pending::Space => "␣",
            Pending::Find(FindKind::Forward) => "f",
            Pending::Find(FindKind::Backward) => "F",
            Pending::Replace => "r",
            Pending::Register => "\"",
            Pending::Match => "m",
            Pending::MatchPair { around: false } => "mi",
            Pending::MatchPair { around: true } => "ma",
            Pending::Surround => "ms",
            Pending::SurroundFrom | Pending::SurroundTo(_) => "mr",
            Pending::Table => "t",
            Pending::Hop { forward: true } => "]",
            Pending::Hop { forward: false } => "[",
            Pending::Conflict => "␣c",
            Pending::Case => "`",
            Pending::Mark => "M",
            Pending::Recall => "'",
        };
        // **In the order it was typed.** Inside a sequence the number comes
        // *after* the prefix — `g3` is on its way to `g3d` — and outside one it
        // comes before the key it multiplies, which is `3w`.
        let mut out = String::new();
        if self.pending != Pending::None {
            out.push_str(word);
            // The columns a sort has already been told about, so `t1a2d` reads
            // back as `t1a2d` and not as `t2d` — the whole point of the
            // grammar is that it is typed a column at a time.
            for &(column, down) in &self.sort_keys {
                out.push_str(&column.to_string());
                out.push(match down {
                    true => 'd',
                    false => 'a',
                });
            }
            match self.sequence.as_ref() {
                Some(sequence) => out.push_str(&sequence.spelled()),
                // A count typed the other way round is still part of what was
                // typed: `3gd` says `3g` here, not `g`. **In the order it was
                // typed** — `2-5g`, not `-52g`, which is what two inserts at
                // index 0 produced.
                None => {
                    let mut before = String::new();
                    if let Some(n) = self.operator_count {
                        before.push_str(&n.to_string());
                    }
                    if let Some((_, to)) = self.column_span {
                        before.push('-');
                        before.push_str(&to.to_string());
                    }
                    out.insert_str(0, &before);
                }
            }
            return out;
        }
        if let Some(n) = self.count {
            out.push_str(&n.to_string());
        }
        if let Some(to) = self.count_to {
            out.push('-');
            if let Some(n) = to {
                out.push_str(&n.to_string());
            }
        }
        out
    }

    // ---- Word segmentation (Feature #24) ----------------------------------

    /// Forget every answer that was worked out with the segmenter in force.
    ///
    /// Where a word ends is an input to two derived answers, not one: the
    /// segmentation overlay **and** the 平仄 margin, which asks the reader for
    /// a *word*'s reading (`了` is `le` in 為了 and `liǎo` in 了解). Both are
    /// kept against a hash of the line's text, and changing the dictionary
    /// changes neither the text nor the hash — so a `:view meter` turned on before
    /// the IME finished loading its dictionary kept marking the 了 in 為了 仄
    /// until the line was edited, which is the exact mistake the feature
    /// exists to catch.
    fn forget_the_words(&mut self) {
        self.segment_cache.borrow_mut().clear();
        self.meter_cache.borrow_mut().clear();
    }

    /// Install the word [`Segmenter`] used by `w`/`b`/`e` and the segmentation
    /// overlay. A [`yumete_cjk::DictionarySegmenter`] groups CJK characters into
    /// words; the default [`yumete_cjk::CategorySegmenter`] treats each as one.
    pub fn set_segmenter(&mut self, segmenter: Box<dyn Segmenter>) {
        self.forget_the_words();
        // The project's own words go on top of whatever was chosen, so the
        // book's names survive a change of dictionary.
        self.segmenter = Box::new(yumete_cjk::WithWords::new(
            segmenter,
            std::rc::Rc::clone(&self.project_words),
        ));
    }

    /// Install the [`Reader`] `:ruby auto` generates readings from (#234).
    ///
    /// The same shape as [`set_segmenter`](Self::set_segmenter) and for the same
    /// reason: the knowledge is 宇浩's tables, which the front end owns and the
    /// editor never opens.
    pub fn set_reader(&mut self, reader: Box<dyn Reader>) {
        self.reader = reader;
        // The 平仄 in the margin were read off the reader that has just been
        // replaced, and nothing about the *text* changed — so the hash they are
        // kept against would say they are still good.
        self.meter_cache.borrow_mut().clear();
    }

    /// Everything `:word` asks — see [`crate::command::WordCommand`].
    ///
    /// **Where one word ends is one subject.** The dictionary decides it, the
    /// colour shows it, the level tunes it; they were three unrelated things to
    /// find out about, and two of them were commands nobody would think to look
    /// for from the third.
    fn word_command(
        &mut self,
        what: crate::command::WordCommand,
    ) -> Result<CommandOutcome, EditorError> {
        use crate::command::WordCommand;
        match what {
            WordCommand::Report | WordCommand::List => {
                self.status = say!(
                    "word.status",
                    self.segmenter.source(),
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
            WordCommand::Discover => {
                let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                self.discover_words(&root)?;
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

    /// How readily characters join into words (`:word level`).
    ///
    /// Kept here as well as pushed into the segmenter, because a segmenter
    /// installed later — the IME finishing its load, `:word list reload` — has
    /// to arrive at the level the reader chose rather than at the default.
    pub fn set_word_level(&mut self, level: yumete_cjk::WordLevel) {
        self.word_level = level;
        self.segmenter.set_level(level);
        self.forget_the_words();
    }

    /// What the dictionary in force calls itself, for the status line.
    pub fn words_in_force(&self) -> String {
        self.segmenter.source()
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

    /// `:word discover` — the words this book has and no dictionary does
    /// (Feature #239).
    ///
    /// **The whole project, not this file.** A name earns its place in the
    /// list by turning up in chapter after chapter, and five sightings spread
    /// over forty files is exactly the evidence a single open buffer cannot
    /// show. Unsaved buffers count as they stand, the way `:grep` reads them.
    ///
    /// **The candidates are written into the list, unsaved.** This is the same
    /// bargain [`Self::replace_found`] strikes: the editor does the work, the
    /// buffer holds it, `u` takes it back, and `:w` is the moment a person says
    /// yes. A listing the writer would have to retype by hand is not an offer,
    /// and a file quietly rewritten on disk is not a question. What lands is a
    /// block of `詞　# 47 次` lines under a comment saying where it came from —
    /// delete the ones that are not words and save.
    ///
    /// Nothing already in the list comes back on a second run: the segmenter
    /// in force is wrapped in [`yumete_cjk::WithWords`], so a listed word is
    /// one it already joins, and [`crate::discover`] never offers those.
    fn discover_words(&mut self, root: &Path) -> Result<(), EditorError> {
        let mut text = String::new();
        let mut files = 0usize;
        walk(root, &mut |path| {
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
            // The join, so a word cannot be found across the seam between two
            // chapters that never touch.
            text.push('\n');
        });
        let found = {
            let joins = |word: &str| self.segmenter.segment(word).len() == 1;
            crate::discover::words(&text, &joins)
        };
        if found.is_empty() {
            self.status = say!("word.discover-none", files);
            return Ok(());
        }
        let total = found.len();
        let path = self.project_words_path();
        self.open_word_list(&path)?;
        let mut block = String::new();
        let list = self.current_buffer().text();
        if !list.is_empty() && !list.ends_with('\n') {
            block.push('\n');
        }
        block.push_str(&say!("word.discover-heading", files));
        block.push('\n');
        for word in found.iter().take(DISCOVER_LIMIT) {
            block.push_str(&say!("word.discover-line", word.word, word.count));
            block.push('\n');
        }
        let at = self.current_buffer().char_count();
        self.snapshot();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().insert(at, &block));
        if !self.applied(done) {
            return Ok(());
        }
        self.clamp_cursor();
        self.forget_the_text();
        self.set_cursor(at);
        // **They segment before they are saved.** The bargain is still the
        // same — nothing is on disk until `:w` — but a candidate list that
        // does not affect anything until it is saved cannot be judged: the
        // way to see whether 落霞鎮 is a word is to walk `w` over it and read
        // the tint. So they go into the list in force now, and the save reads
        // the file back (below), which is what drops the lines struck out.
        {
            let mut words = self.project_words.borrow_mut();
            for word in found.iter().take(DISCOVER_LIMIT) {
                words.add(&word.word);
            }
        }
        self.forget_the_words();
        self.words_request = true;
        self.status = match total > DISCOVER_LIMIT {
            true => say!("word.discover-too-many", DISCOVER_LIMIT, total),
            false => say!("word.discover-found", total, path.display()),
        };
        Ok(())
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
        *self.project_words.borrow_mut() = list;
        self.forget_the_words();
        self.status = match where_from {
            Some(path) => say!("word.project-words-loaded", n, path.display()),
            None => say!("word.no-project-words-file"),
        };
    }

    /// Re-read the word list the save just wrote, if that is what it was.
    ///
    /// **A word list is data, and saving data is the same gesture as applying
    /// it.** `:word discover` writes its candidates into the buffer already
    /// segmenting (they have to, or there is no way to judge them), so the
    /// save is where the writer's weeding — the lines struck out — has to
    /// reach the segmenter; and a list edited by hand had no reason to need a
    /// second command either. The global list is the front end's to load, so
    /// that one is only asked for.
    ///
    /// Answers whether it did anything, because the caller owes the status
    /// line a different sentence when it did.
    fn note_word_list_saved(&mut self) -> bool {
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
        self.show_segmentation
    }

    /// Whether the editor is in the state `need` asks for.
    fn meets(&self, need: command::Need) -> bool {
        match need {
            command::Need::Vertical => self.layout == Layout::Vertical,
            command::Need::Loose => !self.dense,
            command::Need::Table => self.table_here(),
            command::Need::Scheme => self.ime_available,
        }
    }

    /// Bring `need` about, for `force`.
    fn satisfy(&mut self, need: command::Need) {
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
    /// that it is `:word show tint|ink`.
    pub fn set_word_mark(&mut self, mark: yumete_cjk::WordMark) {
        self.word_mark = mark;
    }

    /// How loudly the editor draws what you have typed, beside the caret.
    pub fn hud(&self) -> Hud {
        self.hud
    }

    /// Say how loudly. `:view hud off|basic|full`, and nothing else writes it —
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
        let chars: Vec<char> = text.chars().collect();
        // A boundary the reader can see: whitespace, punctuation, a bracket, a
        // 、 — anything that is not part of a word. The ends of the line count,
        // because a line end is the most visible boundary there is.
        let visible = |at: usize| -> bool {
            match chars.get(at) {
                None => true,
                Some(c) => !c.is_alphanumeric(),
            }
        };
        let ranges: Vec<(usize, usize)> = self
            .segmenter
            .segment(&text)
            .into_iter()
            .filter(|&(a, b)| {
                let before = a == 0 || visible(a - 1);
                let after = visible(b);
                !(before && after)
            })
            .collect();
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
        if matches!(
            self.mode,
            Mode::Command | Mode::Lookfor | Mode::Search | Mode::Ruby
        ) {
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
        self.snapshot();
        self.insert_recording.push_str(text);
        self.insert_str(text);
    }

    /// Handle a single key press according to the current mode.
    pub fn on_key(&mut self, key: Key) -> KeyOutcome {
        // **A question standing takes every key** (#295), before recording and
        // before the sidebar: while one is on the screen there is no keystroke
        // that reaches the manuscript, and none that a macro should capture —
        // the answer is about this file at this moment, not about the sequence
        // being recorded.
        if self.query.is_some() {
            self.answer_query(key);
            return KeyOutcome::Continue;
        }
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
        // A key describes one move, and「was that a jump?」is about *this* one.
        self.jumped = false;
        let watching = !self.repeating_edit;
        if watching {
            // A count is part of the command it prefixes, not a command: `3>`
            // is one change and `.` has to repeat all three levels of it.
            if self.mode == Mode::Normal && self.pending == Pending::None && self.count.is_none() {
                self.edit_keys.clear();
                self.edit_revision = (self.current_buffer().id(), self.current_buffer().revision());
            }
            self.edit_keys.push(key);
        }
        // Where the pane's window was pointed before this key — see
        // [`Self::hold_the_pane`].
        let held = self
            .table
            .as_ref()
            .is_some_and(|view| view.takes_the_pane())
            .then(|| (self.current_buffer().id(), self.cursor));
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
            Mode::Lookfor => {
                self.on_lookfor_key(key);
                KeyOutcome::Continue
            }
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
        self.forget_a_guessed_table();
        self.hold_the_pane(held);
        self.find_the_table_here();
        outcome
    }

    /// **全窗表格 is a window onto one table, and nothing walks out of it**
    /// (author, 2026-09-07: 「理論上不能通過鼠標滾動或者 hjkl 前往正文……衹能
    /// 通過 tq/tf/tb 離開回到其他模式，或者 t[ t] 去下一個表格」).
    ///
    /// The pane used to be a leaky mode: `gg`, `G`, `:120` and a search hit
    /// are file-wide, they walked the cursor into the chapter, and the widget
    /// — which draws between the table's first and last row and nowhere else
    /// — then had nothing to draw. The old bargain was to hand the window
    /// back to the prose page, so 全窗表格 could end without anybody asking
    /// for it. Bounding it deletes that whole class of case, and it is what
    /// lets the gutter number the table's **own** rows: inside the window
    /// every number means the same thing.
    ///
    /// One place, at the end of every key, for the reason
    /// [`Self::forget_a_guessed_table`] gives: a rule kept by forty movement
    /// functions is a rule one of them will break.
    ///
    /// It holds only what it is sure of — the same buffer, the pane still
    /// asked for — so `t o`, `t q`, `:e` and a buffer switch are none of its
    /// business.
    fn hold_the_pane(&mut self, held: Option<(u64, usize)>) {
        // `t ]` and `t [` are the way *to* another table, so they are never
        // held — and the flag is this key's, cleared however this ends.
        let crossed = std::mem::take(&mut self.crossed_tables);
        let Some((buffer, was)) = held else {
            return;
        };
        if crossed {
            return;
        }
        if !self.table.as_ref().is_some_and(|view| view.takes_the_pane()) {
            return;
        }
        if self.current_buffer().id() != buffer {
            return;
        }
        // **Only what this key moved.** A cursor that was already off the rows
        // — parked on a header row, which only an internal call can do — is
        // not something to drag anywhere: holding it there would move the
        // caret under keys that never asked, and `t d` on the header would
        // delete the row below instead of saying 「標題行不能刪」.
        let line = self.cursor_line();
        let rope = self.current_buffer().rope();
        let home = rope.char_to_line(was.min(rope.len_chars()));
        let Some((first, last)) = self.table_row_span_at(home) else {
            return;
        };
        if !(first..=last).contains(&home) {
            return;
        }
        // Still on a row of **this** table — the table the window is showing.
        // A `G` that lands on a row of the *next* table in the document is
        // the window being walked out of just as much as one that lands in
        // the prose between them; `t ]` is the key that means to do that.
        if (first..=last).contains(&line) {
            return;
        }
        self.goto_line(line.clamp(first, last) + 1);
        self.snap_into_the_grid();
        self.status = say!("table.window-holds-you");
    }

    /// Drop a table mode that was **guessed**, once the cursor has left it.
    ///
    /// The author's rule for the second class of file (#275): 「如果一個文件沒
    /// 有確切的表格語法，比如 txt、yaml 用空格制表符隔開…離開表格立刻回到
    /// prose 狀態，如果要再進入表格狀態需要再次按 ti tt。」
    ///
    /// A run of tab-separated lines in a chapter is the editor's inference, not
    /// the file's statement, and an inference should not be left standing on
    /// the screen after the reader has walked away from what suggested it. A
    /// `.csv`, a `.md`, a file a schema claims — those say what they are, and
    /// [`Reach::File`] keeps them on.
    ///
    /// One place, at the end of every key: a mode that has to be undone by
    /// forty movement functions is a mode that will be left on by one of them.
    fn forget_a_guessed_table(&mut self) {
        let guessed = self
            .table
            .as_ref()
            .is_some_and(|v| v.reach == Reach::Cursor);
        if guessed && self.prose_region().is_none() {
            self.leave_table_quietly();
        }
    }

    /// Build the view the **level has already promised** (#283).
    ///
    /// `TableLevel::Basic` is the factory value, and its own words are 「the
    /// columns line up *and* the **keys** belong to the grid where a table
    /// is」. Only the first half was ever delivered: the padding asks the
    /// level (`table_padding_on`), while the keys and the no-soft-wrap rule
    /// ask [`Editor::table`] — which nothing but a `t` key ever built. So
    /// opening a `.md` gave the columns squared up with `hjkl` still walking
    /// letters and the rows still wrapping *inside their own padding*, a
    /// state neither `t o` nor `t b` names. `t b` then「did something」from a
    /// level it was already on, because what it actually did was build this.
    ///
    /// Deliberately narrower than [`Self::enter_table_as`], which is a door a
    /// person opened and may therefore write to the file:
    ///
    /// - **Only a `|` table that already parses** — [`Self::with_md_tables`]'s
    ///   answer, the same set the renderer draws, so the two cannot disagree.
    ///   A header with no `| --- |` under it gets one written by `t b`; a key
    ///   may rewrite the buffer, opening a file may not.
    /// - **Never a `.csv`, a file a schema claims, or a guessed block.** The
    ///   first two are a whole-file takeover with a cursor jump ([`Self::
    ///   table_on_open`] has them already), and the third is an inference the
    ///   author's rule says must be asked for: 「離開表格立刻回到 prose 狀
    ///   態，如果要再進入表格狀態需要再次按 ti tt」.
    /// - **Never the pane**, which is a different question with its own key.
    /// - **Nothing is said, nothing moves, nothing is written.** No status, no
    ///   `snap_to_cell`, no `turn_for_table`: this is the level being read,
    ///   not a command being run.
    ///
    /// Gated on [`Self::table_padding_on`] rather than on the level alone, so
    /// the two halves of the promise arrive together — on a 縱書 page nothing
    /// is padded, so nothing takes the keys either, and `t b` there is still
    /// the reader's own decision. [`Self::table_here`] asks the same question
    /// of an `Md` view every time, so the halves also *leave* together: `t o`
    /// takes the keys back with the padding, and a page turned 縱 after the
    /// view was built goes quiet without the view being thrown away.
    ///
    /// The mirror of [`Self::forget_a_guessed_table`] and called from the same
    /// places: a mode that forty movement functions have to switch on is a
    /// mode that one of them will leave off.
    fn find_the_table_here(&mut self) {
        if self.table.is_some() || !self.table_padding_on() {
            return;
        }
        // **Not while the table is being typed.** The grid refuses a `|` in a
        // cell (`a_pipe_cannot_be_typed_into_a_cell`) — right, once a person
        // has said 「this is a table」, and intolerable before: a writer typing
        // `| --- | --- |` under a fresh header watches the region start
        // parsing halfway along the line and the rest of their pipes get
        // swallowed. A level read off a file may not change what typing does.
        if self.mode == Mode::Insert {
            return;
        }
        if self.syntax() != crate::syntax::Syntax::Markdown {
            return;
        }
        // **The cursor's own line first, and it is one line.** `with_md_tables`
        // walks the file on every edit, and this runs at the end of every key
        // — so a chapter with no table in it must not pay for a scan to be
        // told so. Standing on a `|` row is when the scan is worth having, and
        // it is also when the renderer is about to run it anyway.
        if !self.md_row_at_cursor() {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let Some(first) =
            self.with_md_tables(|rows| rows.iter().find(|&&(a, b)| line >= a && line <= b).map(|&(a, _)| a))
        else {
            return;
        };
        let header = self.line_text(first).unwrap_or_default();
        self.table = Some(TableView {
            schema: crate::mdtable::schema(&header),
            from: PathBuf::new(),
            goal: 0,
            grain: Grain::Cell,
            separator: Separator::Pipe,
            pane: false,
            bounds: Bounds::Md,
            // **A `|` table says what it is on every one of its own lines**,
            // so the view is the file's the moment it exists — which is what
            // keeps a table three screens down from wrapping while the cursor
            // is up here in the prose, padded and folded at once.
            reach: Reach::File,
        });
    }

    /// If the command that just ended changed the buffer, it is what `.`
    /// repeats.
    fn finish_watching(&mut self) {
        if self.mode != Mode::Normal || self.pending != Pending::None {
            return;
        }
        if (self.current_buffer().id(), self.current_buffer().revision()) == self.edit_revision {
            return;
        }
        // A command that ended in a different buffer changed nothing here.
        if self.current_buffer().id() != self.edit_revision.0 {
            return;
        }
        // Four kinds of key change the buffer and are not *changes* in the
        // sense `.` means. Undo is the obvious one — repeating it would make
        // `.` mean "undo again" the moment you used it. A `:` line and a macro
        // are their own way of being repeated, and `.` repeating itself is not
        // a definition.
        // It is the **command's own key** that says which kind this is: the
        // first key after any count digits, and nothing after it. `3` then `.`
        // is recorded as `['3', '.']`, so looking only at the head let `.`
        // adopt a definition of itself and replay the replay — a stack
        // overflow, which is an *abort*, so nothing unwound and every unsaved
        // buffer went with it. Three keystrokes.
        //
        // Scanning the whole sequence instead was worse in the other
        // direction: `i` `3` `.` `1` `4` Esc — typing 3.14 into a cell — has a
        // `.` in it, so the insertion was refused as a definition and `.`
        // silently replayed some older edit into the document. So do neither:
        // ask what command this was. Repeating it is guarded separately, at
        // `repeat_edit`, which is what actually stops the recursion.
        let excluded = self
            .edit_keys
            .iter()
            .find(|key| !matches!(key, Key::Char(c) if c.is_ascii_digit()))
            .is_some_and(|key| {
                matches!(
                    key,
                    Key::Char(':' | '/' | '?' | 'u' | 'U' | '.' | 'q' | 'Q') | Key::Ctrl('r')
                )
            });
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
                // 命令＋選擇＋動作: `t20,20g` is 「table · row 20, column 20 ·
                // go」, and the sequence stays open while the digits arrive.
                //
                // A sort names as many columns as it likes before it acts
                // (`t1a2d8as`), so `a`/`d` are asked first: they close one
                // column and leave the sequence open for the next.
                if self.take_sort_key(key) || self.take_sequence_argument(key) {
                    return;
                }
                self.pending = Pending::None;
                // **A chain that has named a column ends in `s` or `S`.**
                // Without this, every `t` verb stays live in the middle of a
                // sort, and `d` — which is both 「降序」 and 「delete this
                // row」, one missing digit apart — took `t1ad` as 「delete」
                // and threw the columns away. So once a column has been
                // named, the only ways out are the action, `Esc`, and being
                // told what went wrong.
                if !self.sort_keys.is_empty() && !matches!(key, Key::Char('s' | 'S') | Key::Esc) {
                    self.sequence = None;
                    self.sort_keys.clear();
                    self.status = say!("table.sort-wants-its-action");
                    return;
                }
                self.table_structure(key);
                self.sequence = None;
                self.sort_keys.clear();
                return;
            }
            Pending::Hop { forward } => {
                self.pending = Pending::None;
                if key == Key::Char('c') {
                    self.go_to_conflict(forward);
                }
                return;
            }
            Pending::Conflict => {
                self.pending = Pending::None;
                match key {
                    Key::Char('o') => self.resolve_conflict(crate::conflict::Keep::Ours),
                    Key::Char('t') => self.resolve_conflict(crate::conflict::Keep::Theirs),
                    Key::Char('b') => self.resolve_conflict(crate::conflict::Keep::Both),
                    _ => {}
                }
                return;
            }
            Pending::Mark => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.set_mark(c);
                }
                return;
            }
            Pending::Recall => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.go_to_mark(c);
                }
                return;
            }
            Pending::Goto => {
                // **命令＋選擇＋動作.** Inside a sequence the digits are its
                // *argument*, not a repetition: `g3d` is 「goto · column 3 ·
                // definition」 and `g2-5d` names a span of columns, the way
                // `t20,20g` names a cell and `t1a2d8as` names three columns to
                // sort by. The verb ends the sequence, so no separator and no
                // space is needed — and the sequence stays open while digits
                // are being typed.
                if self.take_sequence_argument(key) {
                    return;
                }
                self.pending = Pending::None;
                // Whichever way the number was written: `g3d` puts it here,
                // `3gd` — vi's own order, kept because fifty years of fingers
                // know it — puts it in the count.
                if self.column_span.is_none() {
                    // `g3d`, then `3gd`: the sequence's own argument first,
                    // and failing that the count typed before the `g`, which
                    // the comment above has always promised and nothing read.
                    self.column_span = self
                        .sequence_span()
                        .or_else(|| self.operator_count.map(|n| (n, n)));
                }
                self.handle_goto(key);
                self.operator_count = None;
                self.sequence = None;
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
            Pending::Case => {
                self.pending = Pending::None;
                match key {
                    Key::Char('l') => {
                        self.map_selection(|c| c.to_lowercase().next().unwrap_or(c))
                    }
                    Key::Char('u') => {
                        self.map_selection(|c| c.to_uppercase().next().unwrap_or(c))
                    }
                    Key::Char('`') => self.map_selection(switch_case),
                    _ => {}
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
                    match &mut self.count_to {
                        // The far end of a span: `2-5`.
                        Some(to) => {
                            let n = to.unwrap_or(0);
                            *to = Some(
                                n.saturating_mul(10)
                                    .saturating_add(digit as usize)
                                    .min(1_000_000),
                            );
                        }
                        None => {
                            let n = self.count.unwrap_or(0);
                            self.count = Some(
                                n.saturating_mul(10)
                                    .saturating_add(digit as usize)
                                    .min(1_000_000),
                            );
                        }
                    }
                    return;
                }
            }
            // `2-5` — a **span**, for the keys that take a range of columns
            // rather than a repetition. Only after a number, so `-` is still
            // free on its own.
            if c == '-' && self.count.is_some() && self.count_to.is_none() {
                self.count_to = Some(None);
                return;
            }
        }
        let operator_count = self.count;
        let span = self.count_to.take().flatten().map(|to| (self.count.unwrap_or(1), to));
        self.column_span = span;
        let count = self.take_count();


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
            // **Across the break.** A line is not a wall: `l` off the end of
            // one sentence lands on the start of the next, and `h` walks back
            // over the break the same way. The line-bound pair is still what
            // measures a line; it is not what a reader walking a page wants.
            Key::Char('h') | Key::Left => {
                self.repeat(count, |e| e.move_horizontal(motion::prev_grapheme))
            }
            Key::Char('l') | Key::Right => {
                self.repeat(count, |e| e.move_horizontal(motion::next_grapheme))
            }
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
                e.select_up_to(p);
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
                e.select_up_to(p);
            }),
            Key::Char('(') => self.repeat(count, |e| {
                let p = motion::prev_sentence(e.current_buffer().rope(), e.cursor);
                e.select_to(p);
            }),
            // A mark is where you meant to come *back* to; the jump list is
            // where you came *from*. `M`/`'` rather than vi's `m`/`'`, because
            // `m` here opens match mode.
            Key::Char('M') => self.pending = Pending::Mark,
            Key::Char('\'') => self.pending = Pending::Recall,
            // 「下一個這種東西」, which is where Helix keeps it too.
            Key::Char(']') => self.pending = Pending::Hop { forward: true },
            Key::Char('[') => self.pending = Pending::Hop { forward: false },
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
            // In-line character search (Helix `f`/`F`).
            //
            // **`t` and `T` are gone**, and `t` is the table group in every
            // mode. One letter meant two unrelated things depending on whether
            // the cursor happened to be inside a `|` table, which is the kind
            // of inconsistency a reader cannot hold in their head — and vi's
            // `t` was never reachable here anyway: this editor puts the verb
            // last (`t，d`, not `dt，`), so till was one keystroke away from
            // find and no more.
            Key::Char('f') | Key::Char('F') => {
                self.pending = Pending::Find(match key {
                    Key::Char('f') => FindKind::Forward,
                    _ => FindKind::Backward,
                });
                self.operator_count = operator_count;
            }
            Key::Char('t') => {
                self.pending = Pending::Table;
                self.operator_count = operator_count;
            }
            // Select (extend) mode and collapse (Helix `v` / `;`).
            Key::Char('v') => self.extend = !self.extend,
            // Esc is every modal editor's way out. It leaves select mode and
            // collapses the selection — and, when the page is showing you
            // something in the other work area, it dismisses that first: a
            // preview is the transient thing on the screen, and Esc is the key
            // every reader presses at a transient thing.
            Key::Esc => {
                if self.other.is_some() && self.live_pane == 0 {
                    // **The window goes; the search stays.** Esc dismissed the
                    // pane, and the reader who then presses `n` means the same
                    // 「下一處」 they meant a moment ago — so `n` walks to the
                    // next hit and brings the pane back with it. Throwing the
                    // list away here handed `n` to `/` instead, which answered
                    // about some older pattern, or about nothing at all, and
                    // never said which.
                    self.close_split();
                    return;
                }
                self.extend = false;
                self.anchor = self.cursor;
            }
            // `;` collapses the selection but leaves select mode standing —
            // Helix's own behaviour, and the reason it looks broken to a
            // reader who has just pressed `v`: the very next motion grows the
            // selection again. So it says which of the two happened.
            Key::Char(';') => {
                self.anchor = self.cursor;
                if self.extend {
                    self.status = say!("selection.collapsed-in-extend");
                }
            }
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
                self.hits = None;
            }
            Key::Char('?') => {
                self.mode = Mode::Search;
                self.search_forward = false;
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
                self.hits = None;
            }
            // In a table with a search open, `n` walks *its* answers: they are
            // the last search that happened, which is what `n` has always
            // meant. A plain `/` clears them and takes the key back.
            Key::Char('n') if self.walking_hits() => {
                self.repeat(count, |e| {
                    e.walk_table_hits(true);
                });
            }
            Key::Char('N') if self.walking_hits() => {
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
            // …and the keys a keyboard already has for it. `C-f`/`C-b` are
            // vi's; these are the ones a reader who has never used vi presses,
            // and they used to do nothing at all.
            Key::Ctrl('f') | Key::PageDown => self.move_page(count, false, 1.0),
            Key::Ctrl('b') | Key::PageUp => self.move_page(count, true, 1.0),
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
            // 字形變換 (§5.2.3 ②): `` `l `` 小寫, `` `u `` 大寫, `` `` `` 互換.
            // `~` and `` A-` `` are Helix's and are **unbound** here — the
            // phrasebook catches both and points at this group.
            Key::Char('`') => self.pending = Pending::Case,
            // Replacing the selection with the register. Joining is on `gJ`:
            // `J` turns the page, which a reader presses a hundred times for
            // every once they join two lines.
            Key::Char('R') => self.replace_with_register(),
            // Search for whatever is selected (Helix `*`).
            // **`30G` goes to line 30**, and a bare `G` to the last line —
            // which is what `G` means in vi and in Helix both, and the key was
            // unbound here. `10gg` and `:30` still work; this is the one a
            // reader's fingers already know.
            Key::Char('G') => {
                self.remember_jump();
                // `operator_count`, not `self.count`: the count was taken at
                // the top of this function, so asking `self.count` here always
                // said "no digits" and `1G` went to the *last* line.
                match operator_count.is_some() {
                    true => self.goto_line(count),
                    false => {
                        let rope = self.current_buffer().rope();
                        self.move_head(motion::buffer_end(rope, self.cursor));
                    }
                }
            }

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
                    self.status = said;
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
    fn phrasebook(key: Key) -> Option<String> {
        let c = match key {
            Key::Char(c) => c,
            // **The two keys this editor itself retired.** They did something
            // here until recently, and `Enter` still does in every other
            // editor — so the reader who presses one is not coming from vi,
            // they are coming from last week, and silence is the one answer
            // that teaches nothing.
            Key::Enter => {
                return Some(say!("hint.vi.enter"));
            }
            // Helix's 轉大寫. Its two companions are `~` and `` ` ``; the
            // group took the third, so this is the only one that needs the
            // `Alt` arm.
            Key::Alt('`') => {
                return Some(say!("hint.helix.case-keys"));
            }
            _ => return None,
        };
        Some(match c {
            '*' => say!("hint.vi.star"),
            '$' => say!("hint.vi.dollar"),
            '^' => say!("hint.vi.caret"),
            'D' => say!("hint.vi.d-upper"),
            'C' => say!("hint.vi.c-upper"),
            's' | 'S' => say!("hint.vi.s"),
            'Z' => say!("hint.vi.z-upper"),
            '@' => say!("hint.vi.at"),
            '&' => say!("hint.vi.ampersand"),
            '_' | '+' | '-' => say!("hint.vi.line-motions"),
            '\\' => say!("hint.vi.backslash"),
            // No `` ` `` arm: it is a real binding now (the 字形 group), so the
            // fall-through never reaches here for it. `hint.vi.backtick` moved
            // into that group's menu, where vi's reader will see it anyway.
            '~' => say!("hint.helix.case-keys"),
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
            // `g30g` — the sequence's own argument — and `30gg`, vi's order.
            let line = self.sequence_span().map(|(n, _)| n).or(self.operator_count.take());
            if let Some(n) = line.filter(|&n| n > 0) {
                self.column_span = None;
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
            // Joining two lines of a grid makes one row with twice the fields
            // — the one thing table mode promises cannot happen. It went
            // round the two gates because it edits the rope directly.
            Key::Char('J') if self.joining_welds_a_grid() => {
                self.status = say!("table.join-would-change-columns");
                return;
            }
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
            // **`gx` follows what is written here.** vim and Helix both keep
            // 「open the thing under the cursor」 on this key, and in a
            // manuscript the thing under the cursor is a link.
            Key::Char('x') => return self.follow_link(),
            // **`gd` goes, `gw` shows.** The pair every editor has: `gd` is
            // *go to definition* — on a footnote that is the note, in a 拆分
            // column the row the component names — and `gw` is the same
            // question answered in the other work area, without leaving.
            Key::Char('d') => return self.show_definition(false),
            Key::Char('w') => return self.show_definition(true),
            // **`/` here, `?` over there.** 「這個詞還在哪裏」 — the selection,
            // or what the cursor is on — searched across the whole document.
            // `g/` is the sugar `/` has always wanted: search for *this*,
            // without retyping it. `g?` is the same answer shown in the other
            // work area, so the place you are standing is still on the screen.
            //
            // It used to be `Enter`, which is the key a writer presses by
            // accident: one keystroke too many in Normal mode and the page
            // jumped somewhere else.
            Key::Char('/') => {
                self.definition_preview = false;
                return self.search_the_page();
            }
            Key::Char('?') => {
                self.definition_preview = true;
                return self.search_the_page();
            }
            _ => return,
        };
        self.move_head(pos);
    }

    /// The keys `Space` opens, and what each of them is for — the list the
    /// which-key overlay draws, so what is offered and what happens cannot
    /// drift apart.
    pub const SPACE_KEYS: &'static [(char, &'static str)] = &[
        ('e', "hint.goto.file-sidebar"),
        ('o', "hint.goto.outline"),
        ('f', "hint.goto.open-file"),
        ('b', "hint.goto.switch-buffer"),
        ('/', "hint.goto.search-project"),
        ('?', "hint.goto.all-commands"),
        ('y', "hint.goto.copy-to-clipboard"),
        ('p', "hint.goto.paste-from-clipboard"),
        ('P', "hint.space.paste-before"),
        ('d', "hint.goto.dictionary"),
        ('r', "hint.space.ruby"),
        ('w', "hint.goto.other-pane"),
        ('W', "hint.goto.only-this-pane"),
        ('q', "hint.goto.close-this-pane"),
        ('"', "menu.paste.title"),
        ('c', "hint.conflict.title"),
    ];

    /// What `g` may be finished with.
    const GOTO_KEYS: &'static [(&'static str, &'static str)] = &[
        ("g", "hint.goto.start-of-file"),
        ("e", "hint.goto.end-of-file"),
        ("h l", "hint.goto.line-start-or-end"),
        ("s", "hint.goto.first-non-blank"),
        ("f", "hint.goto.open-this-file"),
        ("x", "hint.goto.follow-link"),
        ("n p", "hint.goto.next-or-previous-file"),
        ("d w", "hint.goto.follow-note"),
        ("/ ?", "hint.goto.word-elsewhere"),
        ("J", "hint.join-with-line-below"),
    ];

    /// What `m` may be finished with — 「這一對」，以及拿它做什麼.
    const MATCH_KEYS: &'static [(&'static str, &'static str)] = &[
        ("m", "hint.match.pair"),
        ("i", "hint.match.inside"),
        ("a", "hint.match.around"),
        ("s", "hint.match.surround"),
        ("d", "hint.match.take-off"),
        ("r", "hint.change"),
    ];

    /// What `` ` `` may be finished with.
    ///
    /// The last row is `hint.vi.backtick`, which had never been read by
    /// anybody: the phrasebook is consulted only for keys that are *not*
    /// bound, and `` ` `` was bound to 轉小寫. As a group prefix it has a menu,
    /// and a vi reader looking for a mark meets the answer here.
    const CASE_KEYS: &'static [(&'static str, &'static str)] = &[
        ("l", "hint.case.lower"),
        ("u", "hint.case.upper"),
        ("`", "hint.case.switch"),
        ("", "hint.vi.backtick"),
    ];

    /// What `]` and `[` may be finished with — 「下一個這種東西」.
    const HOP_KEYS: &'static [(&'static str, &'static str)] = &[("c", "hint.hop.conflict")];

    /// What `空格 c` may be finished with — which side of the conflict to keep.
    const CONFLICT_KEYS: &'static [(&'static str, &'static str)] = &[
        ("o", "hint.conflict.ours"),
        ("t", "hint.conflict.theirs"),
        ("b", "hint.conflict.both"),
    ];

    /// What `t` offers **wherever the cursor is**.
    ///
    /// **The way in comes first.** `t` is a group in every mode (#206), so most
    /// of the time it is pressed by somebody who is *not* in a table yet — and
    /// the menu used to open with 「照這欄順排」 and never once mention `t t`.
    /// These six work anywhere, so they head every list, and on a page with no
    /// table under the cursor they are the whole list.
    const TABLE_KEYS: &'static [(&'static str, &'static str)] = &[
        ("o", "hint.table.back-to-prose"),
        ("b", "hint.table.operate-it"),
        ("f", "hint.table.draw-it"),
        // 摺格子 asks nothing about where the cursor is standing either — it is
        // a preference about how the page is *drawn* — and it had never once
        // been offered by any of the four lists, in the group whose whole
        // purpose is to say what `t` can be finished with.
        ("w", "hint.table.fold-wide-cells"),
        // 折行 stands beside 摺起 because it answers the same question — and
        // a key offered nowhere is a key nobody finds.
        ("a", "hint.table.wrap-wide-cells"),
        ("t", "hint.table.whole-window"),
        ("q", "hint.table.leave-the-window"),
        ("] [", "hint.table.next-or-previous"),
    ];

    /// What `t` adds inside a fenced block. A block is read where it lies
    /// (#216), so the keys that rewrite a file are not offered — because they
    /// are refused.
    const TABLE_KEYS_BLOCK: &'static [(&'static str, &'static str)] = &[
        ("/ ?", "hint.table.search-columns"),
        ("g", "hint.table.go-to-cell"),
        ("y p", "hint.table.yank-or-paste-column"),
    ];

    /// What `t` adds inside a Markdown table.
    const TABLE_KEYS_MD: &'static [(&'static str, &'static str)] = &[
        ("/ ?", "hint.table.search-columns"),
        ("g", "hint.table.go-to-cell"),
        ("r R", "hint.table.add-row"),
        ("c C", "hint.table.add-column"),
        ("d D", "hint.table.delete-row-or-column"),
        ("j k", "hint.table.move-row"),
        ("h l", "hint.table.move-column"),
        ("y p", "hint.table.yank-or-paste-column"),
        ("1s 1S", "hint.table.sort-by-column"),
        ("< = >", "hint.table.align-column"),
        // `F`, not `f` — the lowercase letter is 完整表格 since #283, and a
        // capital is what a command that rewrites every row of the file should
        // have wanted.
        ("F", "hint.table.line-it-up"),
        ("i", "hint.table.detail-panel"),
    ];

    /// What `t` adds in a delimited file, where the columns are the schema's.
    ///
    /// **What this table can actually do**, not what tables can do: `n`/`D`/
    /// `h`/`l` are not offered here because they are refused here, and the menu
    /// that listed them was the one place the editor said a key existed and
    /// then said it did not.
    const TABLE_KEYS_FILE: &'static [(&'static str, &'static str)] = &[
        ("/ ?", "hint.table.search-columns"),
        ("g", "hint.table.go-to-cell"),
        ("1s 1S", "hint.table.sort-by-column"),
        ("r R", "hint.table.add-row"),
        ("d", "hint.table.delete-row"),
        ("j k", "hint.table.move-row"),
        ("y p", "hint.table.yank-or-paste-column"),
        ("H", "hint.table.first-row-is-data"),
        ("e", "hint.table.schema"),
        ("i", "hint.table.detail-panel"),
    ];

    /// A table of key names read out in the reader's language.
    fn said(
        rows: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> Vec<(&'static str, String)> {
        rows.into_iter()
            .map(|(key, what)| (key, crate::messages::say(what, &[])))
            .collect()
    }

    /// The keys `t` offers, given what the cursor is standing in.
    fn table_keys(inside: Option<Bounds>) -> Vec<(&'static str, &'static str)> {
        let mut keys = Self::TABLE_KEYS.to_vec();
        keys.extend(
            match inside {
                Some(Bounds::Block) => Self::TABLE_KEYS_BLOCK,
                Some(Bounds::Md) => Self::TABLE_KEYS_MD,
                Some(Bounds::WholeFile) => Self::TABLE_KEYS_FILE,
                None => &[],
            }
            .iter()
            .copied(),
        );
        keys
    }

    /// Every key that may follow `leader`, in **any** context.
    ///
    /// The menus themselves stay context-sensitive on purpose — `t` inside a
    /// Markdown table offers what a Markdown table can do, and a delimited file
    /// offers what a schema can — so this does not replace them. It is their
    /// union, and it has one reader: the test that asks whether a key sequence
    /// the documents print is a key sequence the editor has. §5.2.2 found the
    /// same name wrong in two places eight times over; this is the second place
    /// made answerable.
    ///
    /// `None` means **any character follows**: `f`, `r`, `"`, `M`, `'` and the
    /// `mi`/`ma`/`ms`/`mr` pairs take a letter or a bracket the writer chooses,
    /// so there is no list for a document to be checked against.
    pub fn keys_after(leader: char) -> Option<Vec<String>> {
        let spell = |rows: &[(&'static str, &'static str)]| -> Vec<String> {
            rows.iter()
                .flat_map(|(keys, _)| keys.split_whitespace().map(str::to_string))
                .collect()
        };
        Some(match leader {
            // 空 is the first character of how the documents spell it: `空格 f`.
            ' ' | '空' => Self::SPACE_KEYS.iter().map(|(k, _)| k.to_string()).collect(),
            'g' => spell(Self::GOTO_KEYS),
            'm' => spell(Self::MATCH_KEYS),
            '`' => spell(Self::CASE_KEYS),
            ']' | '[' => spell(Self::HOP_KEYS),
            't' => [None, Some(Bounds::Block), Some(Bounds::Md), Some(Bounds::WholeFile)]
                .into_iter()
                .flat_map(|inside| spell(&Self::table_keys(inside)))
                .collect(),
            _ => return None,
        })
    }

    /// Run one key of a `Space` sequence.
    fn handle_space(&mut self, key: Key) {
        match key {
            Key::Char('e') => self.show_sidebar(crate::sidebar::View::Explorer),
            // The outline is the sidebar showing the view that has it.
            Key::Char('o') => self.show_sidebar(crate::sidebar::View::Outline),
            // 定義 (#215): the 拆分表 on the character under the cursor. The
            // table detail panel this key used to open is a table key, and now
            // lives in the table group as `t i`.
            Key::Char('d') => match self.char_at_cursor() {
                Some(ch) => self.look_up(ch, true),
                None => self.set_status(say!("ui.nothing-to-look-up")),
            },
            // 旁注 (§5.2.3 ②). A page carries one or two, and a reading is
            // typed at leisure — so it is worth a key, and worth a second one.
            // The levels, `auto` and `format` stay `:ruby` commands: those are
            // said once a document, not once a word.
            Key::Char('r') => self.enter_ruby_mode(),
            Key::Char('"') => self.open_paste_picker(),
            // 衝突 (#249): the three keys that end one. Under `空格` rather
            // than a letter of its own because every letter has one already,
            // and because a merge conflict is a thing that happens to a file
            // a few times a year — not a motion a writer's fingers know.
            Key::Char('c') => self.pending = Pending::Conflict,
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
            // 工作區 (Feature #176): one key, three meanings that are the same
            // meaning — 「另一個工作區」. Nothing open: open one, showing this
            // same place. Open: hand it the keys. `W`:收掉，留下你站着的這半。
            Key::Char('w') => match self.other.is_some() {
                false => {
                    let at = self.cursor;
                    self.open_split(at, None, self.current_buffer().display_name().to_string());
                    self.status = say!("pane.opened");
                }
                true => {
                    self.switch_pane();
                }
            },
            // Two ways out, because there are two things you might mean, and
            // both are one keystroke:
            //
            // `W` — 只留我這一半. This is the common one: you looked at the
            // preview and are done with it, or you decided to work in it and
            // want the window back. vi spells it `C-w o`(nly), and 空格 o is
            // the outline here, so the capital of the pane's own letter says
            // it instead.
            Key::Char('W') => {
                if self.close_split() {
                    self.status = say!("pane.only-one-left");
                }
            }
            // `q` — 關掉我這一半，鍵跟着到另一半. vi's `C-w q`, and the same
            // word: 「這一半我不要了」.
            Key::Char('q') => {
                if self.switch_pane() {
                    self.close_split();
                    self.status = say!("pane.closed");
                }
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
                sidebar.show(view);
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
        sidebar.show(view);
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
            View::Dictionary => self.dictionary_rows(),
        };
        if let Some(sidebar) = self.sidebar.as_mut() {
            sidebar.set_rows(rows);
        }
    }

    /// The 字典 panel's rows — Feature #215.
    ///
    /// The names are padded to the widest of them so the values line up down a
    /// column, and the padding is counted in **columns** rather than characters
    /// (`拆分` is two characters and four columns wide).
    ///
    /// A row with no value is a heading — the character itself at the top, and
    /// the 陸／臺／港 label above each block when the 拆分表 has more than one
    /// answer. `is_dir` is what the sidebar draws headings with; the flat views
    /// already spend the tree's fields on what they have instead of what a tree
    /// has, and this is that.
    fn dictionary_rows(&self) -> Vec<crate::sidebar::Row> {
        use crate::sidebar::Row;
        let heading = |name: String| Row {
            path: PathBuf::new(),
            name,
            depth: 0,
            is_dir: true,
            expanded: false,
        };
        let Some((ch, answer)) = self.dictionary.as_ref() else {
            return Vec::new();
        };
        let mut rows = vec![heading(ch.to_string())];
        let Some(fields) = answer else {
            return rows;
        };
        if fields.is_empty() {
            rows.push(Row {
                path: PathBuf::new(),
                name: say!("ui.not-in-the-table"),
                depth: 0,
                is_dir: false,
                expanded: false,
            });
            return rows;
        }
        let width = fields
            .iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, _)| yumete_cjk::str_width(name))
            .max()
            .unwrap_or(0);
        for (name, value) in fields {
            if value.is_empty() {
                rows.push(heading(name.clone()));
                continue;
            }
            let pad = " ".repeat(width.saturating_sub(yumete_cjk::str_width(name)));
            rows.push(Row {
                path: PathBuf::new(),
                name: format!("{name}{pad}  {value}"),
                depth: 0,
                is_dir: false,
                expanded: false,
            });
        }
        rows
    }

    /// Look this character up in the 拆分表 — `Space d`, and `Tab` on a
    /// candidate (#215).
    ///
    /// The editor does not hold the table: yume does, and only the front end
    /// has it. So the character is parked here and the panel is opened empty;
    /// the answer arrives on the next pass through the loop, one frame later,
    /// which is not long enough for a reader to see the gap.
    pub fn look_up(&mut self, ch: char, focus: bool) {
        self.dictionary_query = Some(ch);
        // Asked, unanswered: what is showing until the answer arrives is the
        // character alone, which is not the same panel as 「查不到」.
        self.dictionary = Some((ch, None));
        match self.sidebar.as_mut() {
            Some(sidebar) => sidebar.show(crate::sidebar::View::Dictionary),
            None => {
                let root = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
                let mut sidebar = crate::sidebar::Sidebar::new(&root);
                sidebar.show(crate::sidebar::View::Dictionary);
                self.sidebar = Some(sidebar);
            }
        }
        // Asked from the page, the keys go with the question. Asked while a
        // word is being typed, they must not — the reader is mid-word, and the
        // panel is only there to be glanced at.
        self.sidebar_focus = focus;
        self.refresh_sidebar();
    }

    /// The character `Space d` or `Tab` asked about, for the front end to
    /// answer once (#215).
    pub fn take_dictionary_query(&mut self) -> Option<char> {
        self.dictionary_query.take()
    }

    /// The answer to [`Editor::take_dictionary_query`].
    ///
    /// Dropped if the reader has since asked about a different character —
    /// the answer to last frame's question must not overwrite this frame's.
    pub fn set_dictionary(&mut self, ch: char, fields: Vec<(String, String)>) {
        if self.dictionary.as_ref().is_some_and(|(at, _)| *at != ch) {
            return;
        }
        self.dictionary = Some((ch, Some(fields)));
        self.refresh_sidebar();
    }

    /// The character the 字典 panel is about, and the answer if one has come.
    pub fn dictionary(&self) -> Option<(char, Option<&[(String, String)]>)> {
        self.dictionary
            .as_ref()
            .map(|(ch, answer)| (*ch, answer.as_deref()))
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
    pub fn sidebar_keys() -> String {
        say!("hint.sidebar.keys")
    }

    /// How many rows `J`/`K` move in a list — a screenful of a sidebar, near
    /// enough. The sidebar does not know how tall it is drawn (the front end
    /// does), and a list moves by a *fixed* amount for the same reason `J`
    /// moves by half a page in the text: the eye keeps its place.
    const PAGE_IN_A_LIST: usize = 12;

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
            // **A list pages by the same keys the page does.** `J`/`K` are
            // half a page in the text; a 700-chapter outline is the one list
            // where walking it by `j` is not walking, and `PageDown` is not on
            // every keyboard a novelist owns.
            Key::Char('J') | Key::PageDown => {
                for _ in 0..Self::PAGE_IN_A_LIST {
                    sidebar.step(true);
                }
            }
            Key::Char('K') | Key::PageUp => {
                for _ in 0..Self::PAGE_IN_A_LIST {
                    sidebar.step(false);
                }
            }
            // …and the ends, spelled as they are in the text.
            Key::Char('g') | Key::Home => sidebar.go_to_end(false),
            Key::Char('G') | Key::End => sidebar.go_to_end(true),
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
                    say!("sidebar.wide")
                } else {
                    say!("sidebar.narrow")
                };
            }
            Key::Char('h') | Key::Left => sidebar.collapse(),
            Key::Char('l') | Key::Right | Key::Enter => {
                let chosen = sidebar.activate();
                match chosen {
                    Some(crate::sidebar::Chosen::File(path)) => {
                        if let Err(err) = self.open_file(&path) {
                            self.status = say!("buffer.cannot-open", path.display(), err);
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
                                self.status = say!("buffer.cannot-open", path.display(), err)
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
            self.status = say!("picker.no-files-here");
            return;
        }
        self.grep_root = Some(root);
        self.picker = Some(crate::picker::Picker::new(&say!("picker.files"), items));
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
        self.picker = Some(crate::picker::Picker::new(&say!("picker.buffers"), items));
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
            Key::PageDown => {
                for _ in 0..10 {
                    picker.step(true);
                }
            }
            Key::PageUp => {
                for _ in 0..10 {
                    picker.step(false);
                }
            }
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
                            self.status = say!("buffer.cannot-open", path, err);
                        }
                    }
                    Some(crate::picker::Item::Buffer(i, _)) => self.show_buffer(i),
                    // The picker belongs to whichever key opened it, so
                    // choosing from it lands the way that key lands.
                    Some(crate::picker::Item::Row(line, _)) => {
                        let preview = self.definition_preview;
                        self.land_on_row(line, preview)
                    }
                    Some(crate::picker::Item::Paste(Some(which), _)) => {
                        self.paste_from_menu(which)
                    }
                    // The system clipboard is the front end's to read.
                    Some(crate::picker::Item::Paste(None, _)) => self.clipboard_paste(true),
                    None => self.status = say!("picker.nothing-matched"),
                }
            }
            Key::Char(c) => picker.push(c),
            // A query is typed text, and typed text is edited in the middle.
            Key::Delete => picker.delete(),
            Key::Left => picker.move_caret(crate::picker::Caret::Left),
            Key::Right => picker.move_caret(crate::picker::Caret::Right),
            Key::Home | Key::Ctrl('a') => picker.move_caret(crate::picker::Caret::Start),
            Key::End | Key::Ctrl('e') => picker.move_caret(crate::picker::Caret::End),
            Key::Ctrl('u') => picker.clear_before_caret(),
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
        // **A spreadsheet's clipboard becomes rows** (Feature #226). Excel,
        // Numbers, LibreOffice and a browser table all put tab-separated lines
        // on the clipboard, and until now the cell refused every one of them
        // for holding a tab — the writer got 「格子裏不能有 Tab」 for the one
        // paste a table editor exists to accept. `t p` had already learned to
        // read a block out of the register; this is the same block arriving by
        // the other door, and it lands the same way.
        if (self.mode == Mode::Insert || self.mode == Mode::Normal) && self.table_here() {
            if let Some(grid) = sniff_grid(text.trim_end_matches(['\n', '\r'])) {
                self.paste_grid(grid);
                return;
            }
        }
        // **Judged before anything happens**, in Insert as well as in Normal:
        // the Insert branch used to hand the text to `insert_str`, which
        // silently drops what a cell refuses, and then say 「貼了 8 個字」 about
        // a paste that had not happened. It also spent an undo point on it.
        if self.mode == Mode::Insert || self.mode == Mode::Normal {
            if let Some(why) = self.cell_refuses_text(text) {
                self.status = why;
                return;
            }
        }
        self.snapshot();
        match self.mode {
            // In Insert it lands where the caret is, like anything typed.
            Mode::Insert => self.insert_str(text),
            // In Normal it replaces the selection, which is what `p` over a
            // selection does — and what a writer means by pasting over
            // something they have just picked out.
            Mode::Normal => {
                // **Both halves judged before either runs**, the way `r` and
                // `R` already do it: pasting a comma into a cell used to
                // delete what was selected and *then* refuse the paste, so the
                // cell came back short and the message only talked about the
                // refusal.
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
            Mode::Command | Mode::Lookfor | Mode::Search | Mode::Ruby => {
                for c in text.chars().filter(|c| !c.is_control()) {
                    self.command_line.push(c);
                }
                self.completion = None;
            }
            Mode::Picker => {}
        }
        self.status = say!("edit.pasted-characters", text.chars().count());
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
            self.status = say!("edit.nothing-selected");
            return;
        }
        // Into the editor's own register too: having copied something, `p` is
        // the next thing a hand reaches for.
        self.store(text.clone());
        let n = text.chars().count();
        self.clipboard_request = Some(text);
        self.status = say!("edit.copied-to-clipboard", n);
    }

    /// Put the cursor at char index `pos`, starting a selection there
    /// (Feature #109).
    pub fn point_at(&mut self, pos: usize) {
        let pos = pos.min(self.current_buffer().char_count());
        self.extend = false;
        self.anchor = pos;
        self.cursor = pos;
        self.refresh_goal_column();
        // **The mouse leaves a guessed block too** (#275). Walking out of one
        // with `j` drops it at the end of `on_key`; clicking out of one never
        // went through `on_key`, so the mode stayed on until the next
        // keystroke — the cursor was in the paragraph and `hjkl` were still
        // walking cells.
        self.forget_a_guessed_table();
        self.find_the_table_here();
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
            self.status = say!("edit.clipboard-empty");
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
        self.move_to_line(n);
    }

    /// The same, without noting a jump.
    ///
    /// For the callers that have already noted one — a mark, `:table jump` — where a
    /// second note would be of the place *after* the file switch, and `C-o`
    /// would then take you to the file you had just arrived in.
    fn move_to_line(&mut self, n: usize) {
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        let line = n.saturating_sub(1).min(last);
        let at = rope.line_to_char(line);
        let pos = motion::line_first_non_blank(rope, at);
        self.move_head(pos);
    }

    // ---- Marks (Feature #45) ----------------------------------------------

    /// Name this place by a letter (`M a`).
    ///
    /// The jump list remembers where you *came from*; a mark remembers where
    /// you meant to come back **to** — the scene you are rewriting, the note
    /// at the end of the file, the chapter you keep checking against. `M` and
    /// `'` rather than vi's `m` and `'`, because `m` here opens match mode.
    fn set_mark(&mut self, name: char) {
        if !name.is_alphanumeric() {
            self.status = say!("goto.mark-name-one-character");
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let spot = match self.current_buffer().path() {
            Some(path) => Spot::InFile(path.to_path_buf(), line),
            None => Spot::InBuffer(self.current_buffer().id(), self.cursor),
        };
        self.marks.insert(name, spot);
        self.status = say!("goto.mark-set", name, self.current_buffer().display_name(), line + 1);
    }

    /// Go back to the place a letter names (`' a`).
    fn go_to_mark(&mut self, name: char) {
        let Some(spot) = self.marks.get(&name).cloned() else {
            self.status = say!("goto.no-such-mark", name);
            return;
        };
        self.remember_jump();
        match spot {
            Spot::InFile(path, line) => {
                if self.current_buffer().path() != Some(path.as_path()) {
                    if let Err(err) = self.open_file(&path) {
                        self.status = say!("buffer.cannot-open", path.display(), err);
                        return;
                    }
                }
                self.move_to_line(line + 1);
                self.status = say!("goto.mark-in-another-file", name, self.current_buffer().display_name(), line + 1);
            }
            Spot::InBuffer(id, pos) => {
                let Some(i) = self.buffer_with(id) else {
                    self.status = say!("goto.mark-buffer-closed", name);
                    return;
                };
                self.show_buffer(i);
                self.set_cursor(pos.min(self.current_buffer().rope().len_chars()));
                self.status = say!("goto.mark-in-this-file", name);
            }
        }
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
        // **This move was a jump**, which is what the page needs to know to
        // decide where to put the cursor: a jump lands in the middle, because
        // a search hit `scrolloff` from an edge shows nothing on one side of
        // the thing that was looked for. Cleared on the next key, so it
        // describes the move that just happened and nothing after it.
        self.jumped = true;
        let here = (self.current_buffer().id(), self.cursor);
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
                self.status = say!("goto.no-earlier-jump");
                return;
            }
            // Stepping back for the first time has to note where we are, or
            // `C-i` would have nowhere to return to.
            if self.jump_at == self.jumps.len() {
                let here = (self.current_buffer().id(), self.cursor);
                if self.jumps.last() != Some(&here) {
                    self.jumps.push(here);
                }
            }
            self.jump_at -= 1;
        } else {
            if self.jump_at + 1 >= self.jumps.len() {
                self.status = say!("goto.no-later-jump");
                return;
            }
            self.jump_at += 1;
        }
        let (id, cursor) = self.jumps[self.jump_at];
        // A buffer that has since been closed leaves its jumps behind rather
        // than sending you to whichever file took its place in the list.
        match self.buffer_with(id) {
            Some(i) if i != self.current => self.show_buffer(i),
            Some(_) => {}
            None => {
                self.status = say!("goto.that-file-is-closed");
                return;
            }
        }
        let len = self.current_buffer().rope().len_chars();
        self.move_head(cursor.min(len));
        self.status = say!("goto.jump-list-position", self.jump_at + 1, self.jumps.len());
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

        let forward = kind == FindKind::Forward;
        let found = if forward {
            (col + 1..chars.len()).find(|&i| chars[i] == target)
        } else {
            (0..col).rev().find(|&i| chars[i] == target)
        };

        let Some(idx) = found else {
            self.status = say!("find.no-such-character-on-this-line", target);
            return;
        };
        let head = match kind {
            FindKind::Forward | FindKind::Backward => line_start + idx,
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
                    self.status = say!("table.esc-before-moving");
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
                    self.status = say!("table.enter-makes-no-newline");
                    return;
                }
                // At the cell's own start there is nothing of this cell to
                // delete, and the character before it is the delimiter.
                Key::Backspace if self.at_cell_start() => {
                    self.status = say!("table.backspace-would-join-cells");
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
            // Forward delete. It did nothing at all before — the key never
            // reached the editor, in any mode.
            Key::Delete => self.delete_at_cursor(),
            // `C-w` and `C-u` are in vi, in Helix, in readline and in every
            // terminal prompt a person has ever typed at, and Insert mode ate
            // both. Through an IME that mattered more than it looks: taking
            // back a 詞 the candidate list got wrong meant holding Backspace down.
            Key::Ctrl('w') => self.delete_word_before_cursor(),
            Key::Ctrl('u') => self.delete_to_line_start(),
            // Across the break, as in 常模 — outside a cell, where the two
            // arms above hold the arrows to the cell they are writing in.
            Key::Left => self.move_horizontal(motion::prev_grapheme),
            Key::Right => self.move_horizontal(motion::next_grapheme),
            Key::Up => self.move_vertical(true),
            Key::Down => self.move_vertical(false),
            // A page at a time, while typing: the same motion Normal makes,
            // because a page is a page whichever mode you are in.
            Key::PageUp => self.move_page(1, true, 1.0),
            Key::PageDown => self.move_page(1, false, 1.0),
            // `C-a`/`C-e` are the same two places, and are what a hand that
            // has ever used a terminal prompt reaches for — the `:` line has
            // taken them all along, and Insert swallowed them.
            Key::Home | Key::Ctrl('a') => {
                let pos = motion::line_start(self.current_buffer().rope(), self.cursor);
                self.set_cursor(pos);
            }
            Key::End | Key::Ctrl('e') => {
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
            // A second `:` on an **empty** line opens the search over what the
            // commands do (#224). Only on an empty one: `:s/:/：/` is a
            // substitution with two colons in it.
            Key::Char(':') if self.command_line.is_empty() => {
                self.mode = Mode::Lookfor;
                self.lookfor_focus = 0;
            }
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

    /// The `::` line: searching the commands by what they **do** (#224).
    ///
    /// Nothing here runs anything. ⇥ — and ⏎, which is the same gesture aimed
    /// at the same row — writes the whole command back into the `:` line and
    /// goes back there with the caret after it, so what Enter finally runs is
    /// always the line the reader can see. `:q!` is not undoable, and a mode
    /// that guessed which command was meant would eventually guess that one.
    fn on_lookfor_key(&mut self, key: Key) {
        match key {
            Key::Esc => self.close_prompt(),
            // Backspacing `::` empty goes back to `:`, not out to the page:
            // the second colon is the last thing there was to take back.
            Key::Backspace if self.command_line.is_empty() => {
                self.mode = Mode::Command;
                self.lookfor_focus = 0;
            }
            Key::Up | Key::BackTab => {
                self.lookfor_focus = self.lookfor_focus.saturating_sub(1)
            }
            Key::Down => {
                let found = lookfor::look(&self.command_line).len();
                if self.lookfor_focus + 1 < found {
                    self.lookfor_focus += 1;
                }
            }
            Key::Tab | Key::Enter => self.adopt_lookfor(),
            other => {
                self.edit_prompt(other);
                self.lookfor_focus = 0;
            }
        }
    }

    /// Take the highlighted row back to the `:` line, whole.
    fn adopt_lookfor(&mut self) {
        let (found, focus) = self.lookfor_menu();
        let Some(hit) = found.get(focus) else {
            // Nothing found, so there is nothing to take. Saying so beats
            // dropping the reader onto an empty `:` line that looks as if the
            // search had been thrown away.
            self.status = say!("lookfor.nothing-to-take");
            return;
        };
        self.command_line = hit.choice.written();
        self.command_caret = self.command_line.chars().count();
        self.completion = None;
        self.lookfor_focus = 0;
        self.mode = Mode::Command;
    }

    /// What the `::` line has turned up, and which row is highlighted.
    ///
    /// Worked out afresh from the line rather than kept: 221 rows scored
    /// against a few characters is microseconds, and a cached list is a list
    /// that can disagree with what is on the prompt.
    pub fn lookfor_menu(&self) -> (Vec<lookfor::Hit>, usize) {
        let found = lookfor::look(&self.command_line);
        let focus = self.lookfor_focus.min(found.len().saturating_sub(1));
        (found, focus)
    }

    /// Shut the prompt and forget what was on it.
    fn close_prompt(&mut self) {
        self.command_line.clear();
        self.command_caret = 0;
        self.history_at = None;
        self.lookfor_focus = 0;
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
            // Forward delete on the prompt: the character *under* the caret,
            // and the caret stays where it is.
            Key::Delete => {
                if self.command_caret < len {
                    let from = byte(&self.command_line, self.command_caret);
                    let to = byte(&self.command_line, self.command_caret + 1);
                    self.command_line.replace_range(from..to, "");
                }
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
                // Counted in **characters**, not bytes: a full-width space —
                // which a Chinese writer types without thinking about it — is
                // three bytes, and `byte index + 1` lands inside it.
                let cut = kept
                    .char_indices()
                    .rev()
                    .find(|(_, c)| c.is_whitespace())
                    .map_or(0, |(i, c)| kept[..i + c.len_utf8()].chars().count());
                let keep: String = head.chars().take(cut).collect();
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
    /// `:q` — close **this file**; leave only when it was the last one.
    ///
    /// Vim's rule and helix's, and the one a writer means: `:q` on the third
    /// of three open chapters puts you back in the second, not out on the
    /// shell. `:qa` is the way out with files still open, and `:q` on the last
    /// one is the same thing.
    fn quit(&mut self, force: bool) -> Result<CommandOutcome, EditorError> {
        if self.buffers.len() > 1 {
            return self.close_buffer(force);
        }
        self.quit_all(force)
    }

    /// `:qa` — leave, however many files are open.
    fn quit_all(&mut self, force: bool) -> Result<CommandOutcome, EditorError> {
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
        if self.refuse_readonly() {
            return;
        }
        let at = self.cursor;
        match self.current_buffer_mut().undo(at) {
            Some(cursor) => {
                self.cursor = cursor;
                self.anchor = cursor;
                self.clamp_cursor();
            }
            None => self.status = say!("edit.undo-at-oldest"),
        }
    }

    /// Redo the last undone change to this buffer (`:redo`).
    fn redo(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let at = self.cursor;
        match self.current_buffer_mut().redo(at) {
            Some(cursor) => {
                self.cursor = cursor;
                self.anchor = cursor;
                self.clamp_cursor();
            }
            None => self.status = say!("edit.redo-at-newest"),
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
        // `n` with nothing to repeat used to do nothing and say nothing, which
        // reads as a key that is broken rather than one with no answer yet.
        if self.last_search.is_empty() {
            self.status = say!("find.nothing-searched-yet");
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
            None => self.status = say!("find.not-found", pattern),
        }
    }

    // ---- Substitute (Feature #15) -----------------------------------------

    /// Replace `pattern` with `replacement` on the cursor's line, or on every
    /// line when `whole_file`; `global` replaces every match on a line.
    fn substitute(&mut self, how: Substitution<'_>) {
        if self.refuse_readonly() {
            return;
        }
        let Substitution {
            pattern,
            replacement,
            global,
            ignore_case,
            count_only,
            reshape,
            rows,
        } = how;
        if pattern.is_empty() {
            self.status = say!("find.empty-pattern");
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
        if count > 0 && !reshape {
            if let Some(why) = self.substitution_breaks_the_grid(&rebuilt) {
                self.status = why;
                return;
            }
        }
        // `n` in vi means "count, and change nothing". It used to substitute.
        if count_only {
            self.status = say!("find.substitute-nothing-changed", count);
            return;
        }
        if count > 0 {
            self.snapshot();
            let len = self.current_buffer().char_count();
            let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(0..len, &rebuilt));
            if !self.applied(done) {
                return;
            }
            self.clamp_cursor();
            self.anchor = self.cursor;
            self.refresh_goal_column();
        }
        self.status = say!("find.substitute-changed", count);
    }

    /// The first and last line a `:s` range names.
    fn substitution_rows(&self, rows: crate::command::Rows) -> (usize, usize) {
        use crate::command::{Bound, Rows};
        let rope = self.current_buffer().rope();
        let last_line = motion::last_line(rope);
        let resolve = |b: Bound| match b {
            Bound::Line(n) => n.saturating_sub(1).min(last_line),
            Bound::Cursor => rope.char_to_line(self.caret().min(rope.len_chars())),
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
        // A locked buffer does not get an Insert mode to type into
        // (Feature #213). Refusing here rather than at each keystroke is the
        // difference between 「只讀」 once and a status line that says it forty
        // times while the writer works out that nothing is going in.
        if self.refuse_readonly() {
            return;
        }
        // 延伸模式 is left at the door. It is a *mode* kept outside `Mode`, so
        // every operation has had to remember to clear it and some did not —
        // `v i X Esc` came back to Normal still extending, and the next `j`
        // grew a selection instead of moving.
        self.extend = false;
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
    /// space and joining two 漢字 with one would insert text nobody ever
    /// typed. Between Latin words the space is kept.
    fn join_lines(&mut self) {
        if self.refuse_readonly() {
            return;
        }
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
        let done = self.current_buffer_mut().replace(end..next, glue);
        if !self.applied(done) {
            return;
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

    /// Replace the selection with already-composed text — `r` running the IME.
    ///
    /// One character keeps [`Self::replace_chars`]'s promise and fills the
    /// whole selection: 「錢塘江」 `r` ■ is ■■■, because that is what `r` has
    /// always meant and one 字 is what `r` has always taken.
    ///
    /// More than one character is a different intent. 「錢」 `r` 春天 is 春天 —
    /// there is no way to fill a three-character selection with a two-character
    /// word, so the selection is replaced instead of written over. A trailing
    /// line ending is still not part of it: `x r` must not run two paragraphs
    /// together, whichever length the reader committed.
    fn replace_str(&mut self, text: &str) {
        let mut chars = text.chars();
        let (first, second) = (chars.next(), chars.next());
        if second.is_none() {
            if let Some(c) = first {
                self.replace_chars(c);
            }
            return;
        }
        let (start, mut end) = self.selection();
        end = end.min(self.current_buffer().char_count());
        let rope = self.current_buffer().rope();
        while end > start && matches!(rope.char(end - 1), '\n' | '\r') {
            end -= 1;
        }
        if start >= end {
            return;
        }
        self.snapshot();
        if !self.overwrite(start, end, text) {
            return;
        }
        // The new text is the selection, the way `c` leaves what it inserted:
        // the reader looked at a word and now looks at the word that took its
        // place.
        let end = start + text.chars().count();
        self.anchor = start;
        self.cursor = motion::prev_grapheme(self.current_buffer().rope(), end).max(start);
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
        // Read before `recall`, which *takes* it: asking afterwards always
        // answered `None`, so the reader who named a register was told 「nothing
        // has been yanked」 about a register they had just named.
        let named = self.pending_register;
        let text = self.recall();
        if text.is_empty() {
            // A key that does nothing and says nothing is indistinguishable
            // from a key that is broken — and the reader whose `y` went to a
            // named register is the one most likely to press this.
            self.status = match named {
                Some(name) => say!("register.empty", name),
                None => say!("register.nothing-yanked"),
            };
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
        if self.refuse_readonly() {
            return;
        }
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
        let done = self
            .current_buffer_mut()
            .replace(line_start + start..line_start + end, &text);
        if !self.applied(done) {
            return;
        }
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

    /// `:ruby auto` — write the readings in, by word (Feature #234).
    ///
    /// **Why a word and not a character.** 了 is `le` in 為了 and `liǎo` in
    /// 了解, and every tool that annotates 拼音 one character at a time gets one
    /// of those wrong — Word's 拼音指南 included, which is why nobody uses it
    /// twice. The reading comes from the 讀音表 by whole word, so the polyphone
    /// is settled by the word it is in rather than by a coin toss over a
    /// dictionary entry; see [`yumete_cjk::Reader`].
    ///
    /// **What it cannot settle: a polyphone whose readings differ only in
    /// tone.** The 讀音表 is toneless — 認為 is stored `ren wei` — so 為 `wéi`
    /// and 為 `wèi` ask it the same question and it gives the same answer;
    /// 難, 好, 教 and 中 are the same shape. Those take the common reading and
    /// may need a hand. 了, 行, 和 and 長 differ in spelling and are settled.
    ///
    /// **`rare` is the one people actually want.** A novel with a reading over
    /// every character is a textbook, not a novel; a novel with a reading over
    /// the handful nobody knows is a novel a reader can finish. So `:ruby auto
    /// rare` keeps only the words holding a character that **no** standard in
    /// current use carries — 通用規範, 通規繁, 臺灣, 香港 — and keeps the
    /// *word*, because one character of a two-character word read alone is
    /// worse typography than either extreme.
    ///
    /// The markup is mono-ruby ([`crate::ruby::SPLIT`]): one group per
    /// character, which is how CJK ruby is set and what [`crate::zong`] already
    /// spaces the 縱 out for.
    fn auto_ruby(&mut self, rare: bool) {
        if self.refuse_readonly() {
            return;
        }
        if !self.reader.available() {
            self.status = say!("ruby.auto-no-readings");
            return;
        }
        let dialect = self.file_dialect();
        // A standing selection is the region; without one it is the whole file,
        // which is the pass a manuscript is actually given. `u` takes all of it
        // back in one step either way.
        let (from, to) = if self.anchor == self.cursor {
            (0, self.current_buffer().char_count())
        } else {
            self.selection()
        };
        let rebuilt = {
            let rope = self.current_buffer().rope();
            let first = rope.char_to_line(from);
            let last = rope.char_to_line(to.saturating_sub(1).max(from));
            let mut edits: Vec<(usize, usize, String)> = Vec::new();
            for line in first..=last.min(rope.len_lines().saturating_sub(1)) {
                let line_start = rope.line_to_char(line);
                let chars: Vec<char> = crate::zong::line_chars(rope, line);
                // Whatever is already annotated stays as it is: a reader who
                // corrected one reading by hand does not get it overwritten by
                // the command that offered to help.
                let groups = crate::ruby::all_groups(&chars);
                // The segmenter itself, not [`Self::segment_line`]: that one
                // drops words the reader can already see the edges of, which is
                // right for the overlay and wrong here — 字 sitting alone after
                // a `</ruby>` is exactly a word this pass has to annotate.
                let text: String = chars.iter().collect();
                for (ws, we) in self.segmenter.segment(&text) {
                    let (start, end) = (line_start + ws, line_start + we);
                    if start < from || end > to || we > chars.len() {
                        continue;
                    }
                    if groups.iter().any(|g| ws < g.end && g.start < we) {
                        continue;
                    }
                    let word: String = chars[ws..we].iter().collect();
                    if word.is_empty() || !word.chars().all(is_han) {
                        continue;
                    }
                    if rare && !word.chars().any(|c| self.reader.is_rare(c) == Some(true)) {
                        continue;
                    }
                    let Some(readings) = self.reader.read(&word) else {
                        continue;
                    };
                    // A reading that does not cover the word is not a reading —
                    // writing it would put syllables over the wrong characters.
                    if readings.len() != we - ws {
                        continue;
                    }
                    let split = crate::ruby::SPLIT.to_string();
                    let markup =
                        crate::ruby::markup(&chars[ws..we], &readings.join(&split), dialect);
                    edits.push((start, end, markup));
                }
            }
            if edits.is_empty() {
                None
            } else {
                let mut out = String::with_capacity(rope.len_chars());
                let mut at = 0usize;
                for (start, end, markup) in &edits {
                    out.push_str(&rope.slice(at..*start).to_string());
                    out.push_str(markup);
                    at = *end;
                }
                out.push_str(&rope.slice(at..rope.len_chars()).to_string());
                Some((edits.len(), out))
            }
        };
        let Some((n, rebuilt)) = rebuilt else {
            self.status = say!("ruby.auto-none");
            return;
        };
        // The same rule `:replace` and `:format ruby` keep: a 拆分表 whose cells
        // hold readings must not gain a field because a command rewrote it.
        if let Some(why) = self.substitution_breaks_the_grid(&rebuilt) {
            self.status = why;
            return;
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.current_buffer_mut().replace(0..len, &rebuilt);
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
        self.status = say!("ruby.auto-added", n);
    }

    /// Rewrite every reading in the buffer into one dialect (`:format-ruby-…`).
    fn format_ruby(&mut self, dialect: Dialect) {
        if self.refuse_readonly() {
            return;
        }
        let text = self.current_buffer().text();
        let Some(formatted) = crate::ruby::reformat(&text, dialect) else {
            self.status = say!("ruby.already-in-that-form", dialect.name());
            return;
        };
        // The same rule `:replace` keeps, in the sibling that rewrites just as
        // much text: `#ruby("永", "ㄩㄥˇ")` carries a comma, so reformatting a
        // 拆分表 whose cells hold readings gave every one of those rows an
        // extra field — silently, in one keystroke, across the whole file.
        if let Some(why) = self.substitution_breaks_the_grid(&formatted) {
            self.status = why;
            return;
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.current_buffer_mut().replace(0..len, &formatted);
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
        self.status = say!("ruby.rewritten-as", dialect.name());
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
        if matches[next].name.is_empty() {
            return;
        }
        // A word promoted out of its parent's list writes the parent too:
        // picking `scheme` out of what `:yume` takes leaves `:yume scheme`.
        let chosen = matches[next].written();
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
            self.status = say!("ruby.put-cursor-in-a-reading");
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
            // Backspacing to empty does *not* leave: an empty reading is a
            // meaningful thing to submit here — it is how an annotation is
            // taken off — so it has to be reachable. Esc is the way out.
            Key::Backspace if self.command_line.is_empty() => {}
            Key::Enter => {
                let reading = std::mem::take(&mut self.command_line);
                let target = self.ruby_target.take();
                self.mode = Mode::Normal;
                self.command_caret = 0;
                if let Some(target) = target {
                    self.apply_reading(target, &reading);
                }
            }
            // **The same prompt keys as everywhere else.** A reading is typed
            // text like a command or a search, and this mode had no caret at
            // all: no `Left`, no `Home`, no `C-a`, no `C-w` — a typo in the
            // middle of a reading meant deleting back to it.
            other => self.edit_prompt(other),
        }
    }

    /// Write `reading` onto `target`, or strip the markup when it is empty.
    fn apply_reading(&mut self, target: RubyTarget, reading: &str) {
        if self.refuse_readonly() {
            return;
        }
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

        // A reading is text, and text going into a row obeys the row's rule:
        // `a,b` typed as a reading used to be written straight into the rope,
        // past every gate, and the row it was on gained a field. Asked of the
        // *file*, not of `:table`, since nobody turns table mode on to annotate
        // a character.
        if let Some(why) = self.replacement_reshapes_the_grid(span, &text) {
            self.status = why;
            return;
        }
        self.snapshot();
        let done = self.current_buffer_mut().replace(span.0..span.1, &text);
        if !self.applied(done) {
            return;
        }
        self.anchor = span.0;
        self.cursor = span.0;
        self.clamp_cursor();
        self.status = if reading.is_empty() {
            say!("ruby.removed")
        } else {
            say!("ruby.set", reading)
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
            self.status = say!("edit.nothing-to-repeat");
            return;
        }
        // …and a guard, the way `replay_macro` has one: whatever the recorder
        // manages to record, a repeat may never repeat itself.
        if self.repeating_edit {
            return;
        }
        let keys = self.last_edit_keys.clone();
        self.repeating_edit = true;
        for key in keys {
            self.on_key(key);
        }
        // An IME `r` left `Pending::Replace` armed — its answer came from the
        // candidate panel, not from a key, so there is nothing in `keys` to
        // supply it. Supply it here, or `.` would leave `r` waiting and eat
        // whatever the reader pressed next.
        if self.pending == Pending::Replace && !self.last_replacement.is_empty() {
            self.pending = Pending::None;
            let text = self.last_replacement.clone();
            self.replace_str(&text);
        }
        self.repeating_edit = false;
    }

    // ---- Match mode (Helix `m`) -------------------------------------------

    /// Link to the bracket matching the one under the cursor (`mm`).
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
            self.status = say!("edit.no-pair-around", open, close);
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
        if self.refuse_readonly() {
            return;
        }
        let Some((open, close)) = pair_of(c) else {
            return;
        };
        let (start, end) = self.selection();
        let end = end.max(start);
        self.snapshot();
        let done = {
            let buffer = self.current_buffer_mut();
            // The closer first, so writing it cannot shift the opener.
            buffer
                .insert(end, &close.to_string())
                .and_then(|()| buffer.insert(start, &open.to_string()))
        };
        if !self.applied(done) {
            return;
        }
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
        if self.refuse_readonly() {
            return;
        }
        let Some((start, end)) = self.innermost_pair() else {
            self.status = say!("edit.no-pair-to-delete");
            return;
        };
        self.snapshot();
        let done = {
            let buffer = self.current_buffer_mut();
            // The closer first, so removing it cannot shift the opener.
            buffer
                .remove(end..end + 1)
                .and_then(|()| buffer.remove(start..start + 1))
        };
        if !self.applied(done) {
            return;
        }
        self.cursor = self.cursor.saturating_sub(1);
        self.anchor = self.cursor;
        self.clamp_cursor();
    }

    /// Swap the innermost pair around the cursor for another (`mr`).
    fn surround_replace(&mut self, from: char, to: char) {
        if self.refuse_readonly() {
            return;
        }
        let (Some((open, close)), Some((new_open, new_close))) = (pair_of(from), pair_of(to))
        else {
            return;
        };
        let rope = self.current_buffer().rope();
        let Some((start, end)) = surrounding(rope, self.cursor, open, close) else {
            self.status = say!("edit.no-pair-around", open, close);
            return;
        };
        self.snapshot();
        let done = {
            let buffer = self.current_buffer_mut();
            // The closer first, so replacing it cannot shift the opener.
            buffer
                .replace(end..end + 1, &new_close.to_string())
                .and_then(|()| buffer.replace(start..start + 1, &new_open.to_string()))
        };
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
    }

    /// The nearest pair of delimiters enclosing the cursor, whichever kind.
    fn innermost_pair(&self) -> Option<(usize, usize)> {
        let rope = self.current_buffer().rope();
        PAIRS
            .iter()
            .filter_map(|&(open, close)| surrounding(rope, self.caret(), open, close))
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
            let fold = |line: usize| self.line_is_folded(line);
            let rope = self.current_buffer().rope();
            // …with the indent, because the indent is where a row *breaks*: a
            // measure without it wraps a different page from the one being
            // drawn, and `j` then lands on the character under a column nobody
            // is looking at. Same for the folds: a row the page does not draw
            // is a row `j` must not stop on.
            //
            // **Wrap off goes the same way**, at [`crate::wrap::NO_WRAP`]: one
            // row per paragraph is what a very large width gives, and the
            // alternative was a second answer to「which column is this」 that
            // did not know what is off the page.
            let width = self.wrap_width().unwrap_or(crate::wrap::NO_WRAP);
            let drawn = |line: usize| self.drawn_on_line(line);
            let typed = |line: usize| self.typed_on_line(line);
            // A table row is one row (#275) — and `j` has to be walking the
            // same page the renderer drew, or it steps into a row that is not
            // on the screen.
            let flat = |line: usize| self.table_row_at(line);
            let m = crate::wrap::Measure::new(width, &hide)
                .with_indent(self.paragraph_indent())
                .with_folds(&fold)
                .with_drawn(&drawn)
                .with_typed_drawn(&typed)
                .with_unwrapped(&flat)
                .with_open_line(self.open_line());
            crate::wrap::column_of(rope, self.cursor, m)
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
            let fold = |line: usize| self.line_is_folded(line);
            let rope = self.current_buffer().rope();
            // …with the indent and the folds, for the same reason: the page
            // `j` steps through has to be the page on the screen. With wrapping
            // off a row is a paragraph, which is what `NO_WRAP` gives — through
            // this same code, so the two cases cannot answer differently about
            // what is off the page.
            let width = self.wrap_width().unwrap_or(crate::wrap::NO_WRAP);
            let drawn = |line: usize| self.drawn_on_line(line);
            let typed = |line: usize| self.typed_on_line(line);
            // A table row is one row (#275) — and `j` has to be walking the
            // same page the renderer drew, or it steps into a row that is not
            // on the screen.
            let flat = |line: usize| self.table_row_at(line);
            let m = crate::wrap::Measure::new(width, &hide)
                .with_indent(self.paragraph_indent())
                .with_folds(&fold)
                .with_drawn(&drawn)
                .with_typed_drawn(&typed)
                .with_unwrapped(&flat)
                .with_open_line(self.open_line());
            if up {
                crate::wrap::prev_row(rope, self.cursor, m, self.goal_column)
            } else {
                crate::wrap::next_row(rope, self.cursor, m, self.goal_column)
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
        // The page is built here, from the same two answers the horizontal
        // side is built from — and it lives only as long as this block, which
        // is what lets the cursor be written after it.
        let (goal, pos) = {
            let hidden = |line: usize| self.markup_hidden_on_line(line);
            let folded = |line: usize| self.line_is_folded(line);
            let drawn = |line: usize| self.drawn_runs_on_line(line);
            let grid = self.grid_with(&hidden, &folded, &drawn);
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
            (goal, pos)
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

    /// Select from here up to — but not including — `pos`.
    ///
    /// The rule `w` already follows, and the one every forward motion that
    /// lands on *the start of the next thing* has to follow: the character that
    /// begins the next sentence belongs to the next sentence. Selecting through
    /// it means `)d` deletes this sentence and the first character of the one
    /// after it — a corruption a proofreader would not notice until the page
    /// was set.
    ///
    /// A motion that cannot advance (already at the end of the writing) leaves
    /// the selection where it is rather than running backwards.
    fn select_up_to(&mut self, pos: usize) {
        // `head.max(cursor)`, with no branch back to `pos`: standing *on* the
        // 。 that ends the sentence puts the next one exactly one grapheme
        // away, and the guard that was here — 「only step back when it
        // advances」 — fell through to `pos` in precisely that case and took
        // the next sentence's first character after all. When the motion
        // cannot advance, `max` collapses the selection where it stands, which
        // is the same thing standing still means everywhere else.
        let head = motion::prev_grapheme(self.current_buffer().rope(), pos);
        self.select_to(head.max(self.cursor));
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
        if self.refuse_readonly() {
            return false;
        }
        if let Some(why) = self.cell_refuses_text_at(Some(at), text) {
            self.status = why;
            return false;
        }
        let done = self.current_buffer_mut().insert(at, text);
        self.applied(done)
    }

    /// Whether a rewritten document would change any row's shape.
    ///
    /// `:s` is the one edit that rewrites whole lines at once, so it is checked
    /// as a whole: same number of rows, and each row with the same number of
    /// cells it had. A substitution that only changes what is *inside* cells
    /// passes, which is the useful kind — `:%s/⿰木/⿰禾/g` over a 拆分表.
    fn substitution_breaks_the_grid(&self, rebuilt: &str) -> Option<String> {
        // **A table is a table whether or not `:table` was typed.** This check
        // used to open with `self.table.as_ref()?`, so `:replace` — which
        // reaches every file `:grep` found, including files never opened — went
        // through 13 rows of this project's own `development.md` and broke them.
        let separator = self.grid_shape_here()?;
        let rows_only = separator == Separator::Pipe;
        // A delimited file is all cells. A document is not: only its table
        // rows are, and a paragraph that gains a `|` has gained a character.
        // Counting the whole document refused `:%s/前文/前 | 文/` on a line
        // nowhere near the table.
        let before = self.current_buffer().rope().to_string();
        // Numbered by the **document's** lines, not by the filtered list: for a
        // Markdown table the filtered index is a table-row number, and 「第 3
        // 行」 then names a line the writer cannot find.
        let count = |text: &str| -> Vec<(usize, usize)> {
            let rows = crate::mdtable::row_lines(text);
            text.lines()
                .enumerate()
                .filter(|(n, _)| !rows_only || rows.get(*n).copied().unwrap_or(false))
                .map(|(n, l)| {
                    // Unescaped only: `\|` is a pipe *inside* a cell, and the
                    // manual promises it works — so a substitution that adds
                    // one must not be refused as if it split a row.
                    let cells = match separator {
                        Separator::Pipe => crate::mdtable::pipes_from(l, false).len(),
                        Separator::Delimiter(d) => l.chars().filter(|&c| c == d).count(),
                    };
                    (n, cells)
                })
                .collect()
        };
        let (was, now) = (count(&before), count(rebuilt));
        if was.len() != now.len() {
            return Some(say!(
                "table.substitution-would-change-rows",
                was.len(),
                now.len()
            ));
        }
        let (line, from, to) = was
            .iter()
            .zip(&now)
            .find(|((_, a), (_, b))| a != b)
            .map(|((n, a), (_, b))| (*n, *a, *b))?;
        // **A refusal that names no way through is a wall.** It used to say
        // 「先 :table off」, which stopped being an escape the moment the check
        // stopped asking whether table mode was on.
        Some(say!(
            "table.substitution-would-change-width",
            line + 1,
            from + 1,
            to + 1
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
        // **This is the one that had no gate of its own** (§5.2.3 ⑤): six
        // callers reach it, and the ones that did not refuse first wrote
        // nothing and then moved the cursor over it. It asks the buffer now,
        // and the buffer answers.
        let done = self.current_buffer_mut().replace(start..end, text);
        self.applied(done)
    }

    /// Take a range out of the buffer, unless it would take a cell boundary
    /// with it.
    ///
    /// The other half of the invariant: **a row's delimiter count never
    /// changes while it is being read as a grid.** Deleting is how it was most
    /// easily broken — `d` on an empty cell sits exactly on the delimiter, so
    /// the collapsed selection covered it and two cells became one.
    fn edit_remove(&mut self, range: std::ops::Range<usize>) -> bool {
        if self.refuse_readonly() {
            return false;
        }
        if let Some(why) = self.cell_refuses_cut(range.clone()) {
            self.status = why;
            return false;
        }
        let done = self.current_buffer_mut().remove(range);
        self.applied(done)
    }

    /// Take the buffer's answer to an edit, and say so if it refused.
    ///
    /// `true` means the text moved and the caller may move the state around it
    /// — the cursor, the anchor, the status line. `false` means the buffer is
    /// read-only, the status line already says so, and **the caller must not
    /// touch any of that**: moving a cursor over text that was never written
    /// is the whole fault this returns a value to prevent (§5.2.3 ⑤).
    ///
    /// Most callers reach it after [`Editor::refuse_readonly`] has already
    /// turned them back, so the `Err` arm is unreachable there. That is the
    /// intent: the guard is the message, this is the proof.
    #[must_use]
    fn applied(&mut self, done: crate::buffer::Edit) -> bool {
        match done {
            Ok(()) => true,
            Err(crate::buffer::ReadOnly) => {
                self.status = say!("readonly.refused");
                false
            }
        }
    }

    /// Say why nothing happened, when the buffer is locked (Feature #213).
    ///
    /// `true` means the caller must not edit. The refusal that *matters* is in
    /// [`Buffer::insert`](crate::buffer::Buffer::insert) — the rope does not
    /// move whatever anyone here forgets. This one exists so the writer is told:
    /// an editor that swallows keystrokes in silence is one you stop trusting
    /// long before you work out why.
    fn refuse_readonly(&mut self) -> bool {
        if !self.current_buffer().is_readonly() {
            return false;
        }
        self.status = say!("readonly.refused");
        true
    }

    /// Whether the buffer on screen refuses to be edited (Feature #213).
    ///
    /// What draws `[只讀]` on the status line.
    pub fn is_readonly(&self) -> bool {
        self.current_buffer().is_readonly()
    }

    /// Lock every file this session opens, including the ones already open
    /// (`--readonly`).
    pub fn set_readonly_default(&mut self, on: bool) {
        self.readonly_default = on;
        if on {
            for buffer in &mut self.buffers {
                buffer.set_readonly(true);
            }
        }
        // Turning it *off* unlocks nothing on its own: a buffer the disk
        // itself calls read-only is locked for a reason of its own, and
        // `:readonly off` is how one buffer says otherwise.
    }

    /// Whether a clean buffer re-reads itself when the file changes underneath
    /// it (Feature #214).
    pub fn reload_auto(&self) -> bool {
        self.reload_auto
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
        if self.refuse_readonly() {
            return;
        }
        let end = motion::line_end(self.current_buffer().rope(), self.cursor);
        let row = self.blank_row();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().insert(end, &format!("\n{row}")));
        if !self.applied(done) {
            return;
        }
        self.cursor = end + 1;
        self.anchor = self.cursor;
        self.enter_insert();
    }

    /// Open a new line above the cursor and enter Insert mode (`O`).
    fn open_line_above(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let start = motion::line_start(self.current_buffer().rope(), self.cursor);
        let row = self.blank_row();
        let done =
            self.without_cell_guard(|e| e.current_buffer_mut().insert(start, &format!("{row}\n")));
        if !self.applied(done) {
            return;
        }
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
            None => motion::line_start(self.current_buffer().rope(), self.caret()),
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
                self.status = say!("table.divider-cannot-be-deleted");
                return;
            }
            let text = self.current_buffer().rope().slice(start..end).to_string();
            self.store(text);
            // **Nothing below this line may run on a refusal**: collapsing the
            // selection onto `start` over text that is still there is the
            // fault §5.2.3 ⑤ was decided to close, and this is where it was.
            let done = self.current_buffer_mut().remove(start..end);
            if !self.applied(done) {
                return;
            }
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
                self.status = say!("macro.recorded-keys", n);
            }
            None => {
                self.recording = Some(Vec::new());
                self.status = say!("macro.recording");
            }
        }
    }

    /// Play the last recorded macro back (Helix `Q`).
    ///
    /// A macro cannot start while one is playing, and cannot play inside
    /// itself: `Q` recorded into a macro would otherwise recurse until the
    /// stack ran out.
    fn replay_macro(&mut self, count: usize) {
        if self.replaying {
            return;
        }
        if self.macro_keys.is_empty() {
            self.status = say!("macro.nothing-recorded");
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
            say!("register.system-clipboard"),
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
            self.status = say!("edit.nothing-yanked-yet");
        }
        self.picker = Some(crate::picker::Picker::new(&say!("picker.paste"), items));
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
        self.status = say!("edit.pasted", name);
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
        self.status = say!("edit.yanked-characters", n);
    }

    /// Paste the register after (`p`) or before (`P`) the selection, and select
    /// the pasted text. Does nothing when the register is empty.
    fn paste(&mut self, after: bool) {
        // Read before `recall`, which *takes* it: asking afterwards always
        // answered `None`, so the reader who named a register was told 「nothing
        // has been yanked」 about a register they had just named.
        let named = self.pending_register;
        let text = self.recall();
        if text.is_empty() {
            // A key that does nothing and says nothing is indistinguishable
            // from a key that is broken — and the reader whose `y` went to a
            // named register is the one most likely to press this.
            self.status = match named {
                Some(name) => say!("register.empty", name),
                None => say!("register.nothing-yanked"),
            };
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

    /// Delete the grapheme **after** the cursor (Insert-mode Delete).
    ///
    /// Backspace's mirror: it takes the character the cursor is sitting in
    /// front of and leaves the cursor where it is. At the end of a line it
    /// takes the newline, joining the line below — which is what Backspace
    /// does at the start of one.
    fn delete_at_cursor(&mut self) {
        let rope = self.current_buffer().rope();
        if self.cursor >= rope.len_chars() {
            return;
        }
        let line = rope.char_to_line(self.cursor);
        let line_end = rope.line_to_char(line) + rope.line(line).len_chars();
        let end = match self.cursor + 1 >= line_end {
            // At the end of the line: the newline itself.
            true => self.cursor + 1,
            false => motion::right(rope, self.cursor),
        };
        let end = end.min(rope.len_chars());
        if end <= self.cursor {
            return;
        }
        if !self.edit_remove(self.cursor..end) {
            return;
        }
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
        // What this file held when the session first saw it, so that a day
        // whose row is opened at four in the afternoon counts from the morning
        // (Feature #244). Asked once per path: reopening a file that is already
        // in the map is the same session still writing it.
        let opened = self.buffers[self.current].path().map(Path::to_path_buf);
        if let Some(path) = opened {
            if !self.opened_with.contains_key(&path) {
                let text = self.buffers[self.current].rope().to_string();
                let han = self.han_in(&text);
                self.opened_with.insert(path, han);
            }
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
        self.md_tables.borrow_mut().take();
        self.table_on_open();
        self.anchor = 0;
        self.goal_column = 0;
        self.mode = Mode::Normal;
        self.extend = false;
        self.pending = Pending::None;
        self.operator_count = None;
        // A half-typed table command belonged to the buffer that is being left
        // — `t1a` and then `:e other.md` must not leave a column named for the
        // next file's table. From the keyboard the key that opens a buffer has
        // already cleared these; a command line and a restored session have
        // not.
        self.sequence = None;
        self.sort_keys.clear();
    }
}

impl Default for Editor {
    fn default() -> Self {
        Self::new()
    }
}

/// A place a mark names.
///
/// By **path and line**, not by buffer index and character offset: a mark is
/// meant to survive the afternoon, and in that time the buffer list will have
/// been reordered and the file edited. A buffer with no file keeps its index,
/// because there is nothing else to call it by.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Spot {
    InFile(PathBuf, usize),
    InBuffer(u64, usize),
}

/// How many files a session remembers.
///
/// Enough for an afternoon's chapters, and few enough that a project-wide
/// `:replace` — which opens every file it changes — cannot turn tomorrow
/// morning into a hundred-and-twenty-file startup.
const SESSION_FILES: usize = 24;

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
    /// The `t` flag: the writer means to change how many cells a row has.
    reshape: bool,
    rows: crate::command::Rows,
}

/// Whether a file is something a tool produced rather than something a writer
/// wrote.
///
/// The manuscript's own words are *in* the export, so searching the export
/// finds every hit twice — the second time in a file that cannot be edited and
/// will be overwritten. The same goes for a typesetter's output.
fn is_build_output(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".html", ".htm", ".pdf", ".epub", ".docx"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

/// Join names for a reader, in the reader's own punctuation.
///
/// 、 in Chinese and `, ` in English. A list built with one and shown in the
/// other is the commonest way a translated program gives itself away.
fn listed(items: &[String]) -> String {
    items.join(&say!("label.comma"))
}

/// Read a block of cells out of pasted text, if that is what it is.
///
/// **Tabs first.** Every spreadsheet — Excel, Numbers, LibreOffice, a browser
/// table — puts tab-separated rows on the clipboard, and a tab is the one
/// character that never appears in a cell by accident. Commas are read only
/// when every line has the same number of them, because a paragraph with two
/// commas in it is a paragraph.
///
/// One cell is not a block: text with no tab and no line break is what `p`
/// has always pasted, and goes on being it.
/// A delimiter as a person can read it on the status line.
///
/// A tab printed raw is a status line that says 「用「   」分欄」 — the one
/// delimiter a reader cannot see is the one this editor's own `:export tsv`
/// writes.
fn named_delimiter(delimiter: char) -> String {
    match delimiter {
        '\t' => "Tab".to_string(),
        d => d.to_string(),
    }
}

fn sniff_grid(text: &str) -> Option<Vec<Vec<String>>> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.is_empty() {
        return None;
    }
    let delimiter = if lines.iter().any(|l| l.contains('\t')) {
        '\t'
    } else if lines.len() > 1 {
        let commas = lines[0].matches(',').count();
        if commas == 0 || !lines.iter().all(|l| l.matches(',').count() == commas) {
            return None;
        }
        ','
    } else {
        return None;
    };
    let grid: Vec<Vec<String>> = lines
        .iter()
        .map(|l| l.split(delimiter).map(str::to_string).collect())
        .collect();
    // One cell in one row is not a block.
    (grid.len() > 1 || grid[0].len() > 1).then_some(grid)
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

mod checks;
mod conflicts;
mod tables;

#[cfg(test)]
mod tests;
