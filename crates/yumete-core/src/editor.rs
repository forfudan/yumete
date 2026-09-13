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

/// Every line's block and every merge conflict in the document, against the
/// buffer they were worked out for and that buffer's revision — the two things
/// that decide whether they are still true.
///
/// The conflicts ride along because they come out of the same walk: the block
/// scan already reads every line's opening, and the four markers are settled
/// by exactly those characters (#249).
type BlockCache = ((u64, u64), Vec<crate::markdown::Block>, Vec<crate::conflict::Conflict>);

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
    /// `空格 m` — what to keep of the merge conflict under the cursor.
    Conflict,
    /// A `:s …c` is asking about one match — `y`/`n`/`a`/`q`/`l` (#415).
    ///
    /// Unlike every other pending here this one is not opened by a key: the
    /// command opens it and it stays open across many keys, one per match.
    Confirm,
    /// `` ` `` — 「不改它說什麼，只改它長什麼樣」 (§5.2.3 ②).
    ///
    /// Helix spends three top-level keys here (`` ` `` 小寫, `` A-` `` 大寫,
    /// `~` 互換) on an operation that is the **identity on 漢字** — only
    /// full-width Ａ↔ａ actually maps. By the level law they can wait for a
    /// second key, so they became a group and the three keys are unbound.
    Case,
}

impl Pending {
    /// Is the key this is waiting for **a character of the document**, rather
    /// than a letter naming a command (#414)?
    ///
    /// The difference is who may type it. A register's name and a mark's are
    /// ASCII by construction — `"a`, `Ma` — but the character `f` looks for,
    /// the character `r` writes and the delimiter `ms` wraps with are all
    /// **what is in the manuscript**, and in this editor the manuscript is
    /// Chinese: 「，」, 「」, 【】. None of those can be typed on a keyboard
    /// with the input method held off, which is what Normal mode does — so
    /// until this question existed, `f，`, `ms「` and `mi《` were spellings
    /// nobody could reach. `r` had the one exception, hand-written.
    ///
    /// Asked once here rather than listed at each gate: the preedit, the
    /// candidate panel and the lone-Shift tap all light up from this one
    /// answer, and a pending added later is a pending this question is asked
    /// about.
    fn takes_a_character(self) -> bool {
        match self {
            Pending::Find(_)
            | Pending::Replace
            | Pending::MatchPair { .. }
            | Pending::Surround
            | Pending::SurroundFrom
            | Pending::SurroundTo(_) => true,
            // `y`/`n`/`a`/`q`/`l` name what to do, not what to write.
            Pending::Confirm | Pending::None
            | Pending::Goto
            | Pending::Space
            | Pending::Register
            | Pending::Case
            | Pending::Match
            | Pending::Table
            | Pending::Mark
            | Pending::Recall
            | Pending::Hop { .. }
            | Pending::Conflict => false,
        }
    }
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

/// How many mined words `:word-discover` writes into the list (Feature #239).
///
/// Two hundred. What is being written is not a report but a *file the writer
/// then reads line by line*, and a thousand lines of it would be deleted
/// unread — which is worse than not offering them, because the ninety good
/// ones go with the rest. Ranked by count, so the two hundred kept are the
/// names that are on every page.
const DISCOVER_LIMIT: usize = 200;

/// How much of a project `:word-discover` reads before it stops.
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

/// What the row below the status line has to say.
///
/// Structured rather than one string, so a key can be set apart from what it
/// does. A run of 「hjkl 走格 · c 換格 · y Y 取格/行」 all in one colour is a
/// wall to read; the same keys lit and their meanings quiet is a thing to
/// glance at, which is the only way the command row earns its row.
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
/// The three names are the shared ones. The factory level is `Full`
/// (2026-09-08): a mark that has to be looked for is not a mark, and the
/// characters it covers come back with one move of the caret. `Basic` is
/// §5.7's answer — identical to `Off` in the worst case — and stays one word
/// away for anyone who would rather not have prose covered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hud {
    /// Nothing beside the caret. The status line's right edge still says it —
    /// that is #193's floor, and no level takes it away.
    Off,
    /// A 藥丸 in the nearest margin: gold on the band, one row high, and
    /// **not one character of the manuscript hidden**. Where it lands is
    /// scored against the caret (#269), so it moves as the page fills.
    Basic,
    /// A bordered panel, pinned under the caret, over whatever is there.
    ///
    /// The frame and the covering are one decision, not two: a panel wants a
    /// rectangle that a page of prose does not have, so a frame forces
    /// covering — and once the mark sits on the same paper as the writing,
    /// the frame is the only thing saying which characters are not yours.
    #[default]
    Full,
}

/// What `:view-hud` says about a level, whether it was just set or only asked.
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

impl MdCache {
    /// Whether this entry answers `asked` — the same line, or **any other line
    /// of the region it already found** (#320).
    ///
    /// A region is a run of rows, and every line in that run walks out to the
    /// same two ends. Keyed by the line alone, `j` down a 10,000-row table
    /// missed on every step and re-walked the whole table to find the ends it
    /// had just found: 25.8 ms a keystroke, which at 30 a second is 78% of a
    /// core to hold a key down. A `None` is never reused — it says nothing
    /// about any line but the one asked.
    fn answers(&self, asked: &(u64, u64, usize, Bounds)) -> bool {
        if self.asked == *asked {
            return true;
        }
        let same_document =
            (self.asked.0, self.asked.1, self.asked.3) == (asked.0, asked.1, asked.3);
        same_document && self.region.as_ref().is_some_and(|r| r.holds(asked.2))
    }
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
    /// **Which punctuation this table is told apart by** (#378). `:table csv`
    /// and its kin change the separator without touching a byte of the file,
    /// and a table squared up on commas is not squared up on tabs.
    wall: crate::mdtable::Wall,
}

impl PadKey {
    /// Whether two keys ask the same table the same way — everything but
    /// **what the caret is doing and which edit this is**.
    ///
    /// Those two are what a keystroke changes, and they change one row of the
    /// table between them: the row that was typed in. A key that matches here
    /// says the rest of the rows are being asked exactly what they were asked
    /// last time, so their answers still stand — see [`PadWork`].
    fn same_shape(&self, other: &PadKey) -> bool {
        self.buffer == other.buffer
            && self.first == other.first
            && self.last == other.last
            && self.render == other.render
            && self.ruby == other.ruby
            && self.syntax == other.syntax
            && self.folds == other.folds
    }
}

/// A table's padding, **and what it was worked out from**.
///
/// The padding itself is one walk down every row, and it is thrown away on
/// every edit because a revision is a revision. But an edit is one row: the
/// other 4,999 rows of a long table are the same text being asked the same
/// question, and re-deriving their answers cost 40 ms of every keystroke
/// (#316). So the inputs are kept beside the output, and a rebuild copies
/// forward every row whose text — and whose place under the caret — has not
/// moved.
struct PadWork {
    /// Each row's text, and the spans the **measure** takes off it.
    rows: Vec<(String, Vec<(usize, usize)>)>,
    /// Where each row's fold marks are drawn.
    marks: Vec<Vec<(usize, usize)>>,
    /// The columns the selection covered on each row when this was worked
    /// out, under 所見即所得 — where the markup came back onto the page.
    /// `None` everywhere else, since nothing then asks.
    cols: Vec<Option<(usize, usize)>>,
    /// What the page draws: the padding on each row.
    runs: Vec<Vec<(usize, String)>>,
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
    /// and the table's own keys are available where a table is: `t d` drops a
    /// row, `Tab` steps to the next cell. **Nothing is hidden, folded or
    /// replaced** — every character the writer typed is still on the page, so
    /// a 縱書 chapter with no table in it is drawn exactly as `Off` draws it.
    ///
    /// **`hjkl` are still letters**, at this level and at every other. This
    /// said they walked cells until 2026-09-11, which was true before #356 and
    /// has been wrong since: the grain defaults to [`Grain::Char`] everywhere
    /// it is set, and `T` is what asks for cells. A stale comment is worse
    /// than none — this one was read as law and an argument built on top of it
    /// (「自动进 tb 会悄悄改掉 hjkl 的含义」), which the author had to knock
    /// down with the obvious answer: 「tb tf to 模式都是按字走的」.
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
    /// **Recognised, not converted.** `:table-pipe` rewrites a block so that
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
    /// `hjkl` walk cells and rows — what `T` asks for.
    Cell,
    /// `hjkl` walk characters and lines, as they do in any other file.
    ///
    /// **The default**, at every level and in the pane, wherever a view is
    /// built. It used to be [`Cell`](Grain::Cell) — 「it is what a grid is
    /// for」 — and #356 turned it round on the grounds that 「the cell is not
    /// what a writer mostly wants — the characters in it are」.
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
///
/// A book that numbers its chapters and writes no 章 at all is read by
/// [`bare_numbered_heading`] instead.
fn chapter_heading(line: &str) -> Option<Heading> {
    const NAMED: &[&str] = &[
        "序", "序章", "序言", "自序", "前言", "引子", "楔子", "小引", "凡例",
        "尾聲", "尾声", "終章", "终章", "後記", "后记", "跋", "附錄", "附录",
        "番外", "外傳", "外传", "目錄", "目录",
    ];
    let text = line.trim();
    let chars: Vec<char> = text.chars().collect();
    // A heading is a line by itself, and a short one. The longest real chapter
    // title in the corpora this was written against is well under this.
    if chars.is_empty() || chars.len() > 40 {
        return None;
    }
    // 序、楔子、後記: no number, so the whole line has to be the name.
    if NAMED.contains(&text) {
        return Some(Heading { depth: 2, key: text.to_string(), numbered: false });
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
    depth(unit).map(|d| Heading {
        depth: d,
        key: format!("{}{}", same(unit), number),
        numbered: true,
    })
}

/// What a line of a manuscript says it is, when the manuscript has no markup.
struct Heading {
    /// 1 for a 卷, 2 for a 章 — how deep the row sits in the outline.
    depth: usize,
    /// **The number, not the line**, so that a chapter written twice running is
    /// one chapter. For 序 and 後記, which have no number, the name itself.
    key: String,
    /// 第三章 and 卷002 are numbered; 序 and 後記 are the front and back matter,
    /// and a book made of nothing but those has no chapters found yet.
    numbered: bool,
}

/// The characters a chapter number is written with, in any of the spellings.
const DIGITS: &str = "一二三四五六七八九十百千萬万零〇兩两0123456789０１２３４５６７８９";

/// The line with the navigation bar a web export printed around it.
///
/// 三國演義 writes every one of its 120 回 as 「◀上一回 第二回　張翼德怒鞭督郵
/// 　何國舅謀誅宦豎 下一回▶」 — the heading is in there, wearing the arrows the
/// web page walked on. 資治通鑑 wears a different pair and wears them on one
/// side: 「第一卷　周紀一 ► 卷二」.
///
/// **An arrow ends the title, and so does the word carrying it.** The frame is
/// half-width-space-separated tokens (a Chinese title's own spaces are 全角),
/// so 「下一回▶」 comes off whole and 「張翼德怒鞭督郵」 stays. A line with no
/// frame comes back as it went in.
fn without_navigation(line: &str) -> &str {
    const ARROWS: [char; 4] = ['◀', '▶', '◄', '►'];
    const ENDS: [&str; 4] = ["全書始", "全书始", "全書終", "全书终"];
    let navigation = |token: &str| token.contains(ARROWS) || ENDS.contains(&token);
    // The head — 「◀上一回 」, 「全書始 」 — comes off a word at a time.
    let mut text = line.trim();
    while let Some((first, rest)) = text.split_once(' ') {
        if !navigation(first) {
            break;
        }
        text = rest.trim_start();
    }
    // …and the tail, with the next chapter's name behind it, is cut off.
    let mut end = text.len();
    let mut at = 0;
    for token in text.split(' ') {
        if navigation(token) {
            end = at;
            break;
        }
        at += token.len() + 1;
    }
    text[..end].trim_end()
}

/// 「一 青衫磊落險峰行」 — a chapter that is a number and a title and nothing
/// else, which is how 天龍八部 writes all fifty of its chapters.
///
/// Returns the number, because one line like this on its own means nothing: a
/// year, a footnote marker and a list item all read the same. What makes it a
/// chapter is that the book counts **1, 2, 3 from the top**, and only the
/// caller can see that. So this is deliberately loose about the line and exact
/// about the number.
fn bare_numbered_heading(line: &str) -> Option<u32> {
    let text = line.trim();
    let chars: Vec<char> = text.chars().collect();
    // Same bar as a 第三章 heading: a heading is a short line by itself.
    if chars.len() > 24 {
        return None;
    }
    let digits = chars.iter().take_while(|c| DIGITS.contains(**c)).count();
    if digits == 0 {
        return None;
    }
    // The number and the title are two things, so something separates them —
    // 「一九九四年一月」 is one thing and is not a chapter.
    if !matches!(chars.get(digits), Some(c) if c.is_whitespace()) {
        return None;
    }
    // And a title is a name, not a sentence.
    let title: String = chars[digits + 1..].iter().collect();
    let title = title.trim();
    if title.chars().count() < 2 {
        return None;
    }
    if title.chars().any(|c| "。，、；：！？「」『』（）〈〉《》.,!?".contains(c)) {
        return None;
    }
    chinese_number(&chars[..digits].iter().collect::<String>())
}

/// 「五十」 → 50, 「一百二十」 → 120, 「38」 → 38.
///
/// Chapter numbers and nothing else: up to a 千, no 萬, no 零 in the middle
/// (「一百〇八」 is written 一百零八 and both read the same here). Anything it
/// cannot read is not a number it is willing to count chapters by.
fn chinese_number(text: &str) -> Option<u32> {
    if let Ok(n) = text.parse::<u32>() {
        return Some(n);
    }
    const ONES: &str = "〇一二三四五六七八九";
    let mut total = 0u32;
    let mut part = 0u32;
    let mut said_a_digit = false;
    for c in text.chars() {
        if let Some(d) = ONES.chars().position(|o| o == c) {
            part = d as u32;
            said_a_digit = true;
            continue;
        }
        let unit = match c {
            '十' => 10,
            '百' => 100,
            '千' => 1000,
            '零' => continue,
            // 兩 and 萬 and the full-width digits: not in a chapter number
            // this is willing to guess at.
            _ => return None,
        };
        // 十一 is eleven — a unit with nothing in front of it is one of it.
        total += if said_a_digit { part } else { 1 } * unit;
        part = 0;
        said_a_digit = false;
    }
    Some(total + part)
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

/// Call `f` for every readable file under `root`, in path order, counting the
/// ones stepped over for being too big.
///
/// Skips what a manuscript directory holds but a writer never searches: hidden
/// directories (`.git`, `.yumete`), build output, files too big to be prose —
/// **and whatever the ignore files say** (#362). Symlinked directories are not
/// followed, so a loop cannot hang the editor.
///
/// **The skip list used to be written out here** and it was three names:
/// `.`-anything, `target`, `node_modules`. It did not read the `.gitignore`
/// lying in the same directory, so in a repository `:grep` searched the source
/// tree along with the book, and the fix for each new offender was another
/// name in the list. [`ignore`] is ripgrep's own walker and answers the
/// question properly; `require_git(false)` because a manuscript folder is as
/// likely to have a `.gitignore` and no `.git` as the other way round.
///
/// **`skipped` is counted because it used to be silent** (#308): a chapter over
/// [`GREP_MAX_BYTES`] was passed over and left out of 「searched N files」 as
/// well, so the number looked right and the answer was short.
fn walk(root: &Path, skipped: &mut usize, f: &mut impl FnMut(&Path)) {
    let walker = ignore::WalkBuilder::new(root)
        .follow_links(false)
        // In path order, so a listing of a novel's chapters comes back in
        // chapter order rather than in whatever order the file system holds
        // them. Also what makes the walk single-threaded and reproducible.
        .sort_by_file_path(|a, b| a.cmp(b))
        .require_git(false)
        // The floor under the ignore files, not a list to keep adding to: a
        // folder with no `.gitignore` at all still holds no prose in these
        // two, and the cost of looking is a whole build tree.
        .filter_entry(|entry| {
            !entry.file_type().is_some_and(|t| t.is_dir())
                || !matches!(&*entry.file_name().to_string_lossy(), "target" | "node_modules")
        })
        .build();
    for entry in walker.flatten() {
        if !entry.file_type().is_some_and(|t| t.is_file()) {
            continue;
        }
        let path = entry.path();
        // Not what this editor just wrote. `:export html` puts the book's own
        // words into a `.html` beside it, and `:grep` then found every one of
        // them twice — the second time in a file the writer cannot edit.
        if is_build_output(&entry.file_name().to_string_lossy()) {
            continue;
        }
        match entry.metadata().is_ok_and(|m| m.len() <= GREP_MAX_BYTES) {
            true => f(path),
            false => *skipped += 1,
        }
    }
}

/// How often a recovery copy is written while typing (Feature #79).
///
/// Five seconds is the most work a crash can cost, and short enough that the
/// writer never thinks about it; the write is atomic and off the rope's own
/// chunks, so it costs nothing at prose speed.
const SWAP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

/// How long one round of recovery copies may spend writing (#317).
///
/// 「costs nothing at prose speed」 above is true of *one* buffer. A hundred
/// chapters left dirty by a whole-book `:replace` is a hundred serialisations
/// and two hundred fsyncs, and the measurement was **1.593 s in the input
/// thread**. So a round writes the buffer being typed into, then as many of
/// the others as thirty milliseconds buys, and the rest wait their turn.
const SWAP_BUDGET: std::time::Duration = std::time::Duration::from_millis(30);

/// How soon the next round is due while a backlog is still draining (#317).
///
/// Not [`SWAP_INTERVAL`]: five seconds a buffer would leave a hundred chapters
/// uninsured for eight minutes. A sixth of a second keeps the drain to a few
/// seconds of wall clock while the input thread stays free between rounds.
const SWAP_BACKLOG_INTERVAL: std::time::Duration = std::time::Duration::from_millis(150);

/// How often the disk is asked whether the file moved, under `:reload-auto on`
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
    /// The `:s …c` walk `Pending::Confirm` is the keyboard half of.
    confirming: Option<Confirming>,
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
    /// Which line the page starts at — see [`Editor::set_page_top`].
    page_top: usize,
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
    /// Where readings come from for `:ruby-auto` (Feature #234). Defaults to
    /// [`NoReader`], which knows nothing; the front end installs a reader over
    /// 宇浩's 字料層 once the data is loaded, exactly as it does the segmenter.
    reader: Box<dyn Reader>,
    /// The project's own words, shared with the segmenter wrapped around the
    /// one in force — so reloading the list reaches a segmenter already handed
    /// out.
    project_words: std::rc::Rc<RefCell<yumete_cjk::WordList>>,
    /// What the segmenter in force has already cut, held by the same handle it
    /// is (#321) — so `forget_the_words` can throw it away when the dictionary
    /// or the book's own list changes and the text does not.
    word_memo: std::rc::Rc<RefCell<yumete_cjk::SegmentMemo>>,
    /// Whether the segmentation overlay (word background tint) is shown.
    show_segmentation: bool,
    word_mark: yumete_cjk::WordMark,
    /// How loudly the editor says what you have typed, beside the caret
    /// (Feature #284).
    hud: Hud,
    /// How readily characters join into words (`:word-level`), kept so a
    /// segmenter installed later arrives at the level the reader chose.
    word_level: yumete_cjk::WordLevel,
    /// `:word-list reload` asking the front end to build the dictionary again
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
    /// **The keys need it.** `t3/`, `t20,20g`, `t1a2d8as` all name a column by
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
    /// A pending `:view-preview`, waiting for the front end — starting a typesetter
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
    /// The one thing opening a file had to say, kept apart from the status.
    ///
    /// **Because the front end wipes the status on its way out of setup**, and
    /// cannot tell 「本檔案格式似乎是 Tab 分欄」 (#380) apart from the chatter
    /// that wipe exists to remove. Set only by a door that guessed, taken by
    /// whoever draws the first frame, and never twice.
    open_notice: Option<String>,
    /// How much of a table is drawn — the reader's standing answer (#283).
    ///
    /// Untouched by opening a file, by walking out of a table, by there being
    /// no table at all. `TableLevel::Basic` in a file with no table draws the
    /// same page `Off` does, to the character, because every gate below asks
    /// `table_here()` first — which is what makes it safe as the factory
    /// value, and what lets `:render` assign it without knowing anything about
    /// the file.
    table_level: TableLevel,
    /// Whether the detail panel is wanted — **`None` until somebody says**.
    ///
    /// It only appears where there is something to say, so this is "show it
    /// when there is", not "show it". And where it opens unasked depends on
    /// where the reader is standing: a level that folds cells away (全, 全窗)
    /// needs it, because it is then the way to read one whole; 基本 folds
    /// nothing, so the panel would be repeating what is already on the page
    /// while taking a fifth of the width to do it (author, 2026-09-11:
    /// 「tb 模式（basic）默认不用打开 information panel」).
    ///
    /// `t i` writes an answer here and that answer outlives the level — the
    /// same bargain `t w` makes about folding: what the reader asked for is
    /// not something a change of level may quietly undo.
    show_detail: Option<bool>,
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
    /// while the other three were live toggles — which is why `:view-dense` had to
    /// exist rather than being three keys anybody could find.
    zong_gap: Option<usize>,
    /// Whether the dense arrangement is on, so the ticks know to stay away.
    dense: bool,
    /// Whether every 句 opens a 縱 of its own (`:view-sentence`, Feature #237).
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
    /// One write may go through with the oversize gate already answered (#306).
    /// Set only for the duration of the write the reader said 「yes」 to.
    oversize_answered: bool,
    /// Word ranges already worked out, per line, against a hash of that line.
    segment_memo: memo::LineMemo<Vec<(usize, usize)>>,
    /// 平仄 in the margin (Feature #247), and the answers already worked out.
    ///
    /// A drawing setting, like [`Self::focus`]: it changes what is in the
    /// margin beside the writing and never what the writing is.
    meter: bool,
    meter_memo: memo::LineMemo<Vec<crate::meter::Mark>>,
    /// Inline notes on the marks a Chinese manuscript got wrong (#248), and
    /// the answers already worked out.
    ///
    /// The first producer of [virtual text](crate::drawn) that is neither the
    /// writer's own typing nor a table's geometry: the editor saying something
    /// *about* the text, in the text's own place, as it is written.
    notes: bool,
    note_memo: memo::LineMemo<Vec<crate::drawn::Run>>,
    /// Which lines are folded away, against the buffer they were worked out
    /// for. One pass over the file per edit — the answer is not line-local (a
    /// blank line inside a fence is code, not a paragraph break), and asking
    /// per line would walk the document once per line.
    fold_cache: RefCell<Option<FoldMap>>,
    /// The Markdown runs of each paragraph, cached the same way and for the
    /// same reason: the renderer asks for every paragraph on screen, every
    /// frame, and the answer only changes when the paragraph does.
    /// Keyed by the buffer's **id** and the line — the key every line memo
    /// uses (#348), because a bare `line` said that line 3 of every file was
    /// the same line: `*強調*` in a Markdown chapter came back as emphasis in a
    /// `:syntax text` manuscript that happened to hold the same words.
    markup_memo: memo::LineMemo<Vec<crate::markdown::Span>>,
    /// The readings laid out on each paragraph, cached the same way.
    ///
    /// **Everything that draws a page asks this, and so does the wrap** — a
    /// reading comes off the page where it is laid out, so it decides where a
    /// row breaks, and `crate::wrap` asks it two to five times a keystroke.
    /// Reading it costs the paragraph materialised into characters and walked
    /// once, which on a chapter written as one 1,000,000-character paragraph
    /// was 2.6 ms an ask and 13 ms of every `j` (#315).
    ruby_memo: memo::LineMemo<Vec<crate::ruby::Ruby>>,
    /// The block of every line, against the buffer it was worked out for and
    /// that buffer's revision.
    block_cache: RefCell<Option<BlockCache>>,
    /// The drawn padding of the table last asked about (Feature #212).
    ///
    /// One table at a time: the page asks per line, every line of a table
    /// needs the widths of all the others, and a document has at most a
    /// screenful of table on it at once.
    pad_cache: RefCell<Option<(PadKey, PadWork)>>,
    /// Which `|` table the cursor is in, against the buffer, its revision and
    /// the line the answer was worked out for.
    ///
    /// The region is asked for several times a frame — the command row, the
    /// status line, and every key that has to know whether the grid's rules
    /// apply here. Walking out from the cursor is cheap for a table of ten
    /// rows and is not cheap for a table of ten thousand, and the answer is
    /// the same all three times.
    md_cache: RefCell<Option<MdCache>>,
    /// Where the `|` table **the padding is drawing** begins and ends, against
    /// the buffer and its revision.
    ///
    /// [`Self::md_cache`] answers the same question for the table the *cursor*
    /// is in, and only while a `t` view is open; the drawn padding asks it of
    /// every line of every table on the page, view or no view. Its own memo
    /// (`pad_cache`) holds a window of rows, so this walk is asked once per
    /// window rather than once per key — but a window is fifty rows and the
    /// walk is the whole table, so on a ten-thousand-row one that was a 5 ms
    /// hitch every time the page slid off the remembered rows (#320).
    pipe_region: RefCell<Option<(u64, u64, crate::mdtable::Region)>>,
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
    /// The reference completion Tab is walking in the text itself — `[^` and
    /// `](#` (#418). Kept for the same reason the command line's is: each Tab
    /// replaces what the last one wrote, so the buffer can no longer say what
    /// the writer had typed.
    reference: Option<complete::Walking>,
    /// Which spellings of a reading **count as one** (Feature #65) — what the
    /// word count subtracts, what `:ruby` edits, what `:ruby-auto` writes.
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
    /// Where the round-robin over the *other* buffers resumes (#317).
    swap_cursor: usize,
    /// Whether the last round ran out of budget with copies still owed, so the
    /// next one is due in [`SWAP_BACKLOG_INTERVAL`] rather than
    /// [`SWAP_INTERVAL`] (#317).
    swap_backlog: bool,
    /// Whether every file opened from here on is locked (Feature #213).
    ///
    /// What `--readonly` sets. Kept on the editor rather than handed to each
    /// buffer at birth because `:open` opens buffers too, and a session started
    /// to *read* a directory of chapters should not go writable at the second
    /// file.
    readonly_default: bool,
    /// Whether a clean buffer re-reads itself when the file changes on disk
    /// (Feature #214). `:reload-auto on`.
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
    /// Whether a pattern with no capital in it ignores case (#301).
    ///
    /// On, because of what this editor is for: a manuscript is 漢字, which has
    /// no case at all, so almost every search takes the insensitive branch for
    /// free. The 西文 that is in a manuscript is mostly names and initialisms —
    /// `TODO`, `ISBN`, 人名 — and those are exactly the searches that *want*
    /// the case, which is what typing a capital asks for.
    smart_case: bool,
    /// The text width the renderer is wrapping at, in cells. `None` until the
    /// terminal size is known; motion falls back to logical lines then.
    wrap_width: Option<usize>,
    /// How far a TAB advances: to the next multiple of this (#374).
    ///
    /// A stop rather than a width, and the difference is the whole point on a
    /// 碼表: `dl` and `bkd` are two and three cells, and a tab of a fixed four
    /// puts their second column at six and seven — not lined up, which is all
    /// a tab-separated file is for. To the next multiple of eight they both
    /// land on eight — 「既然是 Unix 老默认就用他」 (author, 2026-09-10).
    tab_stop: usize,
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
    /// **Nothing was written**: the save would multiply the file, so the
    /// question of #295 is now standing and the answer decides (#306). Callers
    /// that go on to do something after a save — quit, or move to the next
    /// buffer — have to stop here, which is why this is a variant rather than
    /// a silent `Ok`.
    Asked,
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
            confirming: None,
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
            page_top: 0,
            page_columns: 10,
            last_search: String::new(),
            search_forward: true,
            key_aliases: HashMap::new(),
            expanding_alias: false,
            segmenter: Box::new(CategorySegmenter),
            reader: Box::new(NoReader),
            project_words: std::rc::Rc::new(RefCell::new(yumete_cjk::WordList::default())),
            word_memo: std::rc::Rc::new(RefCell::new(yumete_cjk::SegmentMemo::default())),
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
            open_notice: None,
            table_level: TableLevel::default(),
            show_detail: None,
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
            reference: None,
            tatechuyoko: false,
            hanging: false,
            paper: crate::export::Paper::A5,
            viewing: Cell::new(None),
            query: None,
            oversize_answered: false,
            segment_memo: memo::LineMemo::default(),
            meter: false,
            meter_memo: memo::LineMemo::default(),
            notes: false,
            note_memo: memo::LineMemo::default(),
            fold_cache: RefCell::new(None),
            markup_memo: memo::LineMemo::default(),
            ruby_memo: memo::LineMemo::default(),
            block_cache: RefCell::new(None),
            pad_cache: RefCell::new(None),
            md_cache: RefCell::new(None),
            pipe_region: RefCell::new(None),
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
            tab_stop: 8,
            measure: None,
            autosave: true,
            last_swap: None,
            swap_warned: false,
            swap_cursor: 0,
            swap_backlog: false,
            readonly_default: false,
            reload_auto: false,
            last_disk_check: None,
            reload_warned: false,
            compiled: RefCell::new(None),
            smart_case: true,
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

    // Grids — table mode, `|` tables and delimited text — are in
    // `editor/tables.rs` (#296).

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
        // The check every other whole-document rewrite makes (#350): lifting
        // the cell guard is exactly the moment a row can silently gain or lose
        // a cell, and a formatter that reflows a 拆分表 is the likeliest way.
        if let Some(why) = self.substitution_breaks_the_grid(text) {
            self.status = why;
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

    /// The running typesetter's address, for the status bar and for `:view-preview`.
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

    /// Whether `:view-meter` is on — the setting, which the status line reports.
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
    /// `:view-meter on` with no 拆分表 installed reflowed the whole page to make
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

/// How many files may be remembered as 「open this one as source」 (#380).
///
/// A cap rather than a growing list: the note is a convenience about files a
/// reader still opens, and the oldest ones fall off the front.
const SOURCE_MODE_FILES: usize = 200;

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
    /// The `f` flag: 照字面 — the pattern is characters, not a regex.
    literal: bool,
    /// The `c` flag: stop at each match and ask.
    confirm: bool,
    count_only: bool,
    /// The `t` flag: the writer means to change how many cells a row has.
    reshape: bool,
    rows: crate::command::Rows,
}

/// The lines a `:s` will touch, already resolved to 0-based line numbers.
enum Chosen {
    /// Everything from the first to the last, inclusive.
    Span(usize, usize),
    /// Exactly these, in whatever order they were written.
    These(Vec<usize>),
}

impl Chosen {
    /// Is this line one of them?
    fn has(&self, line: usize) -> bool {
        match self {
            Self::Span(first, last) => line >= *first && line <= *last,
            Self::These(lines) => lines.contains(&line),
        }
    }
}

/// One match a `:s …c` is about to ask about.
struct Hit {
    /// Char indices into the buffer.
    start: usize,
    end: usize,
    /// What is there now — the question shows it, so the writer is looking at
    /// the same thing the editor is.
    found: String,
    /// The replacement with its `$1` already resolved **for this match**.
    text: String,
}

/// A `:s …c` in the middle of asking (#415).
///
/// 「防止一下子全部都替换了」 — the whole point of the flag is that the writer
/// sees each match before it changes, so the walk outlives the command that
/// started it and lives here between keystrokes.
struct Confirming {
    re: Regex,
    /// Still with its `$1` in it; expanded per match.
    replacement: String,
    /// `g`: every match on a line, not only the first.
    global: bool,
    /// The lines the range named.
    chosen: Chosen,
    /// `t`: the writer means to let a row's cell count change.
    reshape: bool,
    /// Matches starting before this char index have been decided.
    ///
    /// The match **on screen** is simply the next one at or after this, found
    /// again when the key arrives: nothing can have touched the buffer in
    /// between, because this pending eats every key.
    from: usize,
    changed: usize,
    skipped: usize,
    /// The writer said `a`: go through the rest without asking.
    all: bool,
    /// Whether the undo point has been taken. **One `u` undoes the whole
    /// walk**, however many matches it changed — a writer who says 「算了」
    /// after twenty `y`s means all twenty.
    snapped: bool,
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
fn search_forward(
    rope: &Rope,
    pattern: &Regex,
    from: usize,
    within: std::ops::Range<usize>,
) -> Option<(usize, usize)> {
    // A pane can be handed no lines at all, and「the line the cursor is on」
    // is then not a line of it.
    if within.start >= within.end {
        return None;
    }
    let start_line = rope
        .char_to_line(from.min(rope.len_chars()))
        .clamp(within.start, within.end.saturating_sub(1));
    // From the cursor to the end, then from the top back to the cursor's line,
    // so the wrap covers the part of that line before the cursor too — where
    // 「the end」 and 「the top」 are the ends of `within`, which is the whole
    // buffer unless a table holds the pane (#354).
    scan(rope, pattern, start_line, within.end, from).or_else(|| {
        scan(rope, pattern, within.start, start_line + 1, rope.line_to_char(within.start))
    })
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
/// **Backwards, line by line, stopping at the first line that has one** — the
/// mirror of [`search_forward`], and for the same reason. It used to be one
/// forward pass over the whole range keeping the best answer so far, which is
/// correct and costs the whole file on every press: on a 1.8 MB manuscript `n`
/// was 17 µs and `N` **1.01 ms**, sixty times more for the same distance
/// travelled, and holding `N` down on a ten-megabyte draft stuttered (#319).
///
/// Ropey's line iterator walks either way in constant time with respect to the
/// rope's length, so a step up costs a step, and the char offset of each line
/// is carried along rather than asked for.
fn search_backward(
    rope: &Rope,
    pattern: &Regex,
    from: usize,
    within: std::ops::Range<usize>,
) -> Option<(usize, usize)> {
    // A pane can be handed no lines at all, and「the line the cursor is on」
    // is then not a line of it.
    if within.start >= within.end {
        return None;
    }
    let start_line = rope
        .char_to_line(from.min(rope.len_chars()))
        .clamp(within.start, within.end.saturating_sub(1));
    // From the cursor up to the top, then from the bottom back down to the
    // cursor's line, so the wrap covers the part of that line after the cursor
    // too — the same two halves [`search_forward`] takes, walked the other way.
    scan_back(rope, pattern, within.start, start_line + 1, from)
        .or_else(|| scan_back(rope, pattern, start_line, within.end, usize::MAX))
}

/// The last match starting before `from` within `lines`, searching backward, as
/// a half-open range of char indices.
fn scan_back(
    rope: &Rope,
    pattern: &Regex,
    from_line: usize,
    to_line: usize,
    from: usize,
) -> Option<(usize, usize)> {
    let to_line = to_line.min(rope.len_lines());
    // ⚠️ **An empty document is one empty line, and ropey will not walk back
    // over it**: forwards its iterator yields that line, backwards it yields
    // nothing. `yumete` opens on exactly that document, and `x*` matches in
    // it.
    if rope.len_chars() == 0 {
        return (from_line == 0 && to_line > 0 && from > 0 && pattern.is_match(""))
            .then_some((0, 0));
    }
    let mut at = rope.line_to_char(to_line);
    let mut lines = rope.lines_at(to_line);
    for _ in from_line..to_line {
        let slice = lines.prev()?;
        at -= slice.len_chars();
        let owned;
        let text: &str = match slice.as_str() {
            Some(text) => text,
            None => {
                owned = slice.to_string();
                &owned
            }
        };
        // A line is read forwards whichever way the file is being read: the
        // matches on it have to be found in order to know which is the last.
        let mut byte = 0usize;
        let mut best = None;
        while let Some(m) = text.get(byte..).and_then(|rest| pattern.find(rest)) {
            let start = at + text[..byte + m.start()].chars().count();
            if start >= from {
                break;
            }
            best = Some((start, start + m.as_str().chars().count()));
            // An empty match would otherwise stand still forever.
            byte += m.end().max(m.start() + 1);
        }
        if best.is_some() {
            return best;
        }
    }
    None
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
fn switch_case(c: char) -> String {
    // **Whole expansions, not their first character** (#325): `ß` turns into
    // `SS` and `ﬁ` into `FI`, and a signature that could only hand back one
    // character threw the rest away without saying anything.
    if c.is_lowercase() {
        c.to_uppercase().collect()
    } else if c.is_uppercase() {
        c.to_lowercase().collect()
    } else {
        c.to_string()
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
mod commands;
pub mod complete;
mod conflicts;
mod convert;
mod detail;
mod edits;
mod files;
mod help;
mod hint;
mod jumps;
mod keys;
mod matching;
mod memo;
mod modes;
mod page;
mod prompt;
mod render;
mod ruby;
mod search;
mod session;
mod shell;
mod sidebar;
mod tables;
mod undo;
mod verbs;
mod words;
mod wrap;

#[cfg(test)]
mod tests;
