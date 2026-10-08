//! Soft-wrapping for horizontal layout — the counterpart of [`crate::zong`].
//!
//! Horizontal layout used to draw one logical line per screen row, so a
//! paragraph wider than the terminal simply ran off the right edge and the rest
//! of it could not be seen at all. For an editor whose whole purpose is long
//! CJK prose — where a paragraph is routinely one line of several hundred
//! characters — that is not a limitation but a missing feature.
//!
//! This module answers, for a rope and a text width in cells, the same three
//! questions [`crate::zong`] answers for 縱, and in the same shape:
//!
//! - which visual **rows** a paragraph breaks into ([`line_rows`]),
//! - where a char index sits in that grid ([`position`]),
//! - which rows fill a page starting from an [`Anchor`] ([`rows_from`]).
//!
//! Everything is windowed. The renderer never asks for a global row number,
//! because finding one means walking the document from the top on every
//! keystroke; it scrolls in anchors instead.
//!
//! ## Where a row may break
//!
//! CJK breaks between any two characters — there are no word spaces to break
//! at — so the default is simply "as much as fits". Two adjustments make the
//! result readable rather than merely correct:
//!
//! - **Latin words are kept whole.** Breaking mid-word is right for 漢字 and
//!   wrong for `Helix`, so a break that would split a run of ASCII word
//!   characters retreats to the last space in the row.
//! - **禁則處理 (kinsoku).** A closing mark — 。、」）— may not open a row, and
//!   an opening bracket may not close one. Either case pulls one more character
//!   down to the next row, which is what a typesetter does by hand.
//!
//! Both retreats are bounded and both give up rather than produce an empty row:
//! a row always advances by at least one character, so wrapping terminates on
//! any input.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ropey::Rope;
use yumete_cjk::{grapheme_width, graphemes};

/// The narrowest text area worth wrapping into. Below this the gutter and the
/// wrap rules fight each other, and the honest answer is that the terminal is
/// too narrow; callers clamp to it rather than producing one character per row.
pub const MIN_WRAP_WIDTH: usize = 8;

/// How the page is measured: how wide it is, and which characters of a line
/// are not on it.
///
/// The second half is what makes 所見即所得 wrap correctly. Measuring rows in
/// *source* characters puts a row's worth of hidden markup on a row of its own,
/// which draws as a blank line in the middle of a paragraph. A row holds what
/// fits **on the screen**.
#[derive(Clone, Copy)]
pub struct Measure<'a> {
    width: usize,
    hidden: &'a dyn Fn(usize) -> Vec<(usize, usize)>,
    /// Which whole lines are not on the page at all (Feature #159).
    ///
    /// A blank line between two indented paragraphs is the file saying twice
    /// what the page says once, so the page leaves it out. Part of the measure
    /// for the same reason as `hidden`: everything that asks where a row is —
    /// the renderer, `j`, the mouse — has to be asking about the same page.
    folded: &'a dyn Fn(usize) -> bool,
    /// How many cells open a paragraph (首行縮進).
    ///
    /// Part of the measure and not of the renderer, because it changes **where
    /// a row breaks**: a paragraph's first row is that many cells narrower,
    /// and everything that asks where a character is has to be asking about
    /// the same page.
    indent: usize,
    /// The one paragraph that is shown **as the file has it**: no indent, and
    /// the blank line above it back (Feature #159).
    ///
    /// The paragraph being typed into. Everywhere else the page is a book;
    /// here it is a file, because this is the line whose structure is being
    /// changed and there must be no doubt about what is in it.
    open: Option<usize>,
    /// **Where the caret is** — `(line, 那一行裏第幾個字符)`，`None` 表示沒人
    /// 在這一頁上打字（導出、面板、預覽都是這一種）。
    ///
    /// 只回答一個問題：這一行的行末要不要一格給光標站。見
    /// [`line_rows_for_caret`]。
    caret: Option<(usize, usize)>,
    /// **Text on the page that the file has no bytes for** — the inverse of
    /// `hidden`, as `(column within the line, what is drawn there)`.
    ///
    /// A drawn run stands *before* the character it is anchored at, and takes
    /// its width from the page: the row it is on holds that much less writing,
    /// the caret at the anchor sits after it, and a click on it means the
    /// anchor. Which is the whole point of it being part of the measure: an
    /// inline candidate drawn by the renderer alone would put the caret, `j`,
    /// the mouse and the wrap all on different pages.
    drawn: &'a dyn Fn(usize) -> Vec<(usize, String)>,
    /// The part of `drawn` that stands **before the caret** at its own anchor.
    ///
    /// Two things are drawn at the same anchor and the caret goes between
    /// them: the inline candidate, which the writer typed and the caret is at
    /// the end of, and the padding that squares a table up, which reaches from
    /// there to the closing pipe. Counting both put the caret on the pipe
    /// after every keystroke in a table; counting neither would put it back
    /// inside the candidate. So the caret asks this, and everything else —
    /// where a row breaks, what a click means — asks `drawn`.
    typed: &'a dyn Fn(usize) -> Vec<(usize, String)>,
    /// Which lines are **one row however long they are** (#275).
    ///
    /// A row of a table. 「进入后，表格所在的行不再 soft wrap」 —
    /// and the reason is the whole point of the mode: a cell that has wrapped
    /// onto the next screen row is no longer in its column, so a table folded
    /// to the measure is not a table any more. Part of the measure and not of
    /// the renderer, like everything else here, because the caret, `j`, the
    /// mouse and the page must be reading the same page.
    unwrapped: &'a dyn Fn(usize) -> bool,
    /// **Which document this rope is, and which version of it** — the buffer's
    /// id and revision, when the caller has them.
    ///
    /// Only ever a cache key (#315). The rows a paragraph wraps into are
    /// remembered, and without this the memo is taken by hashing the
    /// paragraph's own text — a walk down the whole paragraph, two to five
    /// times a keystroke, on exactly the paragraphs where that hurts: a
    /// chapter written as one 1,000,000-character line spent 2.5 ms of every
    /// `j` in hashing alone. A revision moves on every edit, so it says the
    /// same thing for less.
    ///
    /// `None` for a caller with no buffer behind its rope, which then pays the
    /// hash. **The rope handed to these functions must be the one this names**
    /// — that is the whole of the contract, and it is why it is set beside the
    /// rope it describes rather than anywhere else.
    version: Option<(u64, u64)>,
    /// **Where the last edit was, and how much longer it made the text**
    /// (#366) — the buffer's own `edit()`, passed through.
    ///
    /// With it, an answer for a paragraph can be *continued* from the one
    /// before the edit instead of being made again; without it the paragraph
    /// is wrapped from its head, which is what typing into a million-character
    /// paragraph used to cost 25.4 ms a key for.
    edit: Option<(usize, isize)>,
    /// **這一行的前綴寬度索引**，有的話（2026-10-08）。見 [`Widths`]。
    ///
    /// 與 `version` 一樣是純粹的省工夫：`None` 的時候每一處都逐字素走，答案一樣
    /// ——只是不折行的一行 1242 萬字上，光標在行尾按一下 `h` 要 223 ms。
    ///
    /// 建一次要走一遍那一行，所以**記在哪裏、按什麼作廢是前端的事**（`Editor`
    /// 那邊是 `width_memo`，鍵是 buffer 的 id 與行號、戳是 revision 加
    /// [`yumete_cjk::ambiguous_is_wide`]）。這裏只負責問。
    ///
    /// 第三個參數是**這一問跨多少個字符**，答的那一邊據此決定值不值得建：一段不
    /// 到一塊長，逐字素走至多 86 µs，而建索引要把那一行整個走一遍（1242 萬字是
    /// 190 ms）。2026-10-08 量到的：`gh` 之後不往右走，少了這一道就白付 192 ms。
    widths: &'a AskWidths<'a>,
}

/// A page with nothing hidden, for callers that show the source as it is.
const NOTHING_HIDDEN: &dyn Fn(usize) -> Vec<(usize, usize)> = &|_| Vec::new();

/// A page with every line on it — and, read the other way, a page every line
/// of which folds to the measure.
const NOTHING_FOLDED: &dyn Fn(usize) -> bool = &|_| false;

/// A page with nothing on it but the file's own characters.
const NOTHING_DRAWN: &dyn Fn(usize) -> Vec<(usize, String)> = &|_| Vec::new();

/// **問一行的前綴寬度索引**那一支：`(rope, 行號, 這一問跨多少個字符)`。見
/// [`Measure::widths`]。
///
/// Warning: **`'a` 要寫出來。** 類型別名裏裸的 `dyn Trait` 默認 `+ 'static`，而這一支
/// 是從 `&self` 上借出來的閉包——寫漏了，調用方那一句 `Measure::new(…)` 報的是
/// 「borrowed data escapes outside of method」，看不出跟別名有關。
type AskWidths<'a> = dyn Fn(&Rope, usize, usize) -> Option<std::rc::Rc<Widths>> + 'a;

/// 沒有誰替這一頁記過寬度——每一處都逐字素走。見 [`Measure::widths`]。
const NOTHING_INDEXED: &AskWidths<'static> = &|_, _, _| None;

impl<'a> Measure<'a> {
    /// `width` cells, with every character on the page.
    pub fn plain(width: usize) -> Measure<'static> {
        Measure {
            width: width.max(1),
            hidden: NOTHING_HIDDEN,
            folded: NOTHING_FOLDED,
            indent: 0,
            open: None,
            caret: None,
            drawn: NOTHING_DRAWN,
            typed: NOTHING_DRAWN,
            unwrapped: NOTHING_FOLDED,
            version: None,
            edit: None,
            widths: NOTHING_INDEXED,
        }
    }

    /// `width` cells, with `hidden` naming each line's markup that is off it.
    pub fn new(width: usize, hidden: &'a dyn Fn(usize) -> Vec<(usize, usize)>) -> Measure<'a> {
        Measure {
            width: width.max(1),
            hidden,
            folded: NOTHING_FOLDED,
            indent: 0,
            open: None,
            caret: None,
            drawn: NOTHING_DRAWN,
            typed: NOTHING_DRAWN,
            unwrapped: NOTHING_FOLDED,
            version: None,
            edit: None,
            widths: NOTHING_INDEXED,
        }
    }

    /// **Does the caret stand past the last character of `line`?** — the one
    /// question [`Measure::caret`] is asked.
    fn caret_ends(self, rope: &Rope, line: usize) -> bool {
        let Some((at, column)) = self.caret else { return false };
        if at != line || line >= rope.len_lines() {
            return false;
        }
        // 行尾那個換行符不算字；一行只有一個換行符的時候，長度就是零。
        let row = rope.line(line);
        let len = row.len_chars()
            - usize::from(row.len_chars() > 0 && row.char(row.len_chars() - 1) == '\n')
            - usize::from(row.len_chars() > 1 && row.char(row.len_chars().saturating_sub(2)) == '\r');
        column >= len
    }

    /// The same measure, with `line` shown as the file has it.
    pub fn with_open_line(self, line: Option<usize>) -> Measure<'a> {
        Measure { open: line, ..self }
    }

    /// **Where the caret is** — `(line, 那一行裏第幾個字符)`. See
    /// [`Measure::caret`].
    pub fn with_caret(self, at: Option<(usize, usize)>) -> Measure<'a> {
        Measure { caret: at, ..self }
    }

    /// The same measure, with `folded` naming the lines that are not drawn.
    pub fn with_folds(self, folded: &'a dyn Fn(usize) -> bool) -> Measure<'a> {
        Measure { folded, ..self }
    }

    /// The same measure, with `unwrapped` naming the lines that never fold
    /// — the rows of a table (#275).
    pub fn with_unwrapped(self, unwrapped: &'a dyn Fn(usize) -> bool) -> Measure<'a> {
        Measure { unwrapped, ..self }
    }

    /// Say which buffer this rope is and which revision it is at, so that a
    /// remembered paragraph can be found without reading it — see
    /// [`Measure::version`]. The rope must be that buffer's.
    pub fn with_version(self, buffer: u64, revision: u64) -> Measure<'a> {
        Measure {
            version: Some((buffer, revision)),
            ..self
        }
    }

    /// Say where the last edit was, so a paragraph's rows can be continued
    /// rather than remade (#366). See the field.
    pub fn with_edit(self, edit: Option<(usize, isize)>) -> Measure<'a> {
        Measure { edit, ..self }
    }

    /// 交進「這一行的前綴寬度索引」那一支（2026-10-08）。見 [`Measure::widths`]。
    pub fn with_widths(
        self,
        widths: &'a AskWidths<'a>,
    ) -> Measure<'a> {
        Measure { widths, ..self }
    }

    /// **一行裏 `[a, b)` 這一段佔多少格**，`hidden` 裏的字符不算。
    ///
    /// 有索引就從索引裏減，沒有就逐字素走——兩條路逐格相同，見
    /// [`Widths::width`]。
    fn width_between(
        self,
        rope: &Rope,
        line: usize,
        a: usize,
        b: usize,
        hidden: &[(usize, usize)],
    ) -> usize {
        match (self.widths)(rope, line, b.saturating_sub(a)) {
            Some(index) => index.width(rope, line, a, b, hidden),
            None => walk_width(rope, line, a, b, hidden),
        }
    }

    /// Whether `line` is one row however long it is.
    pub fn unwrapped(self, line: usize) -> bool {
        (self.unwrapped)(line)
    }

    /// Whether `line` is off the page altogether.
    pub fn folded(self, line: usize) -> bool {
        (self.folded)(line)
    }

    /// The first line at or after `line` that is on the page.
    fn next_shown(self, line: usize, lines: usize) -> usize {
        let mut line = line;
        while line < lines && self.folded(line) {
            line += 1;
        }
        line
    }

    /// The same measure, opening each paragraph `n` cells in.
    pub fn with_indent(self, n: usize) -> Measure<'a> {
        Measure {
            indent: n.min(8),
            ..self
        }
    }

    /// How many cells open a paragraph.
    pub fn indent(self) -> usize {
        self.indent
    }

    /// How far in this row of this line begins.
    ///
    /// Only a paragraph's own first row, and only when it is prose: a heading
    /// or a list item carries its own leading structure, and pushing it two
    /// cells right would say something about it that is not true.
    pub fn indent_of(self, line: usize, text: &str, index_in_line: usize) -> usize {
        match index_in_line == 0 && crate::zong::opens_a_paragraph(text) {
            true => self.indent_on(line),
            false => 0,
        }
    }

    /// How many cells open `line` — none, on the paragraph being typed into.
    fn indent_on(self, line: usize) -> usize {
        match self.open == Some(line) {
            true => 0,
            false => self.indent,
        }
    }

    /// How wide the page is.
    pub fn width(self) -> usize {
        self.width
    }

    /// The markup off `line`, as columns within it.
    fn off(self, line: usize) -> Vec<(usize, usize)> {
        (self.hidden)(line)
    }

    /// The same measure, with `drawn` naming the text drawn into each line
    /// that the file does not contain.
    ///
    /// All of it counts as standing before the caret until
    /// [`Self::with_typed_drawn`] says which part of it does.
    pub fn with_drawn(self, drawn: &'a dyn Fn(usize) -> Vec<(usize, String)>) -> Measure<'a> {
        Measure {
            drawn,
            typed: drawn,
            ..self
        }
    }

    /// The same measure, with `typed` naming the part of the drawn the writer
    /// typed — the part the caret stands after. See [`Self::typed`].
    pub fn with_typed_drawn(self, typed: &'a dyn Fn(usize) -> Vec<(usize, String)>) -> Measure<'a> {
        Measure { typed, ..self }
    }

    /// The drawn text on `line`, anchored at columns within it, in order.
    ///
    /// Public because the renderer and the mouse have to draw and resolve the
    /// very cells this measured: three questions, one answer.
    pub fn drawn_on(self, line: usize) -> Vec<(usize, String)> {
        let mut runs = (self.drawn)(line);
        runs.sort_by_key(|&(at, _)| at);
        runs
    }

    /// The typed part of the drawn on `line` — what the caret stands after.
    fn typed_on(self, line: usize) -> Vec<(usize, String)> {
        (self.typed)(line)
    }
}

/// How many cells the drawn runs anchored at `at` take.
fn drawn_width(drawn: &[(usize, String)], at: usize) -> usize {
    drawn
        .iter()
        .filter(|&&(a, _)| a == at)
        .map(|(_, text)| yumete_cjk::str_width(text))
        .sum()
}

/// One visual row: the slice of a logical line that fits on one screen row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    /// The logical (buffer) line this row is a wrapped piece of.
    pub line: usize,
    /// Which piece of that line it is (`0` for the first).
    pub index_in_line: usize,
    /// Char index of the first character in the row.
    pub start: usize,
    /// Char index one past the last character in the row.
    pub end: usize,
    /// Whether this is the last row the paragraph wraps into — the row whose
    /// end is the paragraph's own end, and so the only one carrying the line
    /// break.
    pub ends_line: bool,
}

impl Row {
    /// Whether this row starts a new paragraph rather than continuing one.
    pub fn starts_line(&self) -> bool {
        self.index_in_line == 0
    }
}

/// Where a char index sits in the wrapped grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// The logical line.
    pub line: usize,
    /// Which visual row of that line.
    pub index_in_line: usize,
    /// The display column within the row, in cells.
    pub column: usize,
}

/// The first row of a page: what the renderer scrolls in.
///
/// Ordered so a viewport can be compared against the cursor's own position
/// without ever counting rows from the top of the document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Anchor {
    pub line: usize,
    pub index_in_line: usize,
}

impl From<Position> for Anchor {
    fn from(p: Position) -> Anchor {
        Anchor {
            line: p.line,
            index_in_line: p.index_in_line,
        }
    }
}

/// The text of `line` with its line break stripped.
fn line_text(rope: &Rope, line: usize) -> String {
    if line >= rope.len_lines() {
        return String::new();
    }
    let slice = rope.line(line);
    let mut text = slice.to_string();
    while text.ends_with('\n') || text.ends_with('\r') {
        text.pop();
    }
    text
}

/// **How many characters are on `line`**, without building the line.
///
/// Warning: **不要為了數一行有多長把它整個變成 `String`**（2026-10-03 量出來的）。一行
/// 四十兆的檔上 `:view-wrap off` 比開着折行慢**七倍半**（14.4 秒對 1.9 秒），整個
/// 差額就是 [`rows_of_line`] 不折行那一支裏的 `line_text(...).chars().count()`——
/// 它每一幀被叫很多次，每一次拷貝兩千萬個字符再數一遍。rope 自己數得出來。
fn line_len_chars(rope: &Rope, line: usize) -> usize {
    if line >= rope.len_lines() {
        return 0;
    }
    let slice = rope.line(line);
    let mut len = slice.len_chars();
    while len > 0 && matches!(slice.char(len - 1), '\n' | '\r') {
        len -= 1;
    }
    len
}

/// Whether `c` is part of a Latin word that should not be split across rows.
fn latin_word_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '\'' | '-' | '’')
}

/// Whether `c` may not begin a row (行頭禁則): the marks that belong to the
/// text before them.
///
/// **clreq §6.1.1, 基本處理** — the level it calls 最推薦:
///
/// > 點號（頓號、逗號、句號、冒號、分號、嘆號、問號）、結束引號、結束括號、
/// > 結束書名號乙式（單雙書名號）、連接號、間隔號、分隔號不能出現在一行的開頭。
///
/// The list used to be derived from [`yumete_cjk::hangs_in_the_margin`], which
/// is a *rendering* question — what fits in a one-cell margin — and answers
/// `false` for every mark with no half-width form. So `》〉』】〕` and the
/// 簡體 quotes `”’` were free to open a column, and did: thirteen offences on
/// one page of 論語集解 at `zong_length 12`, every one of them a 書名號.
/// Line breaking and margin hanging are different questions and are asked
/// separately now.
pub(crate) fn forbidden_at_row_start(c: char) -> bool {
    matches!(
        c,
        // 點號
        '、' | '，' | '。' | '．' | '：' | '；' | '！' | '？' | '｡' | '､'
        // 結束引號・結束括號・結束書名號
        | '」' | '』' | '”' | '’' | '）' | '〕' | '】' | '｝' | '］' | '》' | '〉'
        | '〞' | '﹂' | '﹄' | '︶' | '︸' | '︺' | '︼' | '︾' | '﹀'
        // 連接號・間隔號・分隔號・疊字號
        | '·' | '‧' | '・' | '—' | '－' | '～' | '〜' | '/' | 'ー' | '々' | '〻'
        // …and the Latin marks a Chinese manuscript still contains
        | ')' | ']' | '}' | ',' | '.' | ';' | ':' | '!' | '?' | '…' | '‥'
    )
}

/// Whether `c` may not end a row (行末禁則): a bracket that introduces what
/// follows it.
///
/// > 開始引號、開始括號、開始書名號乙式等符號，不能出現在一行的結尾。
/// > — clreq §6.1.1
pub(crate) fn forbidden_at_row_end(c: char) -> bool {
    matches!(
        c,
        '「' | '『' | '“' | '‘' | '（' | '〔' | '【' | '｛' | '［' | '《' | '〈'
        | '〝' | '﹁' | '﹃' | '︵' | '︷' | '︹' | '︻' | '︽' | '︿'
        | '(' | '[' | '{'
    )
}

/// Whether `c` is half of a mark that takes two squares and may not be split.
///
/// **clreq §6.1.2.1 分離禁止**:
///
/// > 以下標點符號佔用二個漢字的空間，應視為一體，不能拆成兩行。
///
/// `——` and `……` are one mark written twice, and a row that ends with the
/// first half leaves the second stranded at the head of the next — which
/// vertically is worse still, since `︱` and `︙` are *stroke* glyphs and the
/// reader sees a rule that stops and starts again.
pub(crate) fn half_of_a_pair(c: char) -> bool {
    matches!(c, '—' | '…' | '︱' | '︙' | '‥' | '﹏')
}

/// How far a kinsoku adjustment may pull characters onto the next row.
///
/// Two is enough for the cases that actually occur — a stop followed by a
/// closing quote, 」。— and small enough that a row of nothing but punctuation
/// cannot cascade the whole paragraph one character to the right.
pub(crate) const MAX_KINSOKU_RETREAT: usize = 2;

/// The rows `text` wraps into at `width` cells, as char ranges within the line.
///
/// Always returns at least one row, so an empty paragraph still occupies a
/// screen row and can hold the caret.
pub fn line_rows(text: &str, width: usize) -> Vec<(usize, usize)> {
    line_rows_hiding(text, width, &[])
}

/// [`line_rows`], with `hidden` naming the characters that are not on the page.
///
/// They take no width, so a row holds as much *writing* as fits — and a run of
/// markup can never fill a row on its own and draw as a blank line.
pub fn line_rows_hiding(
    text: &str,
    width: usize,
    hidden: &[(usize, usize)],
) -> Vec<(usize, usize)> {
    line_rows_indented(text, width, hidden, 0)
}

/// [`line_rows_hiding`], with `indent` cells taken off the paragraph's first
/// row.
pub fn line_rows_indented(
    text: &str,
    width: usize,
    hidden: &[(usize, usize)],
    indent: usize,
) -> Vec<(usize, usize)> {
    line_rows_drawing(text, width, hidden, indent, &[])
}

/// [`line_rows_indented`], with `drawn` naming text drawn into the line that
/// the line does not contain.
///
/// A drawn run is measured with the character it is anchored at and never
/// split from it, so it cannot be left hanging at the foot of one row with its
/// anchor at the head of the next. It is *not* a character: the rows are still
/// char ranges within the line, and a drawn changes only where they break.
pub fn line_rows_drawing(
    text: &str,
    width: usize,
    hidden: &[(usize, usize)],
    indent: usize,
    drawn: &[(usize, String)],
) -> Vec<(usize, usize)> {
    line_rows_for_caret(text, width, hidden, indent, drawn, false)
}

/// [`line_rows_drawing`], plus **whether the caret is standing at the end of
/// this line** and so needs a cell that the last row has not got.
///
/// Warning: **這一格從前是無條件開的**，理由是「一段正好填滿最後一行，段末的光標
/// 就沒地方站」。理由對，可它一天到晚都在付：**一段話正好排滿一行，屏幕上就
/// 憑空多一個空行**，讀起來是分了段——寫小說最常見的排版事故，2026-09-23 報的。
/// 而且它不只坑編輯器：面板、導出、預覽都走這一支，那裏根本沒有光標。
///
/// 所以現在按 vi 的辦法：**光標到了纔開**。Normal 模式下光標落在字上，行末那一
/// 格沒人要；Insert 模式打到行尾，它就長出來——這也正是 vim 在 `wrap` 下的樣子。
pub fn line_rows_for_caret(
    text: &str,
    width: usize,
    hidden: &[(usize, usize)],
    indent: usize,
    drawn: &[(usize, String)],
    room_at_end: bool,
) -> Vec<(usize, usize)> {
    let width = width.max(1);
    let indent = if crate::zong::opens_a_paragraph(text) {
        indent.min(width.saturating_sub(1))
    } else {
        0
    };
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return vec![(0, 0)];
    }

    // Char index and display width of every grapheme, so a break never lands
    // inside a cluster and a wide glyph is never half on the row.
    let mut cuts: Vec<usize> = Vec::with_capacity(chars.len() + 1);
    let mut widths: Vec<usize> = Vec::with_capacity(chars.len());
    // What stands *before* each grapheme that the line has no characters for.
    // Kept apart from `widths` rather than added into it, because `widths` is
    // also how 禁則 asks 「is this character drawn」: a hidden `**` carrying a
    // drawn would otherwise start counting as writing.
    let mut lead: Vec<usize> = Vec::with_capacity(chars.len());
    let mut at = 0;
    for g in graphemes(text) {
        cuts.push(at);
        // A character that is not drawn takes no room, so it cannot push the
        // one after it onto the next row.
        let off = hidden.iter().any(|&(a, b)| at >= a && at < b);
        widths.push(if off { 0 } else { grapheme_width(g) });
        lead.push(drawn_width(drawn, at));
        at += g.chars().count();
    }
    cuts.push(at);
    // A run at the very end of the line has no character to stand before;
    // it stands after the last one, on the last row.
    let tail = drawn_width(drawn, at);

    let mut rows = Vec::new();
    let mut g = 0; // grapheme index of the row start
    let mut last_width = 0;
    while g < widths.len() {
        // The paragraph's first row is the indent narrower.
        let width = width - if rows.is_empty() { indent } else { 0 };
        // As many graphemes as fit, at least one.
        let mut used = 0;
        let mut end = g;
        while end < widths.len() && (used + lead[end] + widths[end] <= width || end == g) {
            used += lead[end] + widths[end];
            end += 1;
        }
        if end < widths.len() {
            end = g + adjusted_break(&chars, &cuts, &widths, g, end);
        }
        last_width = widths[g..end].iter().chain(&lead[g..end]).sum();
        rows.push((cuts[g], cuts[end]));
        g = end;
    }
    // The last row carries whatever was anchored past the last character.
    last_width += tail;
    // A paragraph that exactly fills its last row leaves the end-of-paragraph
    // caret nowhere to stand: the column after the last glyph is off the row.
    // Open one more, empty, row for it — **and only for it**: see this
    // function's own note on why it is not opened unasked.
    if room_at_end && last_width >= width - if rows.len() == 1 { indent } else { 0 } {
        rows.push((cuts[widths.len()], cuts[widths.len()]));
    }
    rows
}

/// The graphemes of `text` as `(char index, display width)`.
///
/// Columns are measured over *graphemes*, the same unit [`line_rows`] breaks
/// on: counting a combining mark or a flag's second half as its own column puts
/// the caret in a cell that has no glyph in it, and lets `j` land inside a
/// cluster that a later edit would then cut in half.
fn steps(text: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut at = 0;
    graphemes(text).map(move |g| {
        let start = at;
        at += g.chars().count();
        (start, grapheme_width(g))
    })
}

/// Where the row starting at grapheme `g` should really end, given that `end`
/// is where it stops fitting. Returns a count of graphemes, always at least 1.
fn adjusted_break(
    chars: &[char],
    cuts: &[usize],
    widths: &[usize],
    g: usize,
    end: usize,
) -> usize {
    // **The character the reader sees.** A grapheme that is not drawn has no
    // width, and 禁則 is about what stands at a row's head and foot — so a
    // retreat onto a hidden `` ` `` used to stop there, having moved nothing,
    // and the row after it opened with 。 in `development.md` itself.
    // The vertical page was taught this; this is the same rule, on the other
    // side, asked the same way.
    let seen = |i: usize| -> Option<char> {
        (widths.get(i).copied().unwrap_or(1) > 0)
            .then(|| chars.get(cuts.get(i).copied().unwrap_or(0)).copied())
            .flatten()
    };
    // The last drawn character at or before `i`, and the first at or after it.
    let before = |i: usize| (g..i).rev().find_map(seen).unwrap_or(' ');
    let after = |i: usize| (i..widths.len()).find_map(seen).unwrap_or(' ');
    let char_at = |i: usize| chars.get(cuts[i]).copied().unwrap_or(' ');

    // 禁則處理 first: pull the offending character down with its neighbour.
    let kinsoku = |mut cut: usize| {
        for _ in 0..MAX_KINSOKU_RETREAT {
            if cut <= g + 1 {
                break;
            }
            // 分離禁止 first: `——` and `……` are one mark, and a row may not
            // end with half of one.
            let split_a_pair = half_of_a_pair(before(cut)) && before(cut) == after(cut);
            if split_a_pair
                || forbidden_at_row_start(after(cut))
                || forbidden_at_row_end(before(cut))
            {
                // **One retreat is one character the reader can see.** Stepping
                // by grapheme spent both tries walking back over a hidden run —
                // `字**」**。` — and moved the boundary past nothing, so the 。
                // still opened the next row. The markup travels down with the
                // character it belongs to.
                let mut back = cut - 1;
                while back > g + 1 && widths.get(back).copied().unwrap_or(1) == 0 {
                    back -= 1;
                }
                cut = back;
            } else {
                break;
            }
        }
        cut
    };
    let mut cut = kinsoku(end);

    // Then keep a Latin word whole. A space is the obvious place to break at,
    // and in English prose it is the only one there is.
    if latin_word_char(char_at(cut.saturating_sub(1))) && latin_word_char(char_at(cut)) {
        if let Some(space) = (g + 1..cut).rev().find(|&i| char_at(i - 1) == ' ') {
            cut = space;
        } else if let Some(word) = (g + 1..cut).rev().find(|&i| !latin_word_char(char_at(i - 1))) {
            // …and in Chinese prose there is no space, because the word
            // follows a 漢字: 一二三四五六Helix七八 has one word in it and
            // nowhere to break. Retreat to where the word begins instead —
            // but only while at least half the row survives, so a single
            // 40-letter token cannot empty the row it is on.
            if (word - g) * 2 >= end - g {
                cut = word;
            }
        }
    }
    // …and 禁則 again over what the word rule chose, because it may have put a
    // bracket back at the row's end: a URL in `[名字](網址)` retreats to the
    // start of `https`, which is exactly one character after the `(` that
    // 行末禁則 had just pulled down.
    (kinsoku(cut) - g).max(1)
}

/// The width to measure at when soft wrap is **off**.
///
/// Wrap off means one row per paragraph, however long — which is what a very
/// large width gives, through the same code the wrapped page uses. Going round
/// it instead (`motion::down`, over the source) was a second answer to「which
/// column is this character in」, and it did not know what was off the page: with
/// 所見即所得 on and wrap off, `j` landed a glyph left per `**` above it, and
/// could land *on* an asterisk that is not drawn.
pub const NO_WRAP: usize = 1 << 40;

/// How many wrapped paragraphs to remember.
///
/// One keystroke asks where the cursor is, what its goal column is, which rows
/// fill the page and how far the cursor is from its top — four or five walks
/// over the *same* paragraph, and on a chapter typed as one paragraph each walk
/// is the whole chapter. Eight covers the paragraphs a page touches with room
/// to spare.
const REMEMBERED_PARAGRAPHS: usize = 8;

/// One remembered paragraph: the hash of its text, the width it was wrapped at,
/// and the rows that came out.
type Remembered = (u64, usize, Vec<(usize, usize)>);

/// 同一段的上一份答案，連着它當時成立的那個 revision：
/// `(buffer, line, width, revision, rows)`。
///
/// 名字排在 [`Remembered`] 旁邊，因為兩張是同一種表——分別只在拿什麼當鍵。
type Continued = (u64, usize, usize, u64, Vec<(usize, usize)>);

thread_local! {
    /// Wrapped paragraphs, most recently used first: `(text hash, width, rows)`.
    ///
    /// Keyed by a hash of the paragraph's own text rather than by a line number
    /// or a buffer revision, exactly as the segmentation cache is: a matching
    /// hash is a correct answer whatever else in the document — or in another
    /// document — has moved since.
    static ROWS: RefCell<Vec<Remembered>> = const { RefCell::new(Vec::new()) };
    /// **The last answer for a paragraph, kept across the edit that spoils
    /// it** (#366): `(buffer, line, width, revision, rows)`.
    ///
    /// [`ROWS`] is keyed by a hash that has the revision in it, so an edit
    /// makes every entry for that document unreachable — which is correct, and
    /// which also throws away the one thing that would let the next answer be
    /// *continued* rather than remade. This keeps exactly that: one row list
    /// per paragraph, the revision it was true of, and nothing else.
    static LAST: RefCell<Vec<Continued>> = const { RefCell::new(Vec::new()) };
    #[cfg(test)]
    static WHY: RefCell<std::collections::HashMap<String, usize>> =
        RefCell::new(std::collections::HashMap::new());

    /// How many paragraphs have actually been wrapped, for the test that keeps
    /// a keystroke from quietly becoming four passes over a chapter again.
    static WRAPPED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many paragraphs have been wrapped since [`reset_wrap_count`].
#[cfg(test)]
fn wrap_count() -> usize {
    WRAPPED.with(|n| n.get())
}

/// Start counting wrapped paragraphs again.
#[cfg(test)]
fn reset_wrap_count() {
    WRAPPED.with(|n| n.set(0));
}

/// A hash of `line`'s text, taken over the rope's own chunks so that asking
/// costs no allocation. Two identical paragraphs stored differently may hash
/// differently — that is a cache miss, which is merely slow, never wrong.
fn line_hash(rope: &Rope, line: usize) -> u64 {
    let mut hasher = DefaultHasher::new();
    for chunk in rope.line(line).chunks() {
        chunk.hash(&mut hasher);
    }
    hasher.finish()
}

/// The rows `line` wraps into, remembering the last few answers.
///
/// The paragraph's text is only materialised on a miss: turning a 100,000-
/// character paragraph into a `String` four times per keystroke is most of what
/// made an unmemoised `j` slow, and the questions a keystroke asks are all
/// about the same handful of paragraphs.
pub fn rows_of_line_for_test(rope: &Rope, line: usize, m: Measure) -> Vec<(usize, usize)> {
    rows_of_line(rope, line, m)
}

fn rows_of_line(rope: &Rope, line: usize, m: Measure) -> Vec<(usize, usize)> {
    // **A table row is one row** (#275), so there is nothing to measure and
    // nothing to remember: the whole line, whatever the measure is. Ahead of
    // the cache, because the answer does not depend on the width and caching
    // it under one would only make the next width ask again.
    if m.unwrapped(line) {
        return vec![(0, line_len_chars(rope, line))];
    }
    let hidden = m.off(line);
    // The hidden runs are part of the answer, so they are part of the key: the
    // same paragraph wraps differently when the cursor opens a construct in it.
    let mut hasher = DefaultHasher::new();
    // **Which version of the document, not what it says** (#315), whenever the
    // caller could say — see [`Measure::version`].
    match m.version {
        Some(version) => (version, line).hash(&mut hasher),
        None => line_hash(rope, line).hash(&mut hasher),
    }
    hidden.hash(&mut hasher);
    // The indent changes where a row breaks, so it is part of the key too.
    m.indent_on(line).hash(&mut hasher);
    // …and so does the drawn text, for exactly the same reason: the same
    // paragraph wraps differently while a candidate stands in the middle of it.
    let drawn = m.drawn_on(line);
    drawn.hash(&mut hasher);
    // …and whether this line's last row owes the caret a cell — see
    // [`line_rows_for_caret`]. It moves when the caret does, so it is in the
    // key; it is true on at most one line, so at most one line is remade.
    let room = m.caret_ends(rope, line);
    room.hash(&mut hasher);
    let hash = hasher.finish();
    if let Some(rows) = remembered(hash, m.width) {
        return rows;
    }
    // **Continue the last answer rather than remake it** (#366). Only where
    // there is nothing else on the row to move: a hidden run or a drawn one
    // has coordinates of its own that an edit shifts too, and getting that
    // wrong would put the caret in a column the page does not have.
    // Warning: **不走這條路的時候：行末那一格開着。** `LAST` 是按
    // `(buffer, line, width, revision)` 存的，裏面沒有「光標在不在行末」——存
    // 一份帶着那一格的進去，光標一走它就成了假的。那一行只有一行，重排一次不
    // 值得為它擴鍵。
    if hidden.is_empty() && drawn.is_empty() && !room {
        if let Some(rows) = carried_on(rope, line, m) {
            remember(hash, m.width, &rows);
            keep_last(m, line, &rows);
            return rows;
        }
    }
    WRAPPED.with(|n| n.set(n.get() + 1));
    let rows = line_rows_for_caret(
        &line_text(rope, line),
        m.width,
        &hidden,
        m.indent_on(line),
        &drawn,
        room,
    );
    remember(hash, m.width, &rows);
    if !room {
        keep_last(m, line, &rows);
    }
    rows
}

/// Keep this answer as the one the *next* edit will be continued from (#366).
fn keep_last(m: Measure, line: usize, rows: &[(usize, usize)]) {
    let Some((buffer, revision)) = m.version else {
        return;
    };
    LAST.with(|cache| {
        let mut cache = cache.borrow_mut();
        let key = |e: &(u64, usize, usize, u64, Vec<(usize, usize)>)| {
            e.0 == buffer && e.1 == line && e.2 == m.width
        };
        if let Some(i) = cache.iter().position(key) {
            cache.remove(i);
        }
        cache.insert(0, (buffer, line, m.width, revision, rows.to_vec()));
        cache.truncate(REMEMBERED_PARAGRAPHS);
    });
}

/// The rows for `line`, **continued from the answer before this edit** (#366).
///
/// `None` when it cannot be done, and then the caller wraps the paragraph the
/// long way. This is an optimisation and is allowed to give up; it is not
/// allowed to be wrong.
///
/// Typing one character into a paragraph of a million cost **25.4 ms a key**,
/// and 22.7 ms of that was this one call re-deciding every row break in the
/// paragraph — breaks settled by text the edit did not touch. Three facts make
/// the work small:
///
/// - **Nothing before the edit can have moved.** A row's break is decided by
///   the text from its own start, so a row that ends before the edit ends
///   where it did.
/// - **禁則 looks ahead**, so a break just *before* the edit can still change
///   its mind about it. Two rows of slack, and the question does not arise.
/// - **The tail usually lands back where it was.** Walk forward from the
///   resume point and watch for a new row that begins exactly `delta` along
///   from where an old one did: from there on the paragraph wraps as it wrapped
///   before, one character over. In 中文 — no spaces, every row the same width
///   — that happens on the very first row, so the whole of a million-character
///   paragraph is answered by shifting a list of numbers.
fn carried_on(rope: &Rope, line: usize, m: Measure) -> Option<Vec<(usize, usize)>> {
    #[cfg(test)]
    fn note(why: &str) {
        WHY.with(|w| *w.borrow_mut().entry(why.to_string()).or_insert(0usize) += 1);
    }
    #[cfg(not(test))]
    fn note(_why: &str) {}

    let Some((buffer, revision)) = m.version else { note("no version"); return None };
    let Some((at, delta)) = m.edit else { note("no edit"); return None };
    // One edit between the two answers, or the `edit` we were handed is not
    // the one that tells them apart.
    let old = LAST.with(|cache| {
        cache
            .borrow()
            .iter()
            .find(|e| e.0 == buffer && e.1 == line && e.2 == m.width && e.3 + 1 == revision)
            .map(|e| e.4.clone())
    });
    let Some(old) = old else { note("no last"); return None };
    if old.is_empty() || delta == 0 {
        note("empty or no delta");
        return None;
    }
    let start = rope.line_to_char(line);
    let end = start + line_text(rope, line).chars().count();
    // The edit has to be *inside* this paragraph, and has to have left it a
    // paragraph: a newline either way makes this a different question.
    let Some(col) = at.checked_sub(start) else { note("before the line"); return None };
    // **The edit begins at `at` in both texts.** Nothing before it moved,
    // whichever way it went — an insert pushes what follows along and a
    // removal pulls it back, and neither touches what came first.
    //
    // This added the removed length back on until 2026-09-11, which named a
    // row later than the one the edit fell in and so reused *more* of the old
    // list than the edit allowed. Reasoned, not caught: the test that looked
    // as though it had caught it was lying to itself at the time (two cases
    // sharing a buffer number and handing each other their rows), and with
    // that fixed the old arithmetic passes — the two rows of slack below were
    // covering for it. Slack is not a licence to be wrong about where the
    // edit was.
    let was = col;
    if at > end || was > old.last().map_or(0, |r| r.1) {
        note("outside the paragraph");
        return None;
    }

    // Resume two rows before the one the edit fell in — 禁則 looks ahead, so
    // a break settled just before the edit can still change its mind about it.
    // Not a guess: with no slack at all, the test that compares a continued
    // wrap against a fresh one goes red.
    let touched = old.partition_point(|&(s, _)| s <= was).saturating_sub(1);
    let resume = touched.saturating_sub(2);
    // **Nothing to keep is nothing to gain.** An edit in the first rows of a
    // paragraph leaves no prefix to reuse, and going on would only add a copy
    // of the tail to the wrap that has to happen anyway — measured at 31 ms
    // against the plain path's 25 on a million characters.
    if resume == 0 {
        note("nothing before it");
        return None;
    }
    let Some(&(from, _)) = old.get(resume) else { note("no resume row"); return None };
    if from > col {
        note("resume past the edit");
        return None;
    }

    // Walk the new text **from there to the end of the paragraph**. Every row
    // before `resume` is kept as it was, which is the whole of the saving and
    // the only part of it that can be proved: a row's break is decided by the
    // text from its own start, and none of that text moved.
    let upto = end;
    // `to_string` and not `chars().collect()`: the rope hands over its chunks
    // whole, and collecting character by character was **38 times slower**
    // (6.7 ms against 0.18 ms on a million characters).
    let text = rope.slice(start + from..upto).to_string();
    let indent = match resume {
        0 => m.indent_on(line),
        _ => 0,
    };
    let walked = line_rows_drawing(&text, m.width, &[], indent, &[]);
    let mut out: Vec<(usize, usize)> = old[..resume].to_vec();
    for (i, &(rs, re)) in walked.iter().enumerate() {
        out.push((from + rs, from + re));
        // **And the old wrap may come back**, one `delta` along: from a row
        // that begins where an old row began before the edit, the text is the
        // old text exactly, so it wraps the way it wrapped and the rest of the
        // list is the old list moved over. That is worth watching for in prose,
        // where a break lands on a space and an inserted character pushes the
        // whole tail along.
        //
        // It does **not** happen in 中文, and that is not a bug in the test: a
        // CJK line breaks between any two characters, so its breaks are decided
        // by *position* and not by content — every fortieth character, whatever
        // is written there. Insert one character and the tail's boundaries stay
        // on the same grid while its content shifts under them, so no old row
        // ever began where the proof needs one to have begun. The rows happen
        // to come out the same; nothing here can know that, and guessing it
        // would put the caret in a column the page does not have.
        // **Only the first rows are asked**, and by binary search. Scanning
        // the old list for every walked row is quadratic in the rows, which on
        // a million characters is 12,500 × 12,500 comparisons — 240 ms a key,
        // ten times worse than the wrap it was meant to save. And it buys
        // nothing anyway: a tail that lands back on the old wrap does it at
        // once or not at all.
        if i < RESUME_ROWS && i + 1 < walked.len() {
            let want = (from + re).checked_add_signed(-delta)?;
            if let Ok(k) = old[touched..].binary_search_by_key(&want, |&(s, _)| s) {
                let k = touched + k;
                out.extend(old[k..].iter().map(|&(s, e)| {
                    (s.saturating_add_signed(delta), e.saturating_add_signed(delta))
                }));
                note("carried, resynced");
                return Some(out);
            }
        }
    }
    note("carried, walked the tail");
    Some(out)
}

/// How many rows `carried_on` will wrap before it gives up and lets the
/// paragraph be done the long way.
const RESUME_ROWS: usize = 8;

/// The rows remembered for this paragraph, if any, moved back to the front.
fn remembered(hash: u64, width: usize) -> Option<Vec<(usize, usize)>> {
    ROWS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let i = cache
            .iter()
            .position(|&(h, w, _)| h == hash && w == width)?;
        // A page keeps asking about the same paragraphs.
        let entry = cache.remove(i);
        let rows = entry.2.clone();
        cache.insert(0, entry);
        Some(rows)
    })
}

/// Remember `rows` for this paragraph, dropping the least recently asked about.
fn remember(hash: u64, width: usize, rows: &[(usize, usize)]) {
    ROWS.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.insert(0, (hash, width, rows.to_vec()));
        cache.truncate(REMEMBERED_PARAGRAPHS);
    });
}

/// 一塊多少個字符——[`Widths`] 每隔這麼多字符記一條累計寬度。
///
/// 4096 是這麼挑的（2026-10-08 量的，一個字素約 21 ns）：一次查詢的代價是兩頭各
/// 走至多一塊，4096 個字符是 **86 µs**，一幀問四五次也還在半毫秒以內；而一行
/// 1242 萬字的稿子上索引本身只有 12 419 022 / 4096 ≈ 3031 條、48 KB。再大
/// （65536）兩頭那一走漲到 1.4 ms，一幀問五次就看得見了；再小（256）索引漲到
/// 48 500 條而那一走本來就只有 5 µs，省不出什麼來。
///
/// 也是**建不建索引**的門檻：一行不到一塊長，索引裏連一個內部切點都沒有，走的
/// 就是從前那一條路。
pub(crate) const BLOCK: usize = 4096;

/// **這個切點記得下嗎**——只問一句：它左邊那個字符是不是區域指示符（國旗那
/// 半邊，U+1F1E6..U+1F1FF）。
///
/// 索引成立的前提是：切點 `p` 在**從任何一處起**的分段裏都是字素邊界，而且 `p`
/// 右邊的分段與「從 `p` 起重新分段」逐一相同。索引是**從行首**分段算出來的，查
/// 詢卻是**從 `a`** 分段走的（從前那一支就是 `steps(slice(a..b))`），兩者只有在
/// 這種切點上接得起來纔保證逐格相同。
///
/// 一句話夠用，理由是 UAX #29 那十幾條裏只有三條要往左看一個字符以上——GB9c
/// （印度系連寫）、GB11（ZWJ × 繪文字）、GB12／GB13（區域指示符兩個一對）：
///
/// - 其餘各條（CR LF、Control、諺文 GB6-8、Extend／ZWJ、SpacingMark、Prepend）
///   只看切點左右那一個字符。切點本來就是行首那一份分段的邊界，也就是說那一對
///   字符按這些條**斷開**；而「斷不斷」只看這一對，從哪裏起數都一樣。
/// - GB9c 與 GB11 要左邊配上一段（輔音…連接符、繪文字…ZWJ）纔**合併**。從 `a`
///   起分段看到的左邊只會更少，配不上的照樣配不上——所以它們只會多斷，不會少
///   斷，而切點要的正是「斷」。右邊也不會兩樣：那一段配料若落在 `p` 左邊，
///   GB9 早把 `p` 上的 ZWJ／連接符併到左邊去了，`p` 就不會是邊界。
/// - **只剩 GB12／GB13。** 區域指示符是從左邊**數出對**來的：四個 U+1F1E6 從頭數
///   是兩面旗，從第二個字符起數是「一個落單的、一面旗、一個落單的」——同一個下
///   標在一種分段裏是邊界，在另一種裏在字素中間。數的起點只被非區域指示符打斷，
///   所以「左邊那個不是區域指示符」正好把這一條關掉。
///
/// 一行全是國旗就一個切點都記不下來（見那條性質測試），於是照從前那樣逐字素
/// 走——慢，不會錯。
fn clean_cut(before: char) -> bool {
    !matches!(before as u32, 0x1F1E6..=0x1F1FF)
}

/// **一行裏 `[a, b)` 這一段佔多少格**，逐字素走——`hidden` 裏的字符不算。
///
/// 索引算得出來的那一支（[`Widths::width`]）逐格與這一支相同，而這一支就是
/// 2026-10-08 之前 [`position`] 裏那幾行：答案以它為準。
pub fn walk_width(rope: &Rope, line: usize, a: usize, b: usize, hidden: &[(usize, usize)]) -> usize {
    if b <= a {
        return 0;
    }
    let start = rope.line_to_char(line);
    let text = rope.slice(start + a..start + b).to_string();
    steps(&text)
        .filter(|(i, _)| {
            let at = a + i;
            !hidden.iter().any(|&(x, y)| at >= x && at < y)
        })
        .map(|(_, w)| w)
        .sum()
}

/// `hidden` 併成互不相交、按頭排好的一串。
///
/// 從前那道過濾是 `any(…)`，所以重疊的區間只算一次；索引那一支是**減**出來的，
/// 重疊就會減兩遍。併一次，兩條路就說同一句話。
fn merged(hidden: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut spans: Vec<(usize, usize)> = hidden.iter().copied().filter(|&(a, b)| a < b).collect();
    spans.sort_unstable();
    let mut out: Vec<(usize, usize)> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// **一行的前綴寬度索引**（2026-10-08）：在若干個乾淨切點上記下從行首數起的累計
/// 顯示寬度，於是「行首到第 `col` 個字符有多寬」不必每次逐字素走一遍。
///
/// 為什麼要它：不折行的時候一段就是一行，於是 [`position`] 裏那句「只量這一行裏
/// 光標前面那一段」量的是**整個前綴**。一行 1 242 萬字的檔上，光標在行尾按一下
/// `h` 要 **223 ms**（行首 5.5 ms；開着折行則處處都快，因為那時行起點就在光標邊
/// 上，那句註釋說的「量一行不量一段」只在折行開着的時候是真的）。
///
/// 一行走一遍就建得出來，而它只隨編輯改變，所以是 [`Measure::widths`] 交進來的
/// ——記在哪裏、按什麼作廢是前端的事（`Editor::width_memo`）。
#[derive(Debug)]
pub struct Widths {
    /// `(字符下標, 行首到這個下標的累計寬度)`，按下標遞增，第一條恆為 `(0, 0)`、
    /// 最後一條恆為行末。
    cuts: Vec<(usize, usize)>,
}

impl Widths {
    /// 走一遍 `line`，每隔 [`BLOCK`] 個字符記一條（只記在乾淨切點上）。
    pub fn of(rope: &Rope, line: usize) -> Widths {
        Widths::every(rope, line, BLOCK)
    }

    /// [`Self::of`]，塊多大由調用方說。
    ///
    /// 性質測試拿很小的塊（4、8、64）把一行裏每一列都掃一遍——[`BLOCK`] 那麼大的
    /// 塊要掃到同樣的密度得走上億個字素，而塊的大小是個調參，對不對不歸它管。
    fn every(rope: &Rope, line: usize, block: usize) -> Widths {
        let block = block.max(1);
        let text = line_text(rope, line);
        let mut cuts: Vec<(usize, usize)> = vec![(0, 0)];
        let mut at = 0usize;
        let mut width = 0usize;
        let mut before: Option<char> = None;
        for g in graphemes(&text) {
            // 切點記在**這一個字素之前**：它左邊那個字符是上一個字素的末字符。
            if at - cuts[cuts.len() - 1].0 >= block && before.is_some_and(clean_cut) {
                cuts.push((at, width));
            }
            width += grapheme_width(g);
            at += g.chars().count();
            before = g.chars().last();
        }
        // 行末也是一條：它在任何一種分段裏都是邊界，所以當得了右邊那個接點。
        if cuts[cuts.len() - 1].0 != at {
            cuts.push((at, width));
        }
        Widths { cuts }
    }

    /// 最小的、不小於 `at` 的切點。
    fn at_or_after(&self, at: usize) -> Option<(usize, usize)> {
        let i = self.cuts.partition_point(|&(c, _)| c < at);
        self.cuts.get(i).copied()
    }

    /// 最大的、不大於 `at` 的切點。
    fn at_or_before(&self, at: usize) -> Option<(usize, usize)> {
        let i = self.cuts.partition_point(|&(c, _)| c <= at);
        self.cuts.get(i.checked_sub(1)?).copied()
    }

    /// 這一行有多長（行末那一條切點）。
    fn len(&self) -> usize {
        self.cuts[self.cuts.len() - 1].0
    }

    /// **一行裏 `[a, b)` 這一段佔多少格**，`hidden` 裏的字符不算——與
    /// [`walk_width`] 逐格相同，只是中間那一大段是從索引裏減出來的。
    ///
    /// 證明分三句：
    ///
    /// 1. `p` 是乾淨切點，所以它在「從 `a` 起分段」裏也是邊界，於是 `[a, b)` 的
    ///    分段 ＝ `[a, p)` 的分段接上「從 `p` 起」的分段；
    /// 2. 「從 `p` 起」的分段 ＝ 行首那一份在 `[p, …)` 上的截斷（`p` 左邊是個
    ///    平字符，攢不下 ZWJ 與區域指示符的狀態），所以 `[p, q)` 這一段的淨寬就
    ///    是兩條累計之差；
    /// 3. `q` 同樣是乾淨切點，所以尾巴 `[q, b)` 逐字素走出來的那幾個字素，與整
    ///    段走出來的末幾個逐一對應——連 `b` 處那個被切斷的字素也一樣。
    pub fn width(
        &self,
        rope: &Rope,
        line: usize,
        a: usize,
        b: usize,
        hidden: &[(usize, usize)],
    ) -> usize {
        if b <= a {
            return 0;
        }
        // 索引說不了的就照從前走：`b` 超出這一行（不該有，兜底），或者 `a` 與 `b`
        // 之間擠不進兩個切點（那本來也沒什麼可省的）。
        let (Some((p, wp)), Some((q, wq))) = (self.at_or_after(a), self.at_or_before(b)) else {
            return walk_width(rope, line, a, b, hidden);
        };
        if b > self.len() || p >= q {
            return walk_width(rope, line, a, b, hidden);
        }
        let hidden = merged(hidden);
        let mut total = wq - wp
            + walk_width(rope, line, a, p, &hidden)
            + walk_width(rope, line, q, b, &hidden);
        // 中間那一段的淨寬是連 `hidden` 一起算進去的，所以要把落在 `hidden` 裏的
        // 字素減掉。區間少（多半一個都沒有）而且短，所以一段一段地走就夠。
        for &(h0, h1) in &hidden {
            let (x, y) = (h0.max(p), h1.min(q));
            if x < y {
                total -= self.covered(rope, line, x, y);
            }
        }
        total
    }

    /// `[x, y)` 裏起頭的那些字素一共佔多少格——減 `hidden` 用的那一項。
    ///
    /// 從 `x` 左邊那個切點走到 `y` 右邊那個切點：兩頭都是切點，所以走出來的字素
    /// 與行首那一份分段逐一對應，而**起頭在 `[x, y)` 裏**正是從前那道過濾的判準。
    /// 調用方保證 `p <= x < y <= q`，於是這兩個切點都在 `[p, q)` 裏夾得住。
    fn covered(&self, rope: &Rope, line: usize, x: usize, y: usize) -> usize {
        let u = self.at_or_before(x).map_or(x, |(c, _)| c);
        let v = self.at_or_after(y).map_or(y, |(c, _)| c);
        let start = rope.line_to_char(line);
        let text = rope.slice(start + u..start + v).to_string();
        steps(&text)
            .filter(|(i, _)| (x..y).contains(&(u + i)))
            .map(|(_, w)| w)
            .sum()
    }

    /// **從 `s` 往 `goal` 跳過整塊整塊的純文字**（[`char_at_column`] 用）：回答
    /// 「從哪一個切點接着逐字素走」、「走到那裏累計了幾格」、「走到哪裏為止」。
    ///
    /// 只跳 `hidden` 與 `drawn` 都碰不着的塊：那兩樣各有自己的坐標，一格都不能算
    /// 錯。跳到越過 `goal` 的那一塊就停——答案在那一塊裏，所以往後只要走一塊；碰
    /// 上那兩樣（或者塊不夠一整個了）就把終點交回 `e`，從那裏起照從前那樣走。
    fn skip_to(
        &self,
        s: usize,
        e: usize,
        goal: usize,
        col: usize,
        hidden: &[(usize, usize)],
        drawn: &[(usize, String)],
    ) -> Option<(usize, usize, usize)> {
        // 起點本身要是切點，不然「從 `s` 起分段」與行首那一份接不起來。
        let (mut from, mut wf) = self.at_or_before(s).filter(|&(c, _)| c == s)?;
        let mut col = col;
        let mut i = self.cuts.partition_point(|&(c, _)| c <= from);
        loop {
            // 切點用完了，或者下一個已經出了這一行（這一行的末尾），或者這一塊裏
            // 有自己坐標的東西：剩下的照從前那樣逐字素走。
            let Some(&(c, wc)) = self.cuts.get(i).filter(|&&(c, _)| c <= e) else {
                return Some((from, col, e));
            };
            if hidden.iter().any(|&(x, y)| x < c && from < y)
                || drawn.iter().any(|&(x, _)| (from..c).contains(&x))
            {
                return Some((from, col, e));
            }
            if col + (wc - wf) > goal {
                return Some((from, col, c));
            }
            col += wc - wf;
            (from, wf) = (c, wc);
            i += 1;
        }
    }
}

/// How many lines the grid covers — ropey's count, which includes the empty
/// line a trailing newline opens, because that is where the caret sits after
/// `o` and the renderer draws it.
fn line_count(rope: &Rope) -> usize {
    rope.len_lines()
}

/// How many visual rows the logical `line` wraps into (always at least one).
pub fn row_count_in_line(rope: &Rope, line: usize, m: Measure) -> usize {
    rows_of_line(rope, line, m).len()
}

/// Locate the char index `pos` in the wrapped grid.
/// **一行開頭那幾個字**——問「這一行開不開段」只要這麼多（2026-10-08）。
///
/// Warning: **從前這幾處都把整行物化成 `String`。** `Measure::indent_of` 收的是一行文字，
/// 而它轉手交給 `zong::opens_a_paragraph`——那一支只看開頭的空白與第一兩個字。於是
/// 一行 1240 萬字的稿子上，**每動一下光標就拷 37 MB**（`sample` 指的就是這裏）。
///
/// 六十四個字足夠，而且答案與整行逐字相同：
///
/// - 這一段裏有非空白字——`trim_start()` 之後非空，而後面三道關口只看頭兩個字；
/// - 這一段全是空白——那這一行以空白開頭，`starts_with([' ', '\t', '\u{3000}'])`
///   那一道本來就回假；全行都是空白的話第一道也回假。兩條路同一個答案。
pub fn line_head(rope: &Rope, line: usize) -> String {
    let start = rope.line_to_char(line);
    let end = (start + rope.line(line).len_chars()).min(start + 64);
    rope.slice(start..end).to_string()
}

pub fn position(rope: &Rope, pos: usize, m: Measure) -> Position {
    let pos = pos.min(rope.len_chars());
    let line = rope.char_to_line(pos);
    let start = rope.line_to_char(line);
    let col = pos - start;
    let rows = rows_of_line(rope, line, m);

    // The last row that starts at or before the cursor. A cursor resting past
    // the end of the paragraph belongs on the final row, not on a phantom one.
    let index_in_line = rows
        .partition_point(|&(s, _)| s <= col)
        .saturating_sub(1)
        .min(rows.len() - 1);
    let (row_start, _) = rows[index_in_line];
    // Only the part of the row before the cursor is measured — a row, not a
    // paragraph, however long the paragraph is.
    //
    // Warning: **那句話只在折行開着的時候是真的**（2026-10-08 量出來的）。折行關
    // 掉，一段就是一行，於是 `row_start` 恆為 0 而這裏量的是**整個前綴**——一行
    // 1242 萬字的檔上，光標在行尾按一下 `h` 要 223 ms（行首 5.5 ms）。現在中間那
    // 一大段從 [`Widths`] 那份索引裏減出來，兩頭各逐字素走至多一塊。
    //
    // **A character that is not drawn takes no column.** This summed the
    // source, while `line_rows_indented` — the function that decides where the
    // rows actually break — zeroes what is hidden. So with 所見即所得 on, a
    // `j` from a row above a paragraph with `**` in it landed one glyph right
    // per hidden character, and the front end papered over the caret's half of
    // it by subtracting the hidden width again on its way to the screen.
    let hidden = m.off(line);
    let column = m.width_between(rope, line, row_start, col, &hidden);
    // **Text drawn before the caret is page the caret is past.** A run stands
    // before the character it is anchored at, so a run anchored anywhere to
    // the left of the caret is wholly behind it.
    let drawn = m.drawn_on(line);
    let column = column
        + drawn
            .iter()
            .filter(|&&(a, _)| a >= row_start && a < col)
            .map(|(_, text)| yumete_cjk::str_width(text))
            .sum::<usize>();
    // At the caret's *own* anchor the two kinds part company: the candidate is
    // what you typed and the caret is at its end, while the padding that
    // reaches to a table's closing pipe belongs on the far side of the caret.
    // Counting the whole run here drew the caret on the pipe after every
    // keystroke in a table — even an unpadded `|ab|`, whose one space off the
    // pipe is anchored exactly where the caret is.
    let column = column
        + m.typed_on(line)
            .iter()
            .filter(|&&(a, _)| a == col)
            .map(|(_, text)| yumete_cjk::str_width(text))
            .sum::<usize>();
    // The indent is real page: a caret on the paragraph's first character sits
    // two cells in, and `j` from the row below should land under it.
    let column = column + m.indent_of(line, &line_head(rope, line), index_in_line);
    Position {
        line,
        index_in_line,
        column,
    }
}

/// The display column `pos` sits at within its visual row — the goal column
/// preserved by `j` and `k` when soft wrap is on.
pub fn column_of(rope: &Rope, pos: usize, m: Measure) -> usize {
    position(rope, pos, m).column
}

/// The next `n` rows starting at `anchor`, stopping at the end of the buffer.
///
/// Costs one pass over each *paragraph the page touches*, not over the
/// document.
pub fn rows_from(rope: &Rope, anchor: Anchor, m: Measure, n: usize) -> Vec<Row> {
    let lines = line_count(rope);
    let mut out = Vec::with_capacity(n);
    let mut line = anchor.line;
    let mut index = anchor.index_in_line;
    while out.len() < n && line < lines {
        // A folded line has no rows at all; the page carries on below it.
        if m.folded(line) {
            line += 1;
            index = 0;
            continue;
        }
        let start = rope.line_to_char(line);
        let rows = rows_of_line(rope, line, m);
        while index < rows.len() && out.len() < n {
            let (s, e) = rows[index];
            out.push(Row {
                line,
                index_in_line: index,
                start: start + s,
                end: start + e,
                ends_line: index + 1 == rows.len(),
            });
            index += 1;
        }
        line += 1;
        index = 0;
    }
    out
}

/// The anchor `n` rows above `anchor`, clamped to the top of the buffer.
pub fn retreat(rope: &Rope, anchor: Anchor, m: Measure, mut n: usize) -> Anchor {
    let mut line = anchor.line;
    let mut index = anchor.index_in_line;
    loop {
        if index >= n {
            return Anchor {
                line,
                index_in_line: index - n,
            };
        }
        if line == 0 {
            return Anchor::default();
        }
        n -= index + 1;
        line -= 1;
        while m.folded(line) {
            if line == 0 {
                return Anchor::default();
            }
            line -= 1;
        }
        index = row_count_in_line(rope, line, m) - 1;
    }
}

/// The anchor `n` rows below `anchor`, clamped to the last row of the buffer.
pub fn advance(rope: &Rope, anchor: Anchor, m: Measure, mut n: usize) -> Anchor {
    let lines = line_count(rope);
    let mut line = anchor.line.min(lines.saturating_sub(1));
    let mut index = anchor.index_in_line;
    let mut count = row_count_in_line(rope, line, m);
    while n > 0 {
        if index + 1 < count {
            index += 1;
        } else if m.next_shown(line + 1, lines) < lines {
            line = m.next_shown(line + 1, lines);
            index = 0;
            count = row_count_in_line(rope, line, m);
        } else {
            break;
        }
        n -= 1;
    }
    Anchor {
        line,
        index_in_line: index,
    }
}

/// How many rows forward it is from `from` to `to`, or `None` when `to` is
/// behind `from` or further ahead than `limit`.
///
/// Bounded on purpose: the renderer only needs to know where the cursor sits
/// *within the page*.
pub fn distance(rope: &Rope, from: Anchor, to: Anchor, m: Measure, limit: usize) -> Option<usize> {
    let lines = line_count(rope);
    if from.line >= lines || to < from {
        return None;
    }
    let mut line = from.line;
    let mut index = from.index_in_line;
    let mut count = row_count_in_line(rope, line, m);
    for step in 0..=limit {
        if line == to.line && index == to.index_in_line {
            return Some(step);
        }
        index += 1;
        if index >= count {
            line = m.next_shown(line + 1, lines);
            if line >= lines {
                return None;
            }
            index = 0;
            count = row_count_in_line(rope, line, m);
        }
    }
    None
}

/// The char index at display column `goal` of the row `index_in_line` of
/// `line`, clamped to the end of that row.
fn char_at_column(
    rope: &Rope,
    line: usize,
    index_in_line: usize,
    m: Measure,
    goal: usize,
) -> usize {
    let start = rope.line_to_char(line);
    let rows = rows_of_line(rope, line, m);
    let index_in_line = index_in_line.min(rows.len() - 1);
    let (s, e) = rows[index_in_line];
    // The caret may rest one past the last character of a paragraph, but not
    // one past a wrap point in the middle of one — there it would land on the
    // first character of the next row, which is a different place on screen.
    let limit = if index_in_line + 1 == rows.len() {
        e
    } else {
        e.saturating_sub(1)
    };

    // A goal column inside the indent lands on the row's first character:
    // there is nothing in the indent to land on.
    let mut col = m.indent_of(line, &line_head(rope, line), index_in_line);
    let hidden = m.off(line);
    // **The page drawn beside the text counts here too** (2026-10-08 報的：
    // 「行首有 tab 縮進時（go 代碼），按下 j/k 時 cursor 位置不對齊」).
    //
    // Warning: 這一支從前只數了真字符，而 [`position`]（算目標列的那一支）數的是
    // 真字符**加上畫在它們旁邊的那些**。一個 tab 正是後者：它自己佔一格，到下一
    // 個制表位的那幾格是一段 `Ink::Tab` 的 `drawn` 跑出來的（見
    // `Editor::tab_stops_on_line`）。於是一行行首一個 tab，目標列多算七格，而落
    // 點這一頭少算七格——`j` 一下就偏了十四格。表格的填充（`Ink::Padding`）同病。
    //
    // 規矩照 `position` 抄：跑段站在它所錨的那個字**之前**，所以錨點等於眼下這
    // 一格的時候，它整段都在前面。
    let drawn = m.drawn_on(line);
    // Warning: **這一行不許整個物化**（2026-10-08）。從前這裏是
    // `rope.slice(start + s..start + e).to_string()`，而折行關掉的時候一段就是一
    // 行：一行 1242 萬字的檔上，`j`／`k` 每按一下拷 37 MB 再逐字素走最多一遍。
    // 現在先拿 [`Widths`] 那份索引整塊整塊地跳，剩下的只物化至多一塊。
    let (from, upto) = match (m.widths)(rope, line, e.saturating_sub(s)) {
        Some(index) => match index.skip_to(s, e, goal, col, &hidden, &drawn) {
            Some((from, skipped, upto)) => {
                col = skipped;
                (from, upto)
            }
            None => (s, e),
        },
        None => (s, e),
    };
    let row = rope.slice(start + from..start + upto).to_string();
    // `upto` 短過 `e` 的那一條路**一定在這一走裏停下來**：那是「這一塊的淨寬越過
    // 了 `goal`」纔會給的終點，而那一塊裏沒有 `hidden` 也沒有 `drawn`，所以逐字
    // 素累加出來的就是那個淨寬。停不下來就是索引與逐字素走說了兩句話。
    let mut at = e;
    let mut stopped = false;
    for (i, w) in steps(&row) {
        let here = from + i;
        col += drawn
            .iter()
            .filter(|&&(a, _)| a == here)
            .map(|(_, text)| yumete_cjk::str_width(text))
            .sum::<usize>();
        // Hidden markup takes no column here either — the same rule the rows
        // were broken by, asked the same way.
        let w = match hidden.iter().any(|&(a, b)| here >= a && here < b) {
            true => 0,
            false => w,
        };
        if col + w > goal {
            at = here;
            stopped = true;
            break;
        }
        col += w;
    }
    debug_assert!(stopped || upto >= e, "索引跳過了答案所在的那一塊");
    start + at.min(limit)
}

/// The position one visual row below `pos`, keeping the display column `goal`.
pub fn next_row(rope: &Rope, pos: usize, m: Measure, goal: usize) -> usize {
    let here = position(rope, pos, m);
    let count = row_count_in_line(rope, here.line, m);
    let lines = line_count(rope);
    if here.index_in_line + 1 < count {
        char_at_column(rope, here.line, here.index_in_line + 1, m, goal)
    } else if m.next_shown(here.line + 1, lines) < lines {
        char_at_column(rope, m.next_shown(here.line + 1, lines), 0, m, goal)
    } else {
        pos
    }
}

/// The position one visual row above `pos`, keeping the display column `goal`.
pub fn prev_row(rope: &Rope, pos: usize, m: Measure, goal: usize) -> usize {
    let here = position(rope, pos, m);
    if here.index_in_line > 0 {
        char_at_column(rope, here.line, here.index_in_line - 1, m, goal)
    } else if here.line > 0 {
        let mut above = here.line - 1;
        while m.folded(above) {
            if above == 0 {
                return pos;
            }
            above -= 1;
        }
        let last = row_count_in_line(rope, above, m) - 1;
        char_at_column(rope, above, last, m, goal)
    } else {
        pos
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows of `text` as strings, which is what the reader actually sees.
    fn wrapped(text: &str, width: usize) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        line_rows(text, width)
            .into_iter()
            .map(|(s, e)| chars[s..e].iter().collect())
            .collect()
    }

    /// A chapter typed as one paragraph is asked about several times per
    /// keystroke — where the cursor is, what column it is aiming at, which rows
    /// fill the page, how far down the page the cursor is. Wrapping it afresh
    /// for each question is the whole chapter, four times, per `j`.
    #[test]
    fn one_keystroke_wraps_a_paragraph_once() {
        let text: String = "春夏秋冬".repeat(2_500);
        let rope = Rope::from_str(&text);
        let width = Measure::plain(80);

        reset_wrap_count();
        let p = position(&rope, 5_000, width);
        let _ = column_of(&rope, 5_000, width);
        let _ = next_row(&rope, 5_000, width, 0);
        let _ = rows_from(&rope, Anchor::from(p), width, 40);
        let _ = distance(&rope, Anchor::default(), Anchor::from(p), width, 40);
        assert_eq!(
            wrap_count(),
            1,
            "one paragraph, one keystroke, more than one pass over it"
        );

        // Editing it is a different paragraph, and must be wrapped again.
        let mut edited = rope.clone();
        edited.insert(0, "新");
        reset_wrap_count();
        let _ = position(&edited, 0, width);
        assert_eq!(wrap_count(), 1, "an edited paragraph was not re-wrapped");
    }

    /// …and a caller that can say **which version of which document** this
    /// rope is need not have its paragraph read at all to find the memo
    /// (#315): the hash that took it was a walk down the whole paragraph, two
    /// to five times a keystroke.
    #[test]
    fn a_stamped_paragraph_is_found_without_being_read() {
        let one: String = "春夏秋冬".repeat(2_500);
        let two: String = "梅蘭竹菊".repeat(1_000);
        let rope = Rope::from_str(&format!("{one}\n{two}"));
        let at = |buffer, revision| Measure::plain(80).with_version(buffer, revision);

        reset_wrap_count();
        let p = position(&rope, 5_000, at(1, 7));
        let _ = column_of(&rope, 5_000, at(1, 7));
        let _ = rows_from(&rope, Anchor::from(p), at(1, 7), 40);
        assert_eq!(wrap_count(), 1, "one paragraph, one keystroke, one pass");

        // The line is part of what a stamp names: two paragraphs of one
        // document at one revision are two paragraphs.
        reset_wrap_count();
        let _ = position(&rope, one.chars().count() + 3, at(1, 7));
        assert_eq!(wrap_count(), 1, "the second paragraph is not the first");

        // A revision moves on every edit, so it says 「read this again」…
        let mut edited = rope.clone();
        edited.insert(0, "新");
        reset_wrap_count();
        let _ = position(&edited, 0, at(1, 8));
        assert_eq!(wrap_count(), 1, "an edited paragraph was not re-wrapped");

        // …and so does the buffer, which is why it is in the stamp: the same
        // words at the same line of another document are another document's.
        reset_wrap_count();
        let _ = position(&rope, 5_000, at(2, 7));
        assert_eq!(wrap_count(), 1, "another buffer's line 0 is another line");
    }

    #[test]
    fn an_empty_paragraph_still_occupies_a_row() {
        assert_eq!(line_rows("", 10), vec![(0, 0)]);
    }

    #[test]
    fn a_short_line_is_one_row() {
        assert_eq!(wrapped("hello", 10), vec!["hello"]);
    }

    #[test]
    fn wide_glyphs_count_two_cells() {
        // Four 漢字 are eight cells, so a width of eight holds exactly four.
        assert_eq!(wrapped("春夏秋冬春", 8), vec!["春夏秋冬", "春"]);
    }

    #[test]
    fn a_wide_glyph_is_never_split_across_rows() {
        // Width 7 cannot hold four 漢字; the fourth moves down whole.
        assert_eq!(wrapped("春夏秋冬", 7), vec!["春夏秋", "冬"]);
    }

    #[test]
    fn latin_words_are_kept_whole() {
        assert_eq!(
            wrapped("the quick brown fox", 10),
            vec!["the quick ", "brown fox"]
        );
    }

    #[test]
    fn a_latin_word_after_a_han_character_is_kept_whole() {
        // The manual's own example, and it used to break `Heli|x`: in Chinese
        // prose a Latin word follows a 漢字, so there is no space in the row
        // to retreat to and the old rule gave up.
        let rows = line_rows("一二三四五六Helix七八", 16);
        let text: Vec<String> = rows
            .iter()
            .map(|&(a, b)| "一二三四五六Helix七八".chars().skip(a).take(b - a).collect())
            .collect();
        assert_eq!(text[0], "一二三四五六");
        assert!(text[1].starts_with("Helix"), "{text:?}");
    }

    #[test]
    fn a_word_longer_than_the_row_is_broken_rather_than_lost() {
        assert_eq!(
            wrapped("antidisestablishment", 8),
            vec!["antidise", "stablish", "ment"]
        );
    }

    #[test]
    fn a_closing_mark_does_not_open_a_row() {
        // Width 8 would put 。at the head of the second row; it comes down with
        // the character before it instead.
        let rows = wrapped("春夏秋冬。天", 8);
        assert_eq!(rows, vec!["春夏秋", "冬。天"]);
        assert!(!rows[1].starts_with('。'));
    }

    #[test]
    fn an_opening_bracket_does_not_close_a_row() {
        let rows = wrapped("春夏秋「冬」", 8);
        assert_eq!(rows, vec!["春夏秋", "「冬」"]);
    }

    #[test]
    fn wrapping_always_advances() {
        // Punctuation dense enough to defeat every rule must still terminate.
        // Every row but the caret's own carries at least one character.
        let rows = line_rows("。。。。。。。。", 4);
        assert!(rows[..rows.len() - 1].iter().all(|&(s, e)| e > s));
        assert_eq!(rows.last().unwrap().1, 8);
    }

    #[test]
    fn a_paragraph_that_exactly_fills_a_row_opens_one_more_for_the_caret() {
        // Four 漢字 in eight cells leave the caret no column to stand in on
        // that row — the ninth cell is off the row — so it gets the next one.
        assert_eq!(line_rows_for_caret("春夏秋冬", 8, &[], 0, &[], true), vec![(0, 4), (4, 4)]);
        // Warning: **而光標不在那裏的時候，那一格不許開**（2026-09-23）：一段話正好
        // 排滿一行，屏幕上就會憑空多一個空行，讀起來是分了段。
        assert_eq!(line_rows("春夏秋冬", 8), vec![(0, 4)], "没人要就別開");
        // A row that does not fill the width needs no such thing.
        assert_eq!(line_rows("春夏秋", 8), vec![(0, 3)]);

        let rope = Rope::from_str("春夏秋冬");
        let p = position(&rope, 4, Measure::plain(8).with_caret(Some((0, 4))));
        assert_eq!((p.index_in_line, p.column), (1, 0));
    }

    #[test]
    fn columns_are_counted_over_graphemes_not_chars() {
        // か + combining dakuten is one grapheme two cells wide; counted as two
        // characters it would report a column that has no glyph in it.
        let text = "か\u{3099}か\u{3099}か\u{3099}か\u{3099}";
        assert_eq!(line_rows(text, 8), vec![(0, 8)]);
        assert_eq!(
            line_rows_for_caret(text, 8, &[], 0, &[], true),
            vec![(0, 8), (8, 8)],
            "光標在行末的時候纔多一行"
        );
        let rope = Rope::from_str(text);
        assert_eq!(position(&rope, 6, Measure::plain(8)).column, 6);
        // And `k` never lands between a base and its mark.
        assert_eq!(
            prev_row(&rope, 8, Measure::plain(8).with_caret(Some((0, 8))), 2),
            2,
            "the cursor landed inside a grapheme cluster"
        );
    }

    #[test]
    fn a_goal_column_inside_a_wide_glyph_lands_on_that_glyph() {
        // Column 1 is the right half of 甲; `j` belongs on 甲, not on 乙 —
        // which is where an unwrapped `j` lands, and the two must agree.
        let rope = Rope::from_str("abc\n甲乙丙丁\nxyz\n");
        assert_eq!(next_row(&rope, 1, Measure::plain(40), 1), 4);
    }

    #[test]
    fn position_finds_the_row_and_column() {
        let rope = Rope::from_str("春夏秋冬春夏秋冬\n");
        let p = position(&rope, 5, Measure::plain(8)); // 6th char, second row
        assert_eq!(p.line, 0);
        assert_eq!(p.index_in_line, 1);
        assert_eq!(p.column, 2);
    }

    #[test]
    fn a_page_is_built_from_an_anchor_not_from_the_top() {
        let rope = Rope::from_str("春夏秋冬春夏秋冬\nabc\n");
        // 光標站在第一段末尾，所以那一段有為它開的第三行。
        let m = Measure::plain(8).with_caret(Some((0, 8)));
        let rows = rows_from(&rope, Anchor::default(), m, 10);
        // Two rows of text, the caret's row after them, "abc", and the empty
        // line the trailing newline opens.
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].start, 0);
        assert_eq!(rows[1].start, 4);
        assert!(!rows[1].starts_line());
        assert_eq!(rows[3].line, 1);
        assert!(rows[3].starts_line());
    }

    #[test]
    fn rows_and_anchors_agree_on_distance() {
        let rope = Rope::from_str("春夏秋冬春夏秋冬\nabc\n春夏秋冬春夏秋冬\n");
        let far = Anchor {
            line: 2,
            index_in_line: 1,
        };
        // Three rows in the first paragraph (two of text, one for the caret),
        // one for "abc", then the second row of the last paragraph.
        let m = Measure::plain(8).with_caret(Some((0, 8)));
        assert_eq!(distance(&rope, Anchor::default(), far, m, 20), Some(5));
        assert_eq!(retreat(&rope, far, m, 5), Anchor::default());
        assert_eq!(advance(&rope, Anchor::default(), m, 5), far);
    }

    #[test]
    fn retreat_and_advance_clamp_at_the_ends() {
        let rope = Rope::from_str("abc\ndef\n");
        assert_eq!(
            retreat(&rope, Anchor::default(), Measure::plain(8), 99),
            Anchor::default()
        );
        let end = advance(&rope, Anchor::default(), Measure::plain(8), 99);
        assert_eq!(end.line, 2);
    }

    #[test]
    fn j_and_k_walk_visual_rows_within_one_paragraph() {
        let rope = Rope::from_str("春夏秋冬春夏秋冬\n");
        // From the first character, down lands on the fifth — the same column
        // one row lower, still inside the same logical line.
        let down = next_row(&rope, 0, Measure::plain(8), 0);
        assert_eq!(down, 4);
        assert_eq!(prev_row(&rope, down, Measure::plain(8), 0), 0);
    }

    #[test]
    fn the_caret_stops_short_of_the_next_rows_first_character() {
        let rope = Rope::from_str("春夏秋冬春夏秋冬\n");
        // Column 8 is past the end of a full row; the caret clamps to the last
        // character of that row rather than sliding onto the next one.
        assert_eq!(char_at_column(&rope, 0, 0, Measure::plain(8), 99), 3);
        // 末一行照舊「可以停在最後一個字後面一格」——那是段末，每一段都一樣
        // （`線 = e`）。從前這裏寫的是 7，而那是**多出來的那一行**造成的假象：
        // 有它在，第 1 行就不是末一行（2026-09-23 把那一行改成光標到了纔開）。
        assert_eq!(char_at_column(&rope, 0, 1, Measure::plain(8), 99), 8);
        // 光標真在段末的時候，它自己那一行在第 2 行上。
        let m = Measure::plain(8).with_caret(Some((0, 8)));
        assert_eq!(char_at_column(&rope, 0, 1, m, 99), 7, "中間的行停在最後一個字上");
        assert_eq!(char_at_column(&rope, 0, 2, m, 99), 8);
    }

    #[test]
    fn moving_down_crosses_into_the_next_paragraph() {
        let rope = Rope::from_str("abcd\nefgh\n");
        assert_eq!(next_row(&rope, 1, Measure::plain(8), 1), 6);
        assert_eq!(prev_row(&rope, 6, Measure::plain(8), 1), 1);
    }
    /// The rows of `text` with drawn runs standing in it — Feature #210.
    fn drawn_rows(text: &str, width: usize, drawn: &[(usize, &str)]) -> Vec<(usize, usize)> {
        let drawn: Vec<(usize, String)> = drawn.iter().map(|(at, t)| (*at, t.to_string())).collect();
        line_rows_drawing(text, width, &[], 0, &drawn)
    }

    #[test]
    fn drawn_text_takes_room_on_the_row() {
        // Five 漢字 in eight cells is four and one. Two cells of candidate at
        // the head of the line leave room for three.
        assert_eq!(drawn_rows("春夏秋冬春", 8, &[]), vec![(0, 4), (4, 5)]);
        assert_eq!(drawn_rows("春夏秋冬春", 8, &[(0, "候")]), vec![(0, 3), (3, 5)]);
    }

    #[test]
    fn a_run_is_never_split_from_its_anchor() {
        // The run is anchored at the fifth character, which is on the second
        // row. Leaving it at the foot of the first would put a candidate above
        // the character it is a candidate *for*.
        assert_eq!(drawn_rows("春夏秋冬春", 8, &[(4, "候")]), vec![(0, 4), (4, 5)]);
    }

    #[test]
    fn a_run_past_the_last_character_still_takes_room() {
        // Nothing to stand before, so it stands after — on the last row, whose
        // width it fills, which is what opens the row the caret needs.
        assert_eq!(drawn_rows("春夏秋", 8, &[]), vec![(0, 3)]);
        let drawn = [(3usize, "候".to_string())];
        assert_eq!(
            line_rows_for_caret("春夏秋", 8, &[], 0, &drawn, true),
            vec![(0, 3), (3, 3)]
        );
    }

    #[test]
    fn the_caret_sits_after_the_text_it_typed() {
        // You typed the候: the caret belongs at its end, not in front of it.
        // Which is to say a run stands *before* its anchor, and the caret
        // resting on the anchor has already passed it.
        let rope = Rope::from_str("春夏秋冬\n");
        let runs = |line: usize| match line {
            0 => vec![(2usize, "候".to_string())],
            _ => Vec::new(),
        };
        let m = Measure::plain(40).with_drawn(&runs);
        assert_eq!(position(&rope, 1, m).column, 2, "before the run");
        assert_eq!(position(&rope, 2, m).column, 6, "on its anchor, so past it");
        assert_eq!(position(&rope, 3, m).column, 8);
    }

    // ---- 前綴寬度索引（2026-10-08）-----------------------------------------

    /// 定了種子的隨機數源（LCG）——性質測試要重跑得出同一批字，而這個倉不為一個
    /// 測試加依賴。
    struct Roll(u64);

    impl Roll {
        fn roll(&mut self) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as usize
        }
    }

    /// 性質測試餵的那些字。
    ///
    /// 挑的標準是「哪一種寫法會把索引算錯」：按 `char` 數寬度的錯在組合符號與變
    /// 體選擇符上，按字符數切塊的錯在字素跨過塊界的時候，而**從別處重新分段**的
    /// 錯只在 ZWJ 與區域指示符上——那兩條是 UAX #29 裏唯一要往左看任意遠的規則，
    /// 也正是 [`clean_cut`] 存在的理由。
    const TOKENS: &[&str] = &[
        // ASCII：一個 `char`、一個字素、一格
        "a", "Z", "7", " ", "-", "(",
        // 製表符：`grapheme_width` 走 ASCII 那條快路，也是一格
        "\t",
        // 漢字與全角標點：兩格
        "天", "地", "玄", "黃", "宇", "，", "。", "」", "（",
        // 歧義寬度那幾個（這個進程裏是一格，索引與逐字素走問的是同一支）
        "—", "…", "“",
        // 組合符號：兩三個 `char` 一個字素，寬度記在頭一個上
        "e\u{0301}", "a\u{0300}\u{0301}", "o\u{0308}",
        // 繪文字 ＋ 變體選擇符：頭一個單看一格，合起來兩格
        "\u{26A0}\u{FE0F}", "\u{2764}\u{FE0F}",
        // 漢字 ＋ 異體字選擇符
        "葛\u{E0101}",
        // ZWJ 連成的一家子（男 ZWJ 女 ZWJ 女孩）：五個 `char`，一個字素
        "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
        // 區域指示符：兩個一對，而「對」是從左邊數起的——落單的那個會把它後面的
        // 配對整個挪一位，所以切點一旦記在這中間，兩邊的分段就不是同一回事
        "\u{1F1E6}\u{1F1E6}", "\u{1F1FF}",
        // 寬繪文字
        "\u{1F004}", "\u{1F642}",
    ];

    /// 混出至少 `upto` 個字符的一段（用完整的 token，所以可能多出幾個）。
    fn made(seed: u64, upto: usize) -> String {
        let mut roll = Roll(seed);
        let mut text = String::new();
        let mut chars = 0usize;
        while chars < upto {
            let token = TOKENS[roll.roll() % TOKENS.len()];
            text.push_str(token);
            chars += token.chars().count();
        }
        text
    }

    /// 這一行上隨機撒幾段「藏起來的」——起點不管字素邊界，正是最難的那一種。
    fn spans(seed: u64, len: usize, how_many: usize) -> Vec<(usize, usize)> {
        let mut roll = Roll(seed);
        let mut out = Vec::new();
        for _ in 0..how_many {
            let a = roll.roll() % (len + 1);
            let b = (a + 1 + roll.roll() % 7).min(len);
            if a < b {
                out.push((a, b));
            }
        }
        out
    }

    /// 要掃的那幾種行長：剛不到一塊、正好一塊、剛過一塊，以及好幾塊。
    fn lengths(block: usize) -> Vec<usize> {
        vec![
            block.saturating_sub(1),
            block,
            block + 1,
            2 * block,
            3 * block + 7,
            5 * block,
        ]
    }

    /// **索引答得與逐字素走逐格相同——一行裏每一列都問一遍。**
    ///
    /// 種子 `20261008`（加上塊與行長），塊 4／8／64，每個塊六種行長（見
    /// [`lengths`]：剛不到一塊、正好一塊、剛過一塊、兩塊、三塊零七個字、五塊），
    /// 字是 [`TOKENS`] 混出來的——ASCII、漢字、全角標點、組合符號、帶變體選擇符
    /// 的繪文字、異體字選擇符、ZWJ 連成的一家子、區域指示符、製表符。
    ///
    /// 斷言的是 `[0, b)`（光標的那一問）與**每一對** `[a, b)`（折行開着的時候
    /// `a` 是某一行的起點），`b` 從 0 掃到行末。
    ///
    /// Warning: **塊用的是小數**。真正那個 [`BLOCK`] 是 4096，照同樣的密度掃一行
    /// 要走上億個字素；而對不對與塊多大無關，塊多大只是調參。
    #[test]
    fn the_width_index_agrees_with_the_walk_at_every_column() {
        for block in [4usize, 8, 64] {
            for want in lengths(block) {
                let text = made(20261008 + block as u64 * 1000 + want as u64, want);
                let rope = Rope::from_str(&format!("{text}\n"));
                let index = Widths::every(&rope, 0, block);
                let len = line_len_chars(&rope, 0);
                // 索引真的用上了纔算掃過——全退回逐字素走的話，底下那幾千條
                // 斷言是在拿同一支函數和自己比。
                assert!(
                    len <= 2 * block || index.cuts.len() > 2,
                    "block {block}, len {len}: 只有 {} 條切點",
                    index.cuts.len()
                );
                for b in 0..=len {
                    assert_eq!(
                        index.width(&rope, 0, 0, b, &[]),
                        walk_width(&rope, 0, 0, b, &[]),
                        "block {block}, len {len}, [0, {b})"
                    );
                }
                for a in 0..=len {
                    for b in a..=len {
                        assert_eq!(
                            index.width(&rope, 0, a, b, &[]),
                            walk_width(&rope, 0, a, b, &[]),
                            "block {block}, len {len}, [{a}, {b})"
                        );
                    }
                }
            }
        }
    }

    /// **藏起來的那幾段照樣算對。**
    ///
    /// 索引記的是淨寬，所以落在 `hidden` 裏的字素要**減**出去；減的那一項與從前
    /// 那道過濾的判準必須是同一句話——「字素的**起頭**在區間裏就整個不算」。
    ///
    /// 區間是隨機撒的，起點**不管字素邊界**：一段 `hidden` 從組合序列中間開始，
    /// 從前那道過濾照樣把那整個字素去掉，所以減的時候也得整個減。
    #[test]
    fn the_width_index_agrees_with_the_walk_with_things_hidden() {
        for block in [4usize, 8, 64] {
            for want in lengths(block) {
                let text = made(20261008 + want as u64, want);
                let rope = Rope::from_str(&format!("{text}\n"));
                let index = Widths::every(&rope, 0, block);
                let len = line_len_chars(&rope, 0);
                for how_many in [1usize, 3, 9] {
                    let hidden = spans(777 + how_many as u64 + len as u64, len, how_many);
                    for b in 0..=len {
                        assert_eq!(
                            index.width(&rope, 0, 0, b, &hidden),
                            walk_width(&rope, 0, 0, b, &hidden),
                            "block {block}, len {len}, [0, {b}), hidden {hidden:?}"
                        );
                    }
                    for a in 0..=len {
                        assert_eq!(
                            index.width(&rope, 0, a, len, &hidden),
                            walk_width(&rope, 0, a, len, &hidden),
                            "block {block}, len {len}, [{a}, {len}), hidden {hidden:?}"
                        );
                    }
                }
            }
        }
    }

    /// **一段 `hidden` 整個蓋住這一行，與一段都沒有，答案一樣。**
    ///
    /// 兩頭的極端：全藏起來就是零格（一個字素的起頭都不在區間外），而重疊的區間
    /// 只許減一次——從前那道過濾是 `any(…)`，減卻會減兩遍。
    #[test]
    fn hidden_spans_that_overlap_are_only_taken_off_once() {
        let text = made(31415, 600);
        let rope = Rope::from_str(&format!("{text}\n"));
        let index = Widths::every(&rope, 0, 8);
        let len = line_len_chars(&rope, 0);
        let all = [(0usize, len)];
        assert_eq!(index.width(&rope, 0, 0, len, &all), 0, "全藏起來就是零格");
        // 一段蓋一段，再加一段挨着的：併起來是 `[3, 40)`。
        let piled = [(3usize, 20), (10, 30), (30, 40)];
        assert_eq!(
            index.width(&rope, 0, 0, len, &piled),
            walk_width(&rope, 0, 0, len, &piled),
            "重疊的區間只減一次"
        );
    }

    /// **掛上索引與不掛，`position` 與 `char_at_column` 一個格子都不差。**
    ///
    /// 前一個是光標的列，後一個是 `j`／`k` 的落點——兩支共用一份索引（一支拿它
    /// 減、一支拿它整塊整塊地跳），所以兩支都要掃。`drawn` 也餵了：那一段畫在文
    /// 字旁邊，有自己的坐標，跳塊的時候一碰上它就得老老實實走。
    #[test]
    fn the_index_changes_no_column_and_no_landing() {
        let text = made(20261009, 600);
        let rope = Rope::from_str(&format!("{text}\n"));
        let len = line_len_chars(&rope, 0);
        let index = std::rc::Rc::new(Widths::every(&rope, 0, 8));
        // 跨多少個字符不管（見 [`Measure::widths`] 第三個參數）：那一道是省工夫
        // 的閘，而這一條驗的是「掛上索引與不掛，答案一樣」——短行上也要走索引。
        let widths = |_: &Rope, _: usize, _: usize| Some(index.clone());
        let hide = |_: usize| spans(2718, len, 5);
        let runs = |_: usize| vec![(0usize, "候".to_string()), (len / 3, "補".to_string())];
        for width in [NO_WRAP, 40, 9] {
            let plain = Measure::new(width, &hide).with_drawn(&runs).with_indent(2);
            let indexed = plain.with_widths(&widths);
            for pos in 0..=len {
                assert_eq!(
                    position(&rope, pos, plain),
                    position(&rope, pos, indexed),
                    "width {width}, pos {pos}"
                );
            }
            let rows = rows_of_line(&rope, 0, plain).len();
            let total = position(&rope, len, plain).column + 4;
            for row in 0..rows {
                for goal in 0..=total {
                    assert_eq!(
                        char_at_column(&rope, 0, row, plain, goal),
                        char_at_column(&rope, 0, row, indexed, goal),
                        "width {width}, row {row}, goal {goal}"
                    );
                }
            }
        }
    }

    /// **一行全是國旗，索引記不下一個切點——而答案照樣對。**
    ///
    /// 區域指示符是 UAX #29 裏「從哪裏起分段」最要緊的那一種：四個 U+1F1E6 從頭數
    /// 是兩面旗，從第二個字符起數是「一個落單的、一面旗、一個落單的」。所以
    /// [`clean_cut`] 一個切點都不許記在它們中間，而這一行就是那一種——退回逐字素
    /// 走，慢，不會錯。
    #[test]
    fn a_line_of_flags_keeps_no_cut_and_still_measures_right() {
        let text: String = std::iter::repeat_n("\u{1F1E6}", 400).collect();
        let rope = Rope::from_str(&format!("{text}\n"));
        let index = Widths::every(&rope, 0, 8);
        let len = line_len_chars(&rope, 0);
        assert_eq!(index.cuts.len(), 2, "只有行首與行末兩條");
        for b in 0..=len {
            assert_eq!(
                index.width(&rope, 0, 0, b, &[]),
                walk_width(&rope, 0, 0, b, &[]),
                "[0, {b})"
            );
        }
    }

    /// **不到一塊長的行不建索引**——尋常的一段走的還是從前那一條路。
    #[test]
    fn a_paragraph_shorter_than_a_block_is_not_indexed() {
        let mut editor = crate::editor::Editor::new();
        editor.replace_everything(&format!("{}\n", made(161803, 100)));
        let rope = editor.current_buffer().rope().clone();
        assert!(editor.line_widths(&rope, 0, 200).is_none(), "一百個字用不上索引");
        editor.replace_everything(&format!("{}\n", made(161803, BLOCK * 2)));
        let rope = editor.current_buffer().rope().clone();
        let len = line_len_chars(&rope, 0);
        let index = editor.line_widths(&rope, 0, len).expect("兩塊長的行建得出索引");
        // 問得短也不建——建它比逐字素走那一小段貴。
        assert!(
            editor.line_widths(&rope, 0, BLOCK).is_none(),
            "跨不到一塊的那一問用不上索引"
        );
        assert_eq!(
            index.width(&rope, 0, 0, len, &[]),
            walk_width(&rope, 0, 0, len, &[]),
            "記在備忘裏的那一份也答得對"
        );
    }

    #[test]
    fn a_paragraph_is_re_wrapped_when_only_the_candidate_changed() {
        // The wrap memo is keyed on the buffer's revision, and a candidate
        // moves while the buffer does not move at all. Keyed without it, the
        // page would keep answering with the candidate before last.
        let rope = Rope::from_str("春夏秋冬春\n");
        let one = |_: usize| vec![(0usize, "候".to_string())];
        let two = |_: usize| vec![(0usize, "候補".to_string())];
        let rows = |m: Measure| rows_of_line(&rope, 0, m);
        assert_eq!(rows(Measure::plain(8).with_drawn(&one)), vec![(0, 3), (3, 5)]);
        assert_eq!(rows(Measure::plain(8).with_drawn(&two)), vec![(0, 2), (2, 5)]);
    }
}
