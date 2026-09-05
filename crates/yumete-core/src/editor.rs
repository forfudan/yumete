//! The [`Editor`]: top-level state owning the open buffers, the active one, and
//! the modal editing state (mode, cursor, command line).
//!
//! [`Editor::execute`] runs a parsed `:` command, and [`Editor::on_key`] drives
//! the modal state machine (Normal / Insert / Command) from backend-agnostic
//! [`Key`] presses, so the whole interaction can be unit-tested without a
//! terminal.

use crate::say;
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
use crate::lookfor;
use crate::motion;
use crate::ruby::{Dialect, Dialects};
use crate::text_store::TextStore;
use crate::zong::{self, Grid, Layout, DEFAULT_ZONG_LENGTH};

/// A paragraph's word ranges, kept against a hash of the paragraph's text.
type SegmentCache = HashMap<usize, (u64, Vec<(usize, usize)>)>;

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

/// Every line's block, against the buffer it was worked out for and that
/// buffer's revision — the two things that decide whether it is still true.
type BlockCache = ((u64, u64), Vec<crate::markdown::Block>);

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
}

/// The four flavours of in-line character search (`f`/`t`/`F`/`T`).
#[derive(Clone, Copy, PartialEq, Eq)]
enum FindKind {
    ForwardTo,
    BackwardTo,
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
    /// Hand the screen to the platform's own screenshot program.
    Screen,
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

/// What one table's ghost padding was worked out from (Feature #212).
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
    cursor: usize,
    selection: (usize, usize),
    render: Render,
    ruby: Dialects,
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
    /// What the table is drawn on.
    pub surface: Surface,
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

/// What the table is drawn on (#261).
///
/// The author's model, 2026-09-05: 「csv 文件等同于一个从第一行到最后一行都是表格
/// 的普通文本文件」 — a CSV is not a different kind of thing from a table in a
/// document, so which of the two renderers draws it is a fact about the table,
/// not about its splitter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    /// A page of its own, drawn by `yumete-tui`'s grid widget: the columns line
    /// up because the renderer puts them there, the header freezes at the top,
    /// and the file on disk is untouched. Only this surface turns a 縱書 page
    /// horizontal — a grid is read across, and that is the one thing 縱書
    /// cannot do.
    Page,
    /// Drawn as part of the document it sits in, through #212's ghost padding:
    /// the columns line up because the text itself is padded. The paragraph
    /// above the table does not vanish the moment the cursor lands in a cell,
    /// and a 縱書 chapter stays 縱書.
    InProse,
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
    /// Whether this table is drawn on a page of its own.
    pub fn is_page(&self) -> bool {
        self.surface == Surface::Page
    }

    /// Whether it is drawn as part of the document it sits in.
    pub fn in_prose(&self) -> bool {
        self.surface == Surface::InProse
    }

    /// Whether this table takes the whole pane — the grid widget's own case.
    ///
    /// **Not `is_page()`** (#275). 真表格顯示 is a surface a run of lines in the
    /// middle of a chapter can wear now, and the widget that clears the frame
    /// is for the table that reaches both ends of the file. Every place that
    /// used to ask `is_page()` meaning「the whole pane is a grid」asks this.
    pub fn takes_the_pane(&self) -> bool {
        self.surface == Surface::Page && self.bounds == Bounds::WholeFile
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
    /// The project's own words, shared with the segmenter wrapped around the
    /// one in force — so reloading the list reaches a segmenter already handed
    /// out.
    project_words: std::rc::Rc<RefCell<yumete_cjk::WordList>>,
    /// Whether the segmentation overlay (word background tint) is shown.
    show_segmentation: bool,
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
    /// **The keys need it.** `3gd`, `t20-20g`, `t1a2d8as` all name a column by
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
    /// 2 and 5. `(first, Some(last))` once a `-` has been typed.
    sequence: Option<(usize, Option<usize>)>,
    /// The columns a sort has been told about so far, 1-based, `true` for
    /// descending — `t1a2d8a` is three of them, waiting for its `s`.
    ///
    /// **A prefix-free grammar** (the author, 2026-09-05): the old spelling was
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
    /// A pending `:preview`, waiting for the front end — starting a typesetter
    /// is running a program, which only the front end can do.
    preview_request: Option<Preview>,
    /// Where the typesetter that **is running** put its page.
    ///
    /// A preview server is a thing with a life of its own: it holds a port and
    /// a few hundred megabytes for as long as it runs. The editor knows it is
    /// there so it can say so on the status bar, hand the address back when
    /// asked, and refuse to start a second one.
    preview_at: Option<String>,
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
    /// while the other three were live toggles — which is why `:dense` had to
    /// exist rather than being three keys anybody could find.
    zong_gap: Option<usize>,
    /// Whether the dense arrangement is on, so the ticks know to stay away.
    dense: bool,
    /// How many squares open a paragraph (首行縮進), as configured.
    indent: usize,
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
    /// Word ranges already worked out, per line, against a hash of that line.
    segment_cache: RefCell<SegmentCache>,
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
    /// The ghost padding of the table last asked about (Feature #212).
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
    /// Which ruby dialects were being laid out before 所見即所得 turned them
    /// all on, so leaving it gives back what the writer had rather than
    /// nothing.
    ruby_before: Option<Dialects>,
    /// **Text on the page the file has no bytes for** (Feature #210), as
    /// `(line, column, what is drawn there)`.
    ///
    /// Held here rather than worked out here, because what goes on the page
    /// comes from outside the core: the candidate the input method is offering
    /// (#211), the padding that squares a table up without rewriting it (#212).
    /// The core's job is that everything which asks where a character is —
    /// the wrap, the caret, `j`, the mouse — asks about the same page.
    ghost: Vec<(usize, usize, String)>,
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
            project_words: std::rc::Rc::new(RefCell::new(yumete_cjk::WordList::default())),
            show_segmentation: false,
            word_level: yumete_cjk::WordLevel::default(),
            words_request: false,
            table_rules: crate::table::Rules::default(),
            table_numbers: true,
            detail_width: None,
            typewriter: false,
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
            render: Render::On,
            count_to: None,
            column_span: None,
            sequence: None,
            sort_keys: Vec::new(),
            jumped: false,
            language_run: None,
            preview_request: None,
            preview_at: None,
            shell_request: None,
            table: None,
            show_detail: true,
            table_bypass: std::cell::Cell::new(false),
            drafts_dir: None,
            data_dir: None,
            session_file: None,
            turned_for_table: None,
            zong_gap: None,
            dense: false,
            loose_rows: false,
            indent: 0,
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
            segment_cache: RefCell::new(SegmentCache::new()),
            fold_cache: RefCell::new(None),
            markup_cache: RefCell::new(HashMap::new()),
            block_cache: RefCell::new(None),
            pad_cache: RefCell::new(None),
            md_cache: RefCell::new(None),
            md_tables: RefCell::new(None),
            ruby_before: None,
            ghost: Vec::new(),
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
    /// the rows and dropped the reader back into the source (the author,
    /// 2026-09-05: 「表格排序 t1s 會直接回到源碼視圖」).
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
    /// and `:wa` is the moment a person says yes. Writing 120 files from a
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

    /// `:shot` — a picture of the page, drawn rather than taken (#189).
    ///
    /// The whole of the decision is made here, a frame early: which file, and
    /// whether it keeps its colours. What is left is the cells, which only the
    /// front end holds — so the answer is parked in `screenshot_request` and
    /// [`Editor::take_screenshot_request`] hands it over **after** the next
    /// frame is drawn, which is the one with no command line across it.
    ///
    /// The name follows the document with `.shot` before the extension, so a
    /// picture never collides with what `:export` would write and a directory
    /// of chapters keeps its pictures beside them. `.txt` asks for the
    /// plain-text picture; anything else is the coloured one.
    fn take_a_picture(
        &mut self,
        shot: crate::command::Shot,
        force: bool,
    ) -> Result<CommandOutcome, EditorError> {
        let path = match shot {
            crate::command::Shot::Screen => {
                self.screenshot_request = Some(ShotJob::Screen);
                return Ok(CommandOutcome::Continue);
            }
            crate::command::Shot::Page(path) => path,
        };
        let target = match path {
            Some(path) => PathBuf::from(path),
            None => match self.current_buffer().path() {
                Some(source) => {
                    let stem = source
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    source.with_file_name(format!("{stem}.shot.html"))
                }
                None => return Err(EditorError::NoFileName),
            },
        };
        if self.refuse_to_overwrite(&target, force) {
            return Ok(CommandOutcome::Continue);
        }
        let text = target
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("txt"));
        self.screenshot_request = Some(ShotJob::Page { target, text });
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
            Some((cached, blocks)) if *cached == key => {
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
        if let Some((cached, blocks)) = self.block_cache.borrow().as_ref() {
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
        if let Some((cached, _)) = self.block_cache.borrow().as_ref() {
            if *cached == key {
                return;
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
        *self.block_cache.borrow_mut() = Some((key, blocks));
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
            // Inside a fence nothing is markup, so nothing comes off.
            let spans = self.markup_line_in(line, self.block_of(line));
            off.extend(crate::markdown::hidden(&spans, self.selected_columns(line)));
            off.sort_unstable();
        }
        off
    }

    /// The ghost text on `line`: `(column within the line, what is drawn)`.
    ///
    /// Ordered by column, so the renderer, the wrap and the mouse walk it the
    /// same way.
    pub fn ghost_on_line(&self, line: usize) -> Vec<(usize, String)> {
        let mut runs: Vec<(usize, String)> = self.typed_ghost_on_line(line);
        runs.extend(self.table_padding_on_line(line));
        // Stable, so that at a shared anchor the candidate keeps its place
        // ahead of the padding.
        runs.sort_by_key(|&(at, _)| at);
        // **One run per anchor.** Two runs standing before the same character
        // are two answers to "what is drawn here", and the caret, the click
        // map and the wrap would each pick their own. The candidate is drawn
        // first because it continues the word: the padding's job is to reach
        // the pipe, so it belongs on the far side of what was typed.
        runs.dedup_by(|(at, text), (kept, held)| {
            (*at == *kept).then(|| held.push_str(text)).is_some()
        });
        runs
    }

    /// The part of the ghost on `line` that the writer **typed**: the inline
    /// candidate, and nothing derived.
    ///
    /// The caret's own page. A run is drawn before the character it is
    /// anchored at, and a caret resting on that character stands after the
    /// candidate — you typed it — but *before* the padding that reaches from
    /// the same anchor to the pipe. Told apart here so [`crate::wrap`] can put
    /// the caret between them; everything else wants them as one page and
    /// asks [`Self::ghost_on_line`].
    pub fn typed_ghost_on_line(&self, line: usize) -> Vec<(usize, String)> {
        self.ghost
            .iter()
            .filter(|&&(l, _, _)| l == line)
            .map(|(_, at, text)| (*at, text.clone()))
            .collect()
    }

    /// Whether `|` tables are squared up as the page draws them (Feature #212).
    ///
    /// **Horizontal only.** Down a 縱 a row is one column and every character
    /// takes one cell of it, wide or narrow — so padding measured in display
    /// width, which is what squares a table up across a page, aligns nothing
    /// there.
    ///
    /// **Only while the markup means something.** `:render off` asks for the
    /// file exactly as it is; drawing what it does not contain is the one
    /// thing that setting is against.
    ///
    /// Whether or not `:table` was typed, though: a table in a manuscript is a
    /// table because of what it is, and the writer who most needs to see one
    /// squared up is the one editing their own documentation.
    fn table_padding_on(&self) -> bool {
        self.layout == Layout::Horizontal && self.markup_visible()
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
            cursor: self.cursor,
            selection: self.selection(),
            render: self.render,
            ruby: self.ruby,
            syntax: buffer.syntax(),
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
            .map(|i| (self.line_text(i).unwrap_or_default(), self.hidden_on_line(i)))
            .collect();
        let runs = crate::mdtable::padding(&rows, region.rule.map(|at| at - region.first));
        let answer = runs.get(line - region.first).cloned().unwrap_or_default();
        *self.pad_cache.borrow_mut() = Some((key, runs));
        answer
    }

    /// Whether an inline candidate is standing on the page.
    ///
    /// Only the candidate. It used to be "anything the file does not contain",
    /// which stopped being a useful question the day the padding that squares
    /// a table up became ghost too (#212): that padding is **derived**, it is
    /// on nearly every page of documentation, and no caller ever meant it.
    pub fn has_candidate(&self) -> bool {
        !self.ghost.is_empty()
    }

    /// Put `runs` on the page in place of whatever was there.
    ///
    /// Wholesale, never appended: the caller says what the page holds now, so
    /// a candidate that has been committed leaves nothing behind.
    pub fn set_ghost(&mut self, runs: Vec<(usize, usize, String)>) {
        self.ghost = runs;
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
        // read by nobody: `:hanging on` on a horizontal page turned a flag on,
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
                if self.table.as_ref().is_some_and(|v| v.is_page()) && wants_vertical {
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
                match dialect {
                    Some(d) => self.render_ruby(d, on),
                    // Bare `:ruby-on` means the dialect this file is written in;
                    // bare `:ruby-off` means all of them.
                    None if on => self.ruby = Dialects::only(self.file_dialect()),
                    None => self.ruby = Dialects::NONE,
                }
                let listed_names: Vec<String> =
                    self.ruby.iter().map(|d| d.name().to_string()).collect();
                self.status = if listed_names.is_empty() {
                    say!("ruby.layout-off")
                } else {
                    say!("ruby.layout-on", listed(&listed_names))
                };
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
                // `:q!`. So it rebinds, exactly as `:saveas` does, and the file
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
            Command::CheckUsage => {
                self.check_usage();
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
            Command::ToggleHanging => {
                let on = !self.hanging;
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
            Command::YumeLanguage(on) => {
                self.scheme_request = Some(String::from(match on {
                    true => "+",
                    false => "-",
                }));
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
                    Render::On => say!("render.full"),
                    Render::Full => say!("render.wysiwyg"),
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
            Command::SetTable(on) => {
                if on {
                    self.enter_table();
                } else {
                    self.leave_table();
                }
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
                // `:wrap 40` does set the 縱 length, in either layout, but
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
            Command::ToggleChaifen => {
                self.chaifen = !self.chaifen;
                self.chaifen_request = Some(self.chaifen);
                Ok(CommandOutcome::Continue)
            }

        }
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
            // name is `:saveas`, which says so — `:w chapter-copy.md` used to
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
        match &saved {
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

    /// The 0-based **character** column the cursor is at within its line.
    ///
    /// Not [`Self::cursor_visual_column`], which is cells: this is the column
    /// a ghost run is anchored at, and those are counted in characters the way
    /// `hidden` is (Feature #211).
    pub fn cursor_column(&self) -> usize {
        let rope = self.current_buffer().rope();
        let at = self.cursor.min(rope.len_chars());
        at - rope.line_to_char(rope.char_to_line(at))
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
    ///
    /// **真表格顯示** — the surface `t t` asks for. `t i` asks the same door
    /// for the other one.
    pub fn enter_table(&mut self) -> bool {
        self.enter_table_as(Surface::Page)
    }

    /// The same door, told which of the two table modes was asked for (#275).
    ///
    /// The surface is now the **mode**, and the mode is chosen by the key, not
    /// by the kind of table: `t t` draws a grid whether the table is a whole
    /// `.csv` or three lines of a chapter, and `t i` leaves the pipes and the
    /// commas on the page whether or not the file is nothing but table.
    pub fn enter_table_as(&mut self, surface: Surface) -> bool {
        // A `|` table under the cursor is a table, whatever the file is called
        // and whether or not it has been saved — it says what it is on every
        // one of its own lines.
        // …unless a schema already claims the file. A schema is a person
        // saying what this data is, and a row of it that happens to open with
        // a pipe does not get to overrule them.
        if self.md_row_at_cursor()
            && self.table.as_ref().map(|v| v.bounds) != Some(Bounds::WholeFile)
        {
            // A line that opens with `|` inside a fenced block is *an example
            // of* a table — the manual has several — and reformatting one
            // rewrites somebody's quoted text.
            if self.md_row_in_a_fence() {
                self.status = say!("table.table-inside-a-code-block");
                return false;
            }
            return self.enter_md_table_as(surface);
        }
        // **A Markdown file's tables are the file's** (#275), so the mode is
        // reachable from the paragraph between two of them: 「可以在文件任何位
        // 置通過 ti tt 進入表格視圖…對於這個文件中所有的表格都生效」. The
        // cursor is left where it is — the mode is not a jump, and `t ]` is
        // the key for going to a table.
        if self.syntax() == crate::syntax::Syntax::Markdown && self.table.is_none() {
            if let Some(line) = self.first_md_table_line() {
                if self.enter_md_table_at(line, surface) {
                    return true;
                }
            }
        }
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            // A buffer with no name has no schema to find and no name for one
            // to claim — but the lines under the cursor may still be a table
            // (#216), and a 碼表 pasted into a scratch buffer is exactly where
            // somebody wants to look at one.
            if self.enter_block_table(surface) {
                return true;
            }
            self.status = say!("table.no-file-name-no-schema");
            return false;
        };
        let (found, problems) = crate::table::schema_for_reporting(&path);
        // A schema with a typo in it costs every label, both computed fields
        // and the whole link. Saying so is the difference between "this file
        // has no schema" and "your schema has a typo on line 4".
        if !problems.is_empty() {
            self.status = say!("table.schema-problems", listed(&problems));
            return false;
        }
        let (from, schema, how) = match found {
            Some((from, schema)) => {
                let name = from
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                (from, schema, say!("table.by-schema", name))
            }
            // No schema names this file, so its own header row is the schema.
            // Column names and nothing else — but that is enough to line the
            // file up and walk it by cell, which is most of what a grid is for.
            None => {
                // **What separates the columns is guessed, not assumed.** It
                // used to be a hard-coded comma, so `:table` on a `.tsv` split
                // its header into a single column and was told 「不是表格」 —
                // a file this editor's own `:export tsv` had just written.
                // The guess is the sniffer's, over the same first lines
                // `looks_delimited` reads, and a comma when it says nothing.
                let lines = self.first_lines(20);
                let delimiter = crate::table::sniff(&lines).unwrap_or(',');
                let head = self.current_buffer().rope().line(0).to_string();
                let schema = crate::table::Schema::from_header(&head, delimiter);
                if schema.columns.len() < 2
                    || !self.looks_delimited(delimiter, schema.columns.len())
                {
                    // The **file** is not a table. A run of its lines still
                    // may be (#216): a 碼表 under a heading, a `dict.yaml`
                    // whose entries begin after a `---` preamble, a `tabular`
                    // in the middle of a paper. That block is recognised where
                    // it stands rather than converted.
                    if self.enter_block_table(surface) {
                        return true;
                    }
                    self.status = say!("table.file-is-not-a-grid", path.file_name().unwrap_or_default().to_string_lossy());
                    return false;
                }
                (PathBuf::new(), schema, say!("table.header-from-first-row"))
            }
        };
        let columns = schema.columns.len();
        let delimiter = schema.delimiter;
        self.table = Some(TableView {
            schema,
            from,
            goal: 0,
            grain: Grain::Cell,
            separator: Separator::Delimiter(delimiter),
            surface,
            // A file a schema claims, or one whose own header row is the
            // schema, says what it is by its name — so the mode is the file's
            // and stays on when the cursor walks out of a row (#275).
            bounds: Bounds::WholeFile,
            reach: Reach::File,
        });
        // A grid is read across: rows run left to right and columns stack down
        // the page, which is the one thing a 縱書 layout cannot do. Rather than
        // draw something incoherent, table mode is horizontal.
        // …and the cursor lands on the first **data** row. Standing on the
        // header, `cell_position` says row 0 — a row the grid draws frozen at
        // the top and refuses every edit on — while the caret was drawn on the
        // first row of data, so the cell you were told you were in and the
        // cell the caret sat in were two different cells.
        if self.on_header_row() {
            let rope = self.current_buffer().rope();
            if rope.len_lines() > 1 {
                let at = rope.line_to_char(1);
                self.set_cursor(at);
            }
        }
        self.snap_to_cell();
        let turned = self.turn_for_table();
        self.status = match turned {
            true => say!("table.entered-turned-horizontal", columns, how),
            false => say!("table.entered-with-header", columns, how),
        };
        true
    }

    /// Whether the file's own first lines agree that it is a table.
    ///
    /// The header-row fallback used to take any first line with a comma in it,
    /// which meant `:table` on a page of prose whose first sentence held one
    /// turned the manuscript into a two-column grid. A delimited file has the
    /// property prose never has: **every line has the same number of fields**.
    /// Twenty lines is enough to tell, and is what a person would look at.
    fn looks_delimited(&self, delimiter: char, columns: usize) -> bool {
        let lines = self.first_lines(20);
        lines.len() >= 2
            && lines
                .iter()
                .all(|l| crate::table::cells(l, delimiter).len() == columns)
    }

    /// The first `how_many` lines of the buffer that hold anything.
    ///
    /// What both halves of the header-row fallback look at — the sniffer's
    /// guess and the agreement check — so they cannot be looking at different
    /// files. Trailing newlines are off: a line is its text.
    fn first_lines(&self, how_many: usize) -> Vec<String> {
        let rope = self.current_buffer().rope();
        (0..rope.len_lines().min(how_many))
            .map(|i| rope.line(i).to_string())
            .map(|l| l.trim_end_matches(['\n', '\r']).to_string())
            .filter(|l| !l.trim().is_empty())
            .collect()
    }

    /// Read the run of delimited lines under the cursor as a grid (#216).
    ///
    /// **Recognised, not declared.** The other three doors are somebody saying
    /// what a file is — a schema beside it, a name like `.tsv`, a `|` on every
    /// line. This one is the editor looking at a few lines of a document and
    /// agreeing that they are a table: a 碼表 under a heading, a `dict.yaml`
    /// whose entries start after its `---` preamble, a `tabular` in a paper.
    /// Nothing is rewritten and nothing is written down — the block is walked
    /// again from wherever the cursor is, every time it is wanted.
    ///
    /// Answers whether it entered, and says nothing when it did not: the
    /// caller has a better message for 「this is not a table」 than this does,
    /// because the caller knows which door was being tried.
    fn enter_block_table(&mut self, surface: Surface) -> bool {
        let rope = self.current_buffer().rope();
        let at = rope.char_to_line(self.cursor.min(rope.len_chars()));
        // **The walk is the test.** Each candidate is tried by walking the
        // block out with it and asking whether what comes back is rectangular;
        // the first that answers yes is the separator. Guessing first and
        // walking after cannot work — the lines to guess from are the block,
        // and the block is not known until the separator is: a 碼表 sitting
        // directly under `## 第三章` has a heading in its own paragraph, and no
        // count of tabs over *that* run agrees about anything.
        for delimiter in self.separators_worth_trying() {
            let Some(region) = self.delimited_block(at, delimiter) else {
                continue;
            };
            // **Check before entering.** If the walked block's rows disagree
            // about how many cells they have, the separator was guessed wrong,
            // and a crooked grid drawn over somebody's prose is worse than
            // being told no.
            let counts: Vec<usize> = (region.first..=region.last)
                .map(|line| {
                    crate::table::cells(&self.line_text(line).unwrap_or_default(), delimiter)
                        .len()
                })
                .collect();
            if !Self::rows_agree(&counts) {
                continue;
            }
            // The grid is as wide as its **widest** row, not as wide as the
            // count they agreed on: the entries of a `dict.yaml` that carry a
            // 權重 are still carrying it, and a column drawn nowhere is a
            // column that cannot be walked into.
            let columns = counts.iter().copied().max().unwrap_or(0);
            let rows = counts.len();
            self.table = Some(TableView {
                // Its first row is **data**: a 碼表 has no header, and reading
                // one as the column names would lose that row and call one
                // column 「一」. The columns are named by number, which is what
                // the column-number row already draws (#184).
                schema: crate::table::Schema::numbered(columns, delimiter),
                from: PathBuf::new(),
                goal: 0,
                grain: Grain::Cell,
                separator: Separator::Delimiter(delimiter),
                // Drawn as part of the document it sits in. The grid widget
                // clears the frame, and clearing the chapter in order to look
                // at three lines of it is not what was asked for — nor may a
                // 縱書 chapter be turned sideways for them.
                surface,
                bounds: Bounds::Block,
                // **Guessed, so it does not outlive the cursor** (#275). The
                // separator was inferred from a run of tab characters; the
                // moment the reader walks off the block, the file is prose
                // again.
                reach: Reach::Cursor,
            });
            self.snap_to_cell();
            self.status = say!(
                "table.block-entered",
                columns,
                rows,
                named_delimiter(delimiter)
            );
            self.turn_for_table_and_say();
            return true;
        }
        false
    }

    /// The separators to try on the block under the cursor, best first (#216).
    ///
    /// **What the cursor is standing on comes first** — `ci"`'s own idea, so
    /// nothing has to be prompted for, and it is how a person says 「this one」
    /// about a line that holds a tab *and* a comma. Then, if lines are
    /// selected, whichever candidate they agree about; then the rest, in the
    /// order [`crate::table::BLOCK_GUESSES`] puts them.
    fn separators_worth_trying(&self) -> Vec<char> {
        let mut order: Vec<char> = Vec::new();
        let mut add = |c: char| {
            if !order.contains(&c) {
                order.push(c);
            }
        };
        if let Some(c) = self
            .char_at_cursor()
            .filter(|c| crate::table::BLOCK_GUESSES.contains(c))
        {
            add(c);
        }
        let (from, to) = self.selection();
        if to > from + 1 {
            let rope = self.current_buffer().rope();
            let first = rope.char_to_line(from.min(rope.len_chars()));
            let last = rope.char_to_line(to.saturating_sub(1).min(rope.len_chars()));
            let lines: Vec<String> = (first..=last)
                .filter_map(|line| self.line_text(line))
                .map(|line| line.trim_end_matches(['\n', '\r']).to_string())
                .collect();
            if let Some(c) = crate::table::sniff_among(&lines, &crate::table::BLOCK_GUESSES) {
                add(c);
            }
        }
        for c in crate::table::BLOCK_GUESSES {
            add(c);
        }
        order
    }

    /// Whether a block's rows agree about how many cells they have (#216).
    ///
    /// **A short block must agree exactly; a long one only mostly.** With two
    /// or three rows there is no such thing as「most of them」, and letting two
    /// lines out of three carry it is how a paragraph of English with a comma
    /// in it becomes a grid. Past that, the slack is real: a `dict.yaml` has a
    /// 權重 on some entries and not on others, and refusing the whole table
    /// over the entries that lack one would be refusing every real one.
    fn rows_agree(counts: &[usize]) -> bool {
        if counts.len() < 2 {
            return false;
        }
        // Ties go to the wider count, so a table whose rows are half two cells
        // and half three is read as three — the narrow rows are short, not the
        // wide ones long.
        let Some(most) = counts
            .iter()
            .copied()
            .max_by_key(|&n| (counts.iter().filter(|&&m| m == n).count(), n))
        else {
            return false;
        };
        let agreed = counts.iter().filter(|&&n| n == most).count();
        let enough = match counts.len() < 4 {
            true => agreed == counts.len(),
            false => agreed * 3 >= counts.len() * 2,
        };
        most >= 2 && enough
    }

    /// Switch between the two table modes without going back to prose (#275).
    ///
    /// 「三種模式…`t i` 進表格操作，`t t` 進真表格顯示」 — and the two switch
    /// straight into one another, so pressing the other one is never the way
    /// out. `t q` is the way out.
    fn show_table_as(&mut self, want: Surface) {
        let Some(view) = self.table.as_mut() else {
            return;
        };
        if view.surface == want {
            self.status = match want {
                Surface::Page => say!("table.already-drawn"),
                Surface::InProse => say!("table.already-operated"),
            };
            return;
        }
        view.surface = want;
        match want {
            // A grid is read across, so 真表格顯示 turns a 縱書 page horizontal
            // — the same rule wherever the table sits, which is what the author
            // asked for: 「照舊把整頁轉橫」.
            Surface::Page => {
                self.turn_for_table();
                self.status = say!("table.drawn");
            }
            // …and 表格操作 gives the page back, because the writing around the
            // table is being read as writing again.
            Surface::InProse => {
                if let Some(back) = self.turned_for_table.take() {
                    self.layout = back;
                    self.zong_motion = false;
                }
                self.status = say!("table.operated");
            }
        }
    }

    /// Go back to reading the file as plain text.
    pub fn leave_table(&mut self) {
        let turned = self.turned_for_table.is_some();
        self.leave_table_quietly();
        self.status = if turned {
            say!("table.off-back-to-vertical")
        } else {
            say!("table.off")
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
            let delimiter = schema.delimiter;
            self.table = Some(TableView {
                schema,
                from,
                goal: 0,
                grain: Grain::Cell,
                separator: Separator::Delimiter(delimiter),
                surface: Surface::Page,
                bounds: Bounds::WholeFile,
                reach: Reach::File,
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
            self.status = say!("table.schema-problems", listed(&problems));
        }
    }

    /// Turn the page horizontal for a grid, remembering what it was.
    fn turn_for_table(&mut self) -> bool {
        // **The surface, and only the surface** (#261). A table drawn in prose
        // is part of a page, and turning the page sideways to edit three lines
        // of it would throw away everything around them. This line always meant
        // that; it used to have to say it by naming Markdown, which made it
        // read as an exception for one kind of table.
        if !self.table.as_ref().is_some_and(|v| v.is_page()) {
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

    /// Turn the page for a grid, and say so after whatever was just said.
    ///
    /// **The cold door had to say it too** (#275). `t t` on a `|` table in a
    /// 縱書 chapter — or `-t` on the command line — set 真表格顯示 and left the
    /// page vertical, where nothing draws a grid: the status line said 「第 1
    /// 行 · 甲 · 格」 and the screen had not changed by one character. The
    /// whole-file door says both things in one sentence
    /// (`table.entered-turned-horizontal`); these two have their own first
    /// half, so the turn is a clause on the end.
    fn turn_for_table_and_say(&mut self) {
        if self.turn_for_table() {
            let said = std::mem::take(&mut self.status);
            self.status = say!("table.also-turned-horizontal", said);
        }
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
        match self.table.as_ref().map(|v| v.bounds) {
            Some(Bounds::WholeFile) => true,
            // Both of the in-document kinds are a grid for the lines they
            // occupy and nowhere else, which is the same question and now one
            // call: walk out of a 碼表 block into the paragraph under it and
            // `hjkl` are letters again.
            Some(Bounds::Md) | Some(Bounds::Block) => self.prose_region().is_some(),
            None => false,
        }
    }

    /// Whether joining the line the cursor is on with the one below welds two
    /// rows of a grid together.
    ///
    /// **Not `table_here()`.** That asks whether `:table` is on, and a `|`
    /// table in a manuscript is a grid whether or not anybody said so — which
    /// is the state the author's own documentation is edited in. It is also
    /// true one line *above* a table: joining a paragraph onto the header row
    /// gives that row the paragraph's zero cells.
    fn joining_welds_a_grid(&self) -> bool {
        if self.table_here() {
            return true;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        if line + 1 >= rope.len_lines() {
            return false;
        }
        // Only the lines in question, and only the fence state above them: the
        // whole file is scanned once per `gJ`, which is a key nobody holds down.
        let rows = crate::mdtable::row_lines(&self.current_buffer().text());
        rows.get(line).copied().unwrap_or(false) || rows.get(line + 1).copied().unwrap_or(false)
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

    /// Whether `line` reads as a `|` table row, **without copying it out**.
    ///
    /// The same test as [`crate::mdtable::is_row`], asked of the rope: a line
    /// here is a paragraph and a paragraph is routinely a chapter, so
    /// materialising one to look at its first character cost 44 µs a call on a
    /// half-million-character paragraph — and every line the page touches is
    /// asked, several times a frame, in a novel with no table in it at all.
    fn opens_a_row(&self, line: usize) -> bool {
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return false;
        }
        let mut chars = rope.line(line).chars().skip_while(|c| c.is_whitespace());
        chars.next() == Some('|') && chars.any(|c| !c.is_whitespace())
    }

    /// The table **inside the document** the cursor is in, of either kind —
    /// worked out afresh, never stored.
    ///
    /// A remembered `first` is wrong the moment a row is opened above it, and
    /// the walk costs a few lines around the cursor. So the region is a
    /// question the editor asks, not a fact it keeps.
    ///
    /// Two walks answer it, one per [`Bounds`], and everything that only wants
    /// to know **where the table stops** asks this rather than either: cell
    /// motion, the column search and the tint the renderer draws are the same
    /// question whether the cells are cut by pipes or by tabs.
    pub fn prose_region(&self) -> Option<crate::mdtable::Region> {
        let view = self.table.as_ref()?;
        let bounds = view.bounds;
        let separator = view.separator;
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        // Keyed by the buffer's **id**, not by its index — see `blocks_through`.
        let asked = (
            self.current_buffer().id(),
            self.current_buffer().revision(),
            line,
            bounds,
        );
        if let Some(cache) = self.md_cache.borrow().as_ref() {
            if cache.asked == asked {
                return cache.region.clone();
            }
        }
        let region = match (bounds, separator) {
            // The fence is checked **here**, not only on the way in. Checking
            // it at the door was not enough: `gg`, `G`, `:N` and a search all
            // land outside the cells — the manual says so — and from a quoted
            // example in a code block `t t` then reformatted somebody's text.
            // The region is what everything downstream asks about, so this is
            // where a table that is really a quotation has to stop being one.
            (Bounds::Md, _) => match self.md_row_in_a_fence() {
                true => None,
                false => crate::mdtable::region(|i| self.line_text(i), line),
            },
            (Bounds::Block, Separator::Delimiter(d)) => self.delimited_block(line, d),
            _ => None,
        };
        *self.md_cache.borrow_mut() = Some(MdCache {
            asked,
            region: region.clone(),
        });
        region
    }

    /// Every `|` table in the file that **parses as one**, walked once per
    /// edit (#275).
    ///
    /// > markdown 中，表格必須是符合 markdown 語法的，可以被正確 parse 的表格
    /// > 才會进去普通或高级表格视图。否则代码也写不干净。
    ///
    /// — so a header with no `| --- |` under it and a single-column line that
    /// merely opens with a pipe are prose, and so is a table quoted inside a
    /// fenced block, which is *an example of* a table (the manual has
    /// several). One walk answers both of the questions that used to ask
    /// separately, which is why the fence check now covers the second of them
    /// as well.
    fn with_md_tables<T>(&self, f: impl FnOnce(&[(usize, usize)]) -> T) -> T {
        let asked = (self.current_buffer().id(), self.current_buffer().revision());
        if let Some(cache) = self.md_tables.borrow().as_ref() {
            if cache.asked == asked {
                return f(&cache.rows);
            }
        }
        let lines = self.current_buffer().line_count();
        let blocks = self.blocks_through(lines.saturating_sub(1));
        let mut rows = Vec::new();
        let mut line = 0;
        while line < lines {
            let Some(region) = crate::mdtable::region(|i| self.line_text(i), line) else {
                line += 1;
                continue;
            };
            let header = self.line_text(region.first).unwrap_or_default();
            let parses = region.rule.is_some() && crate::mdtable::cells(&header).len() >= 2;
            let quoted = blocks
                .get(region.first)
                .copied()
                .unwrap_or_default()
                .is_literal();
            if parses && !quoted {
                rows.push((region.first, region.last));
            }
            line = region.last.max(line) + 1;
        }
        let answer = f(&rows);
        *self.md_tables.borrow_mut() = Some(MdTables { asked, rows });
        answer
    }

    /// The table `line` belongs to, if the mode that is on covers it (#275).
    ///
    /// **The one question the renderer asks**, per line: prose or table, and if
    /// table, where it starts and stops so the 列號標尺 can be drawn along its
    /// top edge. Which of the two table modes is on it does not ask here —
    /// that is `table().surface`.
    ///
    /// The two classes of file part company in this function and nowhere else:
    ///
    /// - [`Reach::File`] — a `.md`, a `.csv`, a file a schema claims — answers
    ///   for **every** table in the file, so walking the cursor into the
    ///   paragraph between two of them leaves both drawn.
    /// - [`Reach::Cursor`] — a run of tab-separated lines the editor guessed at
    ///   — answers only for the one under the cursor. (It does not have to
    ///   check: `forget_a_guessed_table` has already put the mode away by the
    ///   time the cursor is anywhere else.)
    ///
    /// Inside Markdown only a table that **parses as one** answers: 「表格必須
    /// 是符合 markdown 語法的，可以被正確 parse 的表格才會进去」 — so a header
    /// with no `| --- |` under it, a single-column line that merely opens with
    /// a pipe, and a table quoted inside a fenced block are all prose.
    pub fn table_lines_at(&self, line: usize) -> Option<(usize, usize)> {
        let view = self.table.as_ref()?;
        match view.bounds {
            // 「csv 文件等同于一个从第一行到最后一行都是表格的普通文本文件」.
            Bounds::WholeFile => {
                let last = self.current_buffer().line_count().saturating_sub(1);
                (line <= last).then_some((0, last))
            }
            Bounds::Md if view.reach == Reach::File => {
                self.with_md_tables(|rows| rows.iter().copied().find(|&(a, b)| line >= a && line <= b))
            }
            Bounds::Md | Bounds::Block => {
                let region = self.prose_region()?;
                region.holds(line).then_some((region.first, region.last))
            }
        }
    }

    /// Whether `line` is drawn as a row of a table right now (#275).
    ///
    /// What the「表格所在的行不再 soft wrap」rule is asked through: a row of a
    /// grid is one row, however long it is, because a cell that has wrapped
    /// onto the next screen row is no longer in its column.
    pub fn table_row_at(&self, line: usize) -> bool {
        self.table_lines_at(line).is_some()
    }

    /// Where `line`'s cells are told apart, when it belongs to a table that is
    /// being **drawn** as a grid among the prose (#275).
    ///
    /// `(first line, last line, the character indices the separators sit at)`.
    /// Empty in the other two modes, and empty when the table has a pane of its
    /// own — there the grid is drawn by `crate::table` out of the schema rather
    /// than out of the writer's own punctuation.
    fn grid_walls(&self, line: usize) -> Option<(usize, usize, Vec<usize>)> {
        let view = self.table.as_ref()?;
        if view.surface != Surface::Page || view.takes_the_pane() {
            return None;
        }
        let (first, last) = self.table_lines_at(line)?;
        let text = self.line_text(line)?;
        let at = match view.separator {
            // `\|` inside a cell is a pipe the cell holds, not a wall. The
            // flag is「the character *before* this text was a live backslash」,
            // and there is no character before the start of a line — passing
            // `true` here quietly ate the opening wall of every row.
            Separator::Pipe => crate::mdtable::pipes_from(&text, false),
            Separator::Delimiter(c) => text
                .chars()
                .enumerate()
                .filter(|&(_, ch)| ch == c)
                .map(|(i, _)| i)
                .collect(),
        };
        Some((first, last, at))
    }

    /// The grid drawn over `line`, as `(character, glyph)` (#275).
    ///
    /// **真表格顯示 among the prose.** 「完全画成表格」 — so the `|` the writer
    /// typed is drawn as a rule, and the `| --- |` row as the line between the
    /// head and the body. A character is *replaced*, never taken off the page:
    /// every glyph here is one cell wide, so the file's columns and the page's
    /// columns stay the same columns, and the caret, `j`, the mouse and #212's
    /// padding need to know nothing about any of this.
    pub fn grid_on_line(&self, line: usize) -> Vec<(usize, char)> {
        let Some((_, _, at)) = self.grid_walls(line) else {
            return Vec::new();
        };
        let text = self.line_text(line).unwrap_or_default();
        // The rule row is not a row of the table — it is the drawing of the
        // line under the head, written in `-` because Markdown has no other way
        // to say it. Drawn, it is that line.
        if self.grid_rule_row(line) {
            let (opens, closes) = (at.first().copied(), at.last().copied());
            let len = text.trim_end_matches(['\n', '\r']).chars().count();
            // **Every** character *of the table*, the spaces around the dashes
            // included: a rule with the file's own spacing left in it is a
            // dashed line with four gaps chewed out of it — but the two spaces
            // that indent a table inside a list item, and any space left after
            // the closing wall, are not the table, and drawing them made the
            // rule stick out past the rows above and below it.
            let (from, upto) = match (opens, closes) {
                (Some(a), Some(b)) => (a, b + 1),
                _ => (0, len),
            };
            return (from..upto)
                .map(|i| match at.contains(&i) {
                    false => (i, '┄'),
                    true => match (Some(i) == opens, Some(i) == closes) {
                        (true, _) => (i, '├'),
                        (_, true) => (i, '┤'),
                        _ => (i, '┼'),
                    },
                })
                .collect();
        }
        at.into_iter().map(|i| (i, '┆')).collect()
    }

    /// Whether `line` is the `| --- |` row of a table drawn as a grid (#275).
    ///
    /// Asked by the page as well as by [`Self::grid_on_line`]: the padding that
    /// squares a table up writes that row's own dashes (#212), and on a rule
    /// being *drawn* those have to be drawn too, or the line stops wherever the
    /// file's dashes stopped.
    pub fn grid_rule_row(&self, line: usize) -> bool {
        self.grid_walls(line).is_some()
            && crate::mdtable::rule_of(&self.line_text(line).unwrap_or_default()).is_some()
    }

    /// The cells of `line`, when it is the **first** line of a table drawn as a
    /// grid among the prose (#275) — otherwise empty.
    ///
    /// The 列號標尺 the author asked for: 「畫，貼在表格上緣」. One per table, so
    /// a chapter with three tables in it shows three rulers, each numbering its
    /// own columns. Given as character spans rather than as screen columns
    /// because the page is the only one who knows where a character is drawn —
    /// the markup that came off it and the padding that squared it up have both
    /// moved it.
    pub fn table_ruler_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        let Some((first, _, at)) = self.grid_walls(line) else {
            return Vec::new();
        };
        if line != first {
            return Vec::new();
        }
        // A `|` row is walled on both sides, so its cells are the gaps between
        // the walls. A delimited row's walls stand *between* cells, so its first
        // cell opens at the start of the line and its last runs to the end.
        if self.table.as_ref().map(|v| v.separator) == Some(Separator::Pipe) {
            return at.windows(2).map(|w| (w[0] + 1, w[1])).collect();
        }
        // **Without the trim the last cell is one character too wide**, the
        // ruler asks the layout for a column that is past the end of the row,
        // and the last column's number is silently not drawn.
        let len = self
            .line_text(line)
            .unwrap_or_default()
            .trim_end_matches(['\n', '\r'])
            .chars()
            .count();
        let mut cells = Vec::with_capacity(at.len() + 1);
        let mut opens = 0;
        for wall in at {
            cells.push((opens, wall));
            opens = wall + 1;
        }
        cells.push((opens, len));
        cells
    }

    /// The `|` table the cursor is in.
    ///
    /// The pipe half of [`Self::prose_region`], for the things that are really
    /// about pipes: the rule row, the reflow, `t n`/`t D` and the rest of the
    /// Markdown surgery, none of which mean anything to a block that is being
    /// read where it lies and never rewritten.
    pub fn md_region(&self) -> Option<crate::mdtable::Region> {
        match self.table.as_ref().map(|v| v.bounds) {
            Some(Bounds::Md) => self.prose_region(),
            _ => None,
        }
    }

    /// The delimited block the cursor is in (#216).
    pub fn block_region(&self) -> Option<crate::mdtable::Region> {
        match self.table.as_ref().map(|v| v.bounds) {
            Some(Bounds::Block) => self.prose_region(),
            _ => None,
        }
    }

    /// The run of lines around `at` that `d` cuts into cells (#216).
    ///
    /// **Up and down while the line still holds the separator**, and never
    /// across a blank line. That is the whole rule, and it is the one a person
    /// applies by eye: the table ends where the tabs do. A blank line stops it
    /// as well, so a table with a blank line in the middle of it is two blocks
    /// rather than one — the safe way round, because the other way a single
    /// stray tab three paragraphs down would swallow the prose between.
    ///
    /// The columns are the **widest** row's, not the first's: a `dict.yaml`
    /// whose entries are 「字⇥碼」 with a 權重 on some of them is still one
    /// table, and a schema that named two columns would hide the third.
    fn delimited_block(&self, at: usize, d: char) -> Option<crate::mdtable::Region> {
        let holds = |line: usize| -> bool {
            self.line_text(line).is_some_and(|text| {
                let text = text.trim_end_matches(['\n', '\r']);
                !text.trim().is_empty() && text.contains(d)
            })
        };
        if !holds(at) {
            return None;
        }
        let mut first = at;
        while first > 0 && holds(first - 1) {
            first -= 1;
        }
        let mut last = at;
        let end = self.current_buffer().rope().len_lines();
        while last + 1 < end && holds(last + 1) {
            last += 1;
        }
        let columns = (first..=last)
            .map(|line| {
                crate::table::cells(&self.line_text(line).unwrap_or_default(), d).len()
            })
            .max()
            .unwrap_or(0);
        Some(crate::mdtable::Region {
            first,
            last,
            // A block has no rule row and no alignments: it is somebody's data
            // file, and the only thing being read out of it is where the cells
            // are. `is_rule` is false for every line, which is what lets the
            // cell motions be shared with the `|` table unchanged.
            rule: None,
            aligns: Vec::new(),
            columns,
        })
    }

    /// Read the `|` table under the cursor as a grid, drawn the given way.
    fn enter_md_table_as(&mut self, surface: Surface) -> bool {
        let rope = self.current_buffer().rope();
        let at = rope.char_to_line(self.cursor.min(rope.len_chars()));
        self.enter_md_table_at(at, surface)
    }

    /// The first line of the first `|` table in the file that **parses as one**
    /// (#275).
    ///
    /// > markdown 中，表格必須是符合 markdown 語法的，可以被正確 parse 的表格
    /// > 才會进去普通或高级表格视图。否则代码也写不干净。
    ///
    /// This is the table the file-wide mode is built from when the cursor is
    /// in the prose between two of them. What counts as one is
    /// [`Self::with_md_tables`]'s answer, so the door and the renderer cannot
    /// disagree — they did: `t t` in a file whose only table was quoted inside
    /// a fence entered a mode that then drew nothing.
    fn first_md_table_line(&self) -> Option<usize> {
        self.with_md_tables(|rows| rows.first().map(|&(first, _)| first))
    }

    /// Read the `|` table `at` this line as a grid, drawn the given way.
    fn enter_md_table_at(&mut self, at: usize, surface: Surface) -> bool {
        let Some(region) = crate::mdtable::region(|i| self.line_text(i), at) else {
            self.status = say!("table.cursor-not-in-a-table");
            return false;
        };
        let header = self.line_text(region.first).unwrap_or_default();
        // One column is a line with a pipe in it, not a table — and writing a
        // `| --- |` under a paragraph that happens to start with one is how a
        // convenience becomes damage.
        if region.rule.is_none() && crate::mdtable::cells(&header).len() < 2 {
            self.status = say!("table.one-column-only");
            return false;
        }
        let schema = crate::mdtable::schema(&header);
        self.table = Some(TableView {
            schema,
            from: PathBuf::new(),
            goal: 0,
            grain: Grain::Cell,
            separator: Separator::Pipe,
            surface,
            bounds: Bounds::Md,
            // **A `|` table says what it is on every one of its own lines**,
            // so the mode is the file's (#275): every `|` table in the file is
            // read as one, and walking into the paragraph between two of them
            // leaves both drawn. The author's second class — 「沒有確切的表格
            // 語法，比如 txt、yaml 用空格制表符隔開」 — is the *guessed* block,
            // and that is `enter_block_table`'s.
            reach: Reach::File,
        });
        // A header with no rule under it is a table nobody can render yet —
        // and the person is standing in it, so they meant to write one. Adding
        // it is the difference between a mode that works and a mode that says
        // no to the very first table you try it on.
        let added = region.rule.is_none();
        if added {
            self.snapshot();
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
        // **Looking at a table does not rewrite it.** Entering used to lay the
        // whole region out again — 45 lines of the author's own documentation,
        // `modified` set, and `:table off` does not undo it. The padding is
        // this editor's, not theirs, and `:wa` was one keystroke from
        // committing a diff nobody typed. The layout is kept up *after an
        // edit*, which is where it came from and where it belongs.
        self.snap_to_cell();
        self.status = match added {
            true => say!("table.entered-rule-added", columns),
            false => say!("table.entered", columns),
        };
        self.turn_for_table_and_say();
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
        self.replace_lines(region.first, region.last, lines);
    }

    /// Put `lines` in place of lines `first..=last`, keeping the file's own
    /// line endings and its rule about a trailing newline.
    fn replace_lines(&mut self, first: usize, last: usize, lines: &[String]) {
        let rope = self.current_buffer().rope();
        let start = rope.line_to_char(first.min(rope.len_lines().saturating_sub(1)));
        let ends_file = last + 1 >= rope.len_lines();
        let end = if ends_file {
            rope.len_chars()
        } else {
            rope.line_to_char(last + 1)
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

    // ---- Delimited text and `|` tables, both ways (Feature #227) ----------

    /// The lines a conversion is about: the selection, or the block the cursor
    /// stands in.
    ///
    /// **A blank line is a boundary.** Not the only one — a document is full of
    /// headings and prose — but it is the one every writer already uses to say
    /// 「this much belongs together」, and it costs nothing to honour it. Where
    /// there is a selection it wins outright: a selection is a person pointing.
    fn block_here(&self) -> (usize, usize) {
        let rope = self.current_buffer().rope();
        let last_line = rope.len_lines().saturating_sub(1);
        if self.has_selection() {
            let (a, b) = self.selection();
            let first = rope.char_to_line(a.min(rope.len_chars()));
            let mut last = rope.char_to_line(b.min(rope.len_chars()));
            // A selection that ends at the very start of a line stops before
            // that line, not on it — dragging down one row should not take the
            // row after it.
            if last > first && b == rope.line_to_char(last) {
                last -= 1;
            }
            return (first, last.min(last_line));
        }
        let here = self.cursor_line().min(last_line);
        let blank = |i: usize| {
            self.line_text(i)
                .is_none_or(|l| l.trim().is_empty())
        };
        let mut first = here;
        while first > 0 && !blank(first - 1) {
            first -= 1;
        }
        let mut last = here;
        while last < last_line && !blank(last + 1) {
            last += 1;
        }
        (first, last)
    }

    /// `:table pipe` — the delimited block under the cursor becomes a `|` table.
    fn table_to_pipe(&mut self, delimiter: Option<char>) {
        // `Buffer::insert` would refuse anyway, but silently and far too late:
        // by then the table has been left, the document forgotten and the grid
        // re-entered, and the status line says 「作成了 | 表格：3 行 2 欄」 for
        // a file that did not change a byte. Every caller that edits refuses
        // for itself; these two are callers.
        if self.refuse_readonly() {
            return;
        }
        let (first, last) = self.block_here();
        let lines: Vec<String> = (first..=last)
            .filter_map(|i| self.line_text(i))
            .map(|l| l.trim_end_matches(['\n', '\r']).to_string())
            .filter(|l| !l.trim().is_empty())
            .collect();
        if lines.is_empty() {
            self.status = say!("table.nothing-to-convert");
            return;
        }
        // Already a table: `:table pipe` on one would split every cell again on
        // whatever the sniffer guessed and hand back a wider, wrong table.
        if lines.iter().all(|l| crate::mdtable::is_row(l)) {
            self.status = say!("table.already-a-pipe-table");
            return;
        }
        let Some(delimiter) = delimiter.or_else(|| crate::table::sniff(&lines)) else {
            self.status = say!("table.no-delimiter-in-sight");
            return;
        };
        let out = crate::mdtable::from_delimited(&lines, delimiter);
        // `from_delimited` writes a header, a rule row and one row per line, so
        // with `lines` non-empty there is always a header to measure. Asking
        // for it rather than indexing keeps a broken contract from taking the
        // manuscript down with it; it used to be a separate emptiness check
        // above, which read as a case that could happen and never could.
        let Some(header) = out.first() else {
            self.status = say!("table.nothing-to-convert");
            return;
        };
        let rows = out.len().saturating_sub(1);
        let columns = crate::mdtable::split(header).len();
        self.snapshot();
        self.leave_table_quietly();
        self.replace_lines(first, last, &out);
        let at = self.current_buffer().rope().line_to_char(first);
        self.set_cursor(at);
        self.clamp_cursor();
        self.forget_the_document();
        // Straight into the grid: the writer asked for a table, and a table in
        // this editor is something you walk by cell.
        self.enter_table();
        self.status = say!("table.now-a-pipe-table", rows, columns, delimiter);
    }

    /// `:table csv` — the `|` table under the cursor becomes delimited lines.
    fn table_to_delimited(&mut self, delimiter: char) {
        if self.refuse_readonly() {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let region = match self.md_row_in_a_fence() {
            true => None,
            false => crate::mdtable::region(|i| self.line_text(i), line),
        };
        let Some(region) = region else {
            self.status = say!("table.not-in-a-pipe-table");
            return;
        };
        let lines = self.md_lines(&region);
        let out = match crate::mdtable::to_delimited(&lines, delimiter) {
            Ok(out) => out,
            // Named where the writer can see it: 「row 4, column 2」 is a place
            // in the table on the screen, not an offset in a file.
            Err((row, column)) => {
                self.status = say!(
                    "table.cell-holds-the-delimiter",
                    row + 1,
                    column + 1,
                    delimiter
                );
                return;
            }
        };
        // Every line of `out` is a row: `to_delimited` writes no rule row, and
        // there is nothing to subtract. (`table_to_pipe` *does* write one,
        // which is where the `- 1` this used to have came from — copied across
        // and wrong here: it said 「2 行」 for the three rows it had written.)
        let rows = out.len();
        self.snapshot();
        self.leave_table_quietly();
        self.replace_lines(region.first, region.last, &out);
        let at = self
            .current_buffer()
            .rope()
            .line_to_char(region.first.min(self.current_buffer().rope().len_lines() - 1));
        self.set_cursor(at);
        self.clamp_cursor();
        self.forget_the_document();
        self.status = say!("table.now-delimited", rows, named_delimiter(delimiter));
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

    /// Write an empty `|` table here and stand in its first heading (#276).
    ///
    /// The author, 2026-09-05: 「`:table new 3 4`，迅速在 markdown 中插入一個三
    /// 行四列表格，上下有空白行，光標自動到標題欄最左的一格並進去編輯模式。」
    ///
    /// **`rows` counts the heading**, the way a word processor's「3 × 4」does:
    /// `3 4` is a heading and two rows of data, four columns wide. The rule row
    /// is not a row — it is punctuation, and nobody means it when they say
    /// three.
    ///
    /// The blank line above and below is not tidiness: a `|` row welded to the
    /// paragraph above it is a table Markdown does not see, and the reader who
    /// asked for a table would have got a paragraph with pipes in it.
    fn new_table(&mut self, rows: usize, columns: usize) {
        if self.refuse_readonly() {
            return;
        }
        let rows = rows.max(1);
        let columns = columns.max(1);
        let mut block = vec![
            crate::mdtable::blank_row(columns),
            crate::mdtable::rule_row(columns),
        ];
        for _ in 1..rows {
            block.push(crate::mdtable::blank_row(columns));
        }
        let rope = self.current_buffer().rope();
        let here = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let empty = |e: &Self, line: usize| {
            e.line_text(line)
                .map(|t| t.trim().is_empty())
                .unwrap_or(true)
        };
        // An empty line is where the writer already made room; anywhere else
        // the table goes *under* the line they are standing on rather than
        // through the middle of it.
        let at_line = match empty(self, here) {
            true => here,
            false => here + 1,
        };
        // The blank line above, when the line before is not already one.
        let above = at_line > 0 && !empty(self, at_line - 1);
        if above {
            block.insert(0, String::new());
        }
        // …and below, when what this pushes down is not one either.
        if !empty(self, at_line) {
            block.push(String::new());
        }
        let text = block.join("\n");
        let rope = self.current_buffer().rope();
        let (at, text) = match at_line >= rope.len_lines() {
            true => (rope.len_chars(), format!("\n{text}\n")),
            false => (rope.line_to_char(at_line), format!("{text}\n")),
        };
        self.snapshot();
        self.without_cell_guard(|e| e.current_buffer_mut().insert(at, &text));
        // The heading is the first `|` line of what was just written, which is
        // one further down when a blank line went in ahead of it.
        let heading = at_line + usize::from(above);
        let start = self.current_buffer().rope().line_to_char(heading);
        self.move_head(start);
        // Reading it as a grid is what makes ⇥ walk the cells, so the table is
        // entered before the cursor is put in a cell — `go_to_cell` asks the
        // view which columns are drawn.
        //
        // **表格操作, not 真表格顯示** (#275): the writer is about to fill this
        // in, and the pipes they just asked for should be on the page while
        // they do it. `t t` draws it once it has something in it.
        self.enter_md_table_as(Surface::InProse);
        self.go_to_cell(heading, 0);
        self.enter_insert();
        self.status = say!("table.written", rows.to_string(), columns.to_string());
    }

    /// Put a new row in below the cursor's (or above it).
    fn md_new_row(&mut self, below: bool) {
        let Some((region, mut parts)) = self.md_parts() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        let at = parts.insert_row(if below { row + 1 } else { row });
        self.md_write(&region, &parts, at, cell);
        self.status = say!("table.row-added");
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
                self.status = say!("table.row-deleted");
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
                self.status = match down {
                    true => say!("table.row-moved-down"),
                    false => say!("table.row-moved-up"),
                };
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
        self.status = say!("table.column-added");
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
                self.status = say!("table.column-deleted");
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
                self.status = match right {
                    true => say!("table.column-moved-right"),
                    false => say!("table.column-moved-left"),
                };
            }
            Err(why) => self.status = why.to_string(),
        }
    }

    /// Put the rows in order by the cursor's column.
    fn md_sort(&mut self, descending: bool) {
        self.md_sort_by(&[], descending)
    }

    /// The same, by the columns `t1a2d8as` named — counted from one, and empty
    /// for「the column the cursor is in」.
    fn md_sort_by(&mut self, keys: &[(usize, bool)], descending: bool) {
        let Some((region, mut parts)) = self.md_parts() else {
            return;
        };
        let (_, cell) = self.md_at(&region);
        if parts.rows.len() < 3 {
            self.status = say!("table.too-few-rows-to-sort");
            return;
        }
        // The reader counts columns from one; the table counts from zero.
        let keys: Vec<(usize, bool)> = match keys.is_empty() {
            true => vec![(cell, descending)],
            false => keys.iter().map(|&(c, d)| (c.saturating_sub(1), d)).collect(),
        };
        parts.sort_by_keys(&keys);
        let cell = keys.first().map(|&(c, _)| c).unwrap_or(cell);
        let descending = keys.first().map(|&(_, d)| d).unwrap_or(descending);
        let name = parts
            .rows
            .first()
            .and_then(|r| r.get(cell))
            .cloned()
            .unwrap_or_default();
        // Back to the header, because the row you were standing on is now
        // somewhere else and pretending otherwise would be a lie.
        self.md_write(&region, &parts, 0, cell);
        self.status = match descending {
            true => say!("table.sorted-descending", name),
            false => say!("table.sorted-ascending", name),
        };
    }

    /// **Put the rows in order** by one column or several.
    ///
    /// `:table sort 1 a 2 d 4 a` — first column ascending, then second
    /// descending, then fourth ascending — `t1a2d4as` is the same thing from
    /// the keyboard, and `t1s` / `t1S` the short spelling for one column
    /// from the keyboard. With no columns named it sorts by the one the cursor
    /// is in, which is what `t s` has always meant.
    ///
    /// A grid keeps its rows: sorting moves them, and moves nothing else. The
    /// header stays where it is, and every row's cells are the cells it had.
    /// How many columns the table has, for checking a column number against.
    ///
    /// The widest row rather than the header: a delimited file whose header
    /// line is short still has the columns its body rows have, and a sort by
    /// one of them is a sort a reader can mean.
    fn table_columns(&self) -> usize {
        let Some(view) = self.table.as_ref() else {
            return 0;
        };
        if let Some((_, parts)) = self.md_parts() {
            return parts.columns();
        }
        let rope = self.current_buffer().rope();
        (0..rope.len_lines())
            .map(|line| view.cells(&rope.line(line).to_string()).len())
            .max()
            .unwrap_or(0)
    }

    fn sort_table(&mut self, keys: &[(usize, bool)]) {
        let Some(view) = self.table.as_ref() else {
            self.status = say!("table.not-in-a-table");
            return;
        };
        // A block recognised where it stands is not rewritten (#216) — and
        // the sort below rebuilds the file from its own lines, which for a few
        // lines of a chapter is the whole chapter.
        if view.bounds == Bounds::Block {
            self.status = say!("table.block-is-read-where-it-lies");
            return;
        }
        // **A column that is not there is said, not ignored.** `t99a1ds` used
        // to sort by nothing at all — every cell of column 99 is missing, so
        // every pair compared equal — and then report 「照『』順排」 with an
        // empty column name; `t0s` quietly meant the first column, because
        // the reader counts from one and `saturating_sub` floors at zero.
        let width = self.table_columns();
        if let Some(&(column, _)) = keys.iter().find(|&&(c, _)| c == 0 || c > width) {
            self.status = say!("table.no-such-column", &column.to_string(), &width.to_string());
            return;
        }
        // A `|` table laid out **in the text** already has a sort that keeps
        // that layout right: moving its rows means rewriting them, padding and
        // all, where a grid's rows are moved and the renderer lays them out
        // again.
        if matches!(view.separator, Separator::Pipe) {
            let descending = keys.first().map(|&(_, d)| d).unwrap_or(false);
            if let Some((column, _)) = keys.first() {
                let line = self.cursor_line();
                self.go_to_cell(line, column.saturating_sub(1));
            }
            self.md_sort_by(keys, descending);
            return;
        }
        let here = self.cell_position().map(|(_, c)| c).unwrap_or(0);
        let keys: Vec<(usize, bool)> = match keys.is_empty() {
            true => vec![(here, false)],
            // The reader counts columns from one; the file counts from zero.
            false => keys.iter().map(|&(c, d)| (c.saturating_sub(1), d)).collect(),
        };
        let delimiter = view.schema.delimiter;
        let header = usize::from(view.schema.header);
        let text = self.current_buffer().text();
        let ends_with_newline = text.ends_with('\n');
        // **Whatever this file ends its lines with, it goes on ending them with
        // it.** `str::lines` strips `\r\n` and a naive rejoin writes `\n`, so
        // one keystroke rewrote all 123,381 lines of a Windows-authored 拆分表
        // and nothing on the screen said so.
        let eol = match text.contains("\r\n") {
            true => "\r\n",
            false => "\n",
        };
        let mut lines: Vec<&str> = text.lines().collect();
        if lines.len() <= header + 1 {
            self.status = say!("table.too-few-rows-to-sort");
            return;
        }
        // The trailing empty line a file ending in a newline leaves is not a
        // row and must not be sorted into the middle.
        let body = &mut lines[header..];
        let cell = |line: &str, at: usize| -> String {
            crate::table::cells(line, delimiter)
                .get(at)
                .map(|&s| crate::table::cell_text(line, s))
                .unwrap_or_default()
        };
        // Numbers as numbers, everything else by code point — the same rule the
        // `|` sort follows, so one table does not sort two ways.
        let rank = |value: String| -> (bool, f64, String) {
            match value.trim().parse::<f64>() {
                Ok(n) => (false, n, String::new()),
                Err(_) => (true, 0.0, value),
            }
        };
        body.sort_by(|a, b| {
            for &(column, descending) in &keys {
                let (ka, kb) = (rank(cell(a, column)), rank(cell(b, column)));
                let order = ka
                    .0
                    .cmp(&kb.0)
                    .then(ka.1.total_cmp(&kb.1))
                    .then_with(|| ka.2.cmp(&kb.2));
                let order = match descending {
                    true => order.reverse(),
                    false => order,
                };
                if order != std::cmp::Ordering::Equal {
                    return order;
                }
            }
            std::cmp::Ordering::Equal
        });
        let mut rebuilt = lines.join(eol);
        if ends_with_newline {
            rebuilt.push_str(eol);
        }
        if rebuilt == text {
            self.status = say!("table.already-in-that-order");
            return;
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        self.without_cell_guard(|e| {
            e.current_buffer_mut().remove(0..len);
            e.current_buffer_mut().insert(0, &rebuilt);
        });
        self.clamp_cursor();
        // The *text*, not the document: the table is still this table, and
        // asking the file what it is again would put the grid away.
        self.forget_the_text();
        let named: Vec<String> = keys
            .iter()
            .map(|&(c, d)| {
                let name = self
                    .table
                    .as_ref()
                    .and_then(|v| v.schema.columns.get(c))
                    .map(|col| col.heading().to_string())
                    .unwrap_or_else(|| (c + 1).to_string());
                match d {
                    true => say!("label.sort-descending", name),
                    false => say!("label.sort-ascending", name),
                }
            })
            .collect();
        self.status = say!("table.sorted", named.join(" "));
    }

    /// Change which way this column's cells are set.
    fn md_align(&mut self, align: crate::mdtable::Align) {
        let Some((region, mut parts)) = self.md_parts() else {
            return;
        };
        let (row, cell) = self.md_at(&region);
        if !parts.ruled {
            self.status = say!("table.no-rule-row-to-align");
            return;
        }
        let columns = parts.columns();
        parts.aligns.resize(columns, crate::mdtable::Align::default());
        if cell >= columns {
            return;
        }
        parts.aligns[cell] = align;
        self.md_write(&region, &parts, row, cell);
        self.status = match align {
            crate::mdtable::Align::Left | crate::mdtable::Align::Plain => say!("table.column-left"),
            crate::mdtable::Align::Center => say!("table.column-centred"),
            crate::mdtable::Align::Right => say!("table.column-right"),
        };
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
                    self.status = say!("table.row-added");
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
        if let Some(region) = self.prose_region() {
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
        // A `|` cell's padding is layout, not content: it is not in the span,
        // so landing on a cell lands on its first real character rather than on
        // the space before it.
        view.cells(&rope.line(line).to_string())
    }

    /// Where every cell's **box** begins and ends — its padding included.
    ///
    /// [`Self::row_cells`] answers what an edit takes; this answers what a
    /// reader sees as one cell. The two differ only for a `|` table, where the
    /// spaces around the content are the column's own width: tinting the
    /// content alone leaves the tint ragged where the table is square, and an
    /// **empty** cell — the one you are most likely to be standing in, because
    /// you came here to fill it — has a content span of zero characters and
    /// would not be drawn at all.
    pub fn row_cell_boxes(&self, line: usize) -> Vec<(usize, usize)> {
        let Some(view) = &self.table else {
            return Vec::new();
        };
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return Vec::new();
        }
        // A delimited row has no padding to include: its cells are already
        // contiguous, delimiter to delimiter, which is why `boxes` and `cells`
        // are the same answer there.
        view.boxes(&rope.line(line).to_string())
    }

    /// The buffer range one cell's box covers, padding included.
    pub fn cell_box(&self, line: usize, cell: usize) -> Option<(usize, usize)> {
        let boxes = self.row_cell_boxes(line);
        let &(a, b) = boxes.get(cell)?;
        let start = self.current_buffer().rope().line_to_char(line);
        Some((start + a, start + b))
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

    /// Take a copy of the whole column the cursor is in (`t y`).
    ///
    /// One cell to a line — which is what a column *is*, and what `t p` reads
    /// back. It is also a shape every other tool understands, so a column
    /// yanked here pastes into a spreadsheet as a column.
    fn yank_column(&mut self) {
        let Some((_, cell)) = self.cell_position() else {
            return;
        };
        let values = self.column_values(cell);
        if values.is_empty() {
            self.status = say!("table.column-is-empty");
            return;
        }
        let n = values.len();
        self.store(format!("{}\n", values.join("\n")));
        let name = self
            .table
            .as_ref()
            .and_then(|v| v.schema.columns.get(cell))
            .map(|c| c.heading().to_string())
            .unwrap_or_default();
        self.status = say!("table.yanked-column", name, n);
    }

    /// Every cell of one column, header included, top to bottom.
    fn column_values(&self, cell: usize) -> Vec<String> {
        if let Some((_, parts)) = self.md_parts() {
            return parts
                .rows
                .iter()
                .map(|row| row.get(cell).cloned().unwrap_or_default())
                .collect();
        }
        self.cell_lines()
            .into_iter()
            .map(|line| self.cell_text(line, cell))
            .collect()
    }

    /// The lines a grid's rows sit on, top to bottom.
    ///
    /// Every line of the file when the file *is* the table: a blank line is
    /// still a row, of one empty cell, and a column yanked over it carries the
    /// blank along so that putting it back puts it back where it came from.
    /// What this is for is that `t y` and `t p` ask **one** question — they
    /// used to each walk the lines their own way, and tightening the filter on
    /// one side alone would have shifted every value below the blank by a row
    /// without a single test noticing.
    ///
    /// **A block recognised in a document is bounded by the block** (#216).
    /// Without that, `t p` inside a 碼表 pasted into a chapter would write the
    /// yanked column down the whole manuscript.
    fn cell_lines(&self) -> Vec<usize> {
        let (first, last) = match self.block_region() {
            Some(region) => (region.first, region.last),
            None => (0, motion::last_line(self.current_buffer().rope())),
        };
        (first..=last)
            .filter(|&line| !self.row_cells(line).is_empty())
            .collect()
    }

    /// Write what was yanked down the cursor's column (`t p`).
    fn put_column(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let text = self.recall();
        let values: Vec<String> = text
            .trim_end_matches('\n')
            .lines()
            .map(str::to_string)
            .collect();
        if values.is_empty() {
            self.status = say!("edit.nothing-yanked-yet");
            return;
        }
        let n = values.len();
        if let Some((region, mut parts)) = self.md_parts() {
            let (_, cell) = self.md_at(&region);
            for (r, value) in values.iter().enumerate() {
                while r >= parts.rows.len() {
                    parts.insert_row(parts.rows.len());
                }
                let width = parts.columns().max(cell + 1);
                let row = &mut parts.rows[r];
                row.resize(width, String::new());
                row[cell] = crate::mdtable::escape(value);
            }
            self.md_reschema(&parts);
            self.md_write(&region, &parts, 0, cell);
            self.status = say!("table.pasted-column", n);
            return;
        }
        let Some((_, cell)) = self.cell_position() else {
            return;
        };
        let d = self.table.as_ref().map(|v| v.schema.delimiter).unwrap_or(',');
        let lines = self.cell_lines();
        // **A value holding the delimiter is refused, not filtered.** It used
        // to be stripped out character by character, which is the same silent
        // damage `:table csv` refuses in the other direction: 「長, 久」 went
        // in as 「長 久」 and nothing said so. Refused *before* the snapshot,
        // so a refusal costs the writer nothing to undo.
        if let Some((r, _)) = values.iter().enumerate().find(|(_, v)| v.contains(d)) {
            let row = lines.get(r).map(|l| l + 1).unwrap_or(r + 1);
            self.status = say!("table.cell-holds-the-delimiter", row, cell + 1, d);
            return;
        }
        self.snapshot();
        for (r, value) in values.iter().enumerate() {
            let Some(&line) = lines.get(r) else { break };
            let Some((from, to)) = self.cell_span(line, cell) else {
                continue;
            };
            self.without_cell_guard(|e| {
                e.current_buffer_mut().remove(from..to);
                e.current_buffer_mut().insert(from, value);
            });
        }
        self.snap_to_cell();
        self.status = say!("table.pasted-column", n);
    }

    /// Put a block of cells in, starting at the cursor's.
    ///
    /// Growing the table as it needs to when the table is Markdown's — its
    /// shape is the document's and the document is the writer's. A delimited
    /// file's columns are its schema's, so a block too wide for it is refused
    /// rather than silently shifting every row.
    fn paste_grid(&mut self, grid: Vec<Vec<String>>) {
        if self.refuse_readonly() {
            return;
        }
        let (rows, columns) = (grid.len(), grid.iter().map(Vec::len).max().unwrap_or(0));
        if let Some((region, mut parts)) = self.md_parts() {
            let (row, cell) = self.md_at(&region);
            for (r, line) in grid.iter().enumerate() {
                while row + r >= parts.rows.len() {
                    parts.insert_row(parts.rows.len());
                }
                for (c, text) in line.iter().enumerate() {
                    while cell + c >= parts.columns() {
                        parts.insert_column(parts.columns());
                    }
                    // A pipe in a pasted cell would be a boundary the file did
                    // not mean; it goes in as the escape the manual promises —
                    // and a backslash is doubled with it, which is why this
                    // asks `mdtable` rather than writing the one replacement
                    // it happened to be thinking of.
                    let text = crate::mdtable::escape(text);
                    let width = parts.columns();
                    let at = &mut parts.rows[row + r];
                    at.resize(width, String::new());
                    at[cell + c] = text;
                }
            }
            self.md_reschema(&parts);
            self.md_write(&region, &parts, row, cell);
            self.status = say!("table.pasted-grid", rows, columns);
            return;
        }
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        let Some(view) = self.table.as_ref() else { return };
        let (d, width) = (view.schema.delimiter, view.schema.columns.len());
        if cell + columns > width {
            self.status = say!("table.paste-does-not-fit", width);
            return;
        }
        // The same refusal `t p` makes, for the same reason: a cell holding
        // the delimiter would come back two cells and shift every column right
        // of it. Named by where it will land, and named before anything moves.
        if let Some((r, c)) = grid
            .iter()
            .enumerate()
            .find_map(|(r, row)| row.iter().position(|t| t.contains(d)).map(|c| (r, c)))
        {
            self.status = say!("table.cell-holds-the-delimiter", line + r + 1, cell + c + 1, d);
            return;
        }
        self.snapshot();
        for (r, values) in grid.iter().enumerate() {
            let at = line + r;
            // Past the last row, the block writes new rows of its own — **at
            // the row's own place**, not at the end of the file. A file that
            // ends in a newline has an empty last line, so appending behind
            // that put the new row one line below where the block was being
            // laid down: a two-row paste into 「字,說明 / 木,樹」 came back as
            // 「甲,乙 / 丙 / ,」, the second row's cells written into the empty
            // line and the row that was meant to hold them left blank at the
            // bottom.
            let lines = self.current_buffer().line_count();
            if at >= lines || self.row_cells(at).len() != width {
                let row = self.blank_row();
                if at < lines {
                    let head = self.current_buffer().rope().line_to_char(at);
                    self.without_cell_guard(|e| {
                        e.current_buffer_mut().insert(head, &format!("{row}\n"))
                    });
                } else {
                    let end = self.current_buffer().rope().len_chars();
                    self.without_cell_guard(|e| {
                        e.current_buffer_mut().insert(end, &format!("\n{row}"))
                    });
                }
            }
            for (c, text) in values.iter().enumerate() {
                let Some((from, to)) = self.cell_span(at, cell + c) else {
                    continue;
                };
                self.without_cell_guard(|e| {
                    e.current_buffer_mut().remove(from..to);
                    e.current_buffer_mut().insert(from, text);
                });
            }
        }
        self.go_to_cell(line, cell);
        self.status = say!("table.pasted-grid", rows, columns);
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
        // A table inside a document is a few lines of it, so `j` at its last
        // row stops rather than walking out into the prose — and the rule row,
        // where there is one, is drawn rather than written, so nothing ever
        // lands on it.
        if let Some(region) = self.prose_region() {
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

    /// Page through the rows, keeping to the column.
    ///
    /// A page is as many rows as the screen shows, which is the same number of
    /// steps `move_page` takes — the difference is what a step *is*. Each one
    /// here is [`Self::move_cell_row`], so the goal column survives the whole
    /// run, a ragged row is passed over rather than landed in, and a Markdown
    /// table stops at its last row instead of paging out into the prose.
    fn move_cell_page(&mut self, count: usize, down: bool, fraction: f64) {
        let page = match self.layout {
            // Laid out vertically a row of the table is a 縱, so the page is
            // as many 縱 as fit across.
            Layout::Vertical => self.page_columns,
            Layout::Horizontal => self.page_lines,
        };
        let steps = ((page as f64 * fraction).round() as usize).max(1) * count;
        for _ in 0..steps {
            let before = self.cursor;
            self.move_cell_row(down);
            if self.cursor == before {
                break;
            }
        }
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
                Grain::Cell => say!("table.moving-by-cell"),
                Grain::Char => say!("table.moving-by-character"),
            };
            return true;
        }
        // Reading by character, this is an ordinary file that happens to be
        // drawn as a grid: `hjkl`, the operators and the selection all mean
        // what they mean everywhere else. Only Enter still knows about cells.
        if self.table.as_ref().map(|v| v.grain) == Some(Grain::Char) {
            // …except `t`. Reading by character is how you get *inside* a
            // cell, and the lesson itself asks the reader to press `Tab` and
            // then `t/` — 「找哪些字的拆分裏用了光標下這個字」. When `Enter`
            // did that job it worked in both grains; the key that replaced it
            // has to as well. `f` is still there for a find-till.
            if key == Key::Char('t') {
                self.pending = Pending::Table;
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
            // **Several rows, down the same column.** These mean "a page" for
            // everything else and reach `move_page`, which aims at a
            // *character* column — and a character column is in a different
            // cell on every row a table has, because no two rows are the same
            // width. Half a page from column 5 landed in column 10.
            //
            // `H`/`L` are back and onward here even on a 縱書 page, where the
            // rest of the editor reads `H` as onward because leftward is
            // onward down there. A table is read across whatever the file's
            // layout is — `h` is already the column to the left rather than
            // the next 縱 — so the four capitals follow the table, not the
            // page, and mean what they mean when it is laid out across.
            Key::Char('J') => self.move_cell_page(count, true, 0.5),
            Key::Char('K') => self.move_cell_page(count, false, 0.5),
            Key::Char('L') => self.move_cell_page(count, true, 1.0),
            Key::Char('H') => self.move_cell_page(count, false, 1.0),
            Key::Ctrl('d') => self.move_cell_page(count, true, 0.5),
            Key::Ctrl('u') => self.move_cell_page(count, false, 0.5),
            Key::Ctrl('f') | Key::PageDown => self.move_cell_page(count, true, 1.0),
            Key::Ctrl('b') | Key::PageUp => self.move_cell_page(count, false, 1.0),

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
                self.status = say!("table.nothing-above-the-header");
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

    /// Whether the grid's first row **names the columns** (#217).
    ///
    /// A 碼表 has no header — 「字⇥碼」 all the way down — so reading its first
    /// line as the column names loses that line to the frozen row at the top
    /// and calls one column 「一」. `t H` and `:table header off` say so: row
    /// one becomes an ordinary row and the columns are named by number, which
    /// is what the row above them already draws (#184). `None` flips it,
    /// because a file is asked this once and never again.
    fn set_table_header(&mut self, want: Option<bool>) {
        let Some(view) = self.table.as_ref() else {
            self.status = say!("table.not-in-a-table");
            return;
        };
        // **Only the grid that is a whole file has a first row that could be
        // either.** A `|` table says which its header is in the file itself —
        // the rule row under it — and a block recognised where it stands is
        // headerless already (#216), its first line being data is the whole
        // point of reading it where it lies.
        match view.bounds {
            Bounds::WholeFile => {}
            Bounds::Md => {
                self.status = say!("table.header-is-the-rule-row");
                return;
            }
            Bounds::Block => {
                self.status = say!("table.block-first-row-is-data");
                return;
            }
        }
        let want = want.unwrap_or(!view.schema.header);
        // **Names a person wrote are not undone by a keystroke.** A schema file
        // names the columns itself, so turning its first row into data changes
        // where the rows start and nothing else; it is only the fallback
        // schema — the one built *out of* row one — whose names are this row's.
        let by_a_schema = !view.from.as_os_str().is_empty();
        let delimiter = view.schema.delimiter;
        let head = self.line_text(0).unwrap_or_default();
        let columns = match by_a_schema {
            true => None,
            false => Some(match want {
                true => crate::table::Schema::from_header(&head, delimiter).columns,
                false => {
                    let wide = crate::table::cells(&head, delimiter).len().max(1);
                    crate::table::Schema::numbered(wide, delimiter).columns
                }
            }),
        };
        let Some(view) = self.table.as_mut() else {
            return;
        };
        view.schema.header = want;
        if let Some(columns) = columns {
            view.schema.columns = columns;
        }
        let wide = view.schema.columns.len();
        // 「哪一行是這個字的」 is indexed from the first **data** row, and
        // neither the buffer nor its revision has moved, so nothing else would
        // notice that the answer just changed by one.
        *self.key_index.borrow_mut() = None;
        // The header is drawn frozen at the top and refuses every edit, so a
        // cursor left standing on it is a cursor in a cell nothing can be done
        // to — the same landing `enter_table` makes.
        if want && self.cursor_line() == 0 {
            let rope = self.current_buffer().rope();
            if rope.len_lines() > 1 {
                let at = rope.line_to_char(1);
                self.set_cursor(at);
            }
        }
        self.snap_to_cell();
        self.status = match want {
            true => say!("table.header-is-row-one", wide),
            false => say!("table.header-is-data", wide),
        };
    }

    /// **The schema, in the other work area** (#218).
    ///
    /// A grid drawn from a file's own first row is a guess — which column is
    /// the key, which two of the twenty-eight are empty in all 123,380 rows,
    /// what 「一」 is actually called — and the place to correct a guess is the
    /// schema file. `t e` puts it on screen: the one that already claims this
    /// file, or, when none does, a starting one written next to the data
    /// saying exactly what the grid is doing now.
    ///
    /// **The keys stay on the table.** The schema is opened in the *other*
    /// half, the way `空格 w` reads a place without leaving the one you are
    /// standing in — `空格 w` crosses over when there is something to type.
    fn open_schema(&mut self) {
        let Some(view) = self.table.as_ref() else {
            self.status = say!("table.not-in-a-table");
            return;
        };
        // **A schema is about a file**, and neither a `|` table in a chapter
        // nor a block recognised where it stands is one: they are a table
        // *inside* a document, and a `.yumete/tables/` entry claiming the
        // chapter would claim its prose too.
        if view.bounds != Bounds::WholeFile {
            self.status = say!("table.schema-is-for-a-whole-file");
            return;
        }
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            self.status = say!("table.no-file-name-no-schema");
            return;
        };
        let from = match view.from.as_os_str().is_empty() {
            false => view.from.clone(),
            true => match self.write_starting_schema(&path) {
                Some(file) => file,
                // `write_starting_schema` has already said why.
                None => return,
            },
        };
        // The buffer to come back to, by id: opening the schema may push a new
        // buffer or show one already open, and either can move this one's
        // index.
        let table = self.current_buffer().id();
        if let Err(why) = self.open_file(&from) {
            self.status = say!("buffer.cannot-open", from.display(), why);
            return;
        }
        let name = from
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        // `show_in_split` tags the pane with whatever is current, which is why
        // the schema is opened *first* and the table taken back after.
        self.show_in_split(0, None, name.clone());
        if let Some(index) = self.buffer_with(table) {
            self.show_buffer(index);
        }
        self.status = say!("table.schema-in-the-other-area", name);
    }

    /// Write a schema next to the data that says what the grid is reading it
    /// as, and answer with where it went.
    ///
    /// **Nothing that exists is written over.** A `.toml` already sitting at
    /// that name and not claiming this file is somebody's, and the way to find
    /// out what it says is to open it — which is what happens next.
    fn write_starting_schema(&mut self, path: &Path) -> Option<PathBuf> {
        let Some(view) = self.table.as_ref() else {
            return None;
        };
        let name = path.file_name()?.to_string_lossy().into_owned();
        let stem = path.file_stem()?.to_string_lossy().into_owned();
        let tables = path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".yumete")
            .join("tables");
        let file = tables.join(format!("{stem}.toml"));
        if file.exists() {
            return Some(file);
        }
        let text = crate::table::starting_schema(&name, &view.schema, &say!("table.schema-note"));
        if let Err(why) = std::fs::create_dir_all(&tables) {
            self.status = say!("table.schema-cannot-write", file.display(), why);
            return None;
        }
        if let Err(why) = std::fs::write(&file, text) {
            self.status = say!("table.schema-cannot-write", file.display(), why);
            return None;
        }
        // The view now has a schema file of its own, so a second `t e` opens
        // this one rather than asking to write it again.
        if let Some(view) = self.table.as_mut() {
            view.from = file.clone();
        }
        Some(file)
    }

    /// Empty the cell the cursor is in, keeping its boundaries (`d`).
    fn clear_cell(&mut self) {
        let Some((line, cell)) = self.cell_position() else {
            return;
        };
        if self.md_rule_here() {
            self.status = say!("table.rule-row-is-drawn");
            return;
        }
        let Some((start, end)) = self.cell_span(line, cell) else {
            return;
        };
        if end <= start {
            self.status = say!("table.cell-is-empty");
            return;
        }
        self.snapshot();
        let text = self.current_buffer().rope().slice(start..end).to_string();
        let n = text.chars().count();
        self.store(text);
        if self.edit_remove(start..end) {
            self.set_cursor(start);
            self.format_md_table();
            self.status = say!("table.cell-cleared", n);
        }
    }

    /// Delete the row the cursor is on, in a delimited file.
    ///
    /// The guard that makes a grid safe is what made this impossible: a whole
    /// row is nothing *but* delimiters, so every ordinary way of deleting one
    /// was refused. A table editor that cannot remove a line is not one.
    fn drop_row(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        if self.on_header_row() {
            self.status = say!("table.header-cannot-be-deleted");
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
        self.status = say!("table.row-deleted");
    }

    /// Move the row the cursor is on down (or up), in a delimited file.
    fn shift_row(&mut self, down: bool) {
        if self.refuse_readonly() {
            return;
        }
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
            self.status = say!("table.no-further");
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
        self.status = match down {
            true => say!("table.row-moved-down"),
            false => say!("table.row-moved-up"),
        };
    }

    /// One key of the `t` structural menu.
    ///
    /// Directions mean what they mean in a grid: `j`/`k` are the row, `h`/`l`
    /// are the column, and which one an edit is about never has to be said
    /// twice. The rest is vi's own spelling — `o`/`O` open, `d` deletes.
    fn table_structure(&mut self, key: Key) {
        use crate::mdtable::Align;
        // **The three that work whether or not there is a table here.** `t` is
        // one group in every mode now, so the first thing it has to answer is
        // 「get me into a table」 — from prose, from another table, from the top
        // of a document whose tables are three screens down.
        match key {
            // **The three ways to look at a table** (#275). The author's
            // model, 2026-09-05: 「有三种模式，一种是 prose/source 模式，表格
            // 当作普通文本。第二个是表格操作模式…用 ti 进入。第三个是真表格
            // 显示模式，也就是完全画成表格…用 tt 进入。」
            //
            // `t t` draws it. `t i` leaves the pipes and the commas on the
            // page and gives the keys to the grid. `t q` is the one way back
            // to prose — and the two table modes switch **straight into one
            // another**, so `t i` inside `t t` is the middle mode and not the
            // way out.
            Key::Char('t') | Key::Char('i') => {
                let want = match key {
                    Key::Char('t') => Surface::Page,
                    _ => Surface::InProse,
                };
                // **A file-wide mode is switched from anywhere in the file**
                // — 「可以在文件任何位置通過 ti tt 進入表格視圖」 — so this is
                // not `table_here()`, which is false in the paragraph between
                // two tables and is the right answer for the *keys*.
                if self.table_here() || self.table.as_ref().is_some_and(|v| v.is_file_wide()) {
                    self.show_table_as(want);
                    return;
                }
                if !self.enter_table_as(want) {
                    // `enter_table_as` has already said why.
                    return;
                }
                self.status = match want {
                    Surface::Page => say!("table.drawn"),
                    Surface::InProse => say!("table.operated"),
                };
                return;
            }
            Key::Char('q') => {
                if self.table.is_none() {
                    self.status = say!("table.already-off");
                    return;
                }
                self.leave_table();
                return;
            }
            // `t ]` / `t [` — the next table in the file, and into it. A
            // document's tables are the other thing worth walking between,
            // and the brackets are where Helix keeps 「the next one of these」.
            Key::Char(']') => return self.go_to_table(true),
            Key::Char('[') => return self.go_to_table(false),
            _ => {}
        }
        // **誰用了它**, down the columns rather than across the lines — the
        // other axis of the same verb `g/` is in prose, and the same pair of
        // letters: `/` answers here, `?` answers in the other work area. It
        // used to be `Enter`, which a writer presses by accident.
        //
        // Which columns: the sequence's own argument — `t1/` is the first, and
        // `t2-10?` is the second through the tenth — or, with no argument, the
        // ones a schema's `[table.link] from` names.
        // **`t20-20g` goes to a cell**: row 20, column 20. `t20g` is row 20 in
        // the column you are standing in — the row number is the one a reader
        // has in front of them, from the gutter, and the column number is the
        // one drawn above the header.
        if key == Key::Char('g') {
            if let Some((row, column)) = self.sequence_span() {
                let had = self.sequence.and_then(|(_, to)| to).is_some();
                let cell = match had {
                    true => column.saturating_sub(1),
                    false => self.cell_position().map(|(_, c)| c).unwrap_or(0),
                };
                let lines = self.current_buffer().line_count();
                let line = row.clamp(1, lines).saturating_sub(1);
                self.remember_jump();
                self.goto_line(line + 1);
                self.go_to_cell(line, cell);
                self.status = say!("table.row-and-column", line + 1, cell + 1);
                return;
            }
        }
        // **A block recognised where it stands is read, not rewritten** (#216).
        // It is somebody's 碼表 sitting in a chapter, and every key below this
        // point is written against a file that is nothing *but* the table: the
        // sort rebuilds the file from its own lines, `t n` and `t D` are
        // Markdown's column surgery, and `t o` would put a blank line through
        // the middle of the block and end it there. What is left is what makes
        // sense on a block — walking it, searching down its columns, and
        // taking or writing one column of it, which `cell_lines` bounds.
        if self.block_region().is_some() {
            match key {
                Key::Char('/') | Key::Char('?') => {
                    self.definition_preview = key == Key::Char('?');
                    let span = self.sequence_span();
                    self.search_columns_in(span);
                }
                Key::Char('y') => self.yank_column(),
                Key::Char('p') => self.put_column(),
                Key::Esc => {}
                _ => self.status = say!("hint.table.block-keys"),
            }
            return;
        }
        // `t1s` / `t1S` — sort by a column named by number. `t s` with no
        // number is the column you are standing in, which is what it has always
        // been.
        if matches!(key, Key::Char('s') | Key::Char('S')) {
            let down = key == Key::Char('S');
            // Every column `a`/`d` closed, and then the one still being typed:
            // `t1a2d8as` ends on a bare `s`, `t1s` is a single column with its
            // direction in the verb, and `t1a2ds` is both spellings at once.
            let mut named = self.sort_keys.clone();
            if let Some((column, _)) = self.sequence_span() {
                named.push((column, down));
            }
            if !named.is_empty() {
                self.sort_table(&named);
                return;
            }
            // No number: the column the cursor is standing in. `S` means
            // *down* here too — a delimited file used to sort up either way,
            // silently, which is the worst way to disagree with a keystroke.
            if self.md_region().is_none() {
                let here = self.cell_position().map(|(_, c)| c).unwrap_or(0);
                self.sort_table(&[(here + 1, down)]);
                return;
            }
        }
        if matches!(key, Key::Char('/') | Key::Char('?')) {
            self.definition_preview = key == Key::Char('?');
            let span = self.sequence_span();
            self.search_columns_in(span);
            return;
        }
        // A delimited file's columns are its schema's, and 123,380 rows do not
        // want one inserted by a keystroke — so only the row half applies.
        if self.md_region().is_none() {
            match key {
                Key::Char('y') => self.yank_column(),
                Key::Char('p') => self.put_column(),
                // **The detail panel is a table key** (#215). It answered to
                // `空格 d` for as long as the panel was the only thing that
                // knew a row's twenty-eight fields — but `空格` is the
                // document's menu and `t` is the table's, and a key that only
                // ever does anything inside a table belongs in `t`.
                //
                // `t I`, not `t i`: `t i` is 表格操作模式 since #275, and the
                // capital is the nearest free key to the one this lost — the
                // hand that knew `t i` finds it by pressing harder.
                Key::Char('I') => self.toggle_detail(),
                // Each of these is an edit, and each announces an undo point
                // of its own: without one they were folded into whatever came
                // before, so a single `u` took back the cell you had just
                // finished as well as the row you had just opened.
                Key::Char('o') => {
                    self.snapshot();
                    self.open_line_below();
                }
                Key::Char('O') if self.on_header_row() => {
                    self.snapshot();
                    self.open_line_below();
                    self.status = say!("table.nothing-above-the-header");
                }
                Key::Char('O') => {
                    self.snapshot();
                    self.open_line_above();
                }
                Key::Char('d') => self.drop_row(),
                Key::Char('j') | Key::Down => self.shift_row(true),
                Key::Char('k') | Key::Up => self.shift_row(false),
                // **「第一行是欄名還是資料」** (#217) — the one question a
                // 碼表 asks once, and `h` is the column, so the header is `H`.
                Key::Char('H') => self.set_table_header(None),
                // **`e` 是規格** (#218) — the file that says what these columns
                // are, opened in the other half rather than described on the
                // status line.
                Key::Char('e') => self.open_schema(),
                Key::Esc => {}
                _ => self.status = say!("hint.table.csv-keys"),
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
            Key::Char('y') => self.yank_column(),
            Key::Char('p') => self.put_column(),
            // `t I` — see the note on the delimited file's copy of this key.
            Key::Char('I') => self.toggle_detail(),
            Key::Char('s') => self.md_sort(false),
            Key::Char('S') => self.md_sort(true),
            Key::Char('<') => self.md_align(Align::Left),
            Key::Char('=') => self.md_align(Align::Center),
            Key::Char('>') => self.md_align(Align::Right),
            Key::Char('f') => {
                self.snapshot();
                self.status = if self.format_md_table() {
                    say!("table.lined-up")
                } else {
                    say!("table.already-aligned")
                };
            }
            Key::Esc => {}
            _ => self.status = say!("hint.table.after-t"),
        }
    }

    /// Go to the next `|` table in the file, or the previous one, and read it.
    ///
    /// **A document is mostly not a table**, so the way into one has to be a
    /// key rather than a scroll: 手冊 has forty of them, and 「the one after
    /// this」 is how a writer moves between them.
    fn go_to_table(&mut self, forward: bool) {
        let rope = self.current_buffer().rope();
        let here = self.cursor_line();
        let last = motion::last_line(rope);
        let is_table = |line: usize| -> bool {
            let text = rope.line(line).to_string();
            let trimmed = text.trim_start();
            trimmed.starts_with('|') && trimmed.trim_end().ends_with('|')
        };
        // Out of the table the cursor is in first, or 「next」 lands on the row
        // below and calls it a table.
        let mut line = here;
        let step = |line: usize| match forward {
            true => (line < last).then(|| line + 1),
            false => line.checked_sub(1),
        };
        while is_table(line) {
            match step(line) {
                Some(next) => line = next,
                None => break,
            }
        }
        while !is_table(line) {
            match step(line) {
                Some(next) => line = next,
                None => {
                    self.status = match forward {
                        true => say!("table.no-next"),
                        false => say!("table.no-previous"),
                    };
                    return;
                }
            }
        }
        // Walk to its first row, whichever direction we arrived from.
        while line > 0 && is_table(line - 1) {
            line -= 1;
        }
        self.remember_jump();
        self.goto_line(line + 1);
        if self.table.is_none() {
            self.enter_table();
        }
        self.status = say!("table.jumped-to-line", line + 1);
    }

    /// Enter a cell to type in it.
    fn edit_cell(&mut self, how: CellEdit) {
        if self.md_rule_here() {
            self.status = say!("table.rule-row-is-drawn");
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
        let want = view.schema.columns.len();
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
            ("C-Space", say!("help.chinese.toggle-ime")),
            (":yume scheme", say!("help.chinese.switch-scheme")),
            (":yume chaifen on", say!("help.chinese.chaifen-under-candidates")),
            ("w b e", say!("help.chinese.word-boundaries")),
            (":segment on", say!("help.chinese.word-tint")),
            (":words", say!("help.chinese.reload-project-words")),
            (":ruby", say!("help.chinese.annotate-reading")),
            (":ruby format html", say!("help.chinese.unify-reading-spelling")),
            (":render full", say!("render.wysiwyg")),
            (":indent 2", say!("help.chinese.first-line-indent")),
            (":hanging on", say!("help.chinese.hung-punctuation")),
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
            (":wrap 24", say!("help.vertical.column-length")),
            (":bands 2", say!("help.vertical.bands")),
            (":hanging on", say!("help.vertical.hung-punctuation")),
            (":dense off", say!("help.vertical.loose")),
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
            ("t o t d", say!("help.table.add-or-drop-row")),
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
        self.without_cell_guard(|e| {
            e.current_buffer_mut().remove(0..len);
            e.current_buffer_mut().insert(0, text);
        });
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

    /// The running typesetter's address, for the status bar and for `:preview`.
    pub fn preview_at(&self) -> Option<&str> {
        self.preview_at.as_deref()
    }

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
            Pending::Goto => (say!("hint.goto.title"), vec![
                    ("g", say!("hint.goto.start-of-file")),
                    ("e", say!("hint.goto.end-of-file")),
                    ("h l", say!("hint.goto.line-start-or-end")),
                    ("s", say!("hint.goto.first-non-blank")),
                    ("f", say!("hint.goto.open-this-file")),
                    ("d w", say!("hint.goto.follow-note")),
                    ("/ ?", say!("hint.goto.word-elsewhere")),
                    ("J", say!("hint.join-with-line-below")),
                ]),
            Pending::Find(_) => (say!("hint.find"), vec![("", say!("hint.type-a-character"))]),
            Pending::Replace => (say!("hint.overwrite"), vec![("", say!("hint.type-a-character-to-overwrite"))]),
            Pending::Register => (say!("hint.register.title"), vec![("a–z", say!("hint.register.which-one"))]),
            Pending::Match => (say!("hint.match.title"), vec![
                    ("m", say!("hint.match.pair")),
                    ("i", say!("hint.match.inside")),
                    ("a", say!("hint.match.around")),
                    ("s", say!("hint.match.surround")),
                    ("d", say!("hint.match.take-off")),
                    ("r", say!("hint.change")),
                ]),
            Pending::MatchPair { .. } => (say!("hint.bracket"), vec![("", say!("hint.type-a-bracket-or-quote"))]),
            Pending::Surround => (say!("hint.match.surround"), vec![("", say!("hint.type-a-bracket"))]),
            Pending::SurroundFrom => (say!("hint.match.take-off"), vec![("", say!("hint.type-the-one-to-take-off"))]),
            Pending::SurroundTo(_) => (say!("hint.change-to"), vec![("", say!("hint.type-the-one-to-change-to"))]),
            Pending::Mark => (say!("hint.mark.set-here"), vec![("a–z", say!("hint.mark.name-it"))]),
            Pending::Recall => (say!("hint.mark.go-back"), vec![("a–z", say!("hint.register.which-one"))]),
            // **What this table can actually do**, not what tables can do.
            // A delimited file's columns are its schema's — `n`/`D`/`h`/`l`
            // are not offered there because they are refused there — and the
            // menu listing them was the one place the editor said a key
            // existed and then said it did not.
            // A block is read where it lies (#216), so the keys that rewrite a
            // file are not offered here — because they are refused here.
            Pending::Table => {
                // **The way in comes first.** `t` is a group in every mode
                // (#206), so most of the time it is pressed by somebody who is
                // *not* in a table yet — and the menu used to open with 「照這
                // 欄順排」 and never once mention `t t`. These four work
                // wherever the cursor is, so they head every list, and on a
                // page with no table under the cursor they are the whole list.
                let mut keys = vec![
                    ("t", say!("hint.table.draw-it")),
                    ("i", say!("hint.table.operate-it")),
                    ("q", say!("hint.table.back-to-prose")),
                    ("] [", say!("hint.table.next-or-previous")),
                ];
                // **Which list is a question about the cursor, not the mode.**
                // It used to be `md_region().is_none()`, which is *also* true
                // of a Markdown table nobody has opened yet — so standing in
                // one of 手冊's own tables offered the delimited file's keys.
                match self.table.as_ref().map(|v| v.bounds) {
                    Some(Bounds::Block) if self.block_region().is_some() => keys.extend([
                        ("/ ?", say!("hint.table.search-columns")),
                        ("g", say!("hint.table.go-to-cell")),
                        ("y p", say!("hint.table.yank-or-paste-column")),
                    ]),
                    Some(Bounds::Md) if self.md_region().is_some() => keys.extend([
                        ("/ ?", say!("hint.table.search-columns")),
                        ("g", say!("hint.table.go-to-cell")),
                        ("o O", say!("hint.table.add-row")),
                        ("n N", say!("hint.table.add-column")),
                        ("d D", say!("hint.table.delete-row-or-column")),
                        ("j k", say!("hint.table.move-row")),
                        ("h l", say!("hint.table.move-column")),
                        ("y p", say!("hint.table.yank-or-paste-column")),
                        ("s S", say!("hint.table.sort-by-column")),
                        ("< = >", say!("hint.table.align-column")),
                        // `f`, not `t` — `t` has been the way *into* a table
                        // since 1ffde52 and this row went on saying otherwise,
                        // which is how 「tf 沒有這個選項」 gets reported.
                        ("f", say!("hint.table.line-it-up")),
                        ("I", say!("hint.table.detail-panel")),
                    ]),
                    Some(Bounds::WholeFile) => keys.extend([
                        ("/ ?", say!("hint.table.search-columns")),
                        ("g", say!("hint.table.go-to-cell")),
                        ("s S", say!("hint.table.sort-by-column")),
                        ("o O", say!("hint.table.add-row")),
                        ("d", say!("hint.table.delete-row")),
                        ("j k", say!("hint.table.move-row")),
                        ("y p", say!("hint.table.yank-or-paste-column")),
                        ("H", say!("hint.table.first-row-is-data")),
                        ("e", say!("hint.table.schema")),
                        ("I", say!("hint.table.detail-panel")),
                    ]),
                    _ => {}
                }
                (say!("hint.table.title"), keys)
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
            self.without_cell_guard(|e| {
                e.current_buffer_mut().insert(at, &format!("\n{body}"));
            });
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
    fn search_columns_in(&mut self, span: Option<(usize, usize)>) {
        let needle = self.what_is_here();
        if needle.trim().is_empty() {
            self.status = say!("table.cell-is-empty");
            return;
        }
        // The text, not a pattern — the same rule a search of the selection
        // follows.
        self.search_columns_within(&regex::escape(&needle), span);
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
    /// Search **down one column, then the next** (`:search column`, `Enter`).
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
    fn search_columns_within(&mut self, pattern: &str, span: Option<(usize, usize)>) {
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
        let view = self.table.as_ref().expect("table_here");
        let declared: Option<Vec<usize>> = view.schema.link.as_ref().map(|link| {
            link.from
                .iter()
                .filter_map(|name| view.schema.index_of(name))
                .collect()
        });
        let total = view.schema.columns.len();
        let columns: Vec<usize> = match span {
            // Said outright: 1-based, as the reader counts them.
            Some((a, b)) => {
                let (a, b) = (a.min(b).max(1), a.max(b).max(1));
                (a.min(total)..=b.min(total)).map(|n| n - 1).collect()
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
        match (declared.is_none(), span) {
            (true, None) => {
                self.status = say!(
                    "search.no-jump-scope",
                    found
                );
            }
            (_, Some((a, b))) if a != b => {
                self.status = say!("search.hit-in-column-range", a.min(b), a.max(b), found);
            }
            (_, Some((a, _))) => self.status = say!("search.hit-in-column", a, found),
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
            Some((line, cell)) => self.cell_span(line, cell).map(|(a, _)| a) == Some(self.cursor),
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
        match &self.table {
            Some(view) => match view.separator {
                Separator::Pipe => crate::mdtable::blank_row(view.schema.columns.len()),
                Separator::Delimiter(d) => {
                    d.to_string().repeat(view.schema.columns.len().saturating_sub(1))
                }
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
        // A Markdown table is a page of a document: the question the panel
        // answers there is the document's question — what is this footnote,
        // what does this comment say — not "what are this row's twenty-eight
        // fields", which a two-column table does not have.
        match self.table.as_ref().is_some_and(|v| v.bounds == Bounds::WholeFile) {
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
        let columns = view.schema.columns.len().max(1);
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
        let named: Vec<String> = (first..=last)
            .filter_map(|c| view.schema.columns.get(c).map(|col| col.name.clone()))
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
        let within = self.cursor - rope.line_to_char(line);
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
        self.current_buffer_mut().insert(at, note);
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
        // **Numbered the same way the rows are**, or the panel can never find
        // the field the cursor is in: it compared 「unicode」 against 「 9
        // unicode」, never matched, and so never scrolled to it and never lit
        // it — both of the things it promises.
        let here_name = view
            .schema
            .columns
            .get(cell)
            .map(|c| format!("{:>2} {}", cell + 1, c.heading()))
            .unwrap_or_default();
        let mut rows: Vec<(String, Option<String>)> = view
            .schema
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
                // the third, `t20-20g` goes to a cell by number, and the panel
                // is where a reader finds out which number a field is without
                // counting along the header.
                (format!("{:>2} {}", i + 1, column.heading()), text)
            })
            // **Every column, empty ones included** — an empty field *is* a
            // finding in a 拆分表, and a panel that leaves it out is a panel
            // that cannot answer 「這一格是不是空的」. They were hidden because
            // twenty-three blanks pushed the 部件 list off the bottom; the
            // panel scrolls to the field the cursor is in, so there is
            // somewhere for them to go — and `t20-20g` reaches any of them by
            // number, which is what the numbers are for.
            .collect();
        // Worked out, not stored — and marked as such, so nobody goes looking
        // for a column that is not in the file.
        for detail in &view.schema.details {
            let from = value(detail.compute.column());
            rows.push((
                format!("{}*", detail.name),
                Some(detail.compute.apply(&from, &view.schema.ranges)),
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
            self.status = say!("table.not-in-a-table");
            return;
        };
        let schema = view.schema.clone();
        let separator = view.separator;
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name().to_string());
        let key_at = schema.key.as_deref().and_then(|k| schema.index_of(k));
        let jump_from: Vec<usize> = schema
            .link
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
            let spans = match separator {
                Separator::Pipe => crate::mdtable::cells(&text),
                Separator::Delimiter(d) => crate::table::cells(&text, d),
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
                found.push(say!(
                    "chaifen.wrong-column-count",
                    name,
                    line + 1,
                    spans.len(),
                    want
                ));
            }
            if let Some(at) = key_at {
                let k = cell(at);
                if !k.is_empty() {
                    if let Some(&was) = seen.get(&k) {
                        found.push(say!(
                            "chaifen.duplicate-row-name",
                            name,
                            line + 1,
                            k,
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
            let spans = match separator {
                Separator::Pipe => crate::mdtable::cells(&text),
                Separator::Delimiter(d) => crate::table::cells(&text, d),
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
                found.push(say!("chaifen.component-not-found", name, line + 1, list));
            }
            if found.len() >= GREP_LIMIT {
                break;
            }
        }
        if found.is_empty() {
            self.status = say!("table.check-clean", name, last + 1 - first);
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
        buffer.name_as(&say!("chaifen.lint-results", name));
        self.grep_root = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        self.add_buffer(buffer);
        self.set_cursor(0);
        self.status = say!("table.check-problems", n);
    }

    /// The reader's own 用字 groups, from the config (#233).
    pub fn set_usage_groups(&mut self, groups: Vec<String>) {
        self.usage_groups = groups;
    }

    /// `:check usage` — where the manuscript wrote the other spelling (#233).
    ///
    /// **Not spelling, consistency.** 裏 four hundred times and 裡 three is not
    /// three mistakes — both are correct 漢字 — it is one manuscript that has
    /// not settled, and nothing tells the writer: Word checks 病句, Grammarly
    /// is English, and every spell-checker there is tokenizes on spaces and
    /// sees a chapter as one word. So the question is asked of the document
    /// rather than of a dictionary: a group is reported only when both
    /// spellings are written here, and the one written more is the one it
    /// meant.
    ///
    /// The answer is a jumpable listing, the shape `:grep` and `:table check`
    /// already use — three hundred slips are not a status line, and `gf` on a
    /// row is how a reader goes and fixes one.
    fn check_usage(&mut self) {
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.current_buffer().display_name().to_string());
        let text = self.current_buffer().rope().to_string();
        let slips = crate::usage::check(&text, &self.usage_groups);
        if slips.is_empty() {
            self.status = say!("check.usage-clean", name);
            return;
        }
        let n = slips.len();
        let mut listing = String::new();
        for slip in slips.iter().take(GREP_LIMIT) {
            listing.push_str(&say!(
                "check.usage-slip",
                name,
                slip.line + 1,
                slip.written,
                slip.instead,
                slip.written_count,
                slip.instead_count
            ));
            listing.push('\n');
        }
        let mut buffer = Buffer::from_text(&listing);
        buffer.name_as(&say!("check.usage-results", name));
        self.grep_root = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        self.add_buffer(buffer);
        self.set_cursor(0);
        self.status = say!("check.usage-found", n);
    }

    /// Go to the row this table names by `key` (`:row 木`).
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
        // grid sits. 表格操作 (`t i`) leaves the pipes on the page and does not
        // ask for the turn, so it is not this case; `t i` and `t q` are what
        // give the manuscript back, and both restore the layout the grid took.
        if layout == Layout::Vertical && self.table.as_ref().is_some_and(|v| v.is_page()) {
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
        ghost: &'a dyn Fn(usize) -> Vec<(usize, String)>,
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
            .with_ghost(ghost)
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
        if self.indent == 0 {
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
        match self.indent > 0 {
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
    /// **Not** masked by `:dense`, unlike the readings, the hung 句讀 and the
    /// ticks. Those three each cost a *column* — the width `:dense` exists to
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
        self.status = match self.indent {
            0 => say!("layout.first-line-indent-off"),
            n => say!("layout.first-line-indent", n),
        };
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
        // …and only where 密排 costs anything. It packs the *縱書* page: the
        // reading column is a column off every 縱's width. A horizontal page
        // pays no width for a reading — the row above is only taken where
        // there is one — so there is nothing for packing to win there, and
        // masking it would mean 橫排 could never show a reading at all, since
        // 密排 is the default page.
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
        } else {
            self.ruby.remove(dialect);
        }
    }

    /// Where the cursor sits in the 縱 grid (for the status line).
    pub fn zong_position(&self) -> zong::Position {
        let hidden = |line: usize| self.markup_hidden_on_line(line);
        let folded = |line: usize| self.line_is_folded(line);
        let ghost = |line: usize| self.ghost_on_line(line);
        let grid = self.grid_with(&hidden, &folded, &ghost);
        zong::position(self.current_buffer().rope(), self.cursor, grid)
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
        let buffer = self.current_buffer_mut();
        buffer.remove(0..len);
        buffer.insert(0, &draft);
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
        // 橫排 has no columns to pack, so `:dense` means the other axis there:
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
            let (from, to) = self.sequence.get_or_insert((0, None));
            let at = match to {
                Some(n) => n,
                None => from,
            };
            *at = at.saturating_mul(10).saturating_add(digit as usize).min(1_000_000);
            return true;
        }
        // `2-5`: the far end of a span. Only after a number, so `-` is free.
        if c == '-' {
            if let Some((_, to @ None)) = self.sequence.as_mut() {
                *to = Some(0);
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
        let Some((column, None)) = self.sequence else {
            return false;
        };
        self.sort_keys.push((column, c == 'd'));
        self.sequence = None;
        true
    }

    /// The sequence's argument as a span, if it was given one.
    fn sequence_span(&self) -> Option<(usize, usize)> {
        let (from, to) = self.sequence?;
        Some((from, to.unwrap_or(from)))
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
            Pending::Find(FindKind::ForwardTo) => "f",
            Pending::Find(FindKind::BackwardTo) => "F",
            Pending::Replace => "r",
            Pending::Register => "\"",
            Pending::Match => "m",
            Pending::MatchPair { around: false } => "mi",
            Pending::MatchPair { around: true } => "ma",
            Pending::Surround => "ms",
            Pending::SurroundFrom | Pending::SurroundTo(_) => "mr",
            Pending::Table => "t",
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
            match self.sequence {
                Some((from, to)) => {
                    out.push_str(&from.to_string());
                    if let Some(n) = to {
                        out.push('-');
                        if n > 0 {
                            out.push_str(&n.to_string());
                        }
                    }
                }
                // A count typed the other way round is still part of what was
                // typed: `3gd` says `3g` here, not `g`.
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

    /// Install the word [`Segmenter`] used by `w`/`b`/`e` and the segmentation
    /// overlay. A [`yumete_cjk::DictionarySegmenter`] groups CJK characters into
    /// words; the default [`yumete_cjk::CategorySegmenter`] treats each as one.
    pub fn set_segmenter(&mut self, segmenter: Box<dyn Segmenter>) {
        self.segment_cache.borrow_mut().clear();
        // The project's own words go on top of whatever was chosen, so the
        // book's names survive a change of dictionary.
        self.segmenter = Box::new(yumete_cjk::WithWords::new(
            segmenter,
            std::rc::Rc::clone(&self.project_words),
        ));
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
        self.segment_cache.borrow_mut().clear();
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
        self.segment_cache.borrow_mut().clear();
        self.status = match where_from {
            Some(path) => say!("word.project-words-loaded", n, path.display()),
            None => say!("word.no-project-words-file"),
        };
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
        outcome
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
                // 命令＋選擇＋動作: `t20-20g` is 「table · row 20, column 20 ·
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
                // `t20-20g` names a cell and `t1a2d8as` names three columns to
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
                    Key::Char('f') => FindKind::ForwardTo,
                    _ => FindKind::BackwardTo,
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
            // Case, and replacing the selection with the register. Joining is
            // on `gJ`: `J` turns the page, which a reader presses a hundred
            // times for every once they join two lines.
            Key::Char('~') => self.map_selection(switch_case),
            Key::Char('`') => self.map_selection(|c| c.to_lowercase().next().unwrap_or(c)),
            Key::Alt('`') => self.map_selection(|c| c.to_uppercase().next().unwrap_or(c)),
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
            '`' => say!("hint.vi.backtick"),
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
        ('w', "hint.goto.other-pane"),
        ('W', "hint.goto.only-this-pane"),
        ('q', "hint.goto.close-this-pane"),
        ('"', "menu.paste.title"),
    ];

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
    /// For the callers that have already noted one — a mark, `:row` — where a
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

        let forward = kind == FindKind::ForwardTo;
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
            FindKind::ForwardTo | FindKind::BackwardTo => line_start + idx,
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
            Key::Left => self.move_horizontal(motion::left),
            Key::Right => self.move_horizontal(motion::right),
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
            self.without_cell_guard(|e| {
                e.current_buffer_mut().remove(0..len);
                e.current_buffer_mut().insert(0, &rebuilt);
            });
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
    /// space and joining two 漢字 with one would insert text the author never
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
        let buffer = self.current_buffer_mut();
        buffer.remove(0..len);
        buffer.insert(0, &formatted);
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
        let buffer = self.current_buffer_mut();
        buffer.remove(span.0..span.1);
        buffer.insert(span.0, &text);
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
        if self.refuse_readonly() {
            return;
        }
        let Some((start, end)) = self.innermost_pair() else {
            self.status = say!("edit.no-pair-to-delete");
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
            let ghost = |line: usize| self.ghost_on_line(line);
            let typed = |line: usize| self.typed_ghost_on_line(line);
            // A table row is one row (#275) — and `j` has to be walking the
            // same page the renderer drew, or it steps into a row that is not
            // on the screen.
            let flat = |line: usize| self.table_row_at(line);
            let m = crate::wrap::Measure::new(width, &hide)
                .with_indent(self.paragraph_indent())
                .with_folds(&fold)
                .with_ghost(&ghost)
                .with_typed_ghost(&typed)
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
            let ghost = |line: usize| self.ghost_on_line(line);
            let typed = |line: usize| self.typed_ghost_on_line(line);
            // A table row is one row (#275) — and `j` has to be walking the
            // same page the renderer drew, or it steps into a row that is not
            // on the screen.
            let flat = |line: usize| self.table_row_at(line);
            let m = crate::wrap::Measure::new(width, &hide)
                .with_indent(self.paragraph_indent())
                .with_folds(&fold)
                .with_ghost(&ghost)
                .with_typed_ghost(&typed)
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
            let ghost = |line: usize| self.ghost_on_line(line);
            let grid = self.grid_with(&hidden, &folded, &ghost);
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
        // **A table is a table whether or not `:table` was typed.** This check
        // used to open with `self.table.as_ref()?`, so `:replace` — which
        // reaches every file `:grep` found, including files never opened — went
        // through 13 rows of the author's own documentation and broke them.
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
        if self.refuse_readonly() {
            return false;
        }
        if let Some(why) = self.cell_refuses_cut(range.clone()) {
            self.status = why;
            return false;
        }
        self.current_buffer_mut().remove(range);
        true
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
        self.without_cell_guard(|e| e.current_buffer_mut().insert(end, &format!("\n{row}")));
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
                self.status = say!("table.divider-cannot-be-deleted");
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

    /// Feature #210. The core holds the runs and hands them to whatever asks
    /// where a character is; it does not decide what they say.
    #[test]
    fn ghost_runs_are_held_wholesale_and_answered_by_line() {
        let mut ed = typed("春夏\n秋冬\n");
        assert!(!ed.has_candidate(), "a page with no candidate on it pays nothing");
        assert!(ed.ghost_on_line(0).is_empty());

        // Out of order on the way in, in column order on the way out: the
        // renderer, the wrap and the mouse all walk it forwards.
        ed.set_ghost(vec![
            (0, 2, "補".to_string()),
            (1, 1, "候".to_string()),
            (0, 1, "候".to_string()),
        ]);
        assert!(ed.has_candidate());
        assert_eq!(
            ed.ghost_on_line(0),
            vec![(1, "候".to_string()), (2, "補".to_string())]
        );
        assert_eq!(ed.ghost_on_line(1), vec![(1, "候".to_string())]);
        assert!(ed.ghost_on_line(2).is_empty());

        // Wholesale, never appended — a committed candidate leaves nothing.
        ed.set_ghost(Vec::new());
        assert!(!ed.has_candidate());
        assert!(ed.ghost_on_line(0).is_empty());
    }

    /// The point of #210: **one page**. A candidate the renderer alone knew
    /// about would put the caret, `j` and the mouse on three different ones.
    #[test]
    fn the_caret_and_the_grid_agree_about_a_candidate() {
        let mut ed = typed("春夏秋冬\n");
        ed.set_ghost(vec![(0, 2, "候補".to_string())]);
        // Down the column: two rows of candidate between 夏 and 秋.
        ed.execute(":layout vertical").unwrap();
        assert_eq!(ed.zong_position().slot, 0);
        // Down the column is `j`: the keys follow the screen, not the file.
        ed.on_key(Key::Char('j'));
        ed.on_key(Key::Char('j'));
        assert_eq!(ed.cursor(), 2, "two 字 along");
        assert_eq!(
            ed.zong_position().slot,
            4,
            "…and four rows down, the candidate being two of them"
        );
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
    fn the_retired_keys_say_what_replaced_them() {
        // `Enter` and `*` did something here until they were retired, so the
        // reader pressing one is coming from *last week*, not from vi. The
        // phrasebook exists for exactly that reader.
        let mut ed = typed("那年冬天。\n");
        ed.on_key(Key::Enter);
        assert!(ed.status().contains("g/"), "{}", ed.status());
        let mut ed = typed("那年冬天。\n");
        ed.on_key(Key::Char('*'));
        assert!(ed.status().contains("g/"), "{}", ed.status());
    }

    #[test]
    fn a_column_can_be_named_either_way_round() {
        // `g3d` and `3gd` are the same question — 「in column three」 — and the
        // comment beside the code has said so all along, but the vi-order
        // spelling put its number in the count and nothing ever read it.
        let table = "| 字 | 甲 | 乙 |\n| -- | -- | -- |\n| 木 | 目 | 相 |\n| 甲 | 乙 | 木 |\n| 相 | 木 | 目 |\n";
        let start = |keys: &str| {
            let mut ed = typed(table);
            ed.goto_line(3);
            assert!(ed.enter_table(), "{}", ed.status());
            press(&mut ed, keys);
            ed.cursor_line()
        };
        assert_eq!(start("g3d"), 3, "column three holds 木 on the fourth line");
        assert_eq!(start("3gd"), 3, "and vi's order says the same thing");
        assert_eq!(start("gd"), 2, "no number is the key column, which is here");
    }

    #[test]
    fn a_capital_s_sorts_a_delimited_file_downwards() {
        // `t S` sorted *up* on a delimited file: the branch that handles a
        // bare `s`/`S` never looked at which of the two had been pressed, so
        // the editor did the opposite of the key and said nothing.
        let dir = std::env::temp_dir().join(format!("yumete-sortS-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let sorted = |name: &str, keys: &str| {
            let csv = dir.join(name);
            std::fs::write(&csv, "字,序\n甲,1\n丙,3\n乙,2\n").unwrap();
            let mut ed = Editor::new();
            ed.open_file(&csv).unwrap();
            assert!(ed.enter_table(), "{}", ed.status());
            press(&mut ed, keys);
            ed.current_buffer().text()
        };
        // By code point, which is what the sort promises for anything that is
        // not a number: 丙 U+4E19, 乙 U+4E59, 甲 U+7532.
        assert_eq!(sorted("up.csv", "ts"), "字,序\n丙,3\n乙,2\n甲,1\n");
        assert_eq!(sorted("down.csv", "tS"), "字,序\n甲,1\n乙,2\n丙,3\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn t_still_works_when_the_grid_is_read_by_character() {
        // `Tab` reads the grid by character — which is how you get *inside* a
        // cell, and exactly where the lesson tells the reader to press `t/`.
        // When `Enter` did that job it worked in both grains; `t` did not.
        let mut ed = typed("| 字 | 拆分 |\n| -- | -- |\n| 木 | 木 |\n| 相 | 木目 |\n");
        ed.goto_line(3);
        assert!(ed.enter_table(), "{}", ed.status());
        ed.on_key(Key::Tab);
        ed.on_key(Key::Char('t'));
        assert!(ed.pending_menu().is_some(), "t opened nothing in the char grain");
    }

    #[test]
    fn one_g_goes_to_the_first_line() {
        // `1G` went to the *last* line: the count had already been taken by the
        // time `G` asked whether there was one.
        let mut ed = typed("一\n二\n三\n四\n");
        press(&mut ed, "1G");
        assert_eq!(ed.cursor_line(), 0, "1G is the first line");
        press(&mut ed, "3G");
        assert_eq!(ed.cursor_line(), 2);
        press(&mut ed, "G");
        assert_eq!(ed.cursor_line(), 3, "a bare G is still the last line");
    }

    #[test]
    fn a_sentence_motion_stops_before_the_next_sentence() {
        // `)d` used to delete this sentence *and the first character of the
        // next one*, because the motion lands on the next sentence's start and
        // the selection ran through it. `w` has always stepped back one
        // character for exactly this reason; these had not.
        let mut ed = typed("第一句。第二句。第三句。\n");
        press(&mut ed, ")d");
        assert_eq!(ed.current_buffer().text(), "第二句。第三句。\n");
        // The same for a paragraph.
        let mut ed = typed("第一段。\n第二段。\n");
        press(&mut ed, "}d");
        assert_eq!(ed.current_buffer().text(), "第二段。\n");

        // …and standing **on** the 。 — the next sentence exactly one
        // grapheme away, which is the case the first fix still got wrong.
        let mut ed = typed("第一句。第二句。第三句。\n");
        press(&mut ed, "lll)d");
        assert_eq!(ed.current_buffer().text(), "第一句第二句。第三句。\n");
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
    fn a_packed_page_still_says_where_a_paragraph_begins() {
        // `:dense` is the default page, so masking the indent under it made
        // 首行縮進 invisible out of the box. The three things `:dense` drops
        // each cost a *column*; the indent costs two squares, and it is what
        // replaces the blank line — which costs a whole 縱.
        let mut ed = Editor::new();
        ed.set_layout(crate::zong::Layout::Vertical);
        ed.set_indent(2);
        ed.set_dense(true);
        assert_eq!(ed.paragraph_indent(), 2);
        let nothing = |_: usize| Vec::new();
        let never = |_: usize| false;
        let bare = |_: usize| Vec::new();
        assert_eq!(ed.grid_with(&nothing, &never, &bare).indent, 2);
        assert!(ed.ruby().is_empty(), "…while the reading column still goes");
    }

    #[test]
    fn the_blank_line_an_indent_replaces_comes_off_the_page() {
        // The file is Markdown and keeps its blank lines; the page is a book
        // and shows the indent instead. Both marks at once is the one thing
        // no typesetter does.
        let mut ed = Editor::new();
        ed.current_buffer_mut()
            .insert(0, "第一段\n\n第二段\n\n\n第三段\n# 標題\n\n```\n\n```\n");
        assert!(!ed.line_is_folded(1), "nothing folds until there is an indent");
        ed.set_indent(2);
        assert!(ed.line_is_folded(1), "the one between two paragraphs");
        // Two blanks is a scene break — the writer meant the second one.
        assert!(!ed.line_is_folded(3));
        assert!(!ed.line_is_folded(4));
        // A blank line inside a fence is code, not a paragraph break.
        assert!(!ed.line_is_folded(8), "inside the fence");
        // …and the vertical page, which carries a span rather than a map, is
        // told to leave that whole part of the file alone.
        let (first, last) = ed.fold_free_span();
        assert!(first <= 8 && last >= 8, "the fence is in the span: {first}..{last}");
        // …and a heading is not in it: a novel's chapters are not prose
        // either, and a span from the first heading to the last would be the
        // whole book.
        assert!(first > 6, "the heading is not in the span: {first}..{last}");
        let hidden = |line: usize| ed.markup_hidden_on_line(line);
        let folded = |line: usize| ed.line_is_folded(line);
        let ghost = |line: usize| ed.ghost_on_line(line);
        assert!(!crate::zong::folded(
            ed.current_buffer().rope(),
            8,
            ed.grid_with(&hidden, &folded, &ghost)
        ));
        // And never the line the cursor is on, or you could not type into it.
        ed.execute(":2").unwrap();
        assert_eq!(ed.cursor_line(), 1);
        assert!(!ed.line_is_folded(1));
    }

    #[test]
    fn a_packed_page_does_not_pay_for_a_reading_column() {
        // `:dense` says in its own doc comment, and in the manual's table,
        // that it drops the reading column. It did not: the mask was on the
        // hung 句讀 and not on the readings, so a packed page still reserved
        // two cells a 縱 for a column it was not drawing.
        let mut ed = Editor::new();
        ed.set_layout(crate::zong::Layout::Vertical);
        assert!(!ed.ruby().is_empty(), "readings are laid out by default");
        ed.set_dense(true);
        assert!(ed.ruby().is_empty(), "a packed 縱書 page lays out none");
        // …but a horizontal page pays no width for one, so packing takes
        // nothing away there.
        ed.set_layout(crate::zong::Layout::Horizontal);
        assert!(!ed.ruby().is_empty(), "橫排 is not what 密排 packs");
        ed.set_layout(crate::zong::Layout::Vertical);
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
        assert!(ed.status().contains("已經是"), "{}", ed.status());

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
        assert_eq!(ed.prompt(), Some(("注", "hàn zì")));
    }

    #[test]
    fn ruby_mode_annotates_a_selection() {
        let mut ed = typed("他說口很難");
        press(&mut ed, "gg2lv"); // select 口
        ed.execute(":ruby").unwrap();
        assert_eq!(ed.mode(), Mode::Ruby);
        assert_eq!(ed.prompt(), Some(("注", "")), "a fresh reading");
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
        assert_eq!(ed.prompt(), Some(("注", "kou")), "prefilled, not blank");
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
    fn a_command_says_what_it_is_waiting_for() {
        // 標點旁置 needs a 縱書 page that is not packed. It used to set a flag
        // nobody read: the setting said 「開」, the page did not change, and
        // there was nowhere to find out why.
        let mut ed = Editor::new();
        ed.set_dense(true);
        ed.execute(":hanging on").unwrap();
        assert!(!ed.hanging_punctuation(), "{}", ed.status());
        let said = ed.status().to_string();
        assert!(said.contains("竪排") && said.contains("密排關"), "{said}");
        assert!(said.contains("force"), "{said}");

        // …and `force` brings the prerequisites about, in one line.
        ed.execute(":hanging on force").unwrap();
        assert_eq!(ed.layout(), crate::zong::Layout::Vertical);
        assert!(!ed.dense());
        assert!(ed.hanging_punctuation(), "{}", ed.status());

        // A command whose needs are met says nothing about them.
        ed.execute(":hanging off").unwrap();
        assert!(!ed.hanging_punctuation());
        assert!(!ed.status().contains("需要"), "{}", ed.status());
    }

    #[test]
    fn the_commit_method_rides_the_same_channel_as_the_scheme() {
        // 上屏方式 belongs to the engine, which the core does not hold, so
        // `:yume commit` leaves a request the front end answers (Feature #209).
        let mut ed = Editor::new();
        ed.set_ime_available(true);
        ed.execute(":yume commit").unwrap();
        assert_eq!(ed.take_scheme_request().as_deref(), Some("commit:"));
        ed.execute(":yume commit auto").unwrap();
        assert_eq!(ed.take_scheme_request().as_deref(), Some("commit:unique"));
        assert_eq!(ed.take_scheme_request(), None, "taken once only");
        // It is about typing 漢字, so with no 碼表 it says so rather than
        // leaving a request nobody can answer.
        let mut cold = Editor::new();
        cold.execute(":yume commit fluency").unwrap();
        assert_eq!(cold.take_scheme_request(), None, "{}", cold.status());
        assert!(cold.status().contains("碼表"), "{}", cold.status());
    }

    #[test]
    fn chaifen_command_leaves_a_request_for_the_ime() {
        let mut ed = Editor::new();
        assert_eq!(ed.take_chaifen_request(), None);
        // 拆分 annotates *candidates*, so it needs a 碼表 — and says so, with
        // nothing loaded, instead of leaving a request nobody can answer.
        ed.execute(":yume chaifen").unwrap();
        assert_eq!(ed.take_chaifen_request(), None, "{}", ed.status());
        assert!(ed.status().contains("碼表"), "{}", ed.status());
        ed.set_ime_available(true);
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
        assert_eq!(ed.prompt(), Some(("/", "潮水")));
        assert_eq!(ed.current_buffer().text(), "春江潮水連海平");
        ed.on_key(Key::Enter);
        // The match becomes the selection, so the head sits past its last
        // character and 潮水 is what an edit would act on.
        assert_eq!(ed.selection(), (2, 4), "search selected 潮水");
    }

    /// `::` is a second mode, not a longer `:` line (Feature #224).
    #[test]
    fn a_second_colon_opens_the_search_over_what_the_commands_do() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char(':'));
        assert_eq!(ed.mode(), Mode::Command);
        ed.on_key(Key::Char(':'));
        assert_eq!(ed.mode(), Mode::Lookfor);
        assert_eq!(ed.prompt(), Some(("::", "")), "and it says which line it is");
        // Backspacing it empty goes back to `:` — the second colon is the last
        // thing there was to take back — and again from there to the page.
        ed.on_key(Key::Backspace);
        assert_eq!(ed.mode(), Mode::Command);
        ed.on_key(Key::Backspace);
        assert_eq!(ed.mode(), Mode::Normal);
    }

    /// Only on an **empty** line: `:s/:/：/` is a substitution with two colons
    /// in it, and it used to be typed in a mode that did not exist yet.
    #[test]
    fn a_colon_further_along_the_line_is_only_a_colon() {
        let mut ed = Editor::new();
        for c in ":s/".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Char(':'));
        assert_eq!(ed.mode(), Mode::Command);
        assert_eq!(ed.prompt(), Some((":", "s/:")));
    }

    /// The whole point of the mode: the reader is thinking 「竖排」 and the
    /// command is called `layout vertical`.
    #[test]
    fn a_chinese_word_on_that_line_finds_the_english_command() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char(':'));
        ed.on_key(Key::Char(':'));
        ed.insert_committed("竖排");
        assert_eq!(ed.prompt(), Some(("::", "竖排")), "committed onto the line");
        let (found, focus) = ed.lookfor_menu();
        let names: Vec<String> = found.iter().map(|h| h.choice.written()).collect();
        assert_eq!(
            names.first().map(String::as_str),
            Some("layout vertical"),
            "{names:?}"
        );
        assert_eq!(focus, 0);
        // ⇥ writes the **whole** command back onto the `:` line and goes back
        // there. Nothing has been run: what Enter runs is always the line the
        // reader can see.
        ed.on_key(Key::Tab);
        assert_eq!(ed.mode(), Mode::Command);
        assert_eq!(ed.prompt(), Some((":", "layout vertical")));
        assert_eq!(ed.prompt_caret(), "layout vertical".chars().count());
    }

    /// A typed abbreviation is a subsequence, and the answer is a **subcommand**
    /// — the flat list is the whole tree, not the two dozen top-level words.
    #[test]
    fn an_abbreviation_reaches_a_word_a_command_takes() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char(':'));
        ed.on_key(Key::Char(':'));
        for c in "tbsort".chars() {
            ed.on_key(Key::Char(c));
        }
        let (found, _) = ed.lookfor_menu();
        let names: Vec<String> = found.iter().map(|h| h.choice.written()).collect();
        assert!(names.iter().any(|n| n == "table sort"), "{names:?}");
        // Down walks the list, and the next keystroke of the query puts the
        // highlight back on top — the third row for `竖` is not the third row
        // for `竖排`.
        ed.on_key(Key::Down);
        assert_eq!(ed.lookfor_menu().1, 1);
        ed.on_key(Key::Char('x'));
        assert_eq!(ed.lookfor_menu().1, 0);
    }

    #[test]
    fn tab_cycles_the_command_completion() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char(':'));
        for c in "ru".chars() {
            ed.on_key(Key::Char(c));
        }
        // Tab walks the matches, writing each onto the line — `run` and `ruby`
        // both start with `ru`, in the order the table lists them.
        ed.on_key(Key::Tab);
        assert_eq!(ed.prompt(), Some((":", "run")));
        ed.on_key(Key::Tab);
        assert_eq!(ed.prompt(), Some((":", "ruby")), "one `ruby` now, not three");
        // …and the prefix is remembered rather than re-read from the line, so
        // walking back returns to the same one instead of starting over from
        // what Tab just wrote.
        ed.on_key(Key::BackTab);
        assert_eq!(ed.prompt(), Some((":", "run")));

        // Typing abandons the completion, so the next Tab starts from the line.
        ed.on_key(Key::Char('x'));
        assert_eq!(ed.command_menu().1, None);
    }

    #[test]
    fn the_command_line_guesses_the_rest_of_the_name() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char(':'));
        for c in "reco".chars() {
            ed.on_key(Key::Char(c));
        }
        assert_eq!(ed.prompt_ghost(), "ver", "the rest of `recover`");

        // Tab takes the guess, and then there is nothing left to guess.
        ed.on_key(Key::Tab);
        assert_eq!(ed.prompt(), Some((":", "recover")));
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

    /// The guess and Tab are two spellings of one answer, so they have to
    /// spell it the same way. A deep name (#223) is answered with its whole
    /// path — `:xing` is `yume scheme xingchen` — and the guess used to offer
    /// the leaf alone, which as a line parses as nothing.
    ///
    /// It used to ask this of `:sch`, which was one answer until `:table
    /// schema` (#218) became a second one. A prefix two commands answer to is
    /// a fine thing for the menu and a poor thing to assert about; a leaf only
    /// one word in the tree carries is the case this test is here for.
    #[test]
    fn the_guess_and_tab_agree_on_a_deep_name() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char(':'));
        for c in "xing".chars() {
            ed.on_key(Key::Char(c));
        }
        let (_, before) = ed.prompt().expect("a command line");
        let before = before.to_string();
        let guessed = format!("{before}{}", ed.prompt_ghost());
        ed.on_key(Key::Tab);
        let (_, tabbed) = ed.prompt().expect("a command line");
        assert_eq!(
            tabbed, "yume scheme xingchen",
            "the deep answer is the whole path",
        );
        assert!(
            guessed == before || guessed == tabbed,
            "the guess says {guessed:?} and Tab says {tabbed:?}",
        );
    }

    /// The prompt has had ← → Home End since it was written, so what the IME
    /// commits goes **at the caret**. It used to be appended, which put 中文
    /// typed into the middle of a pattern at the end of it instead.
    #[test]
    fn committed_text_lands_at_the_prompt_caret() {
        let mut ed = typed("春江潮水連海平");
        ed.on_key(Key::Char('/'));
        ed.insert_committed("海平");
        ed.on_key(Key::Home);
        ed.insert_committed("潮水");
        assert_eq!(ed.prompt(), Some(("/", "潮水海平")));
        // …and the caret came with it, so the next commit follows on.
        ed.insert_committed("連");
        assert_eq!(ed.prompt(), Some(("/", "潮水連海平")));
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

        // …and the next search offers the whole of it back before a single
        // key is typed (#274): `/⏎` is 「再找一次這個」, and the prompt says so
        // instead of leaving the writer to remember what it was.
        ed.on_key(Key::Char('/'));
        assert_eq!(ed.prompt_ghost(), "潮水", "the whole of the last pattern");
        ed.on_key(Key::Esc);

        // …and the rest of it once a prefix has been typed.
        ed.on_key(Key::Char('/'));
        ed.on_key(Key::Char('潮'));
        assert_eq!(ed.prompt_ghost(), "水");
        ed.on_key(Key::Tab);
        assert_eq!(ed.prompt(), Some(("/", "潮水")));
        ed.on_key(Key::Enter);
        assert_eq!(ed.selection(), (2, 4), "and it runs");

        // A pattern that is not a prefix of the last one is not guessed at.
        ed.on_key(Key::Char('/'));
        ed.on_key(Key::Char('海'));
        assert_eq!(ed.prompt_ghost(), "");
        // Rubbed out again, the guess comes back — an empty line is an empty
        // line however it got that way.
        ed.on_key(Key::Backspace);
        assert_eq!(ed.prompt_ghost(), "潮水");
        // And `Enter` on it runs the guess, which is what it always did.
        ed.on_key(Key::Enter);
        assert_eq!(ed.selection(), (2, 4), "round to the only 潮水 there is");
    }

    /// A command line with nothing on it guesses nothing: the `:` menu below
    /// is already showing every command there is, and a whole command name in
    /// grey where the writer has typed nothing reads as a line already begun.
    #[test]
    fn an_empty_command_line_guesses_nothing() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char(':'));
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
            Some((":", "w draft")),
            "a file name is not a command name"
        );
    }

    #[test]
    fn the_completed_command_runs() {
        let mut ed = typed("春江潮水");
        ed.on_key(Key::Char(':'));
        ed.on_key(Key::Char('w'));
        ed.on_key(Key::Char('o'));
        ed.on_key(Key::Tab);
        assert_eq!(ed.prompt(), Some((":", "word")));
        ed.on_key(Key::Enter);
        assert!(ed.status().contains("分詞"), "{}", ed.status());
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
        let mut ed = typed("春江春江\n");
        // Two `l` for two characters: a selection here is half-open, so `v`
        // starts one of width zero rather than one covering the cursor's own
        // grapheme the way Helix does.
        press(&mut ed, "vl"); // select 春江
        press(&mut ed, "g?");
        // **Shown, not jumped to**: 「這個詞還在哪裏」 is answered beside the
        // place you are standing, and you are still standing there.
        let (from, to) = ed.other_pane().and_then(|p| p.highlight).expect("a hit");
        assert_eq!(
            ed.current_buffer().rope().slice(from..to).to_string(),
            "春江",
            "the other occurrence of the selected text"
        );
        assert_eq!((from, to), (2, 4));
        // The *next* one after where you are standing, which here is the
        // second of two.
        assert!(ed.status().contains("2/2"), "{}", ed.status());
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
        assert!(report.contains("漢字 21"), "{report}");
        assert!(report.contains("字數 24"), "{report}");
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
        assert!(report.contains("漢字 2"), "{report}");
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
    fn a_block_of_delimited_text_becomes_a_table_and_goes_back() {
        let mut ed = typed("那年冬天。\n\n字,讀音\n永,ㄩㄥˇ\n和,ㄏㄜˊ\n\n雪下得早。\n");
        ed.execute(":3").unwrap();
        ed.execute(":table pipe").unwrap();
        assert_eq!(
            ed.current_buffer().text(),
            "那年冬天。\n\n| 字 | 讀音  |\n| -- | ----- |\n| 永 | ㄩㄥˇ |\n| 和 | ㄏㄜˊ |\n\n雪下得早。\n"
        );
        // The prose either side of the blank lines is untouched — a blank line
        // is where the block stops.
        assert!(ed.status.contains("2"), "rows and columns: {}", ed.status);
        // And it is a grid now, not five lines that happen to start with a pipe.
        assert!(ed.table.is_some(), "walked by cell straight away");

        // Back the other way, from anywhere inside it.
        ed.execute(":5").unwrap();
        ed.execute(":table csv").unwrap();
        assert_eq!(
            ed.current_buffer().text(),
            "那年冬天。\n\n字,讀音\n永,ㄩㄥˇ\n和,ㄏㄜˊ\n\n雪下得早。\n"
        );

        // One undo apiece: a conversion is one edit, not one per row.
        ed.on_key(Key::Char('u'));
        assert!(ed.current_buffer().text().contains("| 永 |"), "{}", ed.current_buffer().text());
    }

    #[test]
    fn a_selection_says_which_lines_the_table_is_made_of() {
        // No blank line anywhere: without a selection the walk would take the
        // heading and the sentence with it.
        let mut ed = typed("# 人物\n甲,乙\n丙,丁\n那年冬天。\n");
        ed.execute(":2").unwrap();
        press(&mut ed, "xx"); // the two data rows, and only those
        ed.execute(":table pipe").unwrap();
        let text = ed.current_buffer().text();
        assert!(text.starts_with("# 人物\n| 甲 | 乙 |\n"), "{text:?}");
        assert!(text.ends_with("| 丙 | 丁 |\n那年冬天。\n"), "{text:?}");
    }

    #[test]
    fn a_conversion_that_would_lose_a_cell_is_refused() {
        // Three rows and three columns, with the comma in **row 3, column 2**
        // — a shape that tells the two numbers apart. On a 2×2 table this
        // said 「第 2 行第 2 欄」 whichever way round the arguments went in.
        let mut ed = typed(
            "| 字 | 註 | 部 |\n| -- | -- | -- |\n| 永 | 水 | 丶 |\n| 之 | 長, 久 | 丿 |\n",
        );
        ed.execute(":3").unwrap();
        let before = ed.current_buffer().text();
        ed.execute(":table csv").unwrap();
        // Named, and nothing written: the file is exactly as it was.
        assert_eq!(ed.current_buffer().text(), before);
        // Row 3 counts the header as row 1 and the rule row not at all.
        let (row, column) = (ed.status.find('3'), ed.status.find('2'));
        assert!(
            matches!((row, column), (Some(r), Some(c)) if r < c),
            "row 3 then column 2, in that order: {}",
            ed.status
        );

        // The writer picks a delimiter the data does not hold, and it goes.
        ed.execute(":table csv tab").unwrap();
        assert_eq!(
            ed.current_buffer().text(),
            "字\t註\t部\n永\t水\t丶\n之\t長, 久\t丿\n"
        );
    }

    #[test]
    fn a_paragraph_is_not_quietly_cut_into_columns() {
        let mut ed = typed("那年冬天，雪下得早。\n他站在門口，看了很久，沒有進去。\n");
        let before = ed.current_buffer().text();
        ed.execute(":table pipe").unwrap();
        // Nothing regular separates these lines, so nothing is guessed at.
        assert_eq!(ed.current_buffer().text(), before);
        assert!(ed.status.contains("tab") || ed.status.contains("分隔"), "{}", ed.status);

        // …but a writer who says what the delimiter is gets what they asked
        // for, even a 、 — they have looked at their data.
        let mut ed = typed("甲、乙\n丙、丁\n");
        ed.execute(":table pipe 、").unwrap();
        assert_eq!(
            ed.current_buffer().text(),
            "| 甲 | 乙 |\n| -- | -- |\n| 丙 | 丁 |\n"
        );

        // And `:table pipe` on a table already made is a no-op, not a table
        // twice as wide.
        let before = ed.current_buffer().text();
        ed.execute(":table pipe").unwrap();
        assert_eq!(ed.current_buffer().text(), before);
    }

    #[test]
    fn a_table_exports_as_a_file_without_being_converted_in_place() {
        let dir = std::env::temp_dir().join(format!("yumete-csv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("人物.md");
        std::fs::write(&path, "# 人物\n\n| 名 | 字 |\n| -- | -- |\n| 淵明 | 元亮 |\n").unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();
        ed.execute(":5").unwrap();
        let before = ed.current_buffer().text();
        ed.execute(":export csv").unwrap();

        // Named after the file, and the manuscript untouched — this is a copy
        // handed out, not a conversion.
        let out = std::fs::read_to_string(dir.join("人物.csv")).unwrap();
        assert_eq!(out, "名,字\n淵明,元亮\n");
        assert_eq!(ed.current_buffer().text(), before);

        // `tsv` is the same table, split another way.
        ed.execute(":export tsv").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("人物.tsv")).unwrap(),
            "名\t字\n淵明\t元亮\n"
        );

        // An existing file is not replaced unless the bang says so — the rule
        // `:w` keeps and the document exports keep. Both messages print the
        // path, so the path is not what tells them apart: what does is the
        // word, and whether the file on disk actually changed.
        std::fs::write(dir.join("人物.csv"), "別動我\n").unwrap();
        ed.execute(":export csv").unwrap();
        assert!(ed.status.contains("已經有"), "{}", ed.status);
        assert_eq!(
            std::fs::read_to_string(dir.join("人物.csv")).unwrap(),
            "別動我\n",
            "refused means the file is untouched"
        );
        ed.execute(":export! csv").unwrap();
        assert!(ed.status.contains("寫好了"), "{}", ed.status);
        assert_eq!(
            std::fs::read_to_string(dir.join("人物.csv")).unwrap(),
            "名,字\n淵明,元亮\n"
        );

        // And never onto something open in the editor — this would otherwise
        // write the table over the manuscript it came from. (A named path is
        // resolved against the working directory, the way `:w` resolves one,
        // so the test names it in full: `人物.md` alone would mean a file in
        // whatever directory the editor was started in.)
        ed.execute(&format!(":export csv {}", path.display())).unwrap();
        assert!(ed.status.contains("這份稿子本身"), "{}", ed.status);
        assert_eq!(ed.current_buffer().text(), before, "the manuscript stands");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            before,
            "…on disk as well as in memory"
        );

        // …and not onto a *different* file that is open either.
        let other = dir.join("地名.md");
        std::fs::write(&other, "# 地名\n").unwrap();
        ed.execute(&format!(":open {}", other.display())).unwrap();
        ed.execute(&format!(":open {}", path.display())).unwrap();
        ed.execute(":5").unwrap();
        ed.execute(&format!(":export! csv {}", other.display())).unwrap();
        assert!(ed.status.contains("正開着"), "{}", ed.status);
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "# 地名\n");

        // Away from any table there is nothing to export.
        ed.execute(":1").unwrap();
        ed.execute(":export csv").unwrap();
        assert!(ed.status.contains('|'), "says what is missing: {}", ed.status);

        // A file that is already a grid exports as itself: reading a `.csv` and
        // writing a `.tsv` is the same conversion by another name.
        let csv = dir.join("表.csv");
        std::fs::write(&csv, "字,讀音\n永,ㄩㄥˇ\n").unwrap();
        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", csv.display())).unwrap();
        assert!(ed.execute(":table").is_ok());
        ed.execute(":export tsv").unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("表.tsv")).unwrap(),
            "字\t讀音\n永\tㄩㄥˇ\n"
        );

        // …and a grid refuses the same conversion a `|` table refuses: a cell
        // that already holds the delimiter being written would come back two
        // cells, and every column right of it would shift. Row 3, column 2,
        // so the two numbers are told apart.
        let tsv = dir.join("表二.tsv");
        std::fs::write(&tsv, "字\t讀音\n永\tㄩㄥˇ\n之\t一, 二\n").unwrap();
        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", tsv.display())).unwrap();
        assert!(ed.execute(":table").is_ok());
        ed.execute(":export csv").unwrap();
        let (row, column) = (ed.status.find('3'), ed.status.find('2'));
        assert!(
            matches!((row, column), (Some(r), Some(c)) if r < c),
            "row 3 then column 2: {}",
            ed.status
        );
        assert!(
            !dir.join("表二.csv").exists(),
            "refused means nothing was written"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_shot_names_a_file_and_waits_for_the_frame_it_is_a_picture_of() {
        let dir = std::env::temp_dir().join(format!("yumete-shot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.md");
        std::fs::write(&path, "永和九年。\n").unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();

        // Nothing is written here: the picture is of the frame that has not
        // been drawn yet, so all `:shot` may do is say which file and how.
        ed.execute(":shot").unwrap();
        assert_eq!(
            ed.take_screenshot_request(),
            Some(ShotJob::Page {
                target: dir.join("chapter.shot.html"),
                text: false,
            })
        );
        // …and taking it takes it: one `:shot`, one picture.
        assert_eq!(ed.take_screenshot_request(), None);

        // `.shot` before the extension, so it never collides with `:export`.
        assert!(!dir.join("chapter.html").exists());

        // The name decides whether the colours come with it.
        ed.execute(":shot page.txt").unwrap();
        assert_eq!(
            ed.take_screenshot_request(),
            Some(ShotJob::Page {
                target: PathBuf::from("page.txt"),
                text: true,
            })
        );

        // `screen` is the other picture entirely — the window, taken by the
        // platform. It names no file, so no guard applies to it.
        ed.execute(":shot screen").unwrap();
        assert_eq!(ed.take_screenshot_request(), Some(ShotJob::Screen));

        // A scratch buffer has no name to derive one from, exactly as an
        // export has not.
        let mut scratch = Editor::new();
        assert!(matches!(
            scratch.execute(":shot"),
            Err(EditorError::NoFileName)
        ));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_shot_refuses_a_file_that_is_already_there_and_one_that_is_open() {
        let dir = std::env::temp_dir().join(format!("yumete-shot2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.md");
        std::fs::write(&path, "永和九年。\n").unwrap();
        let taken = dir.join("chapter.shot.html");
        std::fs::write(&taken, "早就在這裏了\n").unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();

        // Already there: refused, and nothing is parked for the front end —
        // otherwise the refusal would be printed and the file written anyway.
        ed.execute(":shot").unwrap();
        assert_eq!(ed.take_screenshot_request(), None);
        assert!(ed.status().contains("chapter.shot.html"), "{}", ed.status());

        // The bang is the answer, the same one `:export!` takes.
        ed.execute(":shot!").unwrap();
        assert_eq!(
            ed.take_screenshot_request(),
            Some(ShotJob::Page {
                target: taken.clone(),
                text: false,
            })
        );

        // Never onto a file this editor is holding: the chapter itself is the
        // one a hurried `:shot!` would otherwise overwrite with a picture.
        ed.execute(&format!(":shot! {}", path.display())).unwrap();
        assert_eq!(ed.take_screenshot_request(), None);

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

        // A bare `:toc` opens the outline — a list, one heading a line — and
        // says how many there are. It used to join every heading into the
        // status line, which for a novel is 700 chapters on one row.
        ed.execute(":toc").unwrap();
        assert!(ed.sidebar().is_some(), "{}", ed.status());
        assert!(ed.status().contains("4"), "{}", ed.status());
    }

    #[test]
    fn a_novel_with_no_markup_still_has_chapters() {
        // 資治通鑑 is a `.txt` with 700 chapters in it and not one `#`. The
        // outline used to be empty for exactly the file where 「go to chapter
        // 412」 is worth a key.
        let dir = std::env::temp_dir().join(format!("yumete-toc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let novel = dir.join("novel.txt");
        // Three lines of writing under each: a heading with nothing under it
        // is a 目錄 line, and this file is not a 目錄.
        std::fs::write(
            &novel,
            "楔子\n那年冬天。\n雪下得早。\n山路斷了。\n\
             第一卷\n他抬頭看了看那片天。\n雪還在下。\n山路已經看不見了。\n\
             第一章　風雪\n風從北面來。\n院子裏那棵老槐樹壓斷了一根枝。\n他站了很久。\n\
             第二章\n第二天雪停了。\n路上沒有人。\n他一個人走。\n\
             第三章魚是一句話的開頭，不是標題。\n",
        )
        .unwrap();
        let mut ed = Editor::new();
        ed.open_file(&novel).unwrap();
        let headings = ed.outline();
        let titles: Vec<&str> = headings.iter().map(|(_, _, t)| t.as_str()).collect();
        assert_eq!(titles, ["楔子", "第一卷", "第一章　風雪", "第二章"]);
        assert_eq!(headings[1].1, 1, "a 卷 holds 章, so it sits above them");
        assert_eq!(headings[2].1, 2);
        // The third heading is 第一章　風雪, eight lines in.
        ed.execute(":toc 3").unwrap();
        assert_eq!(ed.cursor_line(), headings[2].0);

        // **資治通鑑 writes all 294 of its 卷 as 卷002** — the unit first and no
        // 第 at all, which is the book this was written for and the one the
        // first version found twelve chapters in. 史記 writes 卷一　五帝本紀第一.
        let history = dir.join("history.txt");
        std::fs::write(
            &history,
            "卷002\n漢紀一。\n威烈王二十三年。\n初命晉大夫。\n\
             卷003　夏本紀第二\n周紀二。\n臣光曰。\n夫禮，辨貴賤。\n\
             話說天下大勢，分久必合。\n",
        )
        .unwrap();
        let mut ed = Editor::new();
        ed.open_file(&history).unwrap();
        let titles: Vec<String> = ed.outline().into_iter().map(|(_, _, t)| t).collect();
        assert_eq!(titles, ["卷002", "卷003　夏本紀第二"], "話說 is prose, not a 話");
        let _ = std::fs::remove_dir_all(&dir);
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
             [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
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
    fn a_grid_without_a_schema_is_split_on_what_its_lines_agree_about() {
        // The header-row fallback used to split on a comma and nothing else,
        // so a tab-separated file — which `:export tsv` in this very editor
        // writes — came back 「不是表格」 while a page of prose whose lines
        // happen to hold one comma each still came back a grid. Both are the
        // sniffer's question, so both are asked of the sniffer.
        let dir = std::env::temp_dir().join(format!("yumete-grid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let tsv = dir.join("讀音.tsv");
        std::fs::write(&tsv, "字\t讀音\t部\n永\tㄩㄥˇ\t水\n之\t\u{34E4}\t丿\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&tsv).unwrap();
        assert!(ed.execute(":table").is_ok());
        assert_eq!(ed.cell_text(1, 1), "ㄩㄥˇ", "tabs, not commas");
        assert_eq!(ed.cell_text(2, 2), "丿");

        // …and semicolons, the third of the three the sniffer knows.
        let scsv = dir.join("讀音.txt");
        std::fs::write(&scsv, "字;讀音\n永;ㄩㄥˇ\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&scsv).unwrap();
        assert!(ed.execute(":table").is_ok());
        assert_eq!(ed.cell_text(1, 1), "ㄩㄥˇ");

        // A page of prose is still not a grid: its lines do not agree.
        let prose = dir.join("散文.txt");
        std::fs::write(&prose, "那年冬天，雪下得早。\n他站在門口，看了很久，沒有進去。\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&prose).unwrap();
        ed.execute(":table").unwrap();
        assert!(ed.cell_position().is_none(), "{}", ed.status);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_column_goes_back_on_the_rows_it_was_taken_from() {
        let dir = std::env::temp_dir().join(format!("yumete-col-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("讀音.csv");
        // A blank line inside the file. It is a row of one empty cell, so the
        // column carries a blank of its own over it — which is what keeps the
        // cells below it from all moving up one when the column goes back.
        std::fs::write(&csv, "字,讀音,部\n永,ㄩㄥˇ,水\n\n之,ㄓ,丿\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.execute(":table").is_ok());
        press(&mut ed, "ty");
        press(&mut ed, "ll");
        press(&mut ed, "tp");
        assert_eq!(
            ed.current_buffer().text(),
            "字,讀音,字\n永,ㄩㄥˇ,永\n\n之,ㄓ,之\n",
            "{}",
            ed.status
        );

        // A value that holds the delimiter is refused by row and column, the
        // way `:table csv` refuses one — it used to have the commas quietly
        // filtered out of it, and 「長, 久」 went in as 「長 久」. Refused
        // before anything is written, so there is nothing to undo.
        let before = ed.current_buffer().text();
        ed.store("部\n水\n\n長, 久\n".to_string());
        press(&mut ed, "tp");
        assert_eq!(ed.current_buffer().text(), before, "nothing was written");
        let (row, column) = (ed.status.find('4'), ed.status.find('3'));
        assert!(
            matches!((row, column), (Some(r), Some(c)) if r < c),
            "row 4, column 3 — where it would have landed: {}",
            ed.status
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_cell_pasted_into_a_pipe_table_keeps_its_backslashes() {
        // `\|` is the escape, so a backslash is doubled with it: a cell whose
        // own text is `C:\` written as `C:\` would read back as an escape
        // waiting for the pipe that follows. The paste used to replace the
        // pipe alone, which is `escape`'s job and half of it.
        let mut ed = typed("| 字 | 註 |\n| -- | -- |\n| 永 | 水 |\n");
        ed.execute(":3").unwrap();
        ed.enter_table();
        press(&mut ed, "l");
        ed.store("註\nC:\\ 與 |\n".to_string());
        press(&mut ed, "tp");
        // Written escaped…
        assert!(
            ed.current_buffer().text().contains(r"C:\\ 與 \|"),
            "{}",
            ed.current_buffer().text()
        );
        // …and read back as itself.
        assert_eq!(
            crate::mdtable::unescape(ed.cell_text(2, 1).trim()),
            r"C:\ 與 |"
        );
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
    fn gd_goes_gw_shows_and_a_missing_note_gets_written() {
        // The pair every editor has: `Enter` is 「還在哪裏」 (references), `gd`
        // is 「它在哪裏定義的」. Keeping both on `Enter` meant a word inside a
        // footnote could not be searched for at all.
        let mut ed = typed("那年冬天[^1]，山下起了大雪。\n那年夏天。\n");
        ed.set_render(Render::On);
        ed.goto_line(1);
        for _ in 0..4 {
            ed.on_key(Key::Char('l'));
        }
        // No note for it yet: the stub is written at the foot — and `gd`
        // *goes* there, landing at its end with Insert one keystroke away,
        // which is how a note gets written.
        let was = ed.cursor();
        press(&mut ed, "gd");
        let text = ed.current_buffer().text();
        assert!(text.ends_with("[^1]: "), "{text:?}");
        assert!(ed.status().contains("寫下了"), "{}", ed.status());
        assert_eq!(ed.cursor(), ed.current_buffer().rope().len_chars(), "at its end");
        ed.on_key(Key::Ctrl('o'));
        assert_eq!(ed.cursor(), was, "C-o comes back to the sentence");
        // …and it is one edit, so one `u` takes it back.
        ed.on_key(Key::Char('u'));
        assert!(!ed.current_buffer().text().contains("[^1]: "));

        // `g?` is the other question entirely — asked from a word, since
        // 「還在哪裏」 needs something to be about.
        press(&mut ed, "gg");
        press(&mut ed, "ll");
        press(&mut ed, "g?");
        assert!(
            ed.status().contains("處") || ed.status().contains("只有"),
            "{}",
            ed.status()
        );
    }

    #[test]
    fn both_layouts_ask_the_same_page() {
        // **The differential test.** Every defect in this class was invisible
        // because each side asked its own implementation: 縱書 worked out what
        // was off the page from the bare line — no syntax, no block — while
        // 橫排 was handed the answer. So it ate the `**` inside a fence, hid
        // two asterisks where Typst has one, hid four under `:syntax text`,
        // and folded blank lines by a different rule. This asks both.
        let document = "---\ntitle: 甲\n---\n\n那**年**冬天。\n\n# 第一章\n\n```\n\n程式 **很好** 碼。\n```\n\n最後一段。\n";
        for syntax in [
            crate::syntax::Syntax::Markdown,
            crate::syntax::Syntax::Typst,
            crate::syntax::Syntax::Text,
        ] {
            for indent in [0usize, 2] {
                let mut ed = Editor::new();
                ed.current_buffer_mut().insert(0, document);
                ed.set_default_syntax(Some(syntax));
                ed.set_indent(indent);
                ed.set_render(Render::Full);
                let rope = ed.current_buffer().rope();
                let hidden = |line: usize| ed.markup_hidden_on_line(line);
                let folded = |line: usize| ed.line_is_folded(line);
                let ghost = |line: usize| ed.ghost_on_line(line);
                let grid = ed.grid_with(&hidden, &folded, &ghost);
                for line in 0..rope.len_lines() {
                    assert_eq!(
                        crate::zong::folded(rope, line, grid),
                        ed.line_is_folded(line),
                        "{syntax:?} indent={indent}: line {line} folds differently in the two \
                         layouts"
                    );
                    // …and what is off the page is one answer, not two: the
                    // slots of a 縱 cover exactly the characters that are not
                    // hidden, plus the hidden ones joined to their neighbours.
                    let text = crate::zong::line_chars(rope, line);
                    let slots = crate::zong::line_slots_in(
                        &rope.line(line).to_string(),
                        grid,
                        &ed.markup_hidden_on_line(line),
                    );
                    for at in 0..text.len() {
                        assert!(
                            slots.iter().any(|s| at >= s.start && at < s.end)
                                || slots.iter().all(|s| s.start == 0 && s.end == 0),
                            "{syntax:?}: char {at} of line {line} is in no slot — the cursor \
                             could stand where nothing is drawn"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_repeat_can_never_repeat_itself() {
        // `d`, `3`, `.`, `.` used to abort the process — a stack overflow,
        // which does not unwind, so every unsaved buffer went with it. Two
        // causes, both here: a count made `.` the *second* key of its own
        // definition, and nothing stopped a repeat from re-entering.
        let mut ed = typed("一二三四五六七八九十\n");
        ed.execute("1").unwrap();
        ed.on_key(Key::Char('d'));
        ed.on_key(Key::Char('3'));
        ed.on_key(Key::Char('.'));
        ed.on_key(Key::Char('.'));
        ed.on_key(Key::Char('.'));
        // Still here — and `.` still means the `d`: one, then three, then one,
        // then one.
        assert_eq!(ed.current_buffer().text(), "七八九十\n");
    }

    #[test]
    fn a_hit_list_belongs_to_the_document_it_was_found_in() {
        // The worst thing this editor could hold: a list of char offsets with
        // no owner, holding `n` and `N`, surviving a buffer switch and an
        // edit. 「第 3/78 處」 could be said about a character in a chapter
        // that was never searched — and the next `d` deleted it.
        let mut ed = typed("那年冬天。\n那年夏天。\n");
        ed.goto_line(1);
        press(&mut ed, "g?");
        assert!(ed.current_hit().is_some(), "{}", ed.status());

        // Another file: the hits do not follow, and `n` goes back to `/`.
        ed.execute("new").unwrap();
        ed.current_buffer_mut().insert(0, "完全不相干的一行。\n");
        assert_eq!(ed.current_hit(), None, "another document, no hits");
        let before = ed.cursor();
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.current_hit(), None);
        assert!(!ed.status().contains("處"), "{}", ed.status());
        let _ = before;

        // …and an edit retires them rather than moving them: an offset into
        // the text as it was is not a shorter answer, it is a wrong one.
        ed.execute("buffer previous").unwrap();
        assert!(ed.current_hit().is_some(), "back where they were found");
        ed.on_key(Key::Char('i'));
        ed.on_key(Key::Char('甲'));
        ed.on_key(Key::Esc);
        assert_eq!(ed.current_hit(), None, "the text moved under them");
    }

    #[test]
    fn the_keys_a_keyboard_has_are_not_swallowed() {
        // PageUp/PageDown reached the editor as *nothing*: the table that turns
        // a terminal's keys into the editor's had no line for them, so they
        // fell off the end in every mode. `C-a`/`C-e` were taken by the `:`
        // line and swallowed by Insert.
        let mut ed = typed(&"一行\n".repeat(200));
        ed.set_page(20, 80);
        ed.execute("1").unwrap();
        ed.on_key(Key::PageDown);
        assert!(ed.cursor_line() > 10, "a page down: {}", ed.cursor_line());
        let down = ed.cursor_line();
        ed.on_key(Key::PageUp);
        assert!(ed.cursor_line() < down, "and a page back");

        // …and in Insert, without leaving it.
        ed.on_key(Key::Char('i'));
        let was = ed.cursor_line();
        ed.on_key(Key::PageDown);
        assert!(ed.cursor_line() > was, "a page down while typing");
        assert_eq!(ed.mode(), Mode::Insert, "and still typing");
        ed.on_key(Key::Char('甲'));
        ed.on_key(Key::Ctrl('a'));
        assert_eq!(
            ed.cursor(),
            crate::motion::line_start(ed.current_buffer().rope(), ed.cursor()),
            "C-a is the line's start, as it is on the `:` line"
        );
        ed.on_key(Key::Ctrl('e'));
        assert_eq!(
            ed.cursor(),
            crate::motion::line_end(ed.current_buffer().rope(), ed.cursor())
        );
    }

    #[test]
    fn enter_on_prose_asks_where_else_this_word_is() {
        // One key, one meaning, in a table and out of it: 「在另一個工作區給我
        // 看這個詞還出現在哪裏」. It used to say 「這裏沒有註」 and stop,
        // which is an answer to a question nobody asked.
        let mut ed = typed("那年冬天很冷。\n第二行。\n那年夏天很熱。\n");
        ed.goto_line(1);
        let standing = ed.cursor();
        press(&mut ed, "g?");
        // 那年 is the word under the cursor, and it is on line 3 as well.
        assert_eq!(ed.peeked_line(), Some(2), "{}", ed.status());
        assert_eq!(ed.cursor(), standing, "and you did not go anywhere");
        assert!(ed.status().contains("2/2") || ed.status().contains("1/2"), "{}", ed.status());

        // A word that is only here says so rather than opening an area for it.
        ed.execute("2").unwrap();
        for _ in 0..2 {
            ed.on_key(Key::Char('l'));
        }
        press(&mut ed, "g?");
        assert!(ed.status().contains("只有這一處"), "{}", ed.status());
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
        assert_eq!(d.rows[0].1.as_deref(), Some("據縣志，那是丁丑年。"));
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
        assert_eq!(d.rows[0].1.as_deref(), Some("這裏要改，冬天太早了"));

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
        // **`gd`**, not `Enter`: 「它指着哪裏」 and 「還在哪裏」 are two
        // questions, and `Enter` is the second one everywhere — otherwise a
        // word *inside* a note could never be asked about.
        // `gw` shows it beside the sentence; `gd` goes to it, and `C-o` comes
        // back — the pair every editor has.
        press(&mut ed, "gw");
        assert_eq!(ed.peeked_line(), Some(2), "the note, beside the sentence");
        assert_eq!(ed.cursor(), was, "and the sentence is still under the cursor");
        press(&mut ed, "gd");
        assert_eq!(ed.cursor_line(), 2, "…and this one goes there");
        ed.on_key(Key::Ctrl('o'));
        assert_eq!(ed.cursor(), was, "C-o comes back");
        ed.goto_line(1);
        press(&mut ed, "g?");
        assert_eq!(ed.cursor_line(), 0, "nothing moves");
        // Not on a note, so `Enter` is what it is everywhere else: 「這個詞還
        //在哪裏」, shown in the other work area.
        assert!(ed.status().contains("處") || ed.status().contains("只有"), "{}", ed.status());

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
        // Numbered exactly as the rows are, or the panel can never find the
        // field the cursor is in — which is how it came to scroll to the top
        // and light nothing.
        assert_eq!(d.here, " 1 字", "and it says which field you are in");
        assert!(d.rows.iter().any(|(name, _)| *name == d.here), "and it is one of them");
        // **Numbered, and all of them** — the keys count columns (`3gd`,
        // `t20-20g`), and an empty field is a finding in a 拆分表, not a thing
        // to hide.
        assert_eq!(
            d.rows,
            vec![
                (" 1 字".to_string(), Some("一".to_string())),
                (" 2 ids_y".to_string(), Some("⿰木目".to_string())),
                (" 3 ids_g".to_string(), Some("⿰木目".to_string())),
                // Worked out, not stored, and marked so nobody looks for a
                // column that is not in the file.
                ("unicode*".to_string(), Some("U+4E00".to_string())),
            ]
        );

        // The header is not a row and has nothing to say about itself.
        ed.goto_line(1);
        assert!(ed.detail().is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `gd` in a grid: **the row named by what is here, in one column**.
    ///
    /// `gd` searches the key column, `3gd` column three, `2-5gd` columns two
    /// through five. One column, one exact match, one place to land — which on
    /// a 拆分表 is the row the component is *about*.
    /// A delimited grid sorts by one column or several, and keeps its rows.
    #[test]
    fn a_grid_sorts_by_the_columns_it_is_told() {
        let dir = std::env::temp_dir().join(format!("yumete-sort-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("tables").join("t.toml"),
            "[table]\nfile = ['d.csv']\nkey = 'char'\n\
             [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'block'\n\
             [[table.column]]\nname = 'n'\n",
        )
        .unwrap();
        let csv = dir.join("d.csv");
        std::fs::write(&csv, "char,block,n\n丙,B,2\n甲,A,10\n乙,B,9\n丁,A,1\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        let rows = |ed: &Editor| -> Vec<String> {
            ed.current_buffer().text().lines().skip(1).map(str::to_string).collect()
        };

        // By the first column, ascending — the header stays put.
        ed.execute(":table sort 1 a").unwrap();
        assert!(ed.current_buffer().text().starts_with("char,block,n\n"), "{}", ed.status());
        assert_eq!(rows(&ed), ["丁,A,1", "丙,B,2", "乙,B,9", "甲,A,10"], "{}", ed.status());

        // **Numbers as numbers**: 10 after 9, not before it.
        ed.execute(":table sort 3 a").unwrap();
        assert_eq!(rows(&ed), ["丁,A,1", "丙,B,2", "乙,B,9", "甲,A,10"]);

        // Two columns: block ascending, then n descending inside each block.
        ed.execute(":table sort 2 a 3 d").unwrap();
        assert_eq!(rows(&ed), ["甲,A,10", "丁,A,1", "乙,B,9", "丙,B,2"], "{}", ed.status());

        // The rows are the same rows: nothing gained, nothing lost.
        let mut before: Vec<String> = "丙,B,2 甲,A,10 乙,B,9 丁,A,1".split(' ').map(str::to_string).collect();
        before.sort();
        let mut after = rows(&ed);
        after.sort();
        assert_eq!(before, after);

        // `t3S` is the same thing from the keyboard.
        for key in "t3S".chars() {
            ed.on_key(Key::Char(key));
        }
        assert_eq!(rows(&ed), ["甲,A,10", "乙,B,9", "丙,B,2", "丁,A,1"], "{}", ed.status());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A sort names every column first and acts last: `t1a2d8as`.
    ///
    /// 「我認為正確的語法應該是 t1a2d8as 表示 對第一列升序，第二列降序，第八列升
    /// 序，最後的 s 發出動作指令。原来的设计用的是 t1s2S8s 这样的命令，这个会在
    /// t1s 直接生效（因为他是前綴碼的指令）。」 —— `s` is the action, so it can
    /// never also be a column's direction; `a` and `d` are, and neither of them
    /// acts.
    #[test]
    fn a_sort_names_its_columns_before_it_acts() {
        let dir = std::env::temp_dir().join(format!("yumete-sortkeys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("d.csv");
        std::fs::write(&csv, "char,block,n\n丙,B,2\n甲,A,10\n乙,B,9\n丁,A,1\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.enter_table(), "{}", ed.status());
        let rows = |ed: &Editor| -> Vec<String> {
            ed.current_buffer().text().lines().skip(1).map(str::to_string).collect()
        };

        // Nothing has happened yet, and the half-typed command reads back as
        // what was typed — `t1a2d`, a column at a time.
        for key in "t2a3d".chars() {
            ed.on_key(Key::Char(key));
        }
        assert_eq!(ed.typed_so_far(), "t2a3d", "{}", ed.status());
        assert_eq!(rows(&ed), ["丙,B,2", "甲,A,10", "乙,B,9", "丁,A,1"], "not yet");

        // …and now the action. Block ascending, then n descending inside it.
        ed.on_key(Key::Char('s'));
        assert_eq!(ed.typed_so_far(), "", "the command is spent");
        assert_eq!(rows(&ed), ["甲,A,10", "丁,A,1", "乙,B,9", "丙,B,2"], "{}", ed.status());

        // One column keeps the old short spelling, direction in the verb.
        for key in "t1S".chars() {
            ed.on_key(Key::Char(key));
        }
        assert_eq!(rows(&ed), ["甲,A,10", "乙,B,9", "丙,B,2", "丁,A,1"], "{}", ed.status());

        // Both spellings at once: the last column takes its direction from the
        // verb, the ones before it from their own letter.
        for key in "t2a3S".chars() {
            ed.on_key(Key::Char(key));
        }
        assert_eq!(rows(&ed), ["甲,A,10", "丁,A,1", "乙,B,9", "丙,B,2"], "{}", ed.status());

        // **`d` is only a direction after a plain column number.** `t d` is
        // still 「delete this row」 and a span is still a span.
        ed.goto_line(2);
        for key in "td".chars() {
            ed.on_key(Key::Char(key));
        }
        assert_eq!(rows(&ed), ["丁,A,1", "乙,B,9", "丙,B,2"], "a row went: {}", ed.status());

        // A sort abandoned half-way leaves no columns behind for the next one.
        for key in "t1a2d".chars() {
            ed.on_key(Key::Char(key));
        }
        ed.on_key(Key::Esc);
        assert_eq!(ed.typed_so_far(), "");
        for key in "t3a".chars() {
            ed.on_key(Key::Char(key));
        }
        assert_eq!(ed.typed_so_far(), "t3a", "only this one");
        ed.on_key(Key::Char('s'));
        // **Numbers as numbers**, and only column three had a say.
        assert_eq!(rows(&ed), ["丁,A,1", "丙,B,2", "乙,B,9"], "{}", ed.status());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Sorting a table does not put the table away.
    ///
    /// 「bug：表格排序 t1s 會直接回到源碼視圖。」 A sort rewrites the *text*,
    /// and the old code answered that by forgetting the whole document —
    /// which re-asks 「is this a table?」 from scratch, and a `.csv` with no
    /// schema beside it has only the answer the reader gave it by hand.
    #[test]
    fn sorting_keeps_the_table_open() {
        let dir = std::env::temp_dir().join(format!("yumete-sortview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("plain.csv");
        std::fs::write(&csv, "char,n\n丙,2\n甲,10\n乙,9\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.enter_table(), "{}", ed.status());
        for key in "t1s".chars() {
            ed.on_key(Key::Char(key));
        }
        assert!(ed.table().is_some(), "still a table: {}", ed.status());
        assert!(
            ed.current_buffer().text().starts_with("char,n\n丙,2\n"),
            "and it sorted: {:?}",
            ed.current_buffer().text()
        );

        // The same for a `|` table in a chapter — the mode is the reader's
        // answer there too.
        let mut ed = typed("前文\n| 字 | n |\n| --- | --- |\n| 丙 | 2 |\n| 甲 | 10 |\n");
        press(&mut ed, "gg");
        for _ in 0..3 {
            ed.on_key(Key::Char('j'));
        }
        assert!(ed.enter_table(), "{}", ed.status());
        for key in "t1s".chars() {
            ed.on_key(Key::Char(key));
        }
        assert!(ed.table().is_some(), "still a table: {}", ed.status());
    }

    /// 命令＋選擇＋動作: the digits inside a sequence are its argument, and
    /// nothing leaks out of it.
    #[test]
    fn a_sequence_argument_belongs_to_its_own_sequence() {
        let mut ed = typed(&(1..=40).map(|n| format!("第{n}行。\n")).collect::<String>());

        // `g30g` — the sequence's own argument.
        press(&mut ed, "gg");
        for key in "g30g".chars() {
            ed.on_key(Key::Char(key));
        }
        assert_eq!(ed.cursor_line(), 29, "{}", ed.status());

        // …and it does not leak: the next `j` moves one line, not thirty.
        let before = ed.cursor_line();
        ed.on_key(Key::Char('j'));
        assert_eq!(ed.cursor_line(), before + 1, "the argument was spent");

        // An argument the verb does not use is dropped, not applied to
        // something else.
        for key in "g5h".chars() {
            ed.on_key(Key::Char(key));
        }
        let line = ed.cursor_line();
        ed.on_key(Key::Char('j'));
        assert_eq!(ed.cursor_line(), line + 1, "still one line");

        // Esc in the middle of a sequence leaves nothing behind.
        ed.on_key(Key::Char('g'));
        ed.on_key(Key::Char('2'));
        ed.on_key(Key::Char('-'));
        ed.on_key(Key::Esc);
        assert_eq!(ed.typed_so_far(), "", "the half-typed command is gone");
        let line = ed.cursor_line();
        ed.on_key(Key::Char('j'));
        assert_eq!(ed.cursor_line(), line + 1);

        // The old order still works, because fifty years of fingers know it.
        press(&mut ed, "gg");
        for key in "30G".chars() {
            ed.on_key(Key::Char(key));
        }
        assert_eq!(ed.cursor_line(), 29, "{}", ed.status());

        // A count before a plain key is still a count.
        press(&mut ed, "gg");
        for key in "5j".chars() {
            ed.on_key(Key::Char(key));
        }
        assert_eq!(ed.cursor_line(), 5);
    }

    #[test]
    fn gd_looks_the_cell_up_in_one_named_column() {
        let dir = std::env::temp_dir().join(format!("yumete-gdcol-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("tables").join("t.toml"),
            "[table]\nfile = ['d.csv']\nkey = 'char'\n\
             [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
             [[table.column]]\nname = 'ids_g'\n\
             [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
        )
        .unwrap();
        let csv = dir.join("d.csv");
        std::fs::write(
            &csv,
            "char,ids_y,ids_g\n相,⿰木目,⿰木目\n木,木,朩\n目,目,目\n杏,⿱木口,⿱木囗\n",
        )
        .unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        // Standing on 杏's 拆分, by character, on the 木.
        ed.execute("5").unwrap();
        press(&mut ed, "l");
        ed.on_key(Key::Tab);
        press(&mut ed, "l");
        assert_eq!(ed.char_at_cursor(), Some('木'));

        // `gd` goes to 木's own row and lands in the key cell.
        press(&mut ed, "gd");
        assert_eq!(ed.cursor_line(), 2, "{}", ed.status());
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(0));

        // Back to reading by cell, on 相's 拆分.
        ed.on_key(Key::Tab);
        ed.execute("2").unwrap();
        press(&mut ed, "l");
        assert_eq!(ed.cell_text(1, 1), "⿰木目");
        press(&mut ed, "gd");
        assert!(ed.status().contains("沒有"), "no row is called ⿰木目: {}", ed.status());

        // `3gd` asks the third column instead, where 朩 is 木's spelling.
        ed.execute("3").unwrap();
        press(&mut ed, "l");
        press(&mut ed, "l");
        assert_eq!(ed.cell_text(2, 2), "朩");
        for key in "3gd".chars() {
            ed.on_key(Key::Char(key));
        }
        assert_eq!(ed.cursor_line(), 2, "朩 is in ids_g on 木's row: {}", ed.status());
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "in that column");

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
             [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
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

        // **`gd` is one question**: which row is named by what is here. The
        // cell holds ⿰木目 and no row is called that, so by cell it says so —
        // and `Tab` is how you ask about one component, because `Tab` is what
        // decides what 「here」 means for every other key too.
        let here = ed.cursor_line();
        // **By cell** — which is how a grid is read until `Tab` says otherwise
        // — the whole cell is the question, and no row is called ⿰木目. It says
        // so rather than guessing which third of it you meant.
        press(&mut ed, "gw");
        assert!(ed.status().contains("⿰木目"), "{}", ed.status());
        assert_eq!(ed.mode(), Mode::Normal, "no picker: one question, one answer");

        // `Tab` is how you ask about one component, because `Tab` is what
        // decides what 「here」 means for every other key too.
        ed.on_key(Key::Tab);
        assert_eq!(ed.char_at_cursor(), Some('⿰'));
        press(&mut ed, "gw");
        assert!(ed.status().contains("結構符"), "{}", ed.status());
        press(&mut ed, "l");
        assert_eq!(ed.char_at_cursor(), Some('木'));
        press(&mut ed, "gw");
        assert_eq!(ed.peeked_line(), Some(2), "木's own row: {}", ed.status());
        assert_eq!(ed.cursor_line(), here, "…and the cursor did not move");
        press(&mut ed, "l");
        press(&mut ed, "gw");
        assert_eq!(ed.peeked_line(), Some(3), "and 目's, one character along");
        ed.on_key(Key::Tab);

        // `gd` goes rather than shows, and lands **in the cell**, not merely on
        // the line.
        ed.goto_line(4);
        press(&mut ed, "l");
        press(&mut ed, "gd");
        assert_eq!(ed.cursor_line(), 3, "目 is already its own row");
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(0), "in the key cell");

        // 「誰用了它」 is `Enter`, and stays `Enter`: 相 and 目 both use 目.
        ed.goto_line(4);
        press(&mut ed, "0");
        press(&mut ed, "t?");
        assert_eq!(ed.peeked_line(), Some(1), "相 uses 目");
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
             [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
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
             [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
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

        // Standing on one component, `gw` shows *that* row — nothing to ask
        // about, because the cursor already said which.
        press(&mut ed, "gw");
        assert_eq!(ed.peeked_line(), Some(3), "目's own row");
        assert_eq!(ed.mode(), Mode::Normal, "no picker");

        // Standing on the descriptor itself, there is nothing to go to — it
        // says how the components are arranged, it is not one of them.
        ed.goto_line(2);
        press(&mut ed, "ll");
        assert_eq!(ed.char_at_cursor(), Some('⿰'));
        press(&mut ed, "gw");
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
                ed.status().starts_with("schema：") && ed.status().contains(expect),
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
             [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
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

        // Every 木 in the column the schema names, down it from the top —
        // and the cursor starts at the **first** whatever row it was standing
        // on, so asking about 木 gives the same route every time.
        //
        // Five, not four: 林 is ⿰木木 and that is two of them, exactly as `/`
        // would count two matches on one line. A column search differs from a
        // row search in its *direction* and in nothing else.
        let standing = ed.cursor();
        press(&mut ed, "t?");
        assert_eq!(ed.peeked_line(), Some(1), "木 itself, the first of them");
        assert_eq!(ed.cursor(), standing, "…and you did not go anywhere");
        assert!(ed.status().contains("1/5"), "{}", ed.status());
        // …and the match is what is marked in the other area, exactly the
        // range `/` would have left as the selection.
        let (a, b) = ed.other_pane().and_then(|p| p.highlight).expect("a hit");
        assert_eq!(
            ed.current_buffer().rope().slice(a..b).to_string(),
            "木",
            "the match is what is marked"
        );
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.peeked_line(), Some(2), "相");
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.peeked_line(), Some(3), "林's first 木");
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.peeked_line(), Some(3), "…and its second");
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.peeked_line(), Some(5), "杏");
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.peeked_line(), Some(1), "and round again");
        ed.on_key(Key::Char('N'));
        assert_eq!(ed.peeked_line(), Some(5), "and back");

        // A 拆分 cell still means the other thing: its components' own rows.
        ed.execute("3").unwrap();
        press(&mut ed, "l");
        ed.on_key(Key::Tab);
        press(&mut ed, "ll");
        assert_eq!(ed.char_at_cursor(), Some('目'));
        press(&mut ed, "gw");
        assert_eq!(ed.peeked_line(), Some(4), "目's own row");

        // …and the reverse question again, from a different row.
        ed.on_key(Key::Tab);
        ed.execute("5").unwrap();
        assert_eq!(ed.cell_text(4, 0), "目");
        press(&mut ed, "t?");
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

        // `:reload!` is the other half: take what is on disk and lose what is
        // here.
        std::fs::write(&file, "外面的版本\n").unwrap();
        assert!(ed.execute("reload!").is_ok());
        assert_eq!(ed.current_buffer().text(), "外面的版本\n");
        assert!(!ed.current_buffer().is_modified());
        assert!(ed.execute("w").is_ok(), "and saving is fine again");

        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- Tables squared up on the page (Feature #212) ---------------------

    /// The width of one line **as the page draws it**: what is left after
    /// 所見即所得 has taken its markup off, plus the padding drawn back on.
    fn drawn_width(ed: &Editor, line: usize) -> usize {
        let text = ed.line_text(line).unwrap_or_default();
        let chars: Vec<char> = text.trim_end_matches(['\n', '\r']).chars().collect();
        let hidden = ed.hidden_on_line(line);
        let visible: usize = (0..chars.len())
            .filter(|at| !hidden.iter().any(|&(a, b)| (a..b).contains(at)))
            .map(|at| yumete_cjk::char_width(chars[at]))
            .sum();
        let ghost: usize = ed
            .ghost_on_line(line)
            .iter()
            .map(|(_, text)| text.chars().map(yumete_cjk::char_width).sum::<usize>())
            .sum();
        visible + ghost
    }

    /// #212: a table nobody has formatted is drawn as a table anyway.
    #[test]
    fn a_table_is_squared_up_on_the_page_and_not_in_the_file() {
        const TEXT: &str = "|甲|乙|\n|---|---|\n|一二三|四|\n";
        let mut ed = Editor::new();
        ed.add_buffer(crate::Buffer::from_text(TEXT));
        assert!(!ed.ghost_on_line(0).is_empty(), "the header is padded");
        let widths: Vec<usize> = (0..3).map(|l| drawn_width(&ed, l)).collect();
        assert_eq!(widths[0], widths[1], "{widths:?}");
        assert_eq!(widths[1], widths[2], "{widths:?}");
        // …and the file is exactly what was typed.
        assert_eq!(ed.current_buffer().text(), TEXT);
        assert!(!ed.current_buffer().is_modified());
    }

    /// #212: the ragged page the *file* cannot fix.
    ///
    /// This table is padded perfectly in the source — every other reader of it
    /// sees straight pipes. 所見即所得 then takes six columns off one row and
    /// none off the next, and the page is ragged however the file is written.
    #[test]
    fn the_padding_makes_up_for_what_所見即所得_took() {
        let mut ed = Editor::new();
        ed.add_buffer(crate::Buffer::from_text(
            "| 方案 | 說明                 |\n| ---- | -------------------- |\n| 光華 | `宇浩`系列的**基礎** |\n| 星陳 | 大字集               |\n",
        ));
        // With the markup on the page the file is already square, so nothing
        // is drawn: the padding is not a second opinion about a formatted
        // table.
        ed.execute("render on").unwrap();
        let source: Vec<usize> = (0..4).map(|l| drawn_width(&ed, l)).collect();
        assert!(source.iter().all(|w| *w == source[0]), "{source:?}");
        assert!(ed.ghost_on_line(2).is_empty(), "{:?}", ed.ghost_on_line(2));

        ed.execute("render full").unwrap();
        // Off the marked-up row: the construct the cursor is in is never
        // hidden, which is the one row that would not be short.
        press(&mut ed, "G");
        let widths: Vec<usize> = (0..4).map(|l| drawn_width(&ed, l)).collect();
        assert!(widths.iter().all(|w| *w == widths[0]), "{widths:?}");
        // Six columns of markup came off that one row — `` ` `` twice and
        // `**` twice — and six columns of padding went back on. Only there:
        // the rows that lost nothing are still exactly the file.
        assert_eq!(
            ed.ghost_on_line(2)
                .iter()
                .map(|(_, text)| text.chars().count())
                .sum::<usize>(),
            6,
            "{:?}",
            ed.ghost_on_line(2)
        );
        assert!(ed.ghost_on_line(3).is_empty(), "{:?}", ed.ghost_on_line(3));
    }

    /// #212: every table in the document, not only the one the cursor is in.
    #[test]
    fn every_table_on_the_page_is_squared_up_not_only_the_cursors() {
        let mut ed = Editor::new();
        ed.add_buffer(crate::Buffer::from_text(
            "|甲|乙|\n|---|---|\n|一二三|四|\n\n中間一段散文。\n\n|丙|丁|\n|---|---|\n|五六七|八|\n",
        ));
        press(&mut ed, "gg");
        for table in [0, 6] {
            let widths: Vec<usize> = (table..table + 3).map(|l| drawn_width(&ed, l)).collect();
            assert_eq!(widths[0], widths[1], "table at {table}: {widths:?}");
            assert_eq!(widths[1], widths[2], "table at {table}: {widths:?}");
        }
        assert!(ed.ghost_on_line(4).is_empty(), "prose is not a table");
    }

    /// #212: a table in a fence is writing *about* a table.
    #[test]
    fn a_quoted_table_is_not_padded() {
        let mut ed = Editor::new();
        ed.add_buffer(crate::Buffer::from_text(
            "```\n|甲|乙|\n|---|---|\n|一二三|四|\n```\n",
        ));
        for line in 1..4 {
            assert!(ed.ghost_on_line(line).is_empty(), "line {line}");
        }
    }

    /// #212: the two settings that mean "draw me the file".
    #[test]
    fn the_padding_goes_away_when_the_page_is_the_file() {
        let mut ed = Editor::new();
        ed.add_buffer(crate::Buffer::from_text("|甲|乙|\n|---|---|\n|一二三|四|\n"));
        assert!(!ed.ghost_on_line(0).is_empty());

        // `:render off` is a request for the file exactly as it is.
        ed.execute("render off").unwrap();
        assert!(ed.ghost_on_line(0).is_empty(), "{:?}", ed.ghost_on_line(0));
        ed.execute("render on").unwrap();
        assert!(!ed.ghost_on_line(0).is_empty());

        // Down a 縱 every character takes one cell, so display width squares
        // nothing up.
        ed.set_layout(Layout::Vertical);
        assert!(ed.ghost_on_line(0).is_empty());

        // Neither is a candidate: `has_candidate` answers only for what the
        // writer typed, and the padding is derived.
        assert!(!ed.has_candidate());
    }

    /// #212 with #211: a candidate and the padding on the same line.
    ///
    /// Two runs standing before the same character would be two answers to
    /// "what is drawn here", and the caret, the click map and the wrap would
    /// each pick their own.
    #[test]
    fn a_candidate_and_the_padding_are_one_run_each() {
        let mut ed = Editor::new();
        ed.add_buffer(crate::Buffer::from_text("|甲|乙|\n|---|---|\n|一二三|四|\n"));
        let at = ed.ghost_on_line(0).first().map(|&(at, _)| at).unwrap();
        ed.set_ghost(vec![(0, at, "候".to_string())]);
        let runs = ed.ghost_on_line(0);
        let mut anchors: Vec<usize> = runs.iter().map(|&(at, _)| at).collect();
        anchors.dedup();
        assert_eq!(anchors.len(), runs.len(), "one run per anchor: {runs:?}");
        // **The candidate comes first in it.** It continues the word the caret
        // is in; the padding's job is to reach the closing pipe, so it belongs
        // on the far side of what was typed.
        let held = runs
            .iter()
            .find(|(_, text)| text.contains('候'))
            .map(|(_, text)| text.clone())
            .unwrap_or_else(|| panic!("{runs:?}"));
        assert!(held.starts_with('候'), "{held:?}");
    }

    /// #212 with #211: the caret stands between the two of them.
    ///
    /// A candidate and the padding are drawn at the same anchor, and the caret
    /// goes *after* what was typed and *before* the space that reaches to the
    /// pipe. Counting the whole run drew the caret on the pipe after every
    /// keystroke in a table — even with no candidate at all, since the one
    /// space off a pipe is anchored exactly where a caret typing at the end of
    /// a cell is.
    #[test]
    fn the_caret_stands_after_what_was_typed_and_before_the_padding() {
        let mut ed = Editor::new();
        ed.add_buffer(crate::Buffer::from_text("|a|bbb|
|-|-|
|cc|d|
"));
        // Typing at the end of the first cell: the caret is on the `|` at
        // index 2, and the padding that widens that cell is anchored there.
        ed.set_cursor(2);
        let hide = |line: usize| ed.hidden_on_line(line);
        let fold = |line: usize| ed.line_is_folded(line);
        let ghost = |line: usize| ed.ghost_on_line(line);
        let typed = |line: usize| ed.typed_ghost_on_line(line);
        let m = crate::wrap::Measure::new(crate::wrap::NO_WRAP, &hide)
            .with_folds(&fold)
            .with_ghost(&ghost)
            .with_typed_ghost(&typed);
        let at = crate::wrap::position(ed.current_buffer().rope(), 2, m);
        // `| a` — the caret is right after the `a` it just typed, not out on
        // the pipe two cells further along.
        assert_eq!(at.column, 3, "{:?}", ed.ghost_on_line(0));
    }

    /// #212: `:syntax` changes what comes off the page without touching a byte
    /// of the file, so the padding memo has to be keyed on it too.
    #[test]
    fn the_padding_follows_the_syntax_the_file_is_read_with() {
        let mut ed = Editor::new();
        ed.add_buffer(crate::Buffer::from_text("| `a` | bbbb |
| --- | ---- |
| cc | d |
"));
        ed.execute("render full").unwrap();
        let with_markup_off = ed.ghost_on_line(0);
        ed.execute("syntax text").unwrap();
        let as_plain_text = ed.ghost_on_line(0);
        // With the backticks back on the page the first cell is two cells
        // wider, so it cannot want the same padding.
        assert_ne!(
            with_markup_off, as_plain_text,
            "the memo answered for the other syntax"
        );
    }

    // ---- Read-only, and reading again (Features #213 / #214) --------------

    /// #213: one gate, and everything is behind it.
    ///
    /// The point of putting the refusal in `Buffer::insert`/`remove` rather
    /// than in each command is that a path nobody thought about is still
    /// refused. So this presses the ones that reach the rope by different
    /// routes: Insert mode, `x`, `d`, `o` (which goes round the cell guard),
    /// paste, `J`, `Ctrl-A`, `ms`, `:s` — and `u`.
    #[test]
    fn a_locked_buffer_refuses_every_way_in() {
        // Built with `from_text`, not by typing into it: the buffer starts
        // **clean**, so the `is_modified()` check at the end has something to
        // catch. Built by typing, it would already be dirty and the check
        // could not fail however much leaked through.
        const TEXT: &str = "一二三 1\n四五六\n";
        let mut ed = Editor::new();
        ed.add_buffer(crate::Buffer::from_text(TEXT));
        ed.current_buffer_mut().set_readonly(true);
        let before = ed.current_buffer().text();
        assert!(!ed.current_buffer().is_modified(), "clean to begin with");

        press(&mut ed, "i");
        assert_eq!(ed.mode(), Mode::Normal, "Insert mode is not even entered");
        assert!(ed.status().contains("只讀"), "{}", ed.status());
        ed.on_key(Key::Char('甲'));
        assert_eq!(
            ed.current_buffer().text(),
            before,
            "{}",
            ed.current_buffer().text()
        );

        // Each of these is checked twice: once on a buffer that is *not*
        // locked, to prove the keystroke edits at all — a list of keys that do
        // nothing anywhere would pass the locked half and prove nothing — and
        // once on the locked one.
        let ways: [(&str, &[Key]); 11] = [
            ("d", &[Key::Char('d')]),
            ("yp", &[Key::Char('y'), Key::Char('p')]),
            ("o", &[Key::Char('o')]),
            ("O", &[Key::Char('O')]),
            ("a甲", &[Key::Char('a'), Key::Char('甲')]),
            ("c甲", &[Key::Char('c'), Key::Char('甲')]),
            ("i甲", &[Key::Char('i'), Key::Char('甲')]),
            ("gJ", &[Key::Char('g'), Key::Char('J')]),
            ("ms(", &[Key::Char('m'), Key::Char('s'), Key::Char('(')]),
            ("Ctrl-A", &[Key::Ctrl('a')]),
            ("Ctrl-X", &[Key::Ctrl('x')]),
        ];
        for (name, keys) in ways {
            let mut open = Editor::new();
            open.add_buffer(crate::Buffer::from_text(TEXT));
            press(&mut open, "gg");
            for key in keys {
                open.on_key(*key);
            }
            open.on_key(Key::Esc);
            assert_ne!(
                open.current_buffer().text(),
                before,
                "`{name}` does not edit even an unlocked buffer — bad test"
            );

            ed.set_status(String::new());
            press(&mut ed, "gg");
            for key in keys {
                ed.on_key(*key);
            }
            ed.on_key(Key::Esc);
            assert_eq!(
                ed.current_buffer().text(),
                before,
                "`{name}` moved a locked buffer"
            );
        }

        // The commands go the same way, and say which file refused rather than
        // reporting a count of replacements nobody made.
        let mut open = Editor::new();
        open.add_buffer(crate::Buffer::from_text(TEXT));
        assert!(open.execute("s/一/壹/g").is_ok());
        assert_ne!(
            open.current_buffer().text(),
            before,
            "`:s` does not edit even an unlocked buffer — bad test"
        );
        assert!(ed.execute("s/一/壹/g").is_ok());
        assert_eq!(ed.current_buffer().text(), before, "`:s` moved a locked one");
        assert!(ed.status().contains("只讀"), "{}", ed.status());

        // …and going *back* is still moving it.
        press(&mut ed, "u");
        assert_eq!(ed.current_buffer().text(), before);
        assert!(ed.status().contains("只讀"), "{}", ed.status());
        assert!(
            !ed.current_buffer().is_modified(),
            "and nothing marked it changed"
        );

        // …and unlocking gives it all back.
        assert!(ed.execute("readonly off").is_ok());
        press(&mut ed, "ggi");
        assert_eq!(ed.mode(), Mode::Insert, "unlocked, the door opens again");
        ed.on_key(Key::Char('甲'));
        assert!(
            ed.current_buffer().text().starts_with('甲'),
            "{}",
            ed.current_buffer().text()
        );
    }

    /// #213: `o` on a file whose last line has no newline of its own.
    ///
    /// The refusal returns early, so everything the caller worked out about
    /// where the new line would be is now about a line that does not exist.
    /// This is the shape that used to put the cursor one past the end and
    /// panic on the next frame.
    #[test]
    fn a_locked_buffer_refuses_the_line_that_would_have_been_added() {
        for text in ["一二三", "一二三\n"] {
            let mut ed = Editor::new();
            ed.add_buffer(crate::Buffer::from_text(text));
            ed.current_buffer_mut().set_readonly(true);
            for keys in ["Go", "ggO"] {
                press(&mut ed, keys);
                ed.on_key(Key::Esc);
                assert_eq!(ed.current_buffer().text(), text, "{keys} on {text:?}");
                assert!(
                    ed.cursor() <= ed.current_buffer().char_count(),
                    "{keys} on {text:?}: cursor {} past the end {}",
                    ed.cursor(),
                    ed.current_buffer().char_count()
                );
                // The frame is drawn from the cursor, so this is where the
                // panic used to land.
                let _ = ed.render();
            }
        }
    }

    /// #213: a locked buffer must not *take over* a crashed session's draft.
    ///
    /// `:recover` loads the draft as an undoable edit and adopts the swap
    /// file, which is deleted on quit. A refusal that only skipped the edit
    /// would still have adopted — and thrown the work away.
    #[test]
    fn a_locked_buffer_keeps_the_draft_it_cannot_open() {
        let dir = std::env::temp_dir().join(format!(
            "yumete-lock-rec-{}-{}",
            std::process::id(),
            "a_locked_buffer_keeps_the_draft_it_cannot_open"
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.md");
        let swap = dir.join(".chapter.md.yumete");
        std::fs::write(&path, "第一稿\n").unwrap();
        std::fs::write(&swap, "第一稿，還有三千字沒存的\n").unwrap();

        let mut ed = Editor::new();
        ed.execute(&format!(":open {}", path.display())).unwrap();
        ed.current_buffer_mut().set_readonly(true);
        ed.execute(":recover").unwrap();
        assert!(ed.status().contains("只讀"), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), "第一稿\n", "nothing was loaded");
        assert!(swap.exists(), "and the draft is still there to be recovered");

        // Unlock, and it is all still waiting.
        assert!(ed.execute("readonly off").is_ok());
        ed.execute(":recover").unwrap();
        assert!(ed.current_buffer().text().contains("三千字"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// #213: `:readonly` with no word asks rather than sets.
    #[test]
    fn readonly_says_which_way_it_is() {
        let mut ed = Editor::new();
        assert!(ed.execute("readonly").is_ok());
        assert!(ed.status().contains("off"), "{}", ed.status());
        assert!(ed.execute("ro on").is_ok());
        assert!(ed.is_readonly());
        assert!(ed.execute("readonly").is_ok());
        assert!(ed.status().contains("on"), "{}", ed.status());
        // A word that is neither is a mistake, not a toggle.
        assert!(ed.execute("readonly 也許").is_err());
    }

    /// #213: `--readonly` locks the ones already open *and* the next one.
    #[test]
    fn the_readonly_flag_is_about_the_session_not_one_file() {
        let dir = std::env::temp_dir().join(format!("yumete-ro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.md");
        let b = dir.join("b.md");
        std::fs::write(&a, "甲\n").unwrap();
        std::fs::write(&b, "乙\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&a).unwrap();
        ed.set_readonly_default(true);
        assert!(ed.is_readonly(), "the one already open");
        ed.open_file(&b).unwrap();
        assert!(ed.is_readonly(), "and the next one `:open` reaches for");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// #213: the disk's own answer, read at open rather than at `:w`.
    #[cfg(unix)]
    #[test]
    fn a_file_the_disk_calls_read_only_comes_up_locked() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("yumete-ro444-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("別動.md");
        std::fs::write(&file, "這一份不要改\n").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();

        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        assert!(ed.is_readonly(), "read at open, not discovered at :w");
        press(&mut ed, "i");
        ed.on_key(Key::Char('改'));
        assert_eq!(ed.current_buffer().text(), "這一份不要改\n");

        // …and a file that does not exist yet is *unwritten*, not read-only.
        ed.open_file(dir.join("還沒寫.md")).unwrap();
        assert!(!ed.is_readonly());

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).ok();
        std::fs::remove_dir_all(&dir).ok();
    }

    /// #213 × #214: locked is about *editing*. Taking a fresh copy of the file
    /// is the one thing a reader does want.
    #[test]
    fn a_locked_buffer_can_still_be_re_read() {
        let dir = std::env::temp_dir().join(format!("yumete-rorl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("表.txt");
        std::fs::write(&file, "第一版\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        ed.current_buffer_mut().set_readonly(true);
        std::fs::write(&file, "第二版\n").unwrap();
        assert!(ed.execute("reload").is_ok());
        assert_eq!(ed.current_buffer().text(), "第二版\n");
        assert!(ed.is_readonly(), "and it is still locked afterwards");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// #214: `:reload` will not take your afternoon; `:reload!` will, and says
    /// so in its name.
    #[test]
    fn reload_stops_at_unsaved_changes_and_the_bang_does_not() {
        let dir = std::env::temp_dir().join(format!("yumete-rl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("章.md");
        std::fs::write(&file, "原稿\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        press(&mut ed, "i");
        ed.on_key(Key::Char('改'));
        ed.on_key(Key::Esc);
        assert!(ed.current_buffer().is_modified());

        std::fs::write(&file, "外面的版本\n").unwrap();
        assert!(
            ed.execute("reload").is_err(),
            "unsaved changes are in the way"
        );
        assert!(ed.current_buffer().text().contains('改'), "and still here");

        assert!(ed.execute("reload!").is_ok());
        assert_eq!(ed.current_buffer().text(), "外面的版本\n");
        assert!(!ed.current_buffer().is_modified());

        // A buffer with no file has nothing to re-read.
        let mut scratch = Editor::new();
        assert!(scratch.execute("reload").is_err());

        // `:e!` and `:o!` are gone outright — no alias, no hint.
        assert!(ed.execute("e!").is_err());
        assert!(ed.execute("o!").is_err());
        assert!(ed.execute("edit!").is_err());
        assert!(ed.execute("open!").is_err());
        // **Not even with an argument.** `e` and `o` are `open`'s own aliases,
        // and the walk over the six commands that take a bang used to skip
        // `open` — which takes none — and land on `export`, so `:e! 第三章.md`
        // typed by a hand meaning「re-read it」 wrote an export over it.
        assert!(ed.execute("e! 第三章.md").is_err());
        assert!(ed.execute("o! 第三章.md").is_err());
        assert!(!dir.join("第三章.md").exists(), "nothing was written");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// #214: the automatic half takes a clean buffer and never a dirty one.
    #[test]
    fn auto_reload_takes_a_clean_buffer_and_only_warns_about_a_dirty_one() {
        let dir = std::env::temp_dir().join(format!("yumete-rlauto-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("同步中.md");
        std::fs::write(&file, "第一版\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&file).unwrap();
        // Off by default: nothing happens on its own until it is asked for.
        std::fs::write(&file, "第二版\n").unwrap();
        ed.disk_tick();
        assert_eq!(ed.current_buffer().text(), "第一版\n", "off by default");

        assert!(ed.execute("reload auto on").is_ok());
        ed.disk_tick();
        assert_eq!(ed.current_buffer().text(), "第二版\n", "clean, so it reads");
        assert!(ed.status().contains("外面改了"), "{}", ed.status());

        // Now type into it, and let the file move again.
        press(&mut ed, "i");
        ed.on_key(Key::Char('我'));
        ed.on_key(Key::Esc);
        std::fs::write(&file, "第三版\n").unwrap();
        // Re-asking for the setting also resets the clock, which is what a
        // writer who just turned it on means by turning it on.
        assert!(ed.execute("reload auto on").is_ok());
        ed.disk_tick();
        assert!(
            ed.current_buffer().text().contains('我'),
            "a dirty buffer is never read over: {}",
            ed.current_buffer().text()
        );
        // On `:reload!`, not on the wording: these lines are hand-edited in
        // three languages and the command name is the one part of the sentence
        // that is the same in all of them.
        assert!(ed.status().contains(":reload!"), "{}", ed.status());

        assert!(ed.execute("reload auto off").is_ok());
        assert!(!ed.reload_auto());
        assert!(ed.execute("reload auto").is_ok());
        assert!(ed.status().contains("off"), "{}", ed.status());

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
        let before = ed.current_buffer().text();
        assert!(ed.enter_table(), "{}", ed.status());
        // **Looking does not rewrite.** Entering used to lay the whole region
        // out, which marks a file modified for having been read — 45 lines of
        // the author's own documentation, and `:table off` does not undo it.
        assert_eq!(ed.current_buffer().text(), before, "entering changed nothing");
        // `t f` is the tidy-up, said out loud: the columns line up on the
        // terminal, which is what a Markdown table is supposed to look like.
        // (`t t` is 「read this as a grid」 now — one letter, one meaning.)
        press(&mut ed, "tf");
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
        press(&mut ed, "tf");
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
        assert_eq!(ed.current_buffer().text(), "| 字 | 讀音 |\n| --- | --- |\n");
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
        assert_eq!(ed.current_buffer().text(), "前文\n| a | b |\n| --- | --- |");
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
        // 表格操作, so the page is still the manuscript's to set — `t t` turns
        // it horizontal on purpose (#275).
        press(&mut ed, "ti");
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
             [table.link]\nfrom = [\"ids_y\"]\nto = \"char\"\n",
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
             [table.link]\nfrom = [\"ids\"]\nto = \"char\"\n",
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
        ed.execute("reload!").ok();
        let buffers = ed.buffer_count();
        assert!(ed.execute("table check").is_ok());
        assert!(ed.status().contains("沒查出問題"), "{}", ed.status());
        assert_eq!(ed.buffer_count(), buffers, "no buffer for no findings");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_quoted_table_stays_a_quotation_even_when_walked_into() {
        // Refusing at the door was not enough: `gg`, `G`, `:N` and a search
        // all land outside the cells — the manual says so — and from a quoted
        // example in a code fence `t t` reformatted somebody's text.
        let mut ed = typed(
            "| a | b |\n| --- | --- |\n| x | y |\n\n說明：\n\n```\n|字|讀音|\n|--|--|\n```\n",
        );
        ed.goto_line(1);
        assert!(ed.enter_table(), "{}", ed.status());
        let before = ed.current_buffer().text();
        // Walk into the fence the way a search would.
        ed.goto_line(8);
        assert!(ed.md_region().is_none(), "a quotation is not a table");
        // The structural keys are the ones that used to rewrite it. `t` here
        // is vi's till-motion again, and `o` opens an ordinary line.
        press(&mut ed, "tt");
        assert_eq!(ed.current_buffer().text(), before, "{}", ed.status());
        assert!(
            ed.current_buffer().text().contains("|字|讀音|"),
            "the quotation is as it was written"
        );
        // …and back in the real table the keys work.
        ed.goto_line(1);
        assert!(ed.md_region().is_some());
        press(&mut ed, "to");
        assert_ne!(ed.current_buffer().text(), before);
    }

    #[test]
    fn dot_repeats_the_change_and_the_count_it_was_given() {
        // `3>` indents three levels; `.` used to indent one, because the digit
        // key started a fresh recording and threw the count away.
        let mut ed = typed("一\n二\n");
        ed.goto_line(1);
        press(&mut ed, "3>");
        let three = ed.current_buffer().text();
        press(&mut ed, "j");
        ed.on_key(Key::Char('.'));
        let lines: Vec<usize> = ed
            .current_buffer()
            .text()
            .lines()
            .map(|l| l.len() - l.trim_start().len())
            .collect();
        assert_eq!(lines[0], lines[1], "the same three levels: {three:?}");
    }

    #[test]
    fn switching_buffers_is_not_the_change_dot_repeats() {
        // `finish_watching` compared a revision with a *different* buffer's,
        // so after `gn` a pure motion looked like a change and `.` came to
        // mean 「switch buffer」 — the common case at a hundred chapters.
        let mut ed = typed("一\n");
        ed.execute("new").unwrap();
        ed.on_key(Key::Char('i'));
        ed.on_key(Key::Char('甲'));
        ed.on_key(Key::Esc);
        ed.goto_line(1);
        press(&mut ed, ">");
        let indented = ed.current_buffer().text();
        press(&mut ed, "gp");
        press(&mut ed, "gn");
        ed.on_key(Key::Char('.'));
        assert_ne!(ed.current_buffer().text(), indented, "`.` indented again");
        assert!(ed.current_buffer().text().contains("甲"), "…in this buffer");
    }

    #[test]
    fn a_substitution_names_a_line_the_writer_can_find() {
        // The refusal counted rows and printed the number as a *line*, so in a
        // Markdown table 「第 3 行」 named a line the writer could not find —
        // and it counted raw `|`, so it refused a substitution that inserted
        // the escape the manual tells you to use.
        let mut ed = with_md_table();
        assert!(ed.enter_table());
        // `\|` is a pipe inside a cell, so this does not change any row's shape.
        assert!(ed.execute(r"%s/mu/a\|b/").is_ok(), "{}", ed.status());
        assert!(ed.current_buffer().text().contains(r"a\|b"), "{}", ed.status());
        // A bare one does, and the line it names is the document's.
        let before = ed.current_buffer().text();
        assert!(ed.execute("%s/木/木|/").is_ok());
        assert_eq!(ed.current_buffer().text(), before);
        assert!(ed.status().contains("第 4 行"), "{}", ed.status());
    }

    #[test]
    fn a_block_from_a_spreadsheet_goes_in_as_cells() {
        // Every spreadsheet puts tab-separated rows on the clipboard, and a
        // tab was refused outright — so a writer built tables in a spreadsheet
        // and never brought them here.
        let mut ed = typed("| 字 | 音 |\n| --- | --- |\n| a | b |\n");
        ed.goto_line(1);
        assert!(ed.enter_table(), "{}", ed.status());
        ed.goto_line(3);
        ed.set_register_for_test("木\tmu\n目\tmu\n禾\the\n");
        press(&mut ed, "p");
        assert_eq!(
            ed.current_buffer().text(),
            "| 字 | 音 |\n| -- | -- |\n| 木 | mu |\n| 目 | mu |\n| 禾 | he |\n",
            "{}",
            ed.status()
        );
        assert!(ed.status().contains("3×2"), "{}", ed.status());

        // A pipe in a pasted cell goes in as the escape, not as a boundary.
        ed.goto_line(3);
        ed.set_register_for_test("a|b\tc\n");
        press(&mut ed, "p");
        assert_eq!(ed.row_cells(2).len(), 2, "{}", ed.current_buffer().text());
        assert!(ed.current_buffer().text().contains(r"a\|b"));
    }

    #[test]
    fn a_block_too_wide_for_a_schema_is_refused() {
        let (dir, csv) = a_table("block");
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        let width = ed.table().unwrap().schema.columns.len();
        ed.goto_line(2);
        let wide: String = (0..width + 1).map(|i| format!("{i}\t")).collect();
        ed.set_register_for_test(&wide);
        let before = ed.current_buffer().text();
        press(&mut ed, "p");
        assert_eq!(ed.current_buffer().text(), before, "{}", ed.status());
        assert!(ed.status().contains("貼不下"), "{}", ed.status());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn ordinary_text_is_still_pasted_as_text() {
        // One cell is not a block: no tab, no line break.
        let mut ed = typed("| a | b |\n| --- | --- |\n| x | y |\n");
        ed.goto_line(3);
        assert!(ed.enter_table());
        ed.set_register_for_test("春天");
        press(&mut ed, "p");
        assert_eq!(ed.cell_text(2, 0), "春天", "{}", ed.status());
        // …and a paragraph with commas in it is a paragraph, not a grid.
        ed.set_register_for_test("他說，這樣，那樣");
        press(&mut ed, "p");
        assert!(ed.cell_text(2, 0).contains('，'), "{}", ed.current_buffer().text());
    }

    #[test]
    fn a_whole_column_can_be_taken_and_put_back() {
        // Rows had `Y`; a column could only be moved one step at a time.
        let mut ed = typed("| 字 | 音 |\n| --- | --- |\n| 木 | mu |\n| 目 | mo |\n");
        ed.goto_line(1);
        assert!(ed.enter_table(), "{}", ed.status());
        ed.goto_line(4);
        press(&mut ed, "l");
        press(&mut ed, "ty");
        assert!(ed.status().contains("音"), "{}", ed.status());
        // Put it down the other column: one cell to a line, header included.
        press(&mut ed, "h");
        press(&mut ed, "tp");
        assert_eq!(
            ed.current_buffer().text(),
            "| 音 | 音 |\n| -- | -- |\n| mu | mu |\n| mo | mo |\n",
            "{}",
            ed.status()
        );
    }

    #[test]
    fn a_column_search_reads_down_before_across() {
        // `/` reads the page the way a page is read; a table has a second way
        // a document does not have. The only difference is the direction: the
        // pattern is a regex, the match is the selection, `n` walks, it wraps.
        let dir = std::env::temp_dir().join(format!("yumete-colsearch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("d.csv");
        // Two columns. Read across, 甲 comes at row 1 then row 2; read down,
        // both of column A come before either of column B.
        std::fs::write(&csv, "a,b\n甲一,甲二\n甲三,甲四\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.enter_table(), "{}", ed.status());

        assert!(ed.execute("search column 甲").is_ok(), "{}", ed.status());
        assert!(ed.status().contains("1/4"), "{}", ed.status());
        // The hits are *shown* in the other work area; the cursor stays where
        // it was standing, which is the point of the split (Feature #176).
        let where_am_i = |ed: &Editor| {
            let at = ed.other_pane().expect("the other work area").cursor();
            let line = ed.current_buffer().rope().char_to_line(at);
            let cell = ed
                .row_cells(line)
                .iter()
                .position(|&(from, to)| {
                    let start = ed.current_buffer().rope().line_to_char(line);
                    at >= start + from && at <= start + to
                })
                .unwrap_or(0);
            (line, cell)
        };
        assert_eq!(where_am_i(&ed), (1, 0), "column a, row 1");
        ed.on_key(Key::Char('n'));
        assert_eq!(where_am_i(&ed), (2, 0), "column a, row 2 — still column a");
        ed.on_key(Key::Char('n'));
        assert_eq!(where_am_i(&ed), (1, 1), "only now column b");
        ed.on_key(Key::Char('n'));
        assert_eq!(where_am_i(&ed), (2, 1));
        ed.on_key(Key::Char('n'));
        assert_eq!(where_am_i(&ed), (1, 0), "and it wraps");

        // A row search is `/`, and reads the other way.
        assert!(ed.execute("search row 甲").is_ok());
        assert_eq!(ed.cursor_line(), 1);

        // No direction means row, because that is what a search is anywhere
        // but a table.
        assert!(ed.execute("search 甲").is_ok());
        // A pattern is a pattern in both directions.
        assert!(ed.execute("search column 甲[一三]").is_ok());
        assert!(ed.status().contains("1/2"), "{}", ed.status());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// `J K H L` and `C-d`/`C-u` mean "several rows" in a table, and a table's
    /// rows are not the same width twice — so aiming at a *character* column,
    /// which is what the page motions do everywhere else, drifts sideways as
    /// it goes. The author reported it as 「in column 5, press J, land in
    /// column 10」.
    #[test]
    fn paging_through_a_table_keeps_to_the_column() {
        let dir = std::env::temp_dir().join(format!("yumete-page-column-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("wide.csv");
        // Deliberately ragged, and by a lot: the first field swings between 1
        // and 19 characters, so the character offset of column c on one row is
        // past the end of another row entirely.
        let mut text = String::from("a,b,c,d\n");
        for i in 0..40 {
            let wide = "x".repeat(1 + (i % 7) * 3);
            text.push_str(&format!("{wide},b{i},c{i},d{i}\n"));
        }
        std::fs::write(&csv, &text).unwrap();
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.set_page(8, 8);
        assert!(ed.enter_table(), "{}", ed.status());
        // Row 7 is the widest of the seven; half a page on is one of the
        // narrowest, whose whole line is shorter than where column c starts
        // here.
        ed.goto_line(7);
        press(&mut ed, "ll");
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "column c");
        let row = ed.cursor_line();
        ed.on_key(Key::Char('J'));
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "still column c");
        assert!(ed.cursor_line() > row + 1, "and it moved several rows");
        ed.on_key(Key::Char('K'));
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "and coming back");
        assert_eq!(ed.cursor_line(), row, "to the row it started on");
        ed.on_key(Key::Ctrl('d'));
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "C-d is the same motion");
        ed.on_key(Key::Char('L'));
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "and a whole page of it");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The same motions in a `|` table, which is the shape they can walk
    /// **out** of: a grid is the whole file and clamps at its last line, but a
    /// Markdown table has prose under it, and a page is longer than most
    /// tables anybody writes.
    #[test]
    fn paging_through_a_pipe_table_stops_at_its_last_row() {
        let mut ed = Editor::new();
        let mut text = String::from("前文\n\n| a | b | c |\n| --- | --- | --- |\n");
        for i in 0..6 {
            // Ragged again, so keeping the column is a real claim and not the
            // accident of every row being the same width.
            text.push_str(&format!("| {} | b{i} | c{i} |\n", "x".repeat(1 + i * 4)));
        }
        text.push_str("\n後文\n");
        ed.current_buffer_mut().insert(0, &text);
        ed.set_page(8, 8);
        ed.goto_line(5);
        assert!(ed.enter_table(), "{}", ed.status());
        press(&mut ed, "ll");
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "column c");

        let region = ed.md_region().expect("in the table");
        // Pages down, and keeps going: six rows is shorter than a page, so
        // this runs off the end — and stops on the last row, in its column,
        // rather than carrying on into the prose under the table.
        for _ in 0..3 {
            ed.on_key(Key::Char('J'));
            assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "still column c");
        }
        assert_eq!(ed.cursor_line(), region.last, "and stopped at the last row");

        // …and up again, which stops at the header rather than in the blank
        // line above it. `C-f` and `C-b` are the same motion by another name.
        for _ in 0..3 {
            ed.on_key(Key::Char('K'));
            assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "coming back");
        }
        assert_eq!(ed.cursor_line(), region.first, "the header is the top");
        for _ in 0..3 {
            ed.on_key(Key::Ctrl('f'));
        }
        assert_eq!(ed.cursor_line(), region.last, "C-f pages the same way");
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2));
        for _ in 0..3 {
            ed.on_key(Key::Ctrl('b'));
        }
        assert_eq!(ed.cursor_line(), region.first, "and C-b back");

        // `L` and `H` are the whole page rather than half of it, and page the
        // same way — down and up the rows, not across the columns.
        ed.on_key(Key::Char('L'));
        assert_eq!(ed.cursor_line(), region.last, "a whole page down");
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2), "in its column");
        ed.on_key(Key::Char('H'));
        assert_eq!(ed.cursor_line(), region.first, "and a whole page back");
        assert_eq!(ed.cell_position().map(|(_, c)| c), Some(2));
    }

    #[test]
    fn a_table_with_no_declared_scope_searches_all_of_it() {
        // It used to refuse: 「這張表沒說哪些是拆分欄」. A schema is an
        // optimisation — two columns instead of twenty-eight — not a licence.
        let dir = std::env::temp_dir().join(format!("yumete-scope-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("d.csv");
        std::fs::write(&csv, "a,b\n甲,乙\n丙,甲\n").unwrap();
        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.enter_table(), "{}", ed.status());
        ed.goto_line(2);
        press(&mut ed, "t?");
        assert_eq!(ed.peeked_line(), Some(1), "{}", ed.status());
        assert!(ed.status().contains("未指定"), "and it says so: {}", ed.status());
        assert!(ed.status().contains("1/2"), "{}", ed.status());
        ed.on_key(Key::Char('n'));
        // Down the first column and then down the second: the second hit is
        // the 甲 in row 2's *other* column.
        assert_eq!(ed.peeked_line(), Some(2), "the other column");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// #275. This used to assert that a `|` table is **never** drawn as a
    /// grid — 「a document keeps its layout and its page」 — which was the
    /// editor having only two of the author's three modes and giving the
    /// middle one the top one's key. Both are true now, and which one you get
    /// is which key you pressed.
    #[test]
    fn a_pipe_table_is_drawn_as_a_grid_only_when_that_is_the_key_pressed() {
        // `t i` — 表格操作: the syntax stays on the page, the keys are the
        // grid's, and a 縱書 manuscript is still 縱書.
        let mut ed = with_md_table();
        ed.set_layout(Layout::Vertical);
        press(&mut ed, "ti");
        assert_eq!(ed.layout(), Layout::Vertical, "{}", ed.status());
        assert!(ed.table().unwrap().in_prose(), "{}", ed.status());
        assert!(!ed.table().unwrap().takes_the_pane(), "three lines, not the pane");

        // `t t` — 真表格顯示: 「照舊把整頁轉橫」, because a grid is read across.
        press(&mut ed, "tt");
        assert!(ed.table().unwrap().is_page(), "{}", ed.status());
        assert_eq!(ed.layout(), Layout::Horizontal, "{}", ed.status());
        assert!(!ed.table().unwrap().takes_the_pane(), "still three lines of a chapter");

        // And the two switch straight into one another — `t i` is the middle
        // mode, not the way out — which is what gives the page back.
        press(&mut ed, "ti");
        assert!(ed.table().unwrap().in_prose(), "{}", ed.status());
        assert_eq!(ed.layout(), Layout::Vertical, "{}", ed.status());

        // `t q` is the one way back to prose.
        press(&mut ed, "tq");
        assert!(ed.table().is_none(), "{}", ed.status());
    }

    #[test]
    fn only_the_drawn_mode_draws_the_grid() {
        // 「完全画成表格」 — and only there. 表格操作 keeps 「markdown/csv 的语法
        // 标记」 on the page, which is the whole difference between the two.
        let mut ed = with_md_table();
        press(&mut ed, "ti");
        assert!(ed.grid_on_line(1).is_empty(), "the pipes stay pipes");
        assert!(ed.table_ruler_on_line(1).is_empty(), "and no ruler over them");

        press(&mut ed, "tt");
        let head: Vec<char> = ed.grid_on_line(1).into_iter().map(|(_, g)| g).collect();
        assert_eq!(head, vec!['┆', '┆', '┆'], "| 字 | 讀音 |");
        // The rule row is not a row — it is the line under the head, and every
        // character of it is drawn, the file's spaces included.
        let rule: String = ed.grid_on_line(2).into_iter().map(|(_, g)| g).collect();
        assert_eq!(rule, "├┄┄┄┄┄┼┄┄┄┄┄┤", "{}", ed.status());
        assert!(ed.grid_rule_row(2));
        assert!(!ed.grid_rule_row(1));

        // 「畫，貼在表格上緣」: one ruler, over the first line and no other.
        assert_eq!(ed.table_ruler_on_line(1).len(), 2, "two columns, two numbers");
        for line in [0, 2, 3, 4, 5] {
            assert!(ed.table_ruler_on_line(line).is_empty(), "line {line}");
        }
        // The prose around it is prose.
        assert!(ed.grid_on_line(0).is_empty());
        assert!(ed.grid_on_line(5).is_empty());
    }

    #[test]
    fn every_table_in_a_markdown_file_is_drawn_at_once() {
        // 「對於這個文件中所有的表格都生效」 — and each draws its own ruler,
        // because the numbers over one table are that table's columns.
        let mut ed = typed("| a | b |\n| --- | --- |\n| 1 | 2 |\n\n中間\n\n| c | d | e |\n| --- | --- | --- |\n| 3 | 4 | 5 |\n");
        ed.current_buffer_mut().set_syntax(crate::syntax::Syntax::Markdown);
        ed.goto_line(5);
        assert!(ed.enter_table(), "{}", ed.status());
        assert_eq!(ed.table_ruler_on_line(0).len(), 2, "{}", ed.status());
        assert_eq!(ed.table_ruler_on_line(6).len(), 3, "the second table");
        assert!(!ed.grid_on_line(8).is_empty(), "its last row");
        // The paragraph between them is still a paragraph.
        assert!(ed.grid_on_line(4).is_empty(), "中間");
        assert!(ed.table_ruler_on_line(4).is_empty());
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
             [table.link]\nfrom = ['ids_y']\nto = 'char'\n",
        )
        .unwrap();
        let csv = dir.join("d.csv");
        std::fs::write(&csv, "char,ids_y\n相,⿰木目\n木,木\n目,目\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.execute("2").unwrap();
        press(&mut ed, "l");

        // `gd` goes and leaves a way back; `gw` shows and leaves nothing,
        // because nothing was left.
        ed.on_key(Key::Tab);
        press(&mut ed, "l");
        let was = ed.cursor();
        press(&mut ed, "gw");
        assert_eq!(ed.cursor(), was, "gw does not move you");
        press(&mut ed, "gd");
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

        // A search is a link: you look something up and you want to be back
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
        assert_eq!(ed.prompt(), Some((":", "pipe ")));

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
    fn a_table_with_no_schema_gets_one_written_beside_it() {
        // #218. The author's own 碼表 has no header and a tab between its two
        // columns, and both facts have to survive into the file.
        let dir = std::env::temp_dir().join(format!("yumete-schema-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let codes = dir.join("codes.txt");
        std::fs::write(&codes, "雪\txue\n月\tyue\n語\tyu\n星\txing\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&codes).unwrap();
        assert!(ed.enter_table(), "a tab is a delimiter");
        assert!(ed.table().unwrap().from.as_os_str().is_empty(), "nobody's schema yet");

        // 「第一行是資料」 (#217), and then 「說出來」 (#218).
        ed.on_key(Key::Char('t'));
        ed.on_key(Key::Char('H'));
        ed.on_key(Key::Char('t'));
        ed.on_key(Key::Char('e'));

        let written = dir.join(".yumete").join("tables").join("codes.toml");
        let text = std::fs::read_to_string(&written).expect("a schema was written beside the data");
        assert!(text.contains("file = \"codes.txt\""), "{text}");
        assert!(text.contains("delimiter = \"\\t\""), "the tab is escaped, not typed: {text}");
        assert!(text.contains("header = false"), "what t H just said: {text}");
        assert_eq!(text.matches("[[table.column]]").count(), 2, "{text}");

        // It is open in the other half, and the keys did not go with it.
        let pane = ed.other_pane().expect("the schema is in the other area");
        assert_eq!(pane.caption, "codes.toml");
        assert_eq!(
            ed.buffers[ed.buffer_with(pane.buffer).unwrap()].path(),
            Some(written.as_path()),
            "the pane names the schema"
        );
        assert_eq!(ed.current_buffer().path(), Some(codes.as_path()), "still on the table");
        assert_eq!(ed.live_pane(), 0);

        // And what it says is what was already on screen: reading the file
        // again finds it and changes nothing.
        ed.leave_table();
        assert!(ed.enter_table());
        let view = ed.table().unwrap();
        assert_eq!(view.from, written, "the schema claims the file now");
        assert!(!view.schema.header, "still 「第一行是資料」");
        assert_eq!(view.schema.delimiter, '\t');
        assert_eq!(view.schema.columns.len(), 2);

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

    /// `Space d` asks about the character under the cursor — Feature #215.
    ///
    /// The editor cannot answer: the 拆分表 lives in yume, which only the
    /// front end holds. So what is checked here is the half the editor owns —
    /// the question is parked, the panel is open on it, and the answer, when
    /// it comes, is laid out in columns.
    #[test]
    fn the_dictionary_asks_about_the_character_under_the_cursor() {
        let mut ed = typed("那年冬天");
        ed.on_key(Key::Char(' '));
        ed.on_key(Key::Char('d'));

        assert_eq!(ed.sidebar().map(|s| s.view()), Some(crate::sidebar::View::Dictionary));
        assert!(ed.sidebar_focused(), "asked from the page, the keys go along");
        assert_eq!(ed.take_dictionary_query(), Some('那'));
        assert_eq!(ed.take_dictionary_query(), None, "asked once, answered once");

        // Until the answer arrives the panel is the character alone — not an
        // empty panel, and not last character's answer.
        assert_eq!(ed.sidebar().unwrap().rows().len(), 1);

        ed.set_dictionary(
            '那',
            vec![
                ("拆分".to_string(), "刀二阝".to_string()),
                ("編碼".to_string(), "vfb".to_string()),
            ],
        );
        let rows: Vec<String> = ed
            .sidebar()
            .unwrap()
            .rows()
            .iter()
            .map(|r| r.name.clone())
            .collect();
        assert_eq!(rows[0], "那");
        assert_eq!(rows[1], "拆分  刀二阝");
        assert_eq!(rows[2], "編碼  vfb", "names are padded to a column");

        // `Tab` cannot walk into it, and walking out of it comes back to the
        // tree rather than to a fourth view nobody asked for.
        let mut ed = ed;
        ed.on_key(Key::Tab);
        assert_eq!(ed.sidebar().map(|s| s.view()), Some(crate::sidebar::View::Explorer));
    }

    /// 「查不到」and「還沒問」are different findings.
    #[test]
    fn a_character_the_table_has_nothing_for_says_so() {
        let mut ed = typed("那年冬天");
        ed.on_key(Key::Char(' '));
        ed.on_key(Key::Char('d'));
        ed.take_dictionary_query();
        ed.set_dictionary('那', Vec::new());
        let rows = ed.sidebar().unwrap().rows();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(rows[1].name, say!("ui.not-in-the-table"));
    }

    /// The answer to last frame's question must not overwrite this frame's.
    ///
    /// A reader walking `l l l` with the panel open asks three times before
    /// the first answer is back; the panel has to end up showing the character
    /// the cursor is actually on.
    #[test]
    fn an_answer_for_a_character_nobody_is_asking_about_now_is_dropped() {
        let mut ed = typed("那年冬天");
        ed.on_key(Key::Char(' '));
        ed.on_key(Key::Char('d'));
        ed.take_dictionary_query();
        ed.look_up('年', true);
        ed.set_dictionary('那', vec![("拆分".to_string(), "刀二阝".to_string())]);
        assert_eq!(ed.dictionary().map(|(ch, _)| ch), Some('年'));
        assert_eq!(ed.sidebar().unwrap().rows().len(), 1, "still waiting");
    }

    /// #222: how far a notch of the wheel moves is the reader's, not a
    /// `const` in the front end that nobody can reach.
    #[test]
    fn the_wheel_step_can_be_said_and_asked_about() {
        let mut ed = typed("那年冬天，山下起了大雪。");
        assert_eq!(ed.wheel_step(), 3, "three, as a terminal scrolls three");

        // Asking is a use of its own: the number may come from a config file
        // the reader never wrote.
        ed.execute("wheel").unwrap();
        assert!(ed.status().contains('3'), "{}", ed.status());
        assert_eq!(ed.wheel_step(), 3, "asking changes nothing");

        ed.execute("wheel 1").unwrap();
        assert_eq!(ed.wheel_step(), 1);

        // Zero is the terminal's own step — one unit a notch — and not 「do
        // not scroll」, which is a setting nobody wants and which would be
        // indistinguishable from a broken mouse.
        ed.execute("wheel 0").unwrap();
        assert_eq!(ed.wheel_step(), 1);

        assert!(ed.execute("wheel 三").is_err(), "a word is not a number");
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

    /// 縱書 has no 折行 to turn off, and says so.
    ///
    /// A 縱 is broken by the height of the window. `:wrap off` used to be
    /// taken there and answered 「長段落跑出右邊」 — a right edge this page
    /// does not have — while changing nothing the reader could see.
    #[test]
    fn wrap_on_and_off_are_refused_in_vertical_and_say_why() {
        let mut ed = typed("那年冬天，山下起了大雪。");
        ed.execute("layout vertical").unwrap();
        assert!(ed.soft_wrap(), "on, as it always is");

        ed.execute("wrap off").unwrap();
        assert!(ed.soft_wrap(), "…and untouched: there was nothing to turn");
        assert!(ed.status().contains("縱書不折行"), "{}", ed.status());

        // The measure is a different question, and it *is* answered here.
        ed.execute("wrap 12").unwrap();
        assert_eq!(ed.zong_length(), 12);

        // Horizontally it works as it always did.
        ed.execute("layout horizontal").unwrap();
        ed.execute("wrap off").unwrap();
        assert!(!ed.soft_wrap());
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
        assert_eq!(ed.prompt(), Some((":", "grep ")));
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
        assert_eq!(ed.prompt(), Some(("/", "那年冬天")));
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

        // `:buffer list` opens the picker: with 122 chapters open the list is
        // 1,783 characters and the status line is one row.
        ed.execute(":buffer list").unwrap();
        assert_eq!(ed.mode(), Mode::Picker);
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

    /// **A write is addressed by identity, not by spelling.**
    ///
    /// `main.typ`, `./main.typ`, the absolute path and a symlink pointing at it
    /// are one manuscript. The export guard used to compare `PathBuf`s, so
    /// three of those four spellings walked past it and 90,000 characters of a
    /// book became an export of themselves.
    /// `.` repeats **the edit you just made**, whatever characters it holds.
    ///
    /// The abort guard used to scan the whole recorded sequence for `.`, `u`,
    /// `q` and `:` — which are the *commands* that must not become a
    /// definition — and an insertion is a sequence of typed characters. So
    /// `i` `3` `.` `1` `4` Esc was refused as a definition, and `.` afterwards
    /// silently replayed an older edit into the document.
    /// **A row's cell count does not change**, and not only at the gate.
    ///
    /// Four writers reached the rope without passing one: `gJ`, `:replace`,
    /// `:s` and `:ruby format`. Three of the four asked `self.table` first, so
    /// they were off in exactly the state a `|` table in a manuscript is
    /// normally edited in — nobody types `:table on` to fix a typo in their own
    /// documentation.
    #[test]
    fn no_writer_changes_how_many_cells_a_row_has() {
        let table = "# 標題\n\n| 鍵 | 拆分 |\n| --- | --- |\n| 木 | 木 |\n| 林 | ⿰木木 |\n\n後面一段。\n";

        // `gJ` on a table row, with table mode never turned on.
        let mut ed = typed(table);
        press(&mut ed, "gg");
        for _ in 0..4 {
            press(&mut ed, "j");
        }
        assert!(ed.current_buffer().line(4).unwrap().starts_with("| 木"));
        press(&mut ed, "gJ");
        assert!(ed.status().contains("欄數"), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), table, "the grid is untouched");

        // …and one line *above* the table: joining a paragraph onto the header
        // gives the header the paragraph's zero cells.
        press(&mut ed, "gg");
        press(&mut ed, "j");
        press(&mut ed, "gJ");
        assert!(ed.status().contains("欄數"), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), table);

        // `:s` and `:replace` put a bare `|` into a cell.
        let mut ed = typed(table);
        ed.execute(":%s/木/a|b/g").ok();
        assert_eq!(ed.current_buffer().text(), table, "{}", ed.status());

        // …while a `|` in the prose around it is just a character.
        let mut ed = typed(table);
        ed.execute(":%s/後面/前 | 後/g").unwrap();
        assert!(ed.current_buffer().text().contains("前 | 後"), "{}", ed.status());

        // …and a `|` inside a fence is writing about a table, not a table.
        let quoted = "```\n| 鍵 | 拆分 |\n```\n那年冬天。\n";
        let mut ed = typed(quoted);
        ed.execute(":%s/冬天/冬 | 天/g").unwrap();
        assert!(ed.current_buffer().text().contains("冬 | 天"), "{}", ed.status());
    }

    /// `:ruby format` rewrites as much text as `:replace` and kept none of its
    /// rules: `#ruby("永", "ㄩㄥˇ")` carries a comma into every cell it touches.
    #[test]
    fn reformatting_the_readings_does_not_reshape_a_grid() {
        let dir = std::env::temp_dir().join(format!("yumete-rubygrid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("tables").join("t.toml"),
            "[table]\nfile = ['d.csv']\nkey = 'char'\n\
             [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'note'\n",
        )
        .unwrap();
        let csv = dir.join("d.csv");
        let source = "char,note\n永,<ruby>永<rt>ㄩㄥˇ</rt></ruby>\n和,平\n";
        std::fs::write(&csv, source).unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        ed.execute(":ruby format typst").unwrap();
        assert!(ed.status().contains("格"), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), source, "the grid is untouched");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// A reading and a picker's query are typed text, and typed text is edited
    /// in the middle.
    #[test]
    fn every_prompt_has_a_caret() {
        // Ruby mode: type a reading, go back into it, fix it.
        let mut ed = typed("那年冬天。\n");
        press(&mut ed, "gg");
        ed.on_key(Key::Char('v'));
        ed.execute(":ruby").unwrap();
        assert_eq!(ed.mode(), Mode::Ruby, "{}", ed.status());
        for c in "hàn".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Left);
        ed.on_key(Key::Left);
        ed.on_key(Key::Char('X'));
        assert_eq!(ed.prompt().map(|(_, line)| line), Some("hXàn"));
        ed.on_key(Key::Home);
        ed.on_key(Key::Char('Z'));
        assert_eq!(ed.prompt().map(|(_, line)| line), Some("ZhXàn"));
        assert_eq!(ed.prompt_before_caret(), "Z");
        ed.on_key(Key::Esc);

        // The picker's query, the same way.
        let mut ed = typed("那年冬天。\n");
        ed.open_buffer_picker();
        for c in "abc".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Left);
        ed.on_key(Key::Backspace);
        assert_eq!(ed.picker().map(|p| p.query()), Some("ac"));
        ed.on_key(Key::Home);
        ed.on_key(Key::Delete);
        assert_eq!(ed.picker().map(|p| p.query()), Some("c"));
        assert_eq!(ed.picker().map(|p| p.caret()), Some(0));
    }

    /// A preview server is a running thing: `:preview` while one is up asks
    /// *where* it is, not for a second one.
    /// `:help` is written from what the editor actually runs on.
    /// `:markdown` writes the pieces a manuscript keeps needing.
    #[test]
    fn markdown_writes_a_footnote_and_a_table() {
        let mut ed = typed("那年冬天[^2]，山下起了大雪。\n\n[^2]: 據縣志。\n");
        press(&mut ed, "gg");
        press(&mut ed, "llll");

        // **The next free number**, not one more than the last: 2 is taken.
        ed.execute(":markdown footnote").unwrap();
        let text = ed.current_buffer().text();
        assert!(text.contains("[^1]"), "{text}");
        assert!(text.contains("[^1]: "), "and its note is opened: {text}");
        assert_eq!(ed.mode(), Mode::Insert, "the cursor is in the note");
        // Typing goes into the note, not into the sentence.
        type_keys(&mut ed, "說法不一");
        assert!(ed.current_buffer().text().contains("[^1]: 說法不一"));
        ed.on_key(Key::Esc);

        // An inline note leaves the cursor between the brackets.
        let mut ed = typed("那年冬天。\n");
        press(&mut ed, "gg");
        ed.execute(":markdown footnote inline").unwrap();
        assert_eq!(ed.mode(), Mode::Insert);
        type_keys(&mut ed, "存疑");
        assert!(ed.current_buffer().text().starts_with("^[存疑]"), "{}", ed.current_buffer().text());

    }

    /// #276. The author, 2026-09-05: 「`:table new 3 4`，迅速在 markdown 中插入
    /// 一個三行四列表格，上下有空白行，光標自動到標題欄最左的一格並進去編輯模
    /// 式。」 It used to be `:markdown table 4x3` — columns first, rows meaning
    /// *data* rows, no blank lines and no Insert mode — and that spelling is
    /// gone rather than kept beside this one.
    #[test]
    fn a_new_table_is_written_with_room_around_it_and_typed_into() {
        let mut ed = typed("前文。\n後文。\n");
        press(&mut ed, "gg");
        ed.execute(":table new 3 4").unwrap();
        let text = ed.current_buffer().text();
        let lines: Vec<&str> = text.lines().collect();
        let rows: Vec<&str> = lines.iter().copied().filter(|l| l.starts_with('|')).collect();
        assert_eq!(rows.len(), 4, "a heading, a rule and two more rows: {text}");
        assert_eq!(rows[0].matches('|').count(), 5, "four columns: {text}");
        assert!(rows[1].contains("---"), "{text}");

        // 「上下有空白行」 — and the prose is still on both sides of it.
        let first = lines.iter().position(|l| l.starts_with('|')).unwrap();
        let last = lines.iter().rposition(|l| l.starts_with('|')).unwrap();
        assert_eq!(lines[0], "前文。", "{text}");
        assert!(lines[first - 1].trim().is_empty(), "a blank line above: {text}");
        assert!(lines[last + 1].trim().is_empty(), "a blank line below: {text}");
        assert!(lines.contains(&"後文。"), "the prose under it is still there: {text}");

        // 「光標自動到標題欄最左的一格並進去編輯模式」
        assert_eq!(ed.mode(), Mode::Insert, "typing goes straight in");
        type_keys(&mut ed, "字");
        let text = ed.current_buffer().text();
        let heading = text.lines().find(|l| l.starts_with('|')).unwrap();
        // The row is padded as it is typed (#212), so the cell is asked for
        // rather than the spelling of the line.
        let cells = crate::mdtable::split(heading);
        assert_eq!(cells[0].trim(), "字", "the first heading took it: {heading}");
        assert!(cells[1].trim().is_empty(), "and only it: {heading}");

        // Standing on a blank line uses it rather than pushing one more in.
        let mut ed = typed("前文。\n\n後文。\n");
        press(&mut ed, "gg");
        press(&mut ed, "j");
        ed.execute(":table new 2 2").unwrap();
        let text = ed.current_buffer().text();
        assert!(!text.contains("\n\n\n"), "no line the writer did not ask for: {text:?}");

        // Two numbers, and only sane ones.
        let mut ed = typed("前文。\n");
        assert!(ed.execute(":table new 0 4").is_err());
        assert!(ed.execute(":table new 4 99").is_err());
        // On its own it is a small one rather than an error.
        assert!(ed.execute(":table new").is_ok());
    }

    #[test]
    fn help_is_the_editor_describing_itself() {
        let mut ed = typed("那年冬天。\n");
        ed.execute(":help").unwrap();
        let text = ed.current_buffer().text();
        assert!(ed.buffer_name().contains("help"), "{}", ed.buffer_name());
        // Every key the Space menu declares is in it, because it is *made* of
        // that list rather than written beside it.
        for (key, _) in Editor::SPACE_KEYS {
            assert!(text.contains(&format!("空格 {key}")), "空格 {key} missing");
        }
        assert!(text.contains("g/") && text.contains("gd"), "{text}");

        // …and the command section is the parser's own table.
        ed.execute(":help commands").unwrap();
        let text = ed.current_buffer().text();
        for entry in crate::command::COMMANDS {
            assert!(text.contains(&format!(":{}", entry.name)), "{} missing", entry.name);
        }

        // A section that does not exist says which ones do.
        ed.execute(":help 火星文").unwrap();
        assert!(ed.status().contains("chinese"), "{}", ed.status());
    }

    #[test]
    fn a_second_preview_asks_where_the_first_one_is() {
        let dir = std::env::temp_dir().join(format!("yumete-prev-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let book = dir.join("book.typ");
        std::fs::write(&book, "= 第一章\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&book).unwrap();
        ed.execute(":preview").unwrap();
        assert!(
            matches!(ed.take_preview_request(), Some(Preview::Start { .. })),
            "the first one starts a typesetter"
        );
        // The front end says where it put the page.
        ed.set_preview_at(Some("http://127.0.0.1:23625".to_string()));
        assert_eq!(ed.preview_at(), Some("http://127.0.0.1:23625"));

        ed.execute(":preview").unwrap();
        assert!(
            matches!(ed.take_preview_request(), Some(Preview::Show)),
            "the second one asks for the address, not for another server"
        );

        ed.execute(":preview off").unwrap();
        assert!(matches!(ed.take_preview_request(), Some(Preview::Stop)));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_full_stop_typed_into_the_text_is_still_an_edit() {
        let mut ed = typed("第一行。\n第二行。\n");
        press(&mut ed, "gg");
        // An edit with a `.` in it — a decimal, as in a 拆分表 cell.
        press(&mut ed, "i");
        type_keys(&mut ed, "3.14");
        ed.on_key(Key::Esc);
        // …and one with a `u` and a `q` in it, which are the other two.
        press(&mut ed, "j");
        press(&mut ed, "i");
        type_keys(&mut ed, "qu");
        ed.on_key(Key::Esc);
        assert!(ed.current_buffer().text().contains("qu"));

        // `.` repeats *that*, not something from earlier in the session.
        press(&mut ed, "j");
        ed.on_key(Key::Char('.'));
        let text = ed.current_buffer().text();
        assert_eq!(text.matches("qu").count(), 2, "{text}");
        assert_eq!(text.matches("3.14").count(), 1, "{text}");

        // And the three keystrokes that used to abort the process still do not
        // define anything: `.` after `3.` repeats the insertion, once more.
        press(&mut ed, "3");
        ed.on_key(Key::Char('.'));
        let text = ed.current_buffer().text();
        assert!(text.matches("qu").count() > 2, "{text}");
    }

    /// What a path command **did** is what it says it did.
    ///
    /// `:w copy.md` writes a copy and leaves the chapter unsaved. The caller
    /// used to find that out by sniffing the rendered status line for a Chinese
    /// character — which in English reported 「saved ch1.md」 about a chapter
    /// that had not been saved. It is a value now, not a string.
    #[test]
    fn a_path_command_says_what_it_actually_did() {
        let dir = std::env::temp_dir().join(format!("yumete-said-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let chapter = dir.join("ch1.md");
        std::fs::write(&chapter, "第一稿\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&chapter).unwrap();
        press(&mut ed, "i");
        type_keys(&mut ed, "改");
        ed.on_key(Key::Esc);

        let copy = dir.join("copy.md");
        assert_eq!(
            ed.write_forcing(Some(&copy.display().to_string()), false)
                .unwrap(),
            Wrote::Copied(copy.clone()),
            "a copy is a copy, whatever language the line is in"
        );
        // The chapter is still unsaved, so the line may not say it is.
        assert!(ed.current_buffer().is_modified());
        assert!(!ed.status().contains("ch1.md"), "{}", ed.status());

        // …and the save that follows says so, rather than leaving the copy's
        // message standing.
        assert_eq!(ed.write_forcing(None, false).unwrap(), Wrote::Saved);
        assert!(ed.status().contains("ch1.md"), "{}", ed.status());
        assert!(!ed.current_buffer().is_modified());

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `:wq <名字>` saves the file it names and quits — it does not write a
    /// copy and then refuse to leave.
    #[test]
    fn write_quit_with_a_name_saves_that_name() {
        let dir = std::env::temp_dir().join(format!("yumete-wqn-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let chapter = dir.join("ch1.md");
        std::fs::write(&chapter, "第一稿\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&chapter).unwrap();
        press(&mut ed, "i");
        type_keys(&mut ed, "改");
        ed.on_key(Key::Esc);
        let out = dir.join("ch1-final.md");
        assert_eq!(
            ed.execute(&format!(":wq {}", out.display())).unwrap(),
            CommandOutcome::Quit,
            "{}",
            ed.status()
        );
        assert!(std::fs::read_to_string(&out).unwrap().contains('改'));
        assert!(!ed.current_buffer().is_modified());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A substitution that would reshape a grid names a way through, and the
    /// way through works.
    #[test]
    fn the_grid_refusal_names_a_way_through() {
        let table = "| 鍵 | 拆分 |\n| --- | --- |\n| 木 | 木 |\n";
        let mut ed = typed(table);
        ed.execute(":%s/拆分/拆分 | 註/").ok();
        assert!(ed.status().contains("t 旗標"), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), table);
        // …and with the flag it goes through.
        ed.execute(":%s/拆分/拆分 | 註/t").unwrap();
        assert!(ed.current_buffer().text().contains("拆分 | 註"), "{}", ed.status());
    }

    /// A paste the cell refuses says so — in Insert as well as in Normal.
    ///
    /// The text is a bare `|`, not the tab-separated block this test used to
    /// paste: since #226 a block is not refused, it is a grid.
    #[test]
    fn a_refused_paste_does_not_claim_to_have_happened() {
        let mut ed = typed("| 字 | 說明 |\n| --- | --- |\n| 木 | 樹 |\n");
        ed.goto_line(3);
        assert!(ed.enter_table(), "{}", ed.status());
        let before = ed.current_buffer().text();
        press(&mut ed, "i");
        ed.paste_text("甲 | 乙");
        assert_eq!(ed.current_buffer().text(), before, "{}", ed.status());
        assert!(!ed.status().contains("貼了"), "{}", ed.status());
    }

    /// A spreadsheet's clipboard lands as rows, and widens the table (#226).
    #[test]
    fn a_pasted_spreadsheet_becomes_rows() {
        let mut ed = typed("| 字 | 說明 |\n| --- | --- |\n| 木 | 樹 |\n");
        ed.goto_line(3);
        assert!(ed.enter_table(), "{}", ed.status());
        press(&mut ed, "i");
        // Two rows, three columns, into a table two columns wide: the third
        // column is written rather than refused.
        ed.paste_text("甲\t乙\t丙\n丁\t戊\t己\n");
        let text = ed.current_buffer().text();
        assert!(text.contains("甲"), "{text}\n{}", ed.status());
        assert!(text.contains("己"), "{text}\n{}", ed.status());
        // The row that was standing there is overwritten from the cursor's
        // cell, the way every grid pastes a block.
        assert!(!text.contains("樹"), "{text}");
        // Three columns now, rule row included.
        for line in text.lines().filter(|l| crate::mdtable::is_row(l)) {
            assert_eq!(crate::mdtable::split(line).len(), 3, "{line}");
        }
    }

    /// The same paste into a CSV — the block lands, and one too wide is
    /// refused rather than shifting every column right of it (#226).
    #[test]
    fn a_pasted_spreadsheet_lands_in_a_csv_too() {
        let dir = std::env::temp_dir().join(format!("yumete-paste-grid-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let csv = dir.join("d.csv");
        std::fs::write(&csv, "字,說明\n木,樹\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&csv).unwrap();
        assert!(ed.enter_table(), "{}", ed.status());
        ed.goto_line(2);
        ed.paste_text("甲\t乙\n丙\t丁\n");
        assert_eq!(ed.current_buffer().text(), "字,說明\n甲,乙\n丙,丁\n", "{}", ed.status());

        // Three columns into a table two wide: named, and nothing moves.
        let before = ed.current_buffer().text();
        ed.paste_text("戊\t己\t庚\n");
        assert_eq!(ed.current_buffer().text(), before, "{}", ed.status());
        assert!(!ed.status().contains("貼了"), "{}", ed.status());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A single line with no tab is still a cell, not a grid (#226).
    #[test]
    fn one_cell_of_text_is_not_a_spreadsheet() {
        let mut ed = typed("| 字 | 說明 |\n| --- | --- |\n| 木 | 樹 |\n");
        ed.goto_line(3);
        assert!(ed.enter_table(), "{}", ed.status());
        press(&mut ed, "i");
        ed.paste_text("大樹，很高");
        assert!(ed.current_buffer().text().contains("大樹，很高"), "{}", ed.status());
        assert!(ed.status().contains("貼了"), "{}", ed.status());
    }

    #[test]
    fn no_writer_replaces_a_file_the_editor_is_holding() {
        let dir = std::env::temp_dir().join(format!("yumete-ident-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let book = dir.join("main.typ");
        let source = "= 第一章\n\n那年冬天，山下起了大雪。\n";
        std::fs::write(&book, source).unwrap();

        let mut ed = Editor::new();
        ed.open_file(&book).unwrap();

        // Every spelling of this buffer's own file.
        let link = dir.join("draft.typ");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&book, &link).unwrap();
        let mut spellings = vec![book.display().to_string()];
        #[cfg(unix)]
        spellings.push(link.display().to_string());
        for spelling in &spellings {
            ed.execute(&format!(":export typst {spelling}")).unwrap();
            assert!(
                ed.status().contains("稿子本身") || ed.status().contains("正開着"),
                "{spelling}: {}",
                ed.status()
            );
            assert_eq!(
                std::fs::read_to_string(&book).unwrap(),
                source,
                "{spelling} wrote over the manuscript"
            );
        }

        // …and another *open* buffer's file is just as much a manuscript.
        let other = dir.join("ch2.md");
        std::fs::write(&other, "第二章\n").unwrap();
        ed.open_file(&other).unwrap();
        press(&mut ed, "gp");
        assert_eq!(ed.current_buffer().path(), Some(book.as_path()));
        ed.execute(&format!(":export html {}", other.display()))
            .unwrap();
        assert!(ed.status().contains("正開着"), "{}", ed.status());
        assert_eq!(std::fs::read_to_string(&other).unwrap(), "第二章\n");

        // An export onto a file nobody is holding still asks before it
        // replaces one that is already there.
        let out = dir.join("out.html");
        std::fs::write(&out, "早就有的東西\n").unwrap();
        ed.execute(&format!(":export html {}", out.display()))
            .unwrap();
        assert!(ed.status().contains("已經有"), "{}", ed.status());
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "早就有的東西\n");
        // …and `:export!` is how you say you meant it.
        ed.execute(&format!(":export! html {}", out.display()))
            .unwrap();
        assert!(ed.status().contains("寫好了"), "{}", ed.status());
        assert!(std::fs::read_to_string(&out).unwrap().contains("那年冬天"));

        std::fs::remove_dir_all(&dir).ok();
    }

    /// `:w <path>` copies and stays; `:saveas <path>` rebinds and says so.
    #[test]
    fn writing_a_copy_does_not_move_the_manuscript() {
        let dir = std::env::temp_dir().join(format!("yumete-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let chapter = dir.join("ch1.md");
        std::fs::write(&chapter, "第一稿\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&chapter).unwrap();
        press(&mut ed, "i");
        type_keys(&mut ed, "改");
        ed.on_key(Key::Esc);

        let copy = dir.join("copy.md");
        ed.execute(&format!(":w {}", copy.display())).unwrap();
        assert!(std::fs::read_to_string(&copy).unwrap().contains('改'));
        // The keys are still in the chapter, and so is the next `:w`.
        assert_eq!(ed.current_buffer().path(), Some(chapter.as_path()));
        ed.execute(":w").unwrap();
        assert!(std::fs::read_to_string(&chapter).unwrap().contains('改'));

        // The copy exists now, so a second one is a decision.
        press(&mut ed, "i");
        type_keys(&mut ed, "又");
        ed.on_key(Key::Esc);
        assert!(ed.execute(&format!(":w {}", copy.display())).is_err());
        assert!(!std::fs::read_to_string(&copy).unwrap().contains('又'));

        // `:saveas` is the one that moves house.
        let renamed = dir.join("ch1-final.md");
        ed.execute(&format!(":saveas {}", renamed.display())).unwrap();
        assert_eq!(ed.current_buffer().path(), Some(renamed.as_path()));
        assert!(std::fs::read_to_string(&renamed).unwrap().contains('又'));

        std::fs::remove_dir_all(&dir).ok();
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

        // `t` is the table group now, in every mode — vi's till is gone, and
        // with the verb last (`f，d`) it was one keystroke from `f` anyway.

        // A missing target reports and does not move.
        let was = ed.cursor();
        ed.on_key(Key::Char('f'));
        ed.on_key(Key::Char('z'));
        assert_eq!(ed.cursor(), was);
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
        // `G` is not among them any more: it is bound — `30G` goes to line 30
        // and a bare `G` to the last line, as in vi and in Helix.
        for (key, want) in [('$', "gl"), ('^', "gs"), ('@', "Q")] {
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
        assert_eq!(ed.prompt(), Some((":", "s/x/y/")));
        // Back over the closing `/`, fix the letter, and the tail is still there.
        ed.on_key(Key::Left);
        ed.on_key(Key::Backspace);
        ed.on_key(Key::Char('z'));
        assert_eq!(ed.prompt(), Some((":", "s/x/z/")));
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
        assert_eq!(ed.prompt(), Some((":", "sh wc ")));
        ed.on_key(Key::Ctrl('u'));
        assert_eq!(ed.prompt(), Some((":", "")));

        // A full-width space is three bytes, and `byte index + 1` lands inside
        // it — a Chinese writer types one without thinking about it.
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char(':'));
        for c in "grep 甲　乙".chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Ctrl('w'));
        assert_eq!(ed.prompt(), Some((":", "grep 甲　")));
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
        assert_eq!(ed.prompt(), Some((":", "w")), "the newest first");
        ed.on_key(Key::Up);
        assert_eq!(ed.prompt(), Some((":", "toc")));
        ed.on_key(Key::Up);
        assert_eq!(ed.prompt(), Some((":", "toc")), "and it stops at the oldest");
        ed.on_key(Key::Down);
        assert_eq!(ed.prompt(), Some((":", "w")));
        ed.on_key(Key::Down);
        assert_eq!(ed.prompt(), Some((":", "")), "back to the empty line");
        ed.on_key(Key::Esc);

        // The search prompt keeps its own, because patterns and commands are
        // not the same list.
        press(&mut ed, "/二");
        ed.on_key(Key::Enter);
        ed.on_key(Key::Char('/'));
        ed.on_key(Key::Up);
        assert_eq!(ed.prompt(), Some(("/", "二")));
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
    fn a_book_can_teach_the_editor_its_own_names() {
        // 阿寧 — the name on every page — is the one word no dictionary has,
        // so `w` stepped through it a character at a time and the overlay
        // tinted it as two words.
        let dir = std::env::temp_dir().join(format!("yumete-words-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete")).unwrap();
        let file = dir.join("ch01.md");
        std::fs::write(&file, "阿寧走了。\n").unwrap();

        let mut ed = Editor::new();
        ed.set_segmenter(Box::new(DictionarySegmenter::builtin(0)));
        ed.open_file(&file).unwrap();
        let before = ed.segment_line(0);
        assert!(before.len() >= 2, "two characters, two words: {before:?}");

        std::fs::write(dir.join(".yumete").join("words.txt"), "# 人物\n阿寧\n").unwrap();
        ed.reload_project_words();
        assert_eq!(ed.project_word_count(), 1, "{}", ed.status());
        let after = ed.segment_line(0);
        assert_eq!(after[0], (0, 2), "one word now: {after:?}");
        assert!(ed.status().contains("words.txt"), "{}", ed.status());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_mark_names_a_place_and_survives_the_afternoon() {
        // The jump list remembers where you came *from*; a mark remembers
        // where you meant to come back **to**.
        let dir = std::env::temp_dir().join(format!("yumete-marks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let one = dir.join("ch01.md");
        let two = dir.join("ch02.md");
        std::fs::write(&one, "一\n二\n三\n四\n五\n").unwrap();
        std::fs::write(&two, "甲\n乙\n丙\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&one).unwrap();
        ed.goto_line(4);
        press(&mut ed, "M");
        ed.on_key(Key::Char('a'));
        assert!(ed.status().contains('a'), "{}", ed.status());

        // Off to another chapter, and back by name — the file opens itself.
        ed.open_file(&two).unwrap();
        ed.goto_line(2);
        press(&mut ed, "'");
        ed.on_key(Key::Char('a'));
        assert_eq!(ed.cursor_line(), 3, "{}", ed.status());
        assert_eq!(ed.current_buffer().path(), Some(one.as_path()));

        // …and `C-o` goes back to where `'a` was pressed, because a mark is a
        // jump.
        ed.on_key(Key::Ctrl('o'));
        assert_eq!(ed.current_buffer().path(), Some(two.as_path()));

        // A letter nobody marked says so rather than moving.
        press(&mut ed, "'");
        ed.on_key(Key::Char('z'));
        assert!(ed.status().contains('z'), "{}", ed.status());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_session_opens_again_what_was_open() {
        // Five `:open`s every morning is five too many.
        let dir = std::env::temp_dir().join(format!("yumete-session-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("state")).unwrap();
        let one = dir.join("ch01.md");
        let two = dir.join("ch02.md");
        std::fs::write(&one, "一\n二\n三\n四\n").unwrap();
        std::fs::write(&two, "甲\n乙\n丙\n").unwrap();

        let mut ed = Editor::new();
        ed.keep_session_in(dir.join("state"), &dir);
        ed.open_file(&one).unwrap();
        ed.goto_line(3);
        ed.open_file(&two).unwrap();
        ed.goto_line(2);
        ed.save_session();

        // A new morning.
        let mut ed = Editor::new();
        ed.keep_session_in(dir.join("state"), &dir);
        assert_eq!(ed.restore_session(), 2);
        assert_eq!(ed.buffer_count(), 2);
        // Each at the line it was left on.
        ed.open_file(&one).unwrap();
        assert_eq!(ed.cursor_line(), 2, "ch01 was left on line 3");
        ed.open_file(&two).unwrap();
        assert_eq!(ed.cursor_line(), 1, "ch02 on line 2");

        // A file that has since been deleted is simply not opened — a
        // convenience does not get to put an error on the screen every morning.
        std::fs::remove_file(&two).unwrap();
        let mut ed = Editor::new();
        ed.keep_session_in(dir.join("state"), &dir);
        assert_eq!(ed.restore_session(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn closing_a_file_does_not_send_a_jump_into_a_different_one() {
        // `close_buffer` removes one and every later buffer shifts down, so a
        // jump list and a mark kept by *index* came to name a different
        // chapter than the one they were set in.
        let dir = std::env::temp_dir().join(format!("yumete-ids-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = (dir.join("a.md"), dir.join("b.md"), dir.join("c.md"));
        std::fs::write(&a, "甲一\n甲二\n甲三\n").unwrap();
        std::fs::write(&b, "乙一\n乙二\n乙三\n").unwrap();
        std::fs::write(&c, "丙一\n丙二\n丙三\n").unwrap();

        let mut ed = Editor::new();
        ed.open_file(&a).unwrap();
        ed.open_file(&b).unwrap();
        ed.open_file(&c).unwrap();
        // A mark in c, then a jump away from it.
        ed.goto_line(3);
        press(&mut ed, "M");
        ed.on_key(Key::Char('a'));
        // Close b — every buffer after it used to shift down by one.
        ed.open_file(&b).unwrap();
        ed.execute("buffer close!").unwrap();
        ed.open_file(&a).unwrap();
        ed.goto_line(2);
        // The mark still means c.
        press(&mut ed, "'");
        ed.on_key(Key::Char('a'));
        assert_eq!(ed.current_buffer().path(), Some(c.as_path()), "{}", ed.status());
        // …and `C-o` still means where it was pressed from.
        ed.on_key(Key::Ctrl('o'));
        assert_eq!(ed.current_buffer().path(), Some(a.as_path()));
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
    fn aligning_a_table_measures_what_the_terminal_draws() {
        // 「重排對齊」 that counts characters leaves a column of 漢字 ragged:
        // 木 is one character and two cells, and a table lines up in cells.
        let mut ed = typed("| 字 | 拆分 | 說明 |\n| --- | --- | --- |\n| 木 | 木 | 樹 |\n| 相 | ⿰木目 | 看 |\n| a | bb | ccc |\n");
        ed.goto_line(1);
        assert!(ed.enter_table(), "{}", ed.status());
        press(&mut ed, "tf");
        let text = ed.current_buffer().text();
        let widths: Vec<usize> = text
            .lines()
            .map(yumete_cjk::str_width)
            .collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "every row is the same width on the terminal:\n{text}"
        );
    }

    #[test]
    fn the_word_command_is_one_subject_from_three_sides() {
        let mut ed = Editor::new();
        // 著色: named on and off, and flipped when neither is said.
        assert!(!ed.segmentation_visible());
        ed.execute(":word show on").unwrap();
        assert!(ed.segmentation_visible());
        ed.execute(":word show").unwrap();
        assert!(!ed.segmentation_visible());

        // 粒度: it says which, and it takes which.
        ed.execute(":word level").unwrap();
        assert!(ed.status().contains("balanced"), "{}", ed.status());
        ed.execute(":word level strict").unwrap();
        assert_eq!(ed.word_level(), yumete_cjk::WordLevel::Strict);
        assert!(ed.status().contains("strict"), "{}", ed.status());

        // 詞表: `:word` reports what is in force rather than doing anything.
        ed.execute(":word").unwrap();
        assert!(ed.status().contains("分詞"), "{}", ed.status());

        // …and reload is a question for the front end, which owns the IME.
        ed.execute(":word list reload").unwrap();
        assert!(ed.take_words_request(), "the front end is asked to rebuild");
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
        assert!(ed.status().contains("沒有搶救稿"), "{}", ed.status());

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

    /// Esc dismisses the preview; `n` brings it back on the next hit.
    ///
    /// The reader who presses Esc has closed a window, not called off the
    /// search — and `n` after it used to fall through to `/`'s own repeat,
    /// which answered about whatever was last typed at a `/` prompt, or did
    /// nothing at all and said nothing about why.
    #[test]
    fn escape_shuts_the_preview_and_n_opens_it_again() {
        let mut ed = typed("那年冬天。\n第二行。\n那年夏天。\n又一行。\n那年秋天。\n");
        press(&mut ed, "gg");
        // A previewing search: 那 is on lines 1, 3 and 5.
        press(&mut ed, "g?");
        assert_eq!(ed.peeked_line(), Some(2), "{}", ed.status());
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.peeked_line(), Some(4), "{}", ed.status());

        ed.on_key(Key::Esc);
        assert!(ed.other_pane().is_none(), "Esc shuts the preview");

        // …and `n` is still walking the same list, pane and all.
        ed.on_key(Key::Char('n'));
        assert_eq!(ed.peeked_line(), Some(0), "round to the first 那");
        assert!(ed.other_pane().is_some(), "the preview is back");
        assert!(ed.status().contains("1/3"), "{}", ed.status());
    }

    /// `n` with nothing to repeat says so.
    #[test]
    fn n_with_no_search_behind_it_says_so() {
        let mut ed = typed("那年冬天。\n");
        ed.on_key(Key::Char('n'));
        assert!(
            ed.status().contains("還沒有搜索過") || ed.status().contains("searched"),
            "{}",
            ed.status()
        );
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
        assert!(report.contains("漢字 8"), "{report}");
        assert!(report.contains("字數 10"), "{report}");
    }

    #[test]
    fn the_two_ideographs_outside_the_unified_blocks_count_as_字() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "二〇二五年");
        ed.execute(":count").unwrap();
        let report = ed.status().to_string();
        assert!(report.contains("漢字 5"), "{report}");
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

        // `*` searches for the *text* selected, so its punctuation is literal:
        // the second （甲） is found, and shown in the other work area.
        let mut ed = typed("（甲）乙（甲）\n");
        press(&mut ed, "ggvll");
        press(&mut ed, "g?");
        let (from, to) = ed.other_pane().and_then(|p| p.highlight).expect("a hit");
        assert_eq!(
            ed.current_buffer().rope().slice(from..to).to_string(),
            "（甲）",
            "（甲） found as text, not as a group"
        );
    }

    /// What a search costs on the table this editor was built for.
    ///
    /// A measurement, not an assertion. Run it with
    /// `cargo test -p yumete-core --release searching_a_big_table -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn searching_a_big_table() {
        use std::time::Instant;
        // 宇浩's 拆分表: 123,380 rows of 28 columns.
        let mut csv = String::from("char,ids_y,ids_g,ids_t,ids_h,o1,o2,block,unicode,pinyin,sypy,tupa,meaning,note,ids_j,ids_k,ids_v,ids_u,ids_s,ids_b,ids_m,ids_p,ids_x,ids_z,b1,b2,d1,d2\n");
        let pool: Vec<char> = (0x4E00u32..0x9FA5).filter_map(char::from_u32).collect();
        for i in 0..123_380usize {
            let c = pool[i % pool.len()];
            csv.push_str(&format!(
                "{c},⿰木{c},⿰木{c},⿰木{c},⿰木{c},,,CJK,{:04X},pin,sy,tu,,,,,,,,,,,,,,,,\n",
                0x4E00 + (i % 20000)
            ));
        }
        let dir = std::env::temp_dir().join("yumete-table-bench");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("division.csv");
        std::fs::write(&path, &csv).unwrap();

        let mut ed = Editor::new();
        let t = Instant::now();
        ed.open_file(path.to_str().unwrap()).unwrap();
        println!("open:          {:.1?}", t.elapsed());
        let t = Instant::now();
        assert!(ed.enter_table(), "{}", ed.status());
        println!("enter table:   {:.1?}", t.elapsed());

        let t = Instant::now();
        ed.execute(":search column 龜").ok();
        println!("search column: {:.1?}  ({})", t.elapsed(), ed.status());
        let t = Instant::now();
        ed.execute(":search row 龜").ok();
        println!("search row:    {:.1?}  ({})", t.elapsed(), ed.status());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn delete_takes_the_character_the_cursor_is_in_front_of() {
        // The key did nothing at all before this: `KeyCode::Delete` was not in
        // the table that turns a terminal's keys into the editor's, so it fell
        // off the end in every mode.
        let mut ed = Editor::new();
        ed.current_buffer_mut().insert(0, "那年冬天\n下雪");
        ed.on_key(Key::Char('i'));
        ed.on_key(Key::Delete);
        assert_eq!(ed.current_buffer().text(), "年冬天\n下雪");
        assert_eq!(ed.cursor(), 0, "the cursor stays where it is");
        // Backspace's mirror at the edges: at the end of a line it takes the
        // newline, joining the line below.
        ed.on_key(Key::Char('l'));
        for _ in 0..3 {
            ed.on_key(Key::Delete);
        }
        assert_eq!(ed.current_buffer().text(), "l\n下雪");
        ed.on_key(Key::Delete);
        assert_eq!(ed.current_buffer().text(), "l下雪");
        // …and nothing at the very end of the buffer.
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('G'));
        ed.on_key(Key::Char('i'));
        let before = ed.current_buffer().text();
        for _ in 0..5 {
            ed.on_key(Key::Delete);
        }
        assert!(ed.current_buffer().text().len() <= before.len());

        // On the `:` line it takes the character under the caret.
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char(':'));
        type_keys(&mut ed, "wq");
        ed.on_key(Key::Left);
        ed.on_key(Key::Delete);
        assert_eq!(ed.prompt().map(|(_, line)| line.to_string()), Some("w".into()));
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

    // ── #216 · 文中的表格區塊 ────────────────────────────────────────────
    //
    // The third tier: a run of delimited lines **recognised where it stands**.
    // Not a file that is a table (`.csv`), not a table declared with pipes —
    // a 碼表 somebody pasted into a chapter, a `dict.yaml` under its preamble.

    #[test]
    fn a_code_table_pasted_into_a_chapter_is_read_where_it_stands() {
        let mut ed = typed("## 第三章\n木,AA\n目,BB\n田,CC\n\n那一年的雨下得久。\n");
        ed.execute(":2").unwrap();
        assert!(ed.enter_table(), "{}", ed.status());
        let region = ed.block_region().expect("standing inside the block");
        // The heading above it is not a row of it, and neither is the prose
        // two lines below: **a blank line is a boundary, the heading is the
        // other one** — it holds no comma, so the walk stops there.
        assert_eq!((region.first, region.last), (1, 3));
        // Its first line is data, not a header: a 碼表 has no header, and
        // reading 木 as a column name would lose the row.
        assert_eq!(ed.cell_text(1, 0), "木");
        assert_eq!(ed.cell_text(3, 1), "CC");
        assert!(ed.table_here());
        ed.execute(":6").unwrap();
        assert!(!ed.table_here(), "the paragraph under it is prose");
    }

    #[test]
    fn a_block_is_entered_on_the_delimiter_the_cursor_is_standing_on() {
        // Both a tab and a comma hold every line together. Standing on the
        // comma says which one is meant — nothing has to be prompted for.
        let mut ed = Editor::new();
        let dir = std::env::temp_dir().join(format!("yumete-block-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("章.md");
        std::fs::write(&file, "序\n木\tA,B\n目\tC,D\n田\tE,F\n").unwrap();
        ed.open_file(&file).unwrap();
        ed.execute(":2").unwrap();
        // Column 1 by default — the tab, which BLOCK_GUESSES tries first.
        assert!(ed.enter_table(), "{}", ed.status());
        assert_eq!(ed.cell_text(1, 1), "A,B", "the tab, tried first");
        ed.leave_table();
        // Now stand on the comma and ask again.
        ed.execute(":2").unwrap();
        press(&mut ed, "lll");
        assert_eq!(ed.char_at_cursor(), Some(','), "on the comma");
        assert!(ed.enter_table(), "{}", ed.status());
        assert_eq!(ed.cell_text(1, 0), "木\tA", "the comma, because you were on it");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_dict_under_a_preamble_keeps_the_column_only_some_rows_carry() {
        // A generated `dict.yaml`: three dashes, a preamble, three dashes,
        // then the entries — and a 權重 on the entries that have earned one.
        let mut ed = typed(concat!(
            "---\n",
            "name: yuhao\n",
            "---\n",
            "木,AA\n",
            "目,BB,100\n",
            "田,CC\n",
            "水,DD\n",
        ));
        ed.execute(":4").unwrap();
        assert!(ed.enter_table(), "{}", ed.status());
        let region = ed.block_region().expect("inside the entries");
        assert_eq!((region.first, region.last), (3, 6), "the preamble is not the table");
        // Three columns, not two: the row that carries a 權重 is still
        // carrying it, and a column drawn nowhere cannot be walked into.
        assert_eq!(ed.cell_text(4, 2), "100");
    }

    #[test]
    fn prose_is_not_a_table_because_it_has_commas_in_it() {
        let mut ed = typed("She waited, and waited.\nThen the rain, at last, came.\n");
        assert!(!ed.enter_table(), "two lines that disagree are not a grid");
        assert!(ed.block_region().is_none());
    }

    #[test]
    fn two_lines_of_a_block_must_agree_exactly_but_a_long_one_may_not() {
        // Fewer than four rows: 「most of them」 means nothing, so all of them.
        assert!(!Editor::rows_agree(&[2, 3]));
        assert!(Editor::rows_agree(&[2, 2]));
        assert!(!Editor::rows_agree(&[2, 2, 3]));
        // Past that the slack is real data: a 權重 on some entries only.
        assert!(Editor::rows_agree(&[2, 3, 2, 2]));
        assert!(!Editor::rows_agree(&[2, 3, 4, 2]));
        // One cell is not a column, however many lines agree about it.
        assert!(!Editor::rows_agree(&[1, 1, 1, 1]));
    }

    #[test]
    fn putting_a_column_into_a_block_stops_at_the_blank_line() {
        // `t p` walks the *file* in a `.csv`. In a block it must walk the
        // block — otherwise a 碼表 pasted into a manuscript writes the yanked
        // column down the rest of the chapter.
        let mut ed = typed("木,AA\n目,BB\n田,CC\n\n他寫下,然後停筆\n");
        ed.execute(":1").unwrap();
        assert!(ed.enter_table(), "{}", ed.status());
        press(&mut ed, "ty");
        press(&mut ed, "l");
        press(&mut ed, "tp");
        assert_eq!(ed.cell_text(0, 1), "木");
        assert_eq!(ed.cell_text(2, 1), "田");
        assert_eq!(
            ed.line_text(4).unwrap().trim_end_matches('\n'),
            "他寫下,然後停筆",
            "the prose under the block is not a row of it"
        );
    }

    #[test]
    fn a_block_is_read_where_it_lies_and_not_sorted() {
        // Sorting rewrites the lines. In somebody else's document, the lines
        // around the block are the document — so the answer is no, with the
        // two commands that *would* do it named.
        let mut ed = typed("木,AA\n目,BB\n田,CC\n");
        ed.execute(":1").unwrap();
        assert!(ed.enter_table(), "{}", ed.status());
        ed.execute(":table sort 1").unwrap();
        assert!(ed.status().contains("只讀"), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), "木,AA\n目,BB\n田,CC\n");
    }

    #[test]
    fn the_keys_a_block_does_not_answer_say_which_ones_it_does() {
        let mut ed = typed("木,AA\n目,BB\n田,CC\n");
        ed.execute(":1").unwrap();
        assert!(ed.enter_table(), "{}", ed.status());
        press(&mut ed, "to");
        assert!(ed.status().contains("y p"), "{}", ed.status());
        assert_eq!(ed.current_buffer().text(), "木,AA\n目,BB\n田,CC\n");
    }
}
