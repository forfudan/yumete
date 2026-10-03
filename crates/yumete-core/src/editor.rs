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
use yumete_cjk::{is_han, split_charset, CategorySegmenter, NoReader, Reader, Segmenter};

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
/// **A split does not give the editor a second set of selections** (Feature #176):
/// one pane holds the keys and this holds the place the *other*
/// pane was left at — written when it loses the keys, read when it gets them
/// back. It is the same act `Buffer::cursor` performs when you leave a file,
/// one level up, because two panes can hold one buffer and a buffer has room
/// for one place.
///
/// Warning: **這一句從前寫的是「The editor has one cursor」**，而 2026-09-28 起不再是那樣了
/// （#405，`crate::selection::Selections`）。它要說的其實一直是**分屏**那件事——有幾個
/// 選區和有幾個工作區是兩回事，一個分屏仍舊只有一組選區。
#[derive(Debug, Clone)]
pub struct Pane {
    /// The buffer's **id**, never its index: closing a file shifts every
    /// index, and a pane that kept one would show a different chapter.
    pub buffer: u64,
    cursor: usize,
    anchor: usize,
    /// `j`／`k` 瞄準的那一列。Warning: **這一半收起來的時候只剩一段選區**（`cursor`／`anchor`
    /// 就是那一段），所以這裏存的是那一段的目標列，不是一組。
    goal_column: Option<usize>,
    /// 竪排那一半的同一件事。見上。
    goal_slot: Option<usize>,
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

/// 字典面板裏的一欄：欄名跟欄裏的字（「拆分」——「刀二阝」）。
///
/// 哪幾欄、叫什麼名字，是拆分表說的事，核心不認它們——所以是兩個字串而不是一支
/// 枚舉。
pub type Gloss = (String, String);

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
    /// **`C-g` 剛按下，撤銷已經斷了**——等着把跟在後面那個 `u` 吞掉（2026-09-23）。
    ///
    /// vim 的正統拼法是 `C-g u`（`:h i_CTRL-G_u`，字面意思「斷開 undo 序列」），
    /// 而這裏 `C-g` 底下没有第二件事，那個 `u` 是純儀式。**兩種都收**：斷在
    /// `C-g` 上，`u` 來了就吞掉、不來就算了——Warning: 不吞的話，vim 手打完 `C-g u`
    /// 會在稿子裏留下一個游離的「u」。
    ///
    /// Warning: **helix 那個拼法抄不了**：它用 `C-s`，而終端裏 `C-s` 是 XOFF，按下去
    /// 屏幕會凍住（除非 `stty -ixon`）。
    UndoBreak,
    /// **`空格 t x` — 轉換表格格式**，等着那一個格式名（2026-10-02 作者定）。
    ///
    /// 命令是 `:convert-table`；這一組是它的鍵位，`p` pipe、`c` csv、`t` tsv。
    TableConvert,
    /// An `m` match sequence awaiting its verb (`m`, `i`, `a`, `s`, `d`, `r`).
    Match,
    /// **A vim operator waiting for its motion** — `d`, `c`, `y` under the vim
    /// preset (#429, 2026-09-18).
    ///
    /// `op` is the letter that was pressed; `first` is the key that arrived
    /// after it, when one has — the start of a two-key motion (`g` before
    /// `gg`) or a motion still owed a character (`f` before `f,`). No motion
    /// is longer than two keys, which is why one character is the whole of
    /// what has to be remembered.
    VimOperator {
        op: char,
        first: Option<char>,
    },
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
    /// **`z` 那一層在等哪一行**（`zz`／`zt`／`zb`，2026-09-28）。
    Aim,
    /// `空格 m` — what to keep of the merge conflict under the cursor.
    Conflict,
    /// **`C-w`／`空格 w` 之後那一層：區域**（2026-09-30 定，照 helix 的
    /// `keymap/default.rs:193` 與 `:260`——那兩處是同一組，兩扇門）。
    ///
    /// 原話：「space+w is the parent command for 『region』 operations (helix
    /// uses 『window』)。」Warning: **一組要有好幾個成員纔值一個前綴**：走到哪一
    /// 區、開關哪一欄、切一刀、關掉、只留這一個。從前它們散在 `空格 1234`、
    /// `空格 !@#$`、`空格 w/W/q/Q` 三套詞彙裏。
    Region,
    /// A `:s …c` is asking about one match — `y`/`n`/`a`/`q`/`l` (#415).
    ///
    /// Unlike every other pending here this one is not opened by a key: the
    /// command opens it and it stays open across many keys, one per match.
    Confirm,
    /// **`R` in the search panel is asking once**: change every hit there is?
    ///
    /// A different thing from [`Pending::Confirm`], which asks *per match* on
    /// its way through a `:s …c`. This one is a single yes: one hit and one
    /// file are changes a reader is looking straight at, and every file in a
    /// book is not (#419).
    ReplaceAll,
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
            // `y`/`n`/`a`/`q`/`l` name what to do, not what to write；
            // `C-g` 後面那個 `u` 也不是要寫進去的字。
            Pending::UndoBreak
            | Pending::Confirm | Pending::ReplaceAll | Pending::None
            | Pending::Goto
            | Pending::Space
            | Pending::Register
            | Pending::Case
            | Pending::Match
            | Pending::Table
            | Pending::TableConvert
            | Pending::Mark
            | Pending::Recall
            | Pending::Hop { .. }
            // `z` 那一層等的是 `z`／`t`／`b`，不是一個要寫進去的字。
            | Pending::Aim
            // 區域那一組等的是 `w`／`hjkl`／`e i E I`／`s q o`，一個要寫進去的
            // 字都沒有。
            | Pending::Region
            | Pending::Conflict => false,
            // **A vim operator is waiting for a *motion*, which is keys** — the
            // character `f` asks for is read by the motion itself.
            //
            // Warning: **`di`／`da` 之後也不開輸入法**（2026-09-29 撤回，前一天加的）。
            // 加它的理由是「`di（` 要的是一個全角括號，而 Normal 模式下打不出來」，
            // 而**它等的那一個鍵多半根本不是要寫進去的字**：`diw` 的 `w` 是「詞」，
            // `dip` 的 `p` 是「段」，`dis` 的 `s` 是「句」。開着輸入法的時候 `diw`
            // 把 `w` 當成碼，往稿子裏上屏了一個「中」；關着輸入法的 `--shot` 裏那
            // 一鍵被吃掉，`diw` 和 `di(` 都變成什麽也不發生。
            //
            // 全角括號不靠這一條：`pair_family`（§5.15 三）讓 `di(` 自己就認得
            // （）〔〕「」，ASCII 一個鍵到底。
            Pending::VimOperator { .. } => false,
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
    /// `t` — the same search, stopping one short of what it found.
    Till,
    /// `T` — backwards, stopping one short.
    TillBack,
}

impl FindKind {
    fn forward(self) -> bool {
        matches!(self, FindKind::Forward | FindKind::Till)
    }

    fn till(self) -> bool {
        matches!(self, FindKind::Till | FindKind::TillBack)
    }

    /// The other direction, keeping the 「till」 half — what `,` asks for.
    fn flipped(self) -> FindKind {
        match self {
            FindKind::Forward => FindKind::Backward,
            FindKind::Backward => FindKind::Forward,
            FindKind::Till => FindKind::TillBack,
            FindKind::TillBack => FindKind::Till,
        }
    }
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

/// How many rows a listing holds. Everything is counted; this many are shown.
///
/// `:check` and its family put their findings in a buffer, and a manuscript
/// can have thousands: the count is the useful half of that answer, and the
/// list is for walking.
const LISTING_LIMIT: usize = 500;

/// The two 漢字 [`is_han`] adds by hand — 〇 the year digit and 々 the
/// repetition mark — which `:check-charset` never reports.
///
/// They are in no 字集 list, because those list 漢字 proper and these two are
/// ideographs that live among the punctuation. But they are in the 標點 block
/// every CJK font ships, so reporting them would put 〇 on the page for every
/// 二〇二五年 in the book and bury the finding the check exists for.
const EVERY_FONT_HAS: [char; 2] = ['〇', '々'];

/// The largest file `:grep` will read. A manuscript chapter is kilobytes;
/// anything above this is data that happens to live in the same directory.
const GREP_MAX_BYTES: u64 = 4 * 1024 * 1024;


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

/// **What a writer's files are named** — the extensions `空格 f` lists first.
///
/// Not a filter: a manuscript folder holds a `.png` of a map and a `.csv` of
/// names, and hiding them would make the picker lie about what is there. It is
/// an order, and the order is 「the things you write in, then everything
/// else」.
const PROSE: &[&str] = &["md", "markdown", "txt", "typ", "tex", "org", "rst", "csv", "tsv"];

/// What the file this session opened most recently is worth in the picker's
/// order, dropping by 20 for each one before it and never below 20.
///
/// Small on purpose: a run of two adjacent letters in a name is worth 800, so
/// this settles which of two equally good matches comes first and never what
/// matches.
const VISITED_BONUS: i64 = 400;

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
            // Warning: **判準走 `heading_marks`，和着色那一支、大綱那一支同一份**
            // （2026-09-28）。從前這裏既不要求 `=` 後面有空白也不封頂六級，於是同一行
            // 在正文裏不畫成標題、卻出現在這張清單上。
            let level = crate::markdown::heading_marks(raw, '=')?;
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
    /// The *keys* are not said in any language — `hjkl` is `hjkl` everywhere —
    /// so almost every one of them is a `&'static str`. What they mean is a
    /// `String`, because it is said in the reader's.
    ///
    /// Warning: The key is a [`Cow`] rather than a plain `&'static str` because one
    /// menu's keys are **read off the document**: the register panel lists the
    /// letters that actually hold something, and which letters those are is
    /// only known at the moment the panel is drawn.
    Keys(String, KeyRows),
}

/// One menu's worth of rows: the key to press, and what it does or holds.
pub type KeyRows = Vec<(std::borrow::Cow<'static, str>, String)>;

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
    /// **Cells or 縱 slots** (2026-09-19). `:layout` changes the unit the
    /// padding is counted in without touching a byte of the file, and a table
    /// squared up in cells is ragged when it is stood on end.
    measure: crate::mdtable::Measure,
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
            && self.measure == other.measure
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
    /// (「自动进 tb 会悄悄改掉 hjkl 的含义」), and it had to be knocked
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

/// **`z` 那一層把光標放在哪一行**（`zz`／`zt`／`zb`，2026-09-28）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Aim {
    /// `zt` — 頂上那一行。
    Top,
    /// `zz` — 中間。校對時按得最多的一個。
    Middle,
    /// `zb` — 底下那一行。
    Bottom,
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
/// **The rule, 2026-09-05**, and the reason it is safe:
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

/// **Which folder counts as 「this book」** — the root every listing is
/// rooted at.
///
/// Up from the file for a `.yumete` first — that is where a book already keeps
/// its config, its `words.txt` and its `tables/` — then for a `.git`, and it
/// settles for the folder the file itself is in. Only a session with no named
/// file left in it falls back to `here`.
///
/// Warning: **Every path is resolved against `here` first.** A file opened as
/// `一.md` has a relative path and its `parent()` is the **empty** path; that
/// used to be skipped, so a book opened by a bare name never climbed to its
/// `.yumete` and every listing rooted itself in whatever directory the
/// terminal was standing in. The bug is quiet — the answer is a real folder
/// and a plausible one.
///
/// A free function taking `here` rather than a method reading the process's
/// working directory, because that is the only way to test it: the other way
/// is to **move** the process, and every test running beside it would see.
///
/// Warning: **從工作路徑往上走，不看打開了哪幾個檔**（2026-10-01 改，照 helix 的
/// `find_workspace`）。從前它先問當前緩衝區、再問別的緩衝區，於是 `gd` 跳進
/// rustup 的源碼之後「項目」整個跟着跑了——選擇器、`:grep`、詞表、百科各自在
/// 不同的時刻算，答案還互相對不上。現在它是**工作路徑的函數**：`gd` 跳走它不
/// 動（工作路徑沒動），`:cd` 一改它跟着改。
/// [`book_root`] for callers outside this crate — `main.rs` keys the session
/// by the project, and it has to answer the same question the editor will.
pub fn book_root_of(here: &Path) -> PathBuf {
    book_root(here)
}

pub(crate) fn book_root(here: &Path) -> PathBuf {
    let marked = |mark: &str, flat: &str| {
        here.ancestors()
            .find(|d| d.join(mark).exists() || d.join(flat).exists())
            .map(Path::to_path_buf)
    };
    marked(".yumete", ".yumete.toml")
        .or_else(|| marked(".git", ".git"))
        .unwrap_or_else(|| here.to_path_buf())
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
/// **一趟走最多看幾個檔。**
///
/// 2026-09-27 報的：「位置」那一格是自由文本，隨手打一個 `/` 進去，編輯器就去
/// 遍歷整塊磁盤——沒有進度、沒有上限、按不了取消，只能 `kill -9`。一本書幾百個
/// 檔，一個項目幾千個；到了這個數還沒走完，走的就不是一本書了。
pub(crate) const WALK_CEILING: usize = 20_000;

/// **哪些文件算在裏面** —— 搜索面板底下那三格（2026-10-01 定）。
///
/// Warning: **只對走磁碟的範圍有效。** 本文件與緩衝區不經過 [`walk`]。
#[derive(Debug, Clone, Default)]
pub(crate) struct Sieve {
    /// 連隱藏文件和 `.gitignore` 裏的一起走。出廠關着。
    pub hidden: bool,
    /// 只走這幾條 glob 配得上的，逗號隔開。空着就是不挑。
    pub include: String,
    /// 這幾條 glob 配得上的不走。
    pub exclude: String,
}

impl Sieve {
    /// 兩格 glob 都寫對了嗎 —— 寫錯了呼叫方要說出來。
    pub(crate) fn is_sound(&self, root: &Path) -> bool {
        self.overrides(root).is_some()
    }

    /// **這一個檔過得了篩子嗎** —— 給走查之外的那一處用。
    ///
    /// Warning: **正在編輯的那一份是從內存掃的，不走 [`walk_prose`]**，所以篩子篩不到
    /// 它（2026-10-02 測試逼出來的）：打開着 `a.md`、包含那一格寫 `*.txt`，那一
    /// 份的命中照樣出現在名單上，而框上寫着「只搜 .txt」。
    ///
    /// 寫錯的 glob 當成「什麼都不篩」——那一趟自有它的報錯那條路。
    pub(crate) fn lets_through(&self, root: &Path, path: &Path) -> bool {
        let Some(overrides) = self.overrides(root) else { return true };
        if overrides.is_empty() {
            return true;
        }
        !overrides.matched(path, false).is_ignore()
    }

    /// 兩格 glob 做成一份 [`ignore::overrides::Override`]。
    ///
    /// Warning: **`ignore` 的 override 走的是 gitignore 的語義**，所以 `*.py` 配任何
    /// 深度的 `.py`，而 `docs/*.py` 釘在根上——和 VS Code 的「包含文件」框同形。
    /// 帶 `!` 的那幾條是排除，正是這個機制本來的用法。
    ///
    /// `None` 是「有一條寫錯了」，呼叫方要說出來，不許悄悄當成沒寫。
    fn overrides(&self, root: &Path) -> Option<ignore::overrides::Override> {
        if self.include.trim().is_empty() && self.exclude.trim().is_empty() {
            return Some(ignore::overrides::Override::empty());
        }
        let mut build = ignore::overrides::OverrideBuilder::new(root);
        for one in self.include.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            build.add(one).ok()?;
        }
        for one in self.exclude.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            build.add(&format!("!{one}")).ok()?;
        }
        build.build().ok()
    }
}

/// **走過多少個條目就不往下走了** —— 二進制檔不算進 [`WALK_CEILING`]，可它們自己
/// 也要有個底（2026-10-01 定）。
///
/// 量出來的：`-uu` 走 yumete 這個倉是 51,673 個檔，其中 44,503 個是二進制（86%），
/// 文本只有 7,170。只數文本的話這一趟根本碰不到兩萬——可一塊兩百萬個檔的盤就算全
/// 是二進制，光探頭也要四十秒，所以走查本身也要攔。
pub(crate) const VISIT_CEILING: usize = 200_000;

/// **到了地板之後還肯多走多久**（2026-10-01 定）。
///
/// 作者原話：「如果1ms就扫完了这个limit，那其实我们可以扫更多對吧」。所以上面那兩
/// 個數是**地板不是天花板**：不管多慢都至少走這麼多，走完了看錶，還不到這個數就接
/// 着走。
///
/// Warning: **只有 [`walk_prose`] 肯等**（2026-10-02 修）。挑選器與 `[[` 補全走的是
/// 同一支 [`walk_inner`]，可它們在按鍵上同步跑、中間沒有「正在找…」那一幀，到了
/// 地板就該停。
///
/// Warning: **代價是同一次搜索兩次跑可能給出不同的數目**（盤忙的時候少走幾個）。它只
/// 在本來就要被截斷的那種樹上發生——正常項目連地板都碰不到。
pub(crate) const WALK_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// **走到這裏無論如何都停**（2026-10-01 定，作者原話：「算满3秒，5秒硬停」）。
///
/// 沒有這一道的話最壞情況是「走完地板要多久」，而那是盤說了算的：這台機器上量過，
/// 二十萬個條目要 5.3 秒，兩萬個文本檔要 2.5 秒（暖盤；`-uu` 走 yumete 這個倉是
/// 51,673 個條目、1.37 秒，約三萬七千條目一秒，其中 86% 是二進制、只探 1 KB）。
/// 換一塊慢盤或者一個網絡掛載，同樣的地板可以凍住幾十秒——而這一趟是同步跑完的，
/// 按不了取消。
///
/// Warning: **所以上面那兩個數不是真的「地板」**（2026-10-02 審出來的措辭錯）：這一條
/// 不管有沒有碰到地板都會停，慢盤上走不滿兩萬個檔就被切斷是正常的。而且真正的
/// 上界是「這個數**加上**再處理一個條目的時間」——錶只在 `flatten()` 交出一個條
/// 目的時候讀，卡死在 `readdir` 上的掛載誰也攔不住。
pub(crate) const WALK_DEADLINE: std::time::Duration = std::time::Duration::from_secs(5);

/// **走到這裏就停**——地板、寬限、硬停三條合成一句話。
///
/// Warning: **抽出來是為了測得了它**（2026-10-02）。那三個常量是真實世界的數（兩萬個
/// 檔、二十萬個條目、三秒五秒），測試裏造不出那麼大一棵樹；而這一支是純函數，餵
/// 什麼數都行。走查那一半靠真機實測，判斷這一半靠這裏。
///
/// Warning: **只有搜索那一趟肯為了多走幾個檔多等三秒**（2026-10-02 審出來的回歸）。那
/// 個寬限本來是給 [`walk_prose`] 的，可它從前寫在共用的走查裏，於是挑選器和 `[[`
/// 補全也繼承了——而那兩個是**按鍵上同步跑的**，中間沒有「正在找…」那一幀，而且
/// 它們自己早就夠了（`PICKER_LIMIT` 四千條）。量出來的（`$HOME`，同一個進程同一
/// 棵樹）：到地板就停是 `seen=20000 138ms`，等滿三秒是 `seen=323177 3.00s`。
pub(crate) fn walk_is_done(
    seen: usize,
    visited: usize,
    spent: std::time::Duration,
    prose_only: bool,
) -> bool {
    if spent >= WALK_DEADLINE {
        return true;
    }
    let floored = seen >= WALK_CEILING || visited >= VISIT_CEILING;
    floored && (!prose_only || spent >= WALK_GRACE)
}

/// 一趟走查交代了什麼。
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct Walked {
    /// 太大、跳過去沒看的。
    pub skipped: usize,
    /// **沒走完就停了** —— 交出來的那張單子是半截的，呼叫方有義務說出來。
    pub cut: bool,
}

fn walk(root: &Path, skipped: &mut usize, f: &mut impl FnMut(&Path)) {
    *skipped += walk_inner(root, &Sieve::default(), false, f).skipped;
}

/// [`walk`] for **prose**: a file whose first kilobyte holds a NUL is not
/// something this editor searches, and it does not count towards the ceiling.
///
/// Warning: **The picker does not use this one.** Opening a `.png` from `空格 f` is a
/// reasonable thing to do; searching inside it is not.
///
/// 判準照抄 helix：`BinaryDetection::quit(b'\x00')`
/// （`helix-term/src/commands.rs:2649`，符號搜索那一支 `syntax.rs:235` 同樣一行）。
/// 沒有這一道，開着「搜索隱藏和忽略」搜 yumete 自己的倉要**從盤上讀 5.26 GB 的
/// `.o` 與 `.rlib` 進內存，再一個個因為不是 UTF-8 丟掉**；探頭 1 KB 只要 44 MB。
pub(crate) fn walk_prose(root: &Path, sieve: &Sieve, f: &mut impl FnMut(&Path)) -> Walked {
    walk_inner(root, sieve, true, f)
}

/// 頭 1 KB 裏有沒有 NUL —— 讀不開的也當二進制，反正搜不了。
fn looks_binary(path: &Path) -> bool {
    use std::io::Read;
    let Ok(mut file) = std::fs::File::open(path) else { return true };
    let mut head = [0u8; 1024];
    // Warning: **`read` 可以短讀，也可以回 `Interrupted`**（2026-10-02 審出來的）。
    // 兩個都不是「這是二進制」：短讀會讓第 900 個字節上的 NUL 看不見（網絡文件
    // 系統上合法），而 `Interrupted` 從前被當成二進制，那個檔就這麼從搜索裏消失
    // 了，連 `skipped` 都不算。
    let mut got = 0usize;
    while got < head.len() {
        match file.read(&mut head[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return true,
        }
    }
    head[..got].contains(&0)
}

/// [`walk`] with the panel's three cells applied.
///
/// `Sieve::overrides` returning `None` means a glob the reader mistyped; the
/// walk then does nothing at all rather than quietly walking everything —
/// an answer gathered under a filter that was thrown away is the wrong answer
/// told confidently.
fn walk_inner(
    root: &Path,
    sieve: &Sieve,
    prose_only: bool,
    f: &mut impl FnMut(&Path),
) -> Walked {
    let mut walked = Walked::default();
    let Some(overrides) = sieve.overrides(root) else { return walked };
    let started = std::time::Instant::now();
    let walker = ignore::WalkBuilder::new(root)
        .overrides(overrides)
        // Warning: **五道閘一起開。** 「搜索隱藏和忽略」說的是一句話，而 `ignore`
        // 把它拆成了隱藏、`.gitignore`、`.ignore`、全局 git 排除、`.git/info/exclude`
        // 五項——只開頭一項，`target/` 照樣搜不到，而開關上寫着「和忽略」。
        .hidden(!sieve.hidden)
        .ignore(!sieve.hidden)
        .git_ignore(!sieve.hidden)
        .git_global(!sieve.hidden)
        .git_exclude(!sieve.hidden)
        .follow_links(false)
        // In path order, so a listing of a novel's chapters comes back in
        // chapter order rather than in whatever order the file system holds
        // them. Also what makes the walk single-threaded and reproducible.
        .sort_by_file_path(|a, b| a.cmp(b))
        .require_git(false)
        // The floor under the ignore files, not a list to keep adding to: a
        // folder with no `.gitignore` at all still holds no prose in these
        // two, and the cost of looking is a whole build tree.
        .filter_entry({
            // Warning: **開了那個開關，這道地板也要讓開**（2026-10-01）。它本來是
            // 「沒有 .gitignore 的文件夾也不該搜 build 產物」的兜底，而開關說的
            // 是「全都搜」。
            let floor = !sieve.hidden;
            move |entry| {
                !floor
                    || !entry.file_type().is_some_and(|t| t.is_dir())
                    || !matches!(&*entry.file_name().to_string_lossy(), "target" | "node_modules")
            }
        })
        .build();
    let mut seen = 0usize;
    let mut visited = 0usize;
    for entry in walker.flatten() {
        // Warning: **走到地板、而且錶也到了，纔停。** 停下來交出走到的那些，比卡死強：
        // 交出來的是真的，而卡死的時候屏幕上一個字都沒有。地板與錶的分工見
        // [`WALK_GRACE`]。
        //
        // Warning: **這一句要排在「是不是文件」前面**（2026-10-02 查出來的）。從前它在
        // 後面，於是**錶只在交出一個文件的時候讀**——一棵全是目錄的樹一次都讀不
        // 到，兩道地板和那五秒硬停通通不存在。四十六萬個空目錄、一個文件：`空格 f`
        // 凍了 **13 秒**（量了兩趟），而那正是 [`WALK_CEILING`] 寫出來要擋的那一
        // 幕。走到一半（二十萬個目錄）更糟——那一趟 3.8 秒走完，`cut` 是 false，
        // 名單還說自己是全的。`visited` 本來就該是「看過幾個條目」，不是「看過幾
        // 個文件」；那是 `seen`。
        visited += 1;
        if walk_is_done(seen, visited, started.elapsed(), prose_only) {
            walked.cut = true;
            break;
        }
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
        if !entry.metadata().is_ok_and(|m| m.len() <= GREP_MAX_BYTES) {
            walked.skipped += 1;
            continue;
        }
        // 二進制那些不交出去，也不算進 [`WALK_CEILING`]——那個數說的是「這本稿子有
        // 多少篇」，而 `.o` 不是一篇。
        if prose_only && looks_binary(path) {
            continue;
        }
        seen += 1;
        f(path);
    }
    walked
}

/// What 自動認詞 is asked to read: **this file, and the folder around it**.
///
/// Warning: **Two passes, not one corpus** (#453). Concatenating the folder and
/// reading it once looks equivalent and is not. Measured on 宇浩's own docs,
/// 「宇夢」 comes 8th of 60 candidates in the file that is about it, 46th of
/// 181 in its folder, and **disappears entirely** from the 969 a whole
/// repository yields — where the list is headed by 习习, 火火, 宀八, the
/// 字根 pairs of a few hundred 拆分表. Its count is 24 at every scope: nothing
/// was diluted. What the wider read costs it is the **cap** (the commonest few
/// hundred are kept) and the **pruning** (a longer string that holds it, common
/// elsewhere, absorbs it).
///
/// Neither is fixed by weighting counts — a multiplier cannot un-absorb a word,
/// and it would invent frequencies that are not in the text. What fixes it is a
/// quota: the file is read on its own, and its share of the list is never taken
/// by the folder's.
#[derive(Debug, Clone)]
pub struct DetectAsk {
    /// The text of the file being written, which is read on its own.
    pub text: String,
    /// The folder it is in, read after it — `None` when the buffer has no path
    /// and there is no folder to speak of.
    pub folder: Option<PathBuf>,
}

/// Read a project and work out the words it uses that a dictionary has not
/// got — **with no editor anywhere near it** (#448).
///
/// The counting is three hundred milliseconds on a long novel and the reading
/// is four, so this is the half that has to happen on a thread; everything it
/// touches is a path and a string, which is what makes that possible. The
/// front end calls it, hands the answer back with
/// [`Editor::set_detected_words`], and the writer never waits.
///
/// `joins` is the same oracle [`crate::discover::words`] always takes: a
/// segmenter built in the calling thread, to leave out what is not news. A
/// smaller dictionary there than the one in force only leaves a few redundant
/// words in the list — words already joined cannot be joined harder.
///
/// Answers the words, how many files were read, and **how many 漢字 they held**
/// — the last is what [`crate::discover::cap`] is measured against.
pub fn detect_words_in(
    root: &Path,
    joins: &dyn Fn(&str) -> bool,
) -> (Vec<crate::discover::Found>, usize, usize) {
    let mut text = String::new();
    let mut files = 0usize;
    walk(root, &mut 0, &mut |path| {
        if text.len() >= DISCOVER_MAX_BYTES {
            return;
        }
        let Ok(more) = std::fs::read_to_string(path) else {
            return;
        };
        files += 1;
        text.push_str(&more);
        // The join, so a word cannot be found across the seam between two
        // chapters that never touch.
        text.push('\n');
    });
    let han = crate::discover::han_count(&text);
    (crate::discover::words(&text, joins), files, han)
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
/// **What the server says could come next, and where it was asked** (#53 ④).
///
/// Warning: **The caret's place is part of the answer.** 「什麽接得下去」 is a
/// question about one spot in one file, and the moment the caret leaves that
/// spot the answer is about somewhere else. Keeping the list alive across a
/// move would offer `count` where `count` cannot go.
#[derive(Debug, Clone)]
pub(crate) struct Offering {
    /// Where the caret was when this was asked.
    pub(crate) at: usize,
    /// What the server offered, in the server's own order (it ranks them).
    pub(crate) items: Vec<crate::lsp::Offer>,
    /// Which one is picked — always a real index, because an empty list is
    /// 「no offering」 and is never stored.
    pub(crate) picked: usize,
}


/// The editor: a non-empty list of open buffers and the index of the active one.
pub struct Editor {
    buffers: Vec<Buffer>,
    current: usize,
    mode: Mode,
    /// **選區——複數的那一個**（#405，方案在 `docs/development.md §5.13`）。
    ///
    /// Warning: **Phase 0 裏它永遠只裝一段**，行為和從前的 `cursor`／`anchor` 兩個欄位一字不差
    /// ——這一期的驗收條件就是「`scripts/frames.sh` 那二十幀逐字節不變」。
    ///
    /// 從前直接讀寫那兩個欄位的三百多處，現在走 `self.sel.head()`／`set_head()` 那四支
    /// 門面，它們問的**永遠是主選區**。要作用在全部選區上的入口是 Phase 1 的事。
    sel: crate::selection::Selections,
    /// **剛纔看的是哪一份稿子**（`ga`，2026-09-28）。緩衝區的 **id**，不是下標。
    last_file: Option<u64>,
    /// **正則那一族在等什麽**（`s`／`S`／`A-k`／`A-K`，#405 Phase 2）。
    ///
    /// Warning: **它們借的是搜索那一扇提示行**（`Mode::Search`），所以拼音、簡繁、模糊、正則
    /// 四個開關一起白拿，中文也照打——那一扇本來就開輸入法（`Mode::composes`）。這一格
    /// 記的是「Enter 按下去要做哪一件」，`None` 就是普通的搜索。
    sift: Option<crate::editor::multi::Sift>,
    /// **這一趟是第幾段選區**（`#` 寄存器，#405 Phase 3）。
    ///
    /// `edit_each` 逐段跑的時候填上，跑完清掉。`"#p` 於是在第一段貼「1」、第二段貼
    /// 「2」——**編號列表那個用例就是這麼解開的**（§5.13.1）。
    ///
    /// Warning: **算的是文檔次序，不是執行次序。** `edit_each` 從後往前做，而讀者數的是從上
    /// 往下第幾個。
    edit_nth: Option<usize>,
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
    /// **vim's `V`** — extend mode that means *whole lines* (B3, 2026-09-20).
    ///
    /// vim's linewise visual: the selection grows by lines however far along
    /// one the caret is, and a verb takes those lines entire. `V` used to be
    /// translated to this editor's 「select this line」, which cannot grow:
    /// `3V` took three lines and `Vjj` took one, and a vim hand types the
    /// second.
    vim_lines: bool,
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
    key_aliases: HashMap<String, String>,
    /// The two layers `key_aliases` is laid from (#428).
    user_aliases: HashMap<String, String>,
    key_preset: yumete_cjk::KeyPreset,
    /// The keys typed so far of an alias that may still be completed (#428).
    alias_held: String,
    /// **The digits typed *inside* a held sequence** (2026-09-18).
    ///
    /// vim writes `d10w`, and those digits are not part of the alias's name —
    /// they are a count, and they arrive after the operator rather than before
    /// it. Kept apart from [`Editor::count`] so that both can be written at
    /// once: `2d3w` is six words, the way vim multiplies them.
    alias_count: Option<usize>,
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
    /// The half of [`Self::project_words`] that came out of the writer's own
    /// `.yumete/words.txt` — **yumete never writes that file** (#448). Kept
    /// apart from what autodetect found so that reloading one does not throw
    /// the other away.
    own_words: yumete_cjk::WordList,
    /// 作品百科 (#287) — this book's wiki and the global one, as last read.
    wiki: crate::wiki::Wiki,
    /// How wiki names are marked on the page (§5.8.4).
    wiki_mark: crate::wiki::Mark,
    /// The half autodetect found in the manuscript, which lives **only here**:
    /// no file, no buffer, nothing to accept or refuse. `:word-discover`
    /// writes a copy out to be read, and that copy is not read back.
    detected_words: yumete_cjk::WordList,
    /// **The text** autodetect has been asked to read, taken by the front end
    /// — the counting is a few hundred milliseconds on a long chapter and it
    /// belongs on a thread that is not drawing the page.
    ///
    /// Warning: **The text, not a root** (#452). This handed over a project root and
    /// the front end walked it, which made 自動認詞 answer a question about the
    /// *repository*: in 宇浩's own tree 「宇夢」 never surfaced, because a few
    /// hundred 拆分表 drowned the chapter — 「一堆拆分表形成杂音」. The three
    /// statistics are ratios, so what is read decides what is found, and what
    /// is being written is one file. `:word-discover-working` and its two wider
    /// spellings are how a reader asks for more.
    detect_request: Option<DetectAsk>,
    /// `:theme-fill` — whether a 品色 run gets a ground. The front end holds
    /// the colours, so it holds this too; `None` in the `Some` means 「the
    /// other one」.
    fill_request: Option<Option<bool>>,
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
    /// `:word-list-reload` asking the front end to build the dictionary again
    /// — it owns the IME and the data directory; the editor owns neither.
    words_request: bool,
    /// How a table's columns are told apart (Feature #157).
    table_rules: crate::table::Rules,
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
    /// **`gd` 在代碼檔上問出去的那一句**（#53 ②，2026-09-21）——檔、行、列。
    ///
    /// 和字典那一問同一個形狀（#215）：編輯器不握有答案，服務器纔有，而只有前端
    /// 跟服務器說得上話。所以問題停在這裏，前端下一趟循環取走、發出去，答案回來
    /// 時叫 [`Editor::go_to_definition`]。
    ///
    /// Warning: **列是 UTF-16 碼元**，因爲那是問出去的那一頭要的單位。
    definition_query: Option<(PathBuf, usize, usize)>,
    /// **Normal 模式下按過 Esc，要前端把挂起信號再發一遍**（2026-09-22）。
    ///
    /// 在別的窗口用系統輸入法打完字切回來，輸入法還開着、鍵被吞掉——前端那頭記着
    /// 「已經挂起了」而事實上没有（見 `yumete_tui::system_ime`）。信念會漂，所以
    /// 給人留一條重新聲明的路：Normal 模式下再按一次 Esc。
    ///
    /// Warning: **只在 Esc 没有別的事可做的時候。** Esc 先收窗口、先收選區；那幾件都
    /// 不是這一件，而一個鍵一次只該做一件事。
    say_it_again: bool,
    /// **`:config-reload` 按過了** —— 前端下一趟取走（`take_config_reload`）。
    ///
    /// 核心不知道配置檔在哪，也不該知道：那是 XDG 的事，歸前端。核心只記「有人
    /// 要求了」，同十一支 `take_*_request` 一個形狀。
    config_reload: bool,
    /// **`:settings` 按過了** —— 前端下一趟取走並開那扇面板。
    ///
    /// Warning: **那扇面板的狀態不在核心裏**（它讀 `yumete_config::settings_ui` 那張表，
    /// 而核心不依賴 `yumete-config`），所以這裏只是一張條子。
    settings_request: bool,
    /// 那扇面板此刻開着 —— 前端進出時說一聲。
    ///
    /// 核心要知道，是因為 `:w` 與 `:q` 在它開着的時候說的是**它**：面板上按 `:w`
    /// 想存的是設定，不是眼前這個緩衝區。
    settings_open: bool,
    /// 面板開着時按過 `:w`。
    settings_save: bool,
    /// 面板開着時按過 `:q` —— `Some(force)`，`force` 是打了 `!`（不管沒存的）。
    settings_close: Option<bool>,
    /// **`空格 k` 問出去的那一句**（#53 ③，2026-09-21）——同上一個形狀。
    hover_query: Option<(PathBuf, usize, usize)>,
    /// **`C-n` 問出去的那一句**（#53 ④，2026-09-21）——同上一個形狀。
    completion_query: Option<(PathBuf, usize, usize)>,
    /// **Where the caret was when that question was asked** (#53 ④).
    ///
    /// Warning: **The answer is about that spot, not about wherever the caret is by
    /// the time it comes back.** A list that arrived after two more letters
    /// were typed is a list of what could follow the word as it was — showing
    /// it would offer `counted` after `counting`. Kept separate from the query
    /// because the query is *taken* by the front end and this has to outlive
    /// it until the answer lands.
    completion_at: Option<usize>,
    /// **Whether that question was asked by hand** (`C-n`) rather than by the
    /// letter just typed (#53 ④).
    ///
    /// Warning: **What it decides is whether silence is reported.** 「這裏接不下什麽」
    /// is the answer to a question somebody asked; said after every pause in
    /// typing it is a status line that flickers all day and is never read.
    completion_by_hand: bool,
    /// **服務器提的那些候選，和問它時光標在哪**（#53 ④）。
    offering: Option<Offering>,
    /// **服務器對光標下那個東西說的話，和問它時光標在哪**（#53 ③）。
    ///
    /// Warning: 位置要記下來，因為這一則是**問出來的**：光標一走它就該沒。跟着光標自己
    /// 冒出來的診斷不是這一種——那一種是文件的事實，走到哪都還在。
    hovered: Option<(usize, crate::lsp::Told)>,
    /// **手動叫出來的那一種信息，和叫的時候光標在哪**（#426，2026-09-30）。
    ///
    /// 光標一走就作廢——那一則是問出來的，問題已經過去了。`None` ＝ 沒人叫過，
    /// 那就看即時的那一種。
    info_asked: Option<(crate::sidebar::Info, usize)>,
    /// **按 `Esc` 把浮窗按下去的那一格**（2026-10-01 作者提）。
    ///
    /// 光標還在這一格上就不浮；挪開就又浮得出來。原話：「只是给用户一个可以暂
    /// 时关闭浮窗的可能性」——所以它不是一個開關，記的是「在這裏我不想看」。
    /// 對邊欄不起效（邊欄只由人開由人關）。
    info_hushed: Option<usize>,
    /// **工作路徑**——`空格 F` 搜的那個目錄，`:cd` 改它（2026-10-01）。
    ///
    /// Warning: **不是進程的 cwd。** 真去 `chdir` 會悄悄改掉別處七八個讀
    /// `current_dir()` 的地方（項目根的退路、`:grep` 的退路、百科與詞表的查找），
    /// 而這一格只管一件事。helix 也是自己記一格（`helix_stdx::env`），同理。
    /// 立起來的時候記下 cwd，之後只有 `:cd` 動得了它。
    working_dir: Option<PathBuf>,
    /// `:cd -` 回得去的那一個。
    working_dir_before: Option<PathBuf>,
    /// **`:info <名>` 指定的那一種即時信息**，`None` ＝ 按稿子算（散文百科、
    /// 代碼診斷、表格數據）。見 [`Editor::info_live`]。
    info_live: Option<crate::sidebar::Info>,
    /// 跟着走的時候，上一次問的是哪一格——同一格不重複問。
    docs_asked_at: Option<usize>,
    /// 光標最後一次動是什麽時候，跟着走的那一問等它停穩（`DOCS_SETTLE`）。
    docs_moved: Option<std::time::Instant>,
    /// **浮着的那一則從第幾行畫起**（2026-09-29）——`PageUp`／`PageDown`／
    /// `C-u`／`C-d` 翻它，同 helix（`ui/popup.rs:289-297`：那四個鍵滾浮窗，
    /// 別的鍵一按就關）。#426 起五種裏畫成散文的三種都收這四個鍵。
    info_scroll: usize,
    /// **`:diagnostics-all` 頂上那一行**：哪個語言服務器、在哪、什麽狀態。
    ///
    /// Warning: **前端寫進來的**——`Servers` 住在那一側，核心看不見它。同 `says`
    /// 那一條的理由。空着就是「還沒有服務器接上」，而那與「接上了、一句話都沒說」
    /// 是兩回事，`:diagnostics-all` 分得出來纔說得清。
    server_line: Option<String>,
    /// **語言服務器從什麽時候起沒回話**，`None` ＝ 沒在等（2026-09-30，#426）。
    ///
    /// 給狀態行上轉圈那八個點用。Warning: **前端寫進來的**，同 `server_line` 那
    /// 一條的理由：`Servers` 住在那一側，核心看不見它。存的是**起點**而不是
    /// 「第幾格」——轉到第幾格是時間算得出來的，存它就會有兩處轉得不同步。
    server_busy_since: Option<std::time::Instant>,
    /// The character the 字典 panel is about, and the answer if one has come.
    ///
    /// Three states, because three things can be true. `None`: nobody has
    /// asked. `Some(ch, None)`: asked, and the front end has not answered yet
    /// — the panel shows the character alone. `Some(ch, Some([]))`: answered,
    /// and the 拆分表 has nothing for it, which the panel has to say out loud
    /// rather than draw as an empty box.
    dictionary: Option<(char, Option<Vec<Gloss>>)>,
    /// The other work area, when the page is split (Feature #176).
    other: Option<Pane>,
    /// Which half of the screen holds the keys — **screen order**, so
    /// switching panes never makes the top one jump to the bottom.
    live_pane: usize,
    /// **哪幾欄上鋪一道底紋**（`:rules`，#424，2026-09-30）。空 ＝ 一道都不鋪。
    ///
    /// 作者原話：「在给定的列显示一道竖线（底纹）。比如默认80。用户也可以填
    /// `:rules 80 100 120` 來繪製多條。」Warning: **是底紋不是綫**——一條真的竪
    /// 綫要佔一欄，那一欄就寫不了字；鋪底紋不佔地方。
    rules: Vec<usize>,
    /// Whether the line-number band carries a ground of its own.
    number_fill: bool,
    /// 改動條：行號旁邊那一格說不說「這一行跟 git 那一份不一樣」（#55／#298）。
    diff_gutter: bool,
    /// 每個 buffer 一份 git 逐行差，連着**算它的時候那個 revision**。
    ///
    /// 鍵是 buffer 的 id 而不是它在清單裏的位置：`:bd` 關掉一個，後面每一個都往
    /// 前挪一格，而快取要跟着那一本書走。revision 是「還用不用再喊一次 git」的
    /// 全部判準——存了兩次中間一個字沒改，第二次就不再喊。
    vcs: HashMap<u64, (u64, crate::vcs::Changes)>,
    /// `HEAD` 裏那一份的正文，按 buffer 存着。取它要一個子進程，而它只在存檔、
    /// 換檔、或者 git 那邊動過的時候纔變——**每一次停手**都重取一遍就白花了。
    vcs_base: HashMap<u64, String>,
    /// 上一次**問過** git 的時候，這個 buffer 是哪個 revision。
    ///
    /// Warning: 不能拿 `vcs` 裏有没有答案當「問過了」：不在 git 倉裏的檔問出來是
    /// 「没話說」，那一條會被刪掉——於是它永遠看着像没問過，空閒的鐘就每 300
    /// 毫秒喊一次 git，喊到天亮。
    vcs_asked: HashMap<u64, u64>,
    /// **每個路徑的真名**，問過一次就記住（2026-10-03）。
    ///
    /// 「這個檔開着沒有」是按真名比的，不按拼法——`./甲.md` 與 `甲.md` 是同一個
    /// 檔。可是那個比法對**每一個**已開的緩衝都問一次文件系統，而 `R` 又對每一個
    /// 要改的檔問一次「開着沒有」：三千個檔就是四百五十萬次 `realpath`。
    ///
    /// Warning: **問不出來的不記。** 盤上還沒有那個檔的時候 `canonicalize` 會失敗，
    /// 而它待會兒可能就有了（`:w` 寫一個新檔）——記下來就永遠當它沒有。
    canonical: RefCell<HashMap<std::path::PathBuf, std::path::PathBuf>>,
    /// 語言服務器說了什麽不對（#53／#54），按**路徑**存。
    ///
    /// Warning: **鍵是路徑，不是 buffer id。** 服務器說的是一個檔，而它說的時候那個檔
    /// 不一定開着——rust-analyzer 看一個 crate，報回來的多半是你還没打開的那幾
    /// 個檔。路徑收得下這些，buffer id 收不下。
    problems: crate::problem::Problems,
    /// 剛纔那個**文本對象**沒找到東西（`mi(` 外面沒有括號、`miw` 不在一個詞上）。
    ///
    /// Warning: 一個字的選區和「沒動」在數據上**一模一樣**（`anchor == cursor` 兩者都
    /// 成立），所以操作符不能靠比較位置來判斷對象有沒有命中——`wdiw`（光標停在
    /// 一個空格上）就是這麽被拒掉的。對象自己說。
    object_missed: bool,
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
    /// What Tab types in Insert mode — spaces, or a TAB (Shift-Tab the other).
    tab_spaces: bool,
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
    clipboard_read: Option<Pasting>,
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
    /// **`z` 那一層要把光標放在第幾行**（`zz`／`zt`／`zb`，2026-09-28）。
    ///
    /// Warning: **這個編輯器沒有 viewport。** `Editor::scroll` 的文檔自己寫着：它移的是光標，
    /// 不是視圖——視圖自己滾了，下一幀光標要留在屏幕上的時候會被拉回去。視口住在 TUI
    /// 那一側（`Seats`）。
    ///
    /// 所以 `zz` 不是「滾動」，是**往 [`Editor::page_inset`] 上加一次覆蓋**：那一支本來
    /// 就在回答「光標該坐在頁面第幾行」（跳轉落中間、打字機模式居中都走它）。
    ///
    /// Warning: **一次性的，和 `jumped` 同一套**：按下去的時候立起來，下一個鍵按下去的時候
    /// （`on_key` 開頭）放倒。中間那一幀已經畫過了，視口就停在那裏——再下一幀
    /// `page_inset` 回 `None`（光標舒舒服服在頁面上），沒人會把它拉回去。
    aim: Option<Aim>,
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
    /// **要換的是哪一段**（2026-09-22）——`None` 是整本書。
    ///
    /// `:convert` 換整本；`` ` `` 那一組換**選區**。opencc 是另一個進程，答案
    /// 下一趟循環纔回得來，所以「換哪兒」得跟着問題一起放下——等答案回來的時候，
    /// 選區早就不在了（換完光標要落在換出來的字上）。
    convert_range: Option<std::ops::Range<usize>>,
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
    /// while taking a fifth of the width to do it (2026-09-11:
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
    /// Its own setting now, and nothing else's: `:view-dense` used to force it
    /// to nothing, which made it one of five jobs one switch did.
    zong_gap: Option<usize>,
    /// Which 縱 and rows keep the margin lane (`:view-margin`).
    margin: yumete_cjk::Margin,
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
    /// How many half-width characters share a slot in vertical layout (縦中横);
    /// `0` for none.
    tatechuyoko: usize,
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
    /// Whether fenced code is coloured by its own grammar (#420, `:view-code`).
    code_colours: bool,
    /// Where the fences are, and their code parsed — see `fences.rs`.
    code_cache: RefCell<fences::CodeCache>,
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
    /// **Which way the reader asked for the text to run** (Feature #61).
    ///
    /// Warning: **Not necessarily how it is drawn.** A program file is always drawn
    /// across (`Editor::layout`), and this is what the page goes back to when
    /// one is closed — 2026-09-21 定：「见到程序文件，强制不允许开启竖排模式」，
    /// 而那不能反過來把讀者正在寫的小說也扳平。
    layout_wanted: Layout,
    /// How many graphemes fit in one 縱. The renderer lowers this when the
    /// terminal is too short to draw a full 縱.
    zong_length: usize,
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
    /// The head of the file the picker is standing on, kept until it moves.
    preview: RefCell<Option<(PathBuf, Vec<String>)>>,
    /// The two panel slots, indexed by [`crate::sidebar::Side`] — Feature #94,
    /// #293.
    ///
    /// An array and not two named fields, because everything that walks them
    /// walks both and must not know which is which: the side a panel opens on
    /// is [`Editor::side_for`]'s answer, one function, and the day it reads a
    /// setting instead of returning `Left` nothing else here changes.
    panels: [Option<crate::sidebar::Sidebar>; 2],
    /// Which slot **and which layer** the keys are going to, if any.
    panel_focus: Option<crate::sidebar::Side>,
    /// **每一側多寬**，三檔（2026-09-26）。記在這裏而不是記在 `Sidebar` 上，因為
    /// 寬度是**那一格**的屬性：換視圖不變，關掉再開也不變（定的，原話：「两个侧栏虽然
    /// 关闭，但是还是会记住上次的宽度状态」）。常駐層（字典、懸停、詳情）借的是
    /// 同一格，所以也吃這一檔——一個格子一套規矩。
    width: [crate::sidebar::Width; 2],
    /// **上一幀的窗口有多大**，前端每一幀告訴一次（欄、行）。
    ///
    /// 編輯器本身沒有視口（見 [`Editor::scroll`]），這一格不是視口——它只用來答
    /// 一個問題：**這扇面板擺得下嗎**。出廠是「大得很」，所以沒有前端的時候
    /// （測試、`:export`）一切照舊。
    window: (u16, u16),
    /// **Which slot each panel lives in**, indexed by
    /// [`crate::sidebar::Panel`] (#293).
    ///
    /// One per panel rather than one per group: a reader may want the outline
    /// across from the tree, or the 字典 stacked under it. Two on one side
    /// share that slot — `Tab` walks them, the way the three used to.
    sides: [crate::sidebar::Side; crate::sidebar::Panel::ALL.len()],
    /// The search panel's form and what it found (#419).
    search: crate::search_panel::Search,
    /// **為「看一眼」開出來的那一份緩衝**，記的是它的號（2026-09-27）。
    ///
    /// 在結果名單裏 `jk` 走一步，正文就跳到那一處——跨檔的時候得先把檔開出來。
    /// 走完一本書會開出幾十份，所以同一時間只留一份：走到下一處，上一處那一份
    /// 就還回去（`let_go_of_the_search_preview`）。`Enter` 把它釘住。
    search_preview: Option<u64>,
    /// **上一次替換動過哪幾份緩衝**，記的是它們的號（2026-09-27 審出來的）。
    ///
    /// `R` 一次能改好幾個檔，而 `u` 只撤回**當前那一份**——於是按一下 `u`，一個
    /// 檔回去了、別的檔照舊改着，編輯器還說「已經到最早了」，接着 `:write-all`
    /// 就把沒撤回的那幾個寫進磁盤。記下來，`u` 纔撤得回一整批。
    replaced_in: Vec<u64>,
    /// **那句「換不換」問的是哪一個檔**，`None` ＝ 問的是全部（2026-09-27）。
    ///
    /// 從前只有 `R` 會先問一句，而站在檔名那一行上按 `r` **一聲不吭就把整個檔
    /// 換掉了**——三個試用的人都指出這一條：不確認的那個鍵，正是標籤最容易被截掉
    /// 的那一個。現在兩個都問，而問句要說清楚問的是哪一個。
    replace_this_file: Option<(std::path::PathBuf, Option<u64>)>,
    /// **這一節坐在哪本書上**，開 yumete 的時候定一次，此後不動（2026-09-27 定）。
    ///
    /// 從前沒有這個東西：文件樹問 `project_root()`、「工作區」問 shell 的 cwd、
    /// 「本文件夾」問**當前緩衝**的所在文件夾（那一檔 2026-10-01 刪了），而位置
    /// 那一格裏的 `.` 也跟着當前緩衝走——於是換一個 buffer，`.` 就換了意思，而
    /// 屏幕上看不出來。
    ///
    /// 三家的做法一致（VS Code 的 workspace、Helix 的 workspace root、
    /// projectile 的 project root）：**一個根，不跟當前文件走**。從別處打開一個
    /// 檔只是多一個緩衝，不把根撐大。
    ///
    /// Warning: **2026-10-01 這一格刪了。** 它是第三份答案：全樹只有七處問
    /// [`Editor::root`]，整個 TUI 一次都沒問過，而別處各自按緩衝區現算。helix
    /// 讀了源碼之後發現它只有**一個**可變的東西（cwd），工作區是當場算出來的
    /// （`find_workspace()`）——照辦之後這一格就沒有存在的理由了。
    /// 見 [`Editor::working_dir`] 與 [`Editor::root`]。
    /// **欠着一趟走磁盤的搜索**（2026-09-27 報的：「掃描期間表頭是完全靜止的」）。
    ///
    /// 走一遍一本書要幾百毫秒到幾秒，而它從前就在按鍵那一支裏跑完——屏幕在那段
    /// 時間裏一動不動，按的人分不出它在幹活還是卡住了。所以按鍵只記一筆「欠着」，
    /// **前端畫完一幀**（那一幀的表頭寫着「正在找…」）**再回頭把它跑掉**。
    ///
    /// 同一個辦法在載入碼表那裏用過（`lib.rs` 的 `loading_the_table`）。
    owed_search: bool,
    /// **還欠着的那一批替換**：下一個要動的是名單上第幾個檔（2026-10-03）。
    ///
    /// `R` 動的是每一個有命中的檔，而三千個檔那一趟要十幾秒——從前那十幾秒裏
    /// 屏幕一動不動。同 [`Editor::owed_search`] 的辦法：先畫一幀說話，再做一小
    /// 批，做完再畫一幀。**沒有取消**（2026-10-03 作者定：「不給取消，只給進度」）。
    owed_replace: Option<usize>,
    /// **這一批要動的那幾個檔，開工那一刻抄下來的。**
    ///
    /// Warning: **不許每一批重讀 `search.files`**（2026-10-03 一輪審查報來的）。兩批之間
    /// 前端會叫 `refresh_the_edited_file`，而那一支會把剛改過的那一份從名單裏摘掉
    /// 再插回最前——名單一動，記在手裏的那個下標指的就是別人了，中間那個檔**一聲
    /// 不響地跳過去**，而收尾照樣報「換完了」。
    replace_queue: Vec<(Option<std::path::PathBuf>, Option<u64>)>,
    /// 這一批到此刻換掉了幾處，跨幀累着。
    replace_tally: usize,
    /// 這一批開始的時候人在哪一份緩衝上——換完要回去。
    replace_home: u64,
    /// **屏幕上的落腳點和它們的標籤**（`gw`，#406）——亮着的時候整個鍵盤都是標籤。
    ///
    /// Warning: 不是 `jumps`：那個是 `C-o`／`C-i` 走的跳轉表（#45），兩件事。
    labels: Vec<labels::Jump>,
    /// 標籤已經被打進去的那幾個字母。
    jump_typed: String,
    /// **欠着一次 `gw`**：按鍵記一筆，前端畫完一幀交了範圍再跑（見 `labels.rs`）。
    owed_jump: bool,
    /// **這一頁畫了哪一段**（字符下標），前端每幀交過來一次（`set_page_span`）。
    page_span: (usize, usize),
    /// **一句幾秒之後自己走掉的話**：什麼時候走，和走的是哪一句。
    ///
    /// 2026-09-27 定，原話：「有些不是特别重要的消息可以有个参数「显示时间」，
    /// 比如几秒，过了这个时间就会从命令行消失。比如那个宽度 1/4。这样不遮挡按键
    /// 提示。」
    ///
    /// Warning: **連那句話一起記下來**，不只記一個時刻：狀態行到處都在被直接賦值，
    /// 只記時刻的話，鐘一響就會把後來那一句不相干的話也抹掉。
    status_fades: Option<(std::time::Instant, String)>,
    /// Where the cursor was when the 字典 was asked, so the answer can go when
    /// the cursor leaves without anybody having to take it away (#293).
    dictionary_anchor: Option<usize>,
    /// **一條釘住的詞條，和釘它的時候光標在哪**（2026-09-25）。
    ///
    /// `:wiki <詞條名>` 挑完一條之後，那一條停在眼前——側欄裏或者浮窗裏——**直到
    /// 光標一動**（原話：「按下enter 之后固定浮窗和面板直到光标移动」）。和字典
    /// 那一份同一個辦法（[`Editor::dictionary_anchor`]）：記下當時的光標，走開了
    /// 就當場丟掉。
    ///
    /// 存的是**名字**不是序號：百科隨時可能重讀（存一個百科檔就重讀），序號會過
    /// 期，而名字是這個功能自己的鍵（`Wiki::by_name`）。
    wiki_pinned: Option<(String, usize)>,
    /// **信息那一格讀到第幾行**——那一格唯一的一格狀態，而它有這一格是因為讀完
    /// 一份長答案正是它存在的理由（#293、#426）。
    ///
    /// Warning: 和 [`Editor::info_scroll`]（浮窗那一份）是兩格，因為它們是兩個
    /// 容器：邊欄走得進去、按 `j`／`k` 一行一行讀，浮窗走不進去、只收 `C-u`／
    /// `C-d`。合成一格的話從邊欄退回浮窗會把讀到哪帶過去，而那兩件事沒有關係。
    panel_scroll: usize,
    /// **百科那一頁讀到哪了**，以及讀的是站在哪個字上的那一條（2026-09-22 報的：
    /// 「無法用 j/k/J/K 向上下翻動」）。
    ///
    /// Warning: **百科是一段文章，不是一張單子。** 邊欄裏別的視圖都是行的列表，`j` 走
    /// 下一行；這一個沒有行可走，要的是**滾動**。從前它連 rows 都不產生
    /// （`View::Wiki => return`），於是 `j` 落在 `sidebar.step()` 上什麼都沒發生。
    ///
    /// 記着光標在哪，是因為光標一走詞條就換了——換了還停在第七行，讀的是另一條
    /// 的中間。
    /// Warning: **`Cell`，因為底在哪只有前端知道。** 一條詞條有多少**屏幕**行，要把
    /// 它按邊欄的寬度折一遍纔數得出來（表格還不折），而折是畫的時候做的事。
    /// 所以畫完那一趟順手把夾好的值寫回來——`G` 也是這麽落地的（存進去
    /// `usize::MAX`，前端走到底、算出總數、寫回真正的那個數）。
    /// 2026-09-23 審出來的：從前拿**源碼行數**去夾，長條目的後三分之二到不了，
    /// 短條目按住 `j` 能把整頁滾成空白。
    wiki_scroll: std::cell::Cell<(usize, usize)>,
    /// What an unnamed file's markup is taken to be, from the project's config.
    default_syntax: Option<crate::syntax::Syntax>,
    /// Which markup a file is in, by extension or by exact name.
    syntax_by_name: HashMap<String, crate::syntax::Syntax>,
    /// The directory a `檔名:行號:` listing was gathered from, so `gf` on one of
    /// its lines resolves the same relative path it printed.
    ///
    /// Warning: **挑選器不再借這一格**（2026-10-02 修）。它從前也往這裏寫自己的根，
    /// 於是 `:check` 出一張單子、中間按一下 `空格 f`，再回去 `gf` 就解錯了地
    /// 方。挑選器的根現在存在 `Picker::root` 上。
    listing_root: Option<PathBuf>,
    /// **The files this session has been in, newest first** (2026-09-18).
    ///
    /// What `空格 f` puts at the top of its list. Alphabetical is chapter
    /// order, which is the right answer for reading a book and the wrong one
    /// for coming back to what you were writing five minutes ago — so the
    /// picker asks this, and every editor with a file picker asks something
    /// like it. Held in memory only: it is a fact about this afternoon, like
    /// the session, and a stale one is worse than none.
    visited: Vec<PathBuf>,
    /// The reader's own 用字 groups, from `[editor] usage_groups` (#233).
    ///
    /// The built-in table cannot hold a novel's own names, and a novel's own
    /// names are what a manuscript slips on: 阿嬌 in chapter two and 阿姣 in
    /// chapter nineteen is invisible to every checker there is.
    usage_groups: Vec<String>,
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
    /// land on eight — 「既然是 Unix 老默认就用他」 (2026-09-10).
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

/// **剪貼板拿回來之後拿它做什麼**（2026-10-02）。
///
/// 只有前端讀得了系統剪貼板，所以核心把要求留在這裏等它取。從前這一格是個
/// `bool`（貼在後面還是前面），而 `:paste-table` 要的是第三件事——帶一個格式
/// 回來。一格說一件事，別開第二條一樣的路。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pasting {
    /// `p`／`P`——原樣貼。`after` 是貼在後面。
    AsIs { after: bool },
    /// `:paste-table <格式>`——當成表格數據，轉成那一種再貼。
    ///
    /// `from` 是「剪貼板裏那張表是哪一種」，`None` 讓它自己嗅——和
    /// `:convert-table` 同一條規矩。
    AsTable {
        to: crate::table::Shape,
        from: Option<crate::table::Shape>,
    },
}

/// What a [`Query`] is about.
///
/// It is an enum and not a `bool` because the interface is the point: the next
/// thing that needs to stop and ask adds an arm and a match branch, and
/// inherits the panel, the key routing and the 「Esc is no」 rule without
/// touching any of them. The recovery pair below is what that promise bought.
enum Asking {
    /// `:write` about to make the file on disk very much bigger.
    OversizeWrite {
        /// The path `:write` was given, if it was given one.
        path: Option<String>,
    },
    /// **崩潰前的草稿比文件新**，開檔時立刻問（2026-10-02 作者定）。
    ///
    /// 原話：「recover 必須在用戶重新打開這個文件的時候立刻決定。用戶打了 800
    /// 個字之後再按 recover 這是不對的。」從前開檔只在狀態欄寫一句，而
    /// `:recover` 永遠按得下去——打了一上午再按一下，那一上午就從屏幕上沒了
    /// （`u` 退得回來，但那是「知道的人纔救得回來」）。
    ///
    /// 「暫時不管」之後 `:recover` 再問同樣這三個。
    RecoverDraft,
    /// 選了「恢復」之後再問一次（同一天定的）：直接恢復／打開對比／取消。
    RecoverConfirm,
    /// **`R` 一下動得太多**（2026-10-03 作者定）。
    ///
    /// 原話：「我觉得要同时满足两个条件吧：1. 超过10个文件 2. 超过100处。」
    /// 兩個條件都過了纔走中央這扇窗；不到的照舊是狀態欄上那一行 `y`／`n`。
    /// 和 `:w` 那一條同一個精神——平日改個錯字一次都不彈。
    ReplaceEverywhere,
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
        let mut editor = Editor {
            buffers: vec![Buffer::scratch()],
            current: 0,
            mode: Mode::Normal,
            sel: crate::selection::Selections::at(0),
            last_file: None,
            sift: None,
            edit_nth: None,
            command_line: String::new(),
            command_caret: 0,
            lookfor_focus: 0,
            command_history: Vec::new(),
            search_history: Vec::new(),
            history_at: None,
            status: String::new(),
            pending: Pending::None,
            confirming: None,
            operator_count: None,
            extend: false,
            vim_lines: false,
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
            user_aliases: HashMap::new(),
            key_preset: yumete_cjk::KeyPreset::Helix,
            alias_held: String::new(),
            alias_count: None,
            expanding_alias: false,
            segmenter: Box::new(CategorySegmenter),
            reader: Box::new(NoReader),
            project_words: std::rc::Rc::new(RefCell::new(yumete_cjk::WordList::default())),
            own_words: yumete_cjk::WordList::default(),
            wiki: crate::wiki::Wiki::default(),
            wiki_mark: crate::wiki::Mark::Color,
            detected_words: yumete_cjk::WordList::default(),
            detect_request: None,
            fill_request: None,
            word_memo: std::rc::Rc::new(RefCell::new(yumete_cjk::SegmentMemo::default())),
            show_segmentation: false,
            word_mark: yumete_cjk::WordMark::default(),
            hud: Hud::default(),
            word_level: yumete_cjk::WordLevel::default(),
            words_request: false,
            table_rules: crate::table::Rules::default(),
            table_numbers: true,
            typewriter: false,
            focus: false,
            wheel_step: 3,
            ime_available: false,
            definition_preview: false,
            screenshot_request: None,
            dictionary_query: None,
            definition_query: None,
            say_it_again: false,
            config_reload: false,
            settings_request: false,
            settings_open: false,
            settings_save: false,
            settings_close: None,
            hover_query: None,
            hovered: None,
            info_asked: None,
            info_hushed: None,
            // 啓動時的 cwd，記下來。Warning: **不是每次問的時候再去取**——那就
            // 成了「進程此刻的 cwd」，而這一格的意思是「你敲 yumete 的那個目
            // 錄」。眼下沒有誰 `chdir`，所以兩者相同；記下來是為了它一直相同。
            working_dir: std::env::current_dir().ok(),
            working_dir_before: None,
            info_live: None,
            docs_asked_at: None,
            docs_moved: None,
            info_scroll: 0,
            server_line: None,
            server_busy_since: None,
            completion_query: None,
            completion_at: None,
            completion_by_hand: false,
            offering: None,
            dictionary: None,
            other: None,
            live_pane: 0,
            rules: Vec::new(),
            number_fill: false,
            diff_gutter: false,
            vcs: HashMap::new(),
            vcs_base: HashMap::new(),
            vcs_asked: HashMap::new(),
            canonical: RefCell::new(HashMap::new()),
            problems: crate::problem::Problems::default(),
            object_missed: false,
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
            tab_spaces: true,
            chaifen_request: None,
            scheme_request: None,
            theme_request: None,
            clipboard_request: None,
            clipboard_read: None,
            render: Render::Basic,
            sequence: None,
            sort_keys: Vec::new(),
            jumped: false,
            aim: None,
            language_run: None,
            preview_request: None,
            open_request: None,
            preview_at: None,
            shell_request: None,
            convert_patch: None,
            convert_range: None,
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
            margin: yumete_cjk::Margin::default(),
            sentences: false,
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
            tatechuyoko: 0,
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
            code_colours: true,
            code_cache: RefCell::default(),
            pad_cache: RefCell::new(None),
            md_cache: RefCell::new(None),
            pipe_region: RefCell::new(None),
            md_tables: RefCell::new(None),
            candidate: Vec::new(),
            ruby: Dialects::only(crate::ruby::Dialect::Html),
            ruby_drawn: true,
            layout_wanted: Layout::default(),
            zong_length: DEFAULT_ZONG_LENGTH,
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
            preview: RefCell::new(None),
            panels: [None, None],
            width: [crate::sidebar::Width::default(); 2],
            window: (u16::MAX, u16::MAX),
            panel_focus: None,
            // 檔案／緩衝區／大綱 on the left — 「what is there, and where am
            // I in it」; 字典／詳情 on the right — 「what is this thing I am
            // standing on」. Two questions, two columns.
            search: crate::search_panel::Search::new(),
            search_preview: None,
            replaced_in: Vec::new(),
            replace_this_file: None,
            owed_search: false,
            owed_replace: None,
            replace_queue: Vec::new(),
            replace_tally: 0,
            replace_home: 0,
            labels: Vec::new(),
            jump_typed: String::new(),
            owed_jump: false,
            page_span: (0, 0),
            status_fades: None,
            sides: [
                // 文件、緩衝、大綱、搜索在左；信息在右（它是「這是什麽」那一
                // 格，跟着眼睛走，不該把正文往右推）。
                crate::sidebar::Side::Left,
                crate::sidebar::Side::Left,
                crate::sidebar::Side::Left,
                crate::sidebar::Side::Left,
                crate::sidebar::Side::Right,
            ],
            dictionary_anchor: None,
            wiki_pinned: None,
            panel_scroll: 0,
            wiki_scroll: std::cell::Cell::new((0, usize::MAX)),
            default_syntax: None,
            syntax_by_name: HashMap::new(),
            listing_root: None,
            visited: Vec::new(),
            usage_groups: Vec::new(),
            opened_with: HashMap::new(),
            time_offset: None,
        };
        // **The default segmenter goes on the same way every other one does**
        // (#491). Built straight into the field it was the one segmenter in the
        // program *not* wrapped in `WithWords`, so a front end that never got
        // as far as installing a dictionary had the book's own names and
        // everything 自動認詞 found silently doing nothing. Nobody would see
        // it: `detected_word_count()` counts them, and they are simply never
        // consulted.
        editor.set_segmenter(Box::new(CategorySegmenter));
        editor
    }

    // Grids — table mode, `|` tables and delimited text — are in
    // `editor/tables.rs` (#296).

    /// A typesetter the front end should start or stop.
    /// Whether the move that just happened was a jump, so the page can centre
    /// what it landed on rather than nudge it in from an edge.
    /// **`z` 那一層放在哪一行**。
    ///
    /// Warning: 屏幕上只有三個位置值得一個鍵：頂、中、底。helix 的 `z` 層還有 `zm`（橫向居
    /// 中），而這個倉橫向滾動只在 `:view-wrap off` 下才動得起來，那時整段是一行——
    /// 「橫向居中」在一行裏沒有意義。
    pub fn aim_the_page(&mut self, aim: Aim) {
        self.aim = Some(aim);
        self.status = match aim {
            Aim::Top => say!("page.aimed-top"),
            Aim::Middle => say!("page.aimed-middle"),
            Aim::Bottom => say!("page.aimed-bottom"),
        };
    }

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
        let at = self.sel.head();
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
        // **`z` 那一層先說話**（2026-09-28）：讀者剛剛親口說了這一行要坐在哪。
        //
        // Warning: **`zt` 瞄的是第 `scrolloff` 行，不是第 0 行**，`zb` 同理往回留一截。
        // 這一條不是「順便也留點餘地」，是**它停不停得住的唯一條件**：這個覆蓋是一次性
        // 的（下一個鍵按下去就清掉），而視口是靠下一幀的 `page_inset` 回一句「別動」
        // 纔留在原地的。瞄第 0 行的話，下一幀底下那一句 `d < scrolloff` 立刻把它推開
        // ——按完 `zt` 隨便動一下，頁面自己往下跳三行。vim 的 `zt` 也是留 `scrolloff`
        // 的，同一個理由。
        if let Some(aim) = self.aim {
            return Some(match aim {
                Aim::Top => scrolloff,
                Aim::Middle => last / 2,
                Aim::Bottom => last.saturating_sub(scrolloff),
            });
        }
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
        // 平仄 live in the margin lane, down the page and across it, so no lane
        // means nowhere to draw them — suppressed, not turned off.
        self.meter && self.reader.available() && self.margin.shown()
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
        // There has to *be* a view; nothing else about it is wanted here any
        // more (see the note on the tail below).
        self.table.as_ref()?;
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
        // Warning: **No 「· 字」 tail** (#496). The grain used to be spelt out here
        // and again in the hint row's title, and both were saying what the
        // `T` key is already standing there saying — 「似乎不需要吧。因为挺明显
        // 的」. The status line is a scarce row; what is on it has to be
        // something not said anywhere else.
        Some(name)
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

/// **跟着光標的那一問，等光標停穩多久**（2026-09-29）。
///
/// 同服務器那一頭等打字停下來的那個數（`server.rs` 的 `SETTLE`）。按住 `j` 連走
/// 的時候一格都不問，手一停纔問一次。
const DOCS_SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

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
/// **畫不出來的分隔符要報名字。**
///
/// Warning: **空格和製表符印進句子裏是一樣的空**（2026-10-03）：從前 `' '` 原樣掉下
/// 去，於是「照「 」分的」讀着像壞了，和製表符那一種一模一樣。報的名字就是命令
/// 自己收的那個詞（`:convert-table space pipe` 裏的 `space`），所以不是新造的說
/// 法——`Tab` 本來就是這麼來的。
fn named_delimiter(delimiter: char) -> String {
    match delimiter {
        '\t' => "Tab".to_string(),
        ' ' => "Space".to_string(),
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
    // Warning: **An empty document is one empty line, and ropey will not walk back
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
    // 真正的全角形式（U+FF02／FF07／FF40）。中文輸入法出的是 “” ‘’，這三個少見，
    // 收進來是為了「不分全半角」這條規矩沒有例外（2026-09-28）。
    ('＂', '＂'),
    ('＇', '＇'),
    ('｀', '｀'),
];

/// **一個 ASCII 括號鍵管得到的那一族**（2026-09-28）。
///
/// 起因是一則反饋：「di( di[ 等等目前好像只支持半角字符，能不能处理时不区分全半角，见到
/// 成对的括号都执行内部删除？」查下來配對表本身十九對全收，斷的是**查表是精確匹配**：按
/// `(` 只找 ASCII 的 `()`，而中文稿子裏的括號是全角的。
///
/// Warning: **這不只是全半角摺疊，是我們自己定的一張對應表**（2026-09-28 定，選的是「連中文括號
/// 一起掛上去」那一檔）。`（）` 確實是 `()` 的全角形式，Unicode 明說；可是 `【】`『』
/// `《》` 在 Unicode 裏是獨立的中文標點，不是哪個 ASCII 鍵的全角形式。把它們挂到 ASCII
/// 鍵上是為了順手：寫中文的時候隨手按一個 `[`，拿到的是眼前那一對中文括號。
///
/// Warning: **方引號 `「」`『』掛在 `[` 上，不掛在 `"` 上**（同日定）。它們長得是括號，不是
/// 引號的樣子，手會去按方括號那個鍵。
///
/// Warning: **兩頭都認**：從 `【` 查也回同一族，所以 `di【` 一樣找得到 `[]`。
///
/// Warning: 想「不管哪一對，刪最裏面那一對」用 `md`，那一支掃整張 `PAIRS`，和這裏無關。
const FAMILIES: &[&[(char, char)]] = &[
    &[('(', ')'), ('（', '）')],
    // **單層的歸 `[`，套在裏面那一層的歸 `{`**（2026-09-28 定）。『』是套在「」裏面的
    // 第二層，〖〗是【】的白身；兩個都是「裏面那一層」，而 `{` 在 ASCII 裏也正是套在
    // `[` 裏面用的那一對。這樣兩族也不至於一邊七對一邊兩對。
    &[('[', ']'), ('［', '］'), ('【', '】'), ('〔', '〕'), ('「', '」')],
    &[('{', '}'), ('｛', '｝'), ('〖', '〗'), ('『', '』')],
    &[('<', '>'), ('〈', '〉'), ('《', '》')],
    &[('"', '"'), ('＂', '＂'), ('“', '”')],
    &[('\'', '\''), ('＇', '＇'), ('‘', '’')],
    &[('`', '`'), ('｀', '｀')],
];

/// `c` 這個括號所在的那一族，落單的話就它自己那一對。
pub(crate) fn pair_family(c: char) -> Vec<(char, char)> {
    match FAMILIES
        .iter()
        .find(|family| family.iter().any(|&(open, close)| open == c || close == c))
    {
        Some(family) => family.to_vec(),
        None => pair_of(c).into_iter().collect(),
    }
}

/// The pair a delimiter names — either half selects the whole pair, so `mi「`
/// and `mi」` mean the same thing.
/// The pair a character names, for the vim grammar's objects.
pub(crate) fn pair_for(c: char) -> Option<(char, char)> {
    pair_of(c)
}

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
mod fences;
mod files;
mod help;
mod find;
mod hint;
mod info;
mod jumps;
mod keys;
mod labels;
mod matching;
mod memo;
mod modes;
mod multi;
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
mod wiki;
pub use wiki::{WikiLine, WikiPart, WikiView};
mod words;
mod wrap;

#[cfg(test)]
mod tests;
