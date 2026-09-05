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
    /// **Text on the page that the file has no bytes for** — the inverse of
    /// `hidden`, as `(column within the line, what is drawn there)`.
    ///
    /// A ghost run stands *before* the character it is anchored at, and takes
    /// its width from the page: the row it is on holds that much less writing,
    /// the caret at the anchor sits after it, and a click on it means the
    /// anchor. Which is the whole point of it being part of the measure: an
    /// inline candidate drawn by the renderer alone would put the caret, `j`,
    /// the mouse and the wrap all on different pages.
    ghost: &'a dyn Fn(usize) -> Vec<(usize, String)>,
    /// The part of `ghost` that stands **before the caret** at its own anchor.
    ///
    /// Two things are drawn at the same anchor and the caret goes between
    /// them: the inline candidate, which the writer typed and the caret is at
    /// the end of, and the padding that squares a table up, which reaches from
    /// there to the closing pipe. Counting both put the caret on the pipe
    /// after every keystroke in a table; counting neither would put it back
    /// inside the candidate. So the caret asks this, and everything else —
    /// where a row breaks, what a click means — asks `ghost`.
    typed: &'a dyn Fn(usize) -> Vec<(usize, String)>,
    /// Which lines are **one row however long they are** (#275).
    ///
    /// A row of a table. The author: 「进入后，表格所在的行不再 soft wrap」 —
    /// and the reason is the whole point of the mode: a cell that has wrapped
    /// onto the next screen row is no longer in its column, so a table folded
    /// to the measure is not a table any more. Part of the measure and not of
    /// the renderer, like everything else here, because the caret, `j`, the
    /// mouse and the page must be reading the same page.
    unwrapped: &'a dyn Fn(usize) -> bool,
}

/// A page with nothing hidden, for callers that show the source as it is.
const NOTHING_HIDDEN: &dyn Fn(usize) -> Vec<(usize, usize)> = &|_| Vec::new();

/// A page with every line on it — and, read the other way, a page every line
/// of which folds to the measure.
const NOTHING_FOLDED: &dyn Fn(usize) -> bool = &|_| false;

/// A page with nothing on it but the file's own characters.
const NOTHING_GHOSTED: &dyn Fn(usize) -> Vec<(usize, String)> = &|_| Vec::new();

impl<'a> Measure<'a> {
    /// `width` cells, with every character on the page.
    pub fn plain(width: usize) -> Measure<'static> {
        Measure {
            width: width.max(1),
            hidden: NOTHING_HIDDEN,
            folded: NOTHING_FOLDED,
            indent: 0,
            open: None,
            ghost: NOTHING_GHOSTED,
            typed: NOTHING_GHOSTED,
            unwrapped: NOTHING_FOLDED,
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
            ghost: NOTHING_GHOSTED,
            typed: NOTHING_GHOSTED,
            unwrapped: NOTHING_FOLDED,
        }
    }

    /// The same measure, with `line` shown as the file has it.
    pub fn with_open_line(self, line: Option<usize>) -> Measure<'a> {
        Measure { open: line, ..self }
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

    /// The same measure, with `ghost` naming the text drawn into each line
    /// that the file does not contain.
    ///
    /// All of it counts as standing before the caret until
    /// [`Self::with_typed_ghost`] says which part of it does.
    pub fn with_ghost(self, ghost: &'a dyn Fn(usize) -> Vec<(usize, String)>) -> Measure<'a> {
        Measure {
            ghost,
            typed: ghost,
            ..self
        }
    }

    /// The same measure, with `typed` naming the part of the ghost the writer
    /// typed — the part the caret stands after. See [`Self::typed`].
    pub fn with_typed_ghost(self, typed: &'a dyn Fn(usize) -> Vec<(usize, String)>) -> Measure<'a> {
        Measure { typed, ..self }
    }

    /// The ghost text on `line`, anchored at columns within it, in order.
    ///
    /// Public because the renderer and the mouse have to draw and resolve the
    /// very cells this measured: three questions, one answer.
    pub fn ghost_on(self, line: usize) -> Vec<(usize, String)> {
        let mut runs = (self.ghost)(line);
        runs.sort_by_key(|&(at, _)| at);
        runs
    }

    /// The typed part of the ghost on `line` — what the caret stands after.
    fn typed_on(self, line: usize) -> Vec<(usize, String)> {
        (self.typed)(line)
    }
}

/// How many cells the ghost runs anchored at `at` take.
fn ghost_width(ghost: &[(usize, String)], at: usize) -> usize {
    ghost
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
    line_rows_ghosting(text, width, hidden, indent, &[])
}

/// [`line_rows_indented`], with `ghost` naming text drawn into the line that
/// the line does not contain.
///
/// A ghost run is measured with the character it is anchored at and never
/// split from it, so it cannot be left hanging at the foot of one row with its
/// anchor at the head of the next. It is *not* a character: the rows are still
/// char ranges within the line, and a ghost changes only where they break.
pub fn line_rows_ghosting(
    text: &str,
    width: usize,
    hidden: &[(usize, usize)],
    indent: usize,
    ghost: &[(usize, String)],
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
    // ghost would otherwise start counting as writing.
    let mut lead: Vec<usize> = Vec::with_capacity(chars.len());
    let mut at = 0;
    for g in graphemes(text) {
        cuts.push(at);
        // A character that is not drawn takes no room, so it cannot push the
        // one after it onto the next row.
        let off = hidden.iter().any(|&(a, b)| at >= a && at < b);
        widths.push(if off { 0 } else { grapheme_width(g) });
        lead.push(ghost_width(ghost, at));
        at += g.chars().count();
    }
    cuts.push(at);
    // Ghost text at the very end of the line has no character to stand before;
    // it stands after the last one, on the last row.
    let tail = ghost_width(ghost, at);

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
    // Open one more, empty, row for it — which is also where the reader expects
    // the next character to appear.
    if last_width >= width - if rows.len() == 1 { indent } else { 0 } {
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
    // and the row after it opened with 。 in the author's own documentation.
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

thread_local! {
    /// Wrapped paragraphs, most recently used first: `(text hash, width, rows)`.
    ///
    /// Keyed by a hash of the paragraph's own text rather than by a line number
    /// or a buffer revision, exactly as the segmentation cache is: a matching
    /// hash is a correct answer whatever else in the document — or in another
    /// document — has moved since.
    static ROWS: RefCell<Vec<Remembered>> = const { RefCell::new(Vec::new()) };

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
fn rows_of_line(rope: &Rope, line: usize, m: Measure) -> Vec<(usize, usize)> {
    // **A table row is one row** (#275), so there is nothing to measure and
    // nothing to remember: the whole line, whatever the measure is. Ahead of
    // the cache, because the answer does not depend on the width and caching
    // it under one would only make the next width ask again.
    if m.unwrapped(line) {
        let len = line_text(rope, line).chars().count();
        return vec![(0, len)];
    }
    let hidden = m.off(line);
    // The hidden runs are part of the answer, so they are part of the key: the
    // same paragraph wraps differently when the cursor opens a construct in it.
    let mut hasher = DefaultHasher::new();
    line_hash(rope, line).hash(&mut hasher);
    hidden.hash(&mut hasher);
    // The indent changes where a row breaks, so it is part of the key too.
    m.indent_on(line).hash(&mut hasher);
    // …and so does the ghost text, for exactly the same reason: the same
    // paragraph wraps differently while a candidate stands in the middle of it.
    let ghost = m.ghost_on(line);
    ghost.hash(&mut hasher);
    let hash = hasher.finish();
    if let Some(rows) = remembered(hash, m.width) {
        return rows;
    }
    WRAPPED.with(|n| n.set(n.get() + 1));
    let rows = line_rows_ghosting(
        &line_text(rope, line),
        m.width,
        &hidden,
        m.indent_on(line),
        &ghost,
    );
    remember(hash, m.width, &rows);
    rows
}

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
    let ahead = rope.slice(start + row_start..start + col).to_string();
    // **A character that is not drawn takes no column.** This summed the
    // source, while `line_rows_indented` — the function that decides where the
    // rows actually break — zeroes what is hidden. So with 所見即所得 on, a
    // `j` from a row above a paragraph with `**` in it landed one glyph right
    // per hidden character, and the front end papered over the caret's half of
    // it by subtracting the hidden width again on its way to the screen.
    let hidden = m.off(line);
    let column: usize = steps(&ahead)
        .filter(|(i, _)| {
            let at = row_start + i;
            !hidden.iter().any(|&(a, b)| at >= a && at < b)
        })
        .map(|(_, w)| w)
        .sum();
    // **Ghost text before the caret is page the caret is past.** A run stands
    // before the character it is anchored at, so a run anchored anywhere to
    // the left of the caret is wholly behind it.
    let ghost = m.ghost_on(line);
    let column = column
        + ghost
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
    let column = column + m.indent_of(line, &line_text(rope, line), index_in_line);
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

    // The grapheme whose own columns cover `goal` — not the one after it, which
    // is where `col >= goal` would stop and would put `j` one glyph right of
    // the column it was aiming at whenever that column is inside a wide glyph.
    let row = rope.slice(start + s..start + e).to_string();
    // A goal column inside the indent lands on the row's first character:
    // there is nothing in the indent to land on.
    let mut col = m.indent_of(line, &line_text(rope, line), index_in_line);
    let hidden = m.off(line);
    let mut at = e;
    for (i, w) in steps(&row) {
        // Hidden markup takes no column here either — the same rule the rows
        // were broken by, asked the same way.
        let w = match hidden.iter().any(|&(a, b)| s + i >= a && s + i < b) {
            true => 0,
            false => w,
        };
        if col + w > goal {
            at = s + i;
            break;
        }
        col += w;
    }
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
        let rows = line_rows("春夏秋冬", 8);
        assert_eq!(rows, vec![(0, 4), (4, 4)]);
        // A row that does not fill the width needs no such thing.
        assert_eq!(line_rows("春夏秋", 8), vec![(0, 3)]);

        let rope = Rope::from_str("春夏秋冬");
        let p = position(&rope, 4, Measure::plain(8));
        assert_eq!((p.index_in_line, p.column), (1, 0));
    }

    #[test]
    fn columns_are_counted_over_graphemes_not_chars() {
        // か + combining dakuten is one grapheme two cells wide; counted as two
        // characters it would report a column that has no glyph in it.
        let text = "か\u{3099}か\u{3099}か\u{3099}か\u{3099}";
        assert_eq!(line_rows(text, 8), vec![(0, 8), (8, 8)]);
        let rope = Rope::from_str(text);
        assert_eq!(position(&rope, 6, Measure::plain(8)).column, 6);
        // And `k` never lands between a base and its mark.
        assert_eq!(
            prev_row(&rope, 8, Measure::plain(8), 2),
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
        let rows = rows_from(&rope, Anchor::default(), Measure::plain(8), 10);
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
        assert_eq!(
            distance(&rope, Anchor::default(), far, Measure::plain(8), 20),
            Some(5)
        );
        assert_eq!(retreat(&rope, far, Measure::plain(8), 5), Anchor::default());
        assert_eq!(advance(&rope, Anchor::default(), Measure::plain(8), 5), far);
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
        assert_eq!(char_at_column(&rope, 0, 1, Measure::plain(8), 99), 7);
        // The row opened for the caret is where the end of the paragraph is.
        assert_eq!(char_at_column(&rope, 0, 2, Measure::plain(8), 99), 8);
    }

    #[test]
    fn moving_down_crosses_into_the_next_paragraph() {
        let rope = Rope::from_str("abcd\nefgh\n");
        assert_eq!(next_row(&rope, 1, Measure::plain(8), 1), 6);
        assert_eq!(prev_row(&rope, 6, Measure::plain(8), 1), 1);
    }
    /// The rows of `text` with ghost runs standing in it — Feature #210.
    fn ghosted(text: &str, width: usize, ghost: &[(usize, &str)]) -> Vec<(usize, usize)> {
        let ghost: Vec<(usize, String)> = ghost.iter().map(|(at, t)| (*at, t.to_string())).collect();
        line_rows_ghosting(text, width, &[], 0, &ghost)
    }

    #[test]
    fn ghost_text_takes_room_on_the_row() {
        // Five 漢字 in eight cells is four and one. Two cells of candidate at
        // the head of the line leave room for three.
        assert_eq!(ghosted("春夏秋冬春", 8, &[]), vec![(0, 4), (4, 5)]);
        assert_eq!(ghosted("春夏秋冬春", 8, &[(0, "候")]), vec![(0, 3), (3, 5)]);
    }

    #[test]
    fn a_ghost_is_never_split_from_its_anchor() {
        // The run is anchored at the fifth character, which is on the second
        // row. Leaving it at the foot of the first would put a candidate above
        // the character it is a candidate *for*.
        assert_eq!(ghosted("春夏秋冬春", 8, &[(4, "候")]), vec![(0, 4), (4, 5)]);
    }

    #[test]
    fn ghost_past_the_last_character_still_takes_room() {
        // Nothing to stand before, so it stands after — on the last row, whose
        // width it fills, which is what opens the row the caret needs.
        assert_eq!(ghosted("春夏秋", 8, &[]), vec![(0, 3)]);
        assert_eq!(ghosted("春夏秋", 8, &[(3, "候")]), vec![(0, 3), (3, 3)]);
    }

    #[test]
    fn the_caret_sits_after_the_ghost_it_typed() {
        // You typed the候: the caret belongs at its end, not in front of it.
        // Which is to say a run stands *before* its anchor, and the caret
        // resting on the anchor has already passed it.
        let rope = Rope::from_str("春夏秋冬\n");
        let runs = |line: usize| match line {
            0 => vec![(2usize, "候".to_string())],
            _ => Vec::new(),
        };
        let m = Measure::plain(40).with_ghost(&runs);
        assert_eq!(position(&rope, 1, m).column, 2, "before the run");
        assert_eq!(position(&rope, 2, m).column, 6, "on its anchor, so past it");
        assert_eq!(position(&rope, 3, m).column, 8);
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
        assert_eq!(rows(Measure::plain(8).with_ghost(&one)), vec![(0, 3), (3, 5)]);
        assert_eq!(rows(Measure::plain(8).with_ghost(&two)), vec![(0, 2), (2, 5)]);
    }
}
