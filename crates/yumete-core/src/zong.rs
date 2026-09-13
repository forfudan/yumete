//! 縱 (zong) — the text model behind [`Layout::Vertical`], Feature #61.
//!
//! Set vertically, "line" and "column" each mean two things, so yumete borrows
//! one unambiguous term: a **縱** (*zong*) is one run of text read top to
//! bottom, and successive 縱 stack from the right edge leftward. It is what a
//! line is in horizontal layout, and nothing else in yumete is named for it.
//!
//! A 縱 is a *visual* unit, not a buffer line: a paragraph (one logical line) is
//! soft-wrapped into as many 縱 as it needs, [`DEFAULT_ZONG_LENGTH`] graphemes
//! at a time. Novels are typeset at 24–32 characters per 縱 — beyond that the
//! eye loses the return sweep — so the wrap length is a deliberate typographic
//! setting, not "however tall the terminal happens to be".
//!
//! Every grapheme cluster occupies exactly one **slot**: one terminal row, two
//! cells wide. That keeps the grid square (a terminal cell is about 1:2), and it
//! is why the model counts graphemes rather than characters or display widths —
//! a 漢字 and a Latin letter each take one slot.
//!
//! The functions here are pure and local: they never build the whole document's
//! layout to answer a question about the cursor, so 縱-crossing motion costs one
//! walk of the current logical line. [`layout`] materialises the full list only
//! for the renderer, which needs to know how many 縱 precede the viewport.

use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use ropey::Rope;

use crate::ruby::Dialects;
use yumete_cjk::{grapheme_width, graphemes};

// The layout choice and the typographic defaults are CJK typesetting facts, not
// editor state, so they live in `yumete-cjk` and are re-exported here where the
// rest of the core reaches for them.
pub use yumete_cjk::vertical::{Layout, DEFAULT_ZONG_GAP, DEFAULT_ZONG_LENGTH};

/// How a buffer is gridded into 縱.
///
/// Two settings travel together everywhere, so they travel as one value: the
/// wrap length, and whether `<ruby>` markup is *laid out* or left as the text it
/// is. Both change where a slot boundary falls, so any function that answers a
/// question about slots needs both — a cursor positioned under one and drawn
/// under the other would sit in the wrong row.
/// Not `Debug`/`PartialEq`: a page is partly two closures, and the useful
/// question about two grids is never whether they are equal but whether they
/// answer the same — which is what the differential tests ask.
#[derive(Clone, Copy)]
pub struct Grid<'a> {
    /// **Which characters of a line are not on the page** — the markup
    /// 所見即所得 takes off, as columns within the line.
    ///
    /// A hidden run joins the slot beside it rather than taking one of its
    /// own, so the cursor steps over `**` in one press and the 縱 length counts
    /// writing rather than asterisks — the same thing a ruby group's tags have
    /// always done.
    ///
    /// **Handed in, never re-derived.** This used to be a `bool` and the
    /// answer was worked out here, from the bare line, as
    /// `markdown::spans(text)` — with no syntax and no block. So a 縱書 page
    /// ate the `**` inside a code fence, hid two asterisks where Typst has
    /// one, and hid four characters under `:syntax text`, while the horizontal
    /// page — which is *given* the answer, by [`crate::wrap::Measure`] — hid
    /// exactly none. One document, two pages, and no test could see it,
    /// because each side asked its own implementation.
    hidden: &'a dyn Fn(usize) -> Vec<(usize, usize)>,
    /// **Which document, and which version of it** — for the memo, and for
    /// nothing else.
    ///
    /// A laid-out paragraph is remembered so that a page of forty 縱 lays it
    /// out once rather than forty times, and the memo has to be able to say
    /// whether a remembered answer is about the text in front of it. Hashing
    /// the paragraph would answer that, and does in [`crate::wrap`] — but a
    /// 縱書 page asks per 縱, and hashing 500,000 characters forty times a
    /// frame costs more than the layout it was saving. The editor already
    /// knows: a buffer's id and its revision.
    ///
    /// `0` means「no stamp」— a grid built by hand, in a test — and nothing is
    /// remembered for it, because without a stamp two different documents look
    /// the same to the key.
    pub stamp: u64,
    /// Graphemes per 縱.
    pub zong_len: usize,
    /// Which ruby dialects are laid out as readings. Empty shows the markup as
    /// the text it is.
    pub ruby: Dialects,
    /// Whether 句讀 hang in the margin rather than taking a square each
    /// (標點旁置).
    pub hanging: bool,
    /// Whether every 句 opens a 縱 of its own (`:view-sentence`, Feature #237).
    ///
    /// A **view**: the file is not touched, which is the whole point — the
    /// manual has taught `:%s/。/。\n/g` for proofreading since the beginning,
    /// and that edits the manuscript to read it.
    pub sentences: bool,
    /// Which whole lines are not on the page (Feature #159) — by the same
    /// argument: one rule for folding a blank line, and both layouts ask it.
    folded: &'a dyn Fn(usize) -> bool,
    /// The paragraph shown as the file has it: no indent, and its blank line
    /// back. `usize::MAX` for none.
    pub open_line: usize,

    /// Whether a pair of half-width characters shares one slot (縦中横).
    ///
    /// Off by default. Turned sideways a pair reads as a syllable — `yume` set
    /// as `yu` over `me` invites the eye to read two of them — and one character
    /// to a row, hung right, is what a reader of vertical text expects. It stays
    /// available because a two-digit year genuinely does read better packed.
    pub tatechuyoko: bool,
    /// How many empty squares open a paragraph (首行縮進).
    ///
    /// A Chinese paragraph is marked by an indent of two 字, not by a blank
    /// line — the blank line is Markdown's way of saying "new paragraph", and
    /// it costs a whole 縱 on the page.
    ///
    /// This is a **view**, never a rewrite: the file keeps its blank lines, so
    /// it still exports as the paragraphs it is. The indent is made of *padding
    /// slots* — the same thing a long reading already opens to make room for
    /// itself — which is why nothing downstream had to learn about it: the
    /// layout, the cursor, the mouse and the caret all read the slot list, and
    /// a padding slot holds no character for the cursor to sit on.
    pub indent: usize,
    /// **Text on the page the file has no bytes for** ([`crate::drawn`]).
    ///
    /// The exact inverse of `hidden`, and handed in for the same reason: the
    /// horizontal page is given the same answer by [`crate::wrap::Measure`],
    /// and a candidate the renderer alone knew about would put the caret, the
    /// wrap and the mouse on three different pages.
    ///
    /// A run stands in slots of its own — no characters, so `start == end` and
    /// the cursor steps past them exactly as it steps past the indent's. Each
    /// carries the ink it is drawn in, so that a note (#248) is not read as a
    /// word of the manuscript.
    drawn: &'a dyn Fn(usize) -> Vec<crate::drawn::Run>,
}

/// A page with every character on it — for callers that show the source as it
/// is, and for tests, which say what they mean by passing their own.
const NOTHING_HIDDEN: &dyn Fn(usize) -> Vec<(usize, usize)> = &|_| Vec::new();

/// A page with every line on it.
const NOTHING_FOLDED: &dyn Fn(usize) -> bool = &|_| false;

/// A page holding nothing but the file's own characters.
const NOTHING_DRAWN: &dyn Fn(usize) -> Vec<crate::drawn::Run> = &|_| Vec::new();

impl<'a> Grid<'a> {
    /// The same grid, told what is off the page: the markup, by line.
    pub fn with_hidden(self, hidden: &'a dyn Fn(usize) -> Vec<(usize, usize)>) -> Grid<'a> {
        Grid { hidden, ..self }
    }

    /// The same grid, told what stands on the page that the file has not got.
    pub fn with_drawn(self, drawn: &'a dyn Fn(usize) -> Vec<crate::drawn::Run>) -> Grid<'a> {
        Grid { drawn, ..self }
    }

    /// What is drawn into `line` that the file has no bytes for, in the order
    /// it is drawn.
    pub fn drawn_on(self, line: usize) -> Vec<crate::drawn::Run> {
        crate::drawn::compose((self.drawn)(line))
    }

    /// The same grid, told which lines are off the page.
    pub fn with_folds(self, folded: &'a dyn Fn(usize) -> bool) -> Grid<'a> {
        Grid { folded, ..self }
    }

    /// The same grid, told which document and which version of it this is.
    pub fn with_stamp(self, stamp: u64) -> Grid<'a> {
        Grid { stamp, ..self }
    }

    /// Whether `line` is off the page altogether.
    pub fn folded(self, line: usize) -> bool {
        (self.folded)(line)
    }

    /// A grid with **nothing off the page** — the source as the file has it.
    ///
    /// Named the way [`crate::wrap::Measure::plain`] is named, and for the same
    /// reason: a page that was never told what is hidden or folded shows a
    /// different document from the one the reader is looking at, and a
    /// constructor called `new` does not say so at the call site. The page the
    /// editor draws is built by `Editor::grid_with`, which cannot leave them
    /// out.
    pub fn plain(zong_len: usize, ruby: Dialects) -> Grid<'static> {
        Grid {
            stamp: 0,
            zong_len: zong_len.max(1),
            ruby,
            hanging: false,
            sentences: false,
            tatechuyoko: false,
            hidden: NOTHING_HIDDEN,
            folded: NOTHING_FOLDED,
            indent: 0,
            open_line: usize::MAX,
            drawn: NOTHING_DRAWN,
        }
    }

    /// The same grid, with `line` shown as the file has it.
    pub fn with_open_line(self, line: Option<usize>) -> Grid<'a> {
        Grid {
            open_line: line.unwrap_or(usize::MAX),
            ..self
        }
    }

    /// The same grid, opening each paragraph with `n` empty squares.
    pub fn with_indent(self, n: usize) -> Grid<'a> {
        Grid {
            indent: n.min(8),
            ..self
        }
    }

    /// The same grid, hanging 句讀 in the margin.
    pub fn with_hanging(self, on: bool) -> Grid<'a> {
        Grid {
            hanging: on,
            ..self
        }
    }

    /// The same grid, opening a 縱 at every 句.
    pub fn with_sentences(self, on: bool) -> Grid<'a> {
        Grid {
            sentences: on,
            ..self
        }
    }

    /// The same grid, packing half-width pairs into one slot.

    pub fn with_tatechuyoko(self, on: bool) -> Grid<'a> {
        Grid {
            tatechuyoko: on,
            ..self
        }
    }

    /// The same grid at a different wrap length — what the renderer does once
    /// the terminal's height is known.
    pub fn with_zong_len(self, zong_len: usize) -> Grid<'a> {
        Grid {
            zong_len: zong_len.max(1),
            ..self
        }
    }
}

impl Default for Grid<'static> {
    fn default() -> Grid<'static> {
        Grid::plain(
            DEFAULT_ZONG_LENGTH,
            Dialects::only(crate::ruby::Dialect::Html),
        )
    }
}

/// One 縱: a single vertical run of text on screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Zong {
    /// The logical (buffer) line this 縱 is a wrapped piece of.
    pub line: usize,
    /// Which piece of that line it is (`0` for the first).
    pub index_in_line: usize,
    /// Char index of the first character in the 縱.
    pub start: usize,
    /// Char index one past the last character in the 縱.
    pub end: usize,
    /// How many grapheme slots the 縱 fills.
    pub slots: usize,
}

impl Zong {
    /// Whether this 縱 starts a new paragraph (rather than continuing one).
    pub fn starts_line(&self) -> bool {
        self.index_in_line == 0
    }
}

/// Where the cursor sits in the 縱 grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Position {
    /// The logical line.
    pub line: usize,
    /// Which 縱 of that line.
    pub index_in_line: usize,
    /// The slot within the 縱, top to bottom.
    ///
    /// Equals the 縱's slot count when the cursor rests *past* the last
    /// character of a paragraph — the caret then sits on the spare row below
    /// the text, which is why the renderer reserves one row more than the wrap
    /// length.
    pub slot: usize,
}

/// How many lines the 縱 grid covers.
///
/// This is ropey's own count, which treats a file's trailing newline as opening
/// one more (empty) line — deliberately, because that empty line is where the
/// caret sits after `o` or a closing Enter, and the horizontal view draws it
/// too. It is *not* [`crate::motion::last_line`], which hides that line so `j`
/// cannot walk onto it.
fn line_count(rope: &Rope) -> usize {
    rope.len_lines()
}

/// The characters of `line`, line break stripped — what the ruby parser and the
/// slot layout both work over.
pub fn line_chars(rope: &Rope, line: usize) -> Vec<char> {
    line_text(rope, line).chars().collect()
}

/// The text of `line` with its line break stripped.
fn line_text(rope: &Rope, line: usize) -> String {
    let mut text = rope.line(line).to_string();
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    text
}

/// The longest run of half-width characters that will be set 縦中横 — turned a
/// quarter turn and packed sideways into a single slot.
///
/// Two, because a slot is two cells and each half-width character takes one.
/// This is what makes 「第<b>12</b>章」 read as a number rather than a stack of
/// loose digits, and it is why the vertical layout can show a year at all.
const TATECHUYOKO: usize = 2;

/// One row of a 縱: what it draws, and which characters it stands for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Slot {
    /// Char offset within the line where this row's text begins.
    pub start: usize,
    /// One past its last character. Equal to `start` for a padding row opened to
    /// make space for a long reading.
    pub end: usize,
    /// The two-cell body, punctuation already rotated. Empty for a padding row.
    pub text: String,
    /// The reading character drawn in the column to the right, if any.
    pub ruby: Option<char>,
    /// A 句讀 mark hung in the margin beside this character, if any.
    ///
    /// It shares the column with a reading, and wins it: the mark belongs
    /// against the character it follows, so the reading is the one that gives
    /// way (see [`line_slots`]).
    pub mark: Option<char>,
    /// The ink this row is drawn in, when it is not the file's own text
    /// ([`crate::drawn`]) — `None` for every character the file holds, and for
    /// the padding that opens a line.
    pub ink: Option<crate::drawn::Ink>,
}

impl Slot {
    /// Whether this row is [drawn](crate::drawn) rather than the file's own
    /// text — a candidate, a table's padding, a note.
    ///
    /// The ink answers it: everything the file holds has none, and so does the
    /// padding that opens a line, which stands for no characters and draws
    /// nothing.
    pub fn is_drawn(&self) -> bool {
        self.ink.is_some()
    }
}

/// Split a line into the rows a 縱 draws it as.
///
/// With `ruby` off this is just the slot run: one grapheme per row, half-width
/// alphanumerics paired 縦中横. With it on, a `<ruby>` group is *laid out*: the
/// markup disappears, the reading is dealt out down the ruby column, and the
/// base is centred over however many rows the reading needs. That spacing is
/// what real typesetting does and is why two adjacent readings never collide.
/// …with **nothing off the page**, whatever the grid was told.
///
/// The name says so now: this used to be spelled `line_slots`, take a `Grid`
/// carrying the answer, and ignore it — public API that contradicts the value
/// it is handed. Everything that draws goes through [`line_slots_in`].
pub fn line_slots_plain(text: &str, grid: Grid) -> Vec<Slot> {
    line_slots_in(text, grid, &[])
}

/// [`line_slots`], told which characters of this line are off the page.
///
/// The ranges are columns within the line and come from the editor — the same
/// answer the horizontal page is given. This function used to work them out
/// itself and got them wrong in three ways; see [`Grid::hidden`].
pub fn line_slots_in(text: &str, grid: Grid, hidden: &[(usize, usize)]) -> Vec<Slot> {
    let chars: Vec<char> = text.chars().collect();
    let groups = crate::ruby::groups(&chars, grid.ruby);
    let mut slots = Vec::new();
    // 首行縮進: empty squares before the paragraph's first character. They are
    // padding slots — no text, no reading, and `start == end == 0`, so a cursor
    // at the line's start resolves past them onto the first real character.
    if opens_a_paragraph(text) {
        for _ in 0..grid.indent {
            slots.push(Slot {
                start: 0,
                end: 0,
                text: String::new(),
                ruby: None,
                mark: None,
                ink: None,
            });
        }
    }
    let mut at = 0usize;
    // An opening bracket waits for the character it introduces, and that
    // character may be inside the next ruby group — 「<ruby>漢…. The wait has to
    // outlive the plain run, or the bracket is left behind as a row of its own
    // and the reader loses the very square hanging it was meant to save.
    let mut opening = None;
    // Which characters are markup rather than writing. A hidden run takes no
    // slot of its own; it joins the slot beside it, so the cursor steps over
    // `**` in one press and the wrap length counts writing.
    for group in &groups {
        push_plain(
            &mut slots,
            &chars,
            at,
            group.start,
            grid,
            &mut opening,
            hidden,
        );
        push_ruby(&mut slots, &chars, group, grid, &mut opening, hidden);
        at = group.end;
    }
    push_plain(
        &mut slots,
        &chars,
        at,
        chars.len(),
        grid,
        &mut opening,
        hidden,
    );
    // Markup at the very end of the line has no slot after it to join, so it
    // joins the one before — the line's last slot then covers it, and the
    // end-of-line caret still sits past the whole thing rather than inside it.
    let from = slots.last().map_or(0, |s| s.end);
    if from < chars.len()
        && (from..chars.len()).all(|i| hidden.iter().any(|&(a, b)| i >= a && i < b))
    {
        if let Some(last) = slots.last_mut() {
            last.end = chars.len();
        }
    }
    // Whatever is still waiting had nothing to hang on; it is drawn on its own.
    if let Some((at, mark)) = opening {
        slots.push(Slot {
            start: at,
            end: chars.len(),
            text: String::new(),
            ruby: None,
            mark: Some(mark),
            ink: None,
        });
    }
    slots
}

/// Put the line's drawn text into `slots`, before the character each run is
/// anchored at.
///
/// The runs take rows of their own, standing for no characters — so the 縱's
/// length counts them (a candidate takes room on the page like anything else)
/// while the cursor steps straight past them onto the file's own text, which
/// is what the indent's padding has always done.
fn insert_drawn(slots: &mut Vec<Slot>, chars_len: usize, grid: Grid, drawn: &[crate::drawn::Run]) {
    for run in drawn.iter().rev() {
        let text = &run.text;
        let anchor = run.column.min(chars_len);
        // Before the first row that holds the anchor's own character. Padding
        // is not that row — a candidate typed at the head of a paragraph
        // stands after the indent, not in front of it.
        let at = slots
            .iter()
            .position(|s| s.end > s.start && s.end > anchor)
            .unwrap_or(slots.len());
        let offsets = slot_offsets(text, grid.tatechuyoko);
        let body: Vec<char> = text.chars().collect();
        for (i, w) in offsets.windows(2).enumerate() {
            let piece: String = body[w[0]..w[1]].iter().collect();
            slots.insert(
                at + i,
                Slot {
                    start: anchor,
                    end: anchor,
                    text: rotate(&piece),
                    ruby: None,
                    mark: None,
                    ink: Some(run.ink),
                },
            );
        }
    }
}

/// Whether this line is a paragraph of prose, and so takes the indent.
///
/// Prose is what is left when the markup lead-ins are taken out: a heading, a
/// list item, a quote, a rule, a fence and a table row all carry their own
/// leading structure, and pushing them two squares right would say something
/// about them that is not true. The test is the first character or two, the
/// same few the block scan looks at.
pub(crate) fn opens_a_paragraph(text: &str) -> bool {
    let trimmed = text.trim_start();
    if trimmed.trim_end().is_empty() {
        return false;
    }
    // An indented line is already saying something about itself — **and a
    // Chinese manuscript indents with 　 (U+3000)**, not with spaces. 226 of
    // 紅樓夢's first 400 paragraphs open with two of them, and 395 of 三体's;
    // every one of those was getting two more squares on top.
    if text.starts_with([' ', '\t', '\u{3000}']) {
        return false;
    }
    // A `[` is only structure when it opens a footnote definition; a
    // paragraph may perfectly well begin with a link.
    if trimmed.starts_with("[^") {
        return false;
    }
    !trimmed.starts_with(['#', '=', '-', '*', '+', '>', '|', '`', '~'])
}

/// Lay out `chars[from..to]` as ordinary rows.
fn push_plain(
    slots: &mut Vec<Slot>,
    chars: &[char],
    from: usize,
    to: usize,
    grid: Grid,
    opening: &mut Option<(usize, char)>,
    hidden: &[(usize, usize)],
) {
    if from >= to {
        return;
    }
    // Markup that comes off the page joins the slot after it — or, at the end
    // of a run, the slot before it. Either way it is *inside* a slot, so a
    // motion crosses it in one step and the cursor never lands in text that is
    // not on the screen.
    let is_hidden = |at: usize| hidden.iter().any(|&(a, b)| at >= a && at < b);
    let mut swallowed: Option<usize> = None;
    let text: String = chars[from..to].iter().collect();
    let offsets = slot_offsets(&text, grid.tatechuyoko);
    for w in offsets.windows(2) {
        let body: String = chars[from + w[0]..from + w[1]].iter().collect();
        let at = from + w[0];

        if is_hidden(at) {
            swallowed = Some(swallowed.unwrap_or(at));
            continue;
        }

        // 標點旁置: a 句讀 mark stops being a row of its own and hangs beside the
        // character it follows. It joins that character's slot, so the wrap
        // length, the cursor and every motion agree that 「文。」 is one row.
        // …**whatever came off the page just before it**. This used to give up
        // the moment a hidden run had been swallowed, so 那年**冬天**。 took a
        // whole square for its 。 while 那年冬天。 hung it — turning
        // 所見即所得 on quietly turned 標點旁置 off. The run has somewhere to
        // go in every branch below: it joins whichever slot the mark joins.
        if grid.hanging && body.chars().count() == 1 {
            let mark = body.chars().next().expect("one character");
            // The half-width form is what hangs, and a mark that has none does
            // not hang at all — it keeps its square. So this is one question,
            // not two, and the two can no longer disagree.
            if let Some(hung) = yumete_cjk::margin_form(mark) {
                // **An opener still waiting goes into the margin first.** It is
                // waiting for the character it introduces, and a mark is not
                // that character — so `（；，。` used to leave the （ to be
                // attached to whatever came *after* the three marks, giving its
                // row a start behind rows already pushed. `position` looks a
                // caret up in these rows with a binary search, which needs them
                // in document order and answered quietly with the wrong row
                // when they were not.
                if let Some((opened_at, earlier)) = opening.take() {
                    // ⚠️ **Its own square, not an empty one with the glyph in
                    // the margin** (#231): an opener that never reached a base
                    // — `（（` or `（。` — is still a mark, and a blank square
                    // in the middle of the column is the one thing 標點旁置
                    // exists to avoid.
                    slots.push(Slot {
                        start: opened_at,
                        end: at,
                        text: rotate(&chars[opened_at..at.min(chars.len())].iter().collect::<String>()),
                        ruby: None,
                        mark: None,
                        ink: None,
                    });
                    swallowed = None;
                    // That row covers everything from the bracket to here, the
                    // markup between them included. Leaving the run pending
                    // would hand the same characters to the row after it as
                    // well, and two rows holding one character is the same
                    // defect as none holding it.
                }
                if yumete_cjk::opens_a_pair(mark) {
                    // A second opener while one is already waiting — `（「` —
                    // reads down the margin in the order the two were written:
                    // the first has just taken a row of its own above, and this
                    // one waits in its place.
                    *opening = Some((swallowed.take().unwrap_or(at), hung));
                    continue;
                }
                match slots.last_mut() {
                    // The usual case: it joins the character it follows.
                    Some(previous) if previous.mark.is_none() && !previous.text.is_empty() => {
                        // The extension covers the hidden run between them too.
                        previous.end = from + w[1];
                        previous.mark = Some(hung);
                        swallowed = None;
                        continue;
                    }
                    // **Two marks running do not hang at all.** `。」` ends
                    // most sentences of Chinese dialogue, and only one of them
                    // can sit beside the character it follows — so the second
                    // used to take a margin row of its own with the text
                    // square beside it left empty, which is the hole a reader
                    // sees in the middle of the column.
                    //
                    // Print does not do that. JLREQ §3.1.4① and clreq §6.3.2.2
                    // set a 句點 followed by a closing bracket **solid**: each
                    // is a half-em glyph, and the pair fills exactly one em. A
                    // terminal square is two cells and each half-width mark is
                    // one, so the pair fills a square here too — and the
                    // margin, which can only ever hold one, is left out of it.
                    // (clreq §6.1.3 says the same from the other side: 連續標點
                    // 不作懸掛.)
                    //
                    // The first mark therefore comes back *out* of the margin
                    // and joins the second in a square of its own. Left to
                    // right within it, which is the order the file has them in.
                    // …and only when the two really are **consecutive marks**.
                    // In 「秋「冬」」 the 冬 slot also carries a mark, but that
                    // one is the *opener* waiting for the base it introduces,
                    // with 冬 written between them: pulling it out would put
                    // 「 after the character it opens.
                    //
                    // ⚠️ **…and only where both marks have a *true* narrow
                    // form** (#230). 。 and 、 are the ones: half-em glyphs
                    // whose right half is blank, which is what a bracket nests
                    // into. ？ and ！ fill their em — clreq §6.3.2 separates
                    // them from the 句號 group for exactly that — and the only
                    // narrow twin Unicode has for them is the **ASCII** mark,
                    // so squeezing wrote `?」` and `,」` into a manuscript set
                    // in Chinese. Those pairs do not hang at all: both marks
                    // come back and take a square each, which is what
                    // 連續標點不作懸掛 says anyway.
                    Some(previous)
                        if previous.end == at
                            && !previous.text.is_empty()
                            && at > from
                            && yumete_cjk::margin_form(chars[at - 1]).is_some()
                            && !yumete_cjk::opens_a_pair(chars[at - 1]) =>
                    {
                        let earlier = previous.mark.take().expect("a mark to unhang");
                        // The base keeps its own square; the mark that was
                        // hanging on it moves into the new one.
                        let earlier_at = previous.end.saturating_sub(1);
                        previous.end = earlier_at;
                        let solid = yumete_cjk::narrow_form(chars[at - 1]).is_some()
                            && yumete_cjk::narrow_form(mark).is_some();
                        if solid {
                            slots.push(Slot {
                                start: earlier_at,
                                end: from + w[1],
                                text: format!("{earlier}{hung}"),
                                ruby: None,
                                mark: None,
                                ink: None,
                            });
                            continue;
                        }
                        // Neither hangs: the earlier one goes back to a square
                        // of its own, full width, and so does this one.
                        slots.push(Slot {
                            start: earlier_at,
                            end: at,
                            text: rotate(&chars[at - 1].to_string()),
                            ruby: None,
                            mark: None,
                            ink: None,
                        });
                        slots.push(Slot {
                            start: at,
                            end: from + w[1],
                            text: rotate(&body),
                            ruby: None,
                            mark: None,
                            ink: None,
                        });
                        swallowed = None;
                        continue;
                    }
                    // **Anything else that finds the margin taken keeps its
                    // own square** — Feature #231.
                    //
                    // ⚠️ It used to take a margin row with the text square
                    // beside it **empty**, which is a hole in the middle of
                    // the column — and not a rare one: 秋「冬」」 makes two of
                    // them and 春（。）」 makes four, because a base already
                    // carrying an opener has no margin left to give. A row is
                    // a row either way, so putting the glyph in the square
                    // costs nothing and leaves nothing blank.
                    Some(_) => {
                        slots.push(Slot {
                            start: swallowed.take().unwrap_or(at),
                            end: from + w[1],
                            text: rotate(&body),
                            ruby: None,
                            mark: None,
                            ink: None,
                        });
                        continue;
                    }
                    // Nothing before it — a mark at the head of a paragraph
                    // has nothing to hang on, so it keeps its square.
                    None => {}
                }
            }
        }

        // A waiting opener takes this character's margin, and its own start, so
        // the cursor steps over the pair as one row.
        let (start, mark) = match opening.take() {
            Some((opened_at, mark)) => (opened_at, Some(mark)),
            None => (at, None),
        };
        let start = swallowed.take().unwrap_or(start).min(start);
        slots.push(Slot {
            start,
            end: from + w[1],
            text: rotate(&body),
            ruby: None,
            mark,
            ink: None,
        });
    }

    // A hidden run still pending when the run ends — the markup before a ruby
    // group, say — has no slot after it to join, so it joins the one before.
    // Left dropped, its characters would belong to no slot at all, and the
    // cursor could be put on one of them.
    //
    // …unless a bracket is still waiting. Its own row starts before this run
    // and is drawn covering the rest of the line, so handing the run to the
    // slot *before* the bracket would put those characters in two rows at once.
    if let Some(at) = swallowed.filter(|_| opening.is_none()) {
        match slots.last_mut() {
            Some(last) => last.end = last.end.max(to),
            // Nothing before it either: the whole run is markup, and it still
            // needs somewhere for the cursor to stand.
            None => slots.push(Slot {
                start: at,
                end: to,
                text: String::new(),
                ruby: None,
                mark: None,
                ink: None,
            }),
        }
    }
}

/// Lay out one ruby group: the base centred against its reading.
fn push_ruby(
    slots: &mut Vec<Slot>,
    chars: &[char],
    group: &crate::ruby::Ruby,
    grid: Grid,
    opening: &mut Option<(usize, char)>,
    hidden: &[(usize, usize)],
) {
    let base = group.base_text(chars);
    let reading: Vec<char> = group.reading_text(chars).to_vec();
    let is_hidden = |at: usize| hidden.iter().any(|&(a, b)| at >= a && at < b);
    // The base's own rows, then as many more as the reading needs.
    //
    // **The base is text like any other**, so the markup in it comes off the
    // page like any other: `<ruby>**永和**<rt>` is 永和 in bold, and this
    // function was the one place that was never handed the answer — so 縱書
    // drew the asterisks that 橫排 hid, in the class of bug the page-as-data
    // refactor was written to close.
    let base_rows: Vec<(usize, usize)> = {
        let text: String = base.iter().collect();
        let squares: Vec<(usize, usize)> = slot_offsets(&text, grid.tatechuyoko)
            .windows(2)
            .map(|w| (group.base.0 + w[0], group.base.0 + w[1]))
            .collect();
        // A square with nothing left in it takes no row: it joins its
        // neighbour, exactly as a hidden run does in a plain one, so the rows
        // still tile the base and no character belongs to nothing.
        let mut rows: Vec<(usize, usize)> = Vec::new();
        for (a, b) in squares {
            let all_gone = (a..b).all(is_hidden);
            match rows.last_mut() {
                // A run held at the head joins the first row with writing in it.
                Some(last) if (last.0..last.1).all(is_hidden) => last.1 = b,
                Some(last) if all_gone => last.1 = b,
                _ => rows.push((a, b)),
            }
        }
        rows
    };
    // Where the reading goes relative to its base.
    //
    // Normally it brackets the base, which is centred in the span — mono-ruby
    // with spacing, and the tightest arrangement in which two adjacent readings
    // do not collide.
    //
    // With 句讀 hanging in the margin, they cannot share: a mark belongs against
    // the character it follows, on that character's own row, and the margin is
    // one cell wide. So the reading **gives way upward** — it takes the rows
    // above the base outright, opening a gap between this character and the one
    // before it, and leaving every row of the base free for a mark.
    let (rows, top) = if grid.hanging {
        (base_rows.len() + reading.len(), reading.len())
    } else {
        let rows = base_rows.len().max(reading.len()).max(1);
        (rows, (rows - base_rows.len()) / 2)
    };
    let rows = rows.max(1);
    // **And the reading is centred against the base** (JLREQ §3.3.6), which is
    // the same rule the base follows against the span two lines up. A reading
    // shorter than what it reads — 「上海話」 read 「zon」 — used to start at
    // the group's first row and stop two rows above the base's last, pointing
    // at the top character rather than at the word.
    //
    // Except when marks hang: there the reading has *taken* the rows above the
    // base outright so every base row is free for a mark, and centring it
    // would push it back down into them.
    let reading_top = match grid.hanging {
        true => 0,
        false => (rows - reading.len().min(rows)) / 2,
    };

    let base_end = base_rows.last().map_or(group.base.0, |&(_, b)| b);
    for row in 0..rows {
        let (start, end, body) = match row.checked_sub(top).and_then(|i| base_rows.get(i)) {
            Some(&(a, b)) => (
                a,
                b,
                (a..b).filter(|&i| !is_hidden(i)).map(|i| chars[i]).collect::<String>(),
            ),
            // A padding row stands for no characters of its own. It still has to
            // sit in document order — or the rows stop being sorted and nothing
            // can look the cursor up in them.
            //
            // **The group's own markup lives on the first of them.** It used to
            // be given to the first row as an afterthought, which wrote
            // `start = group.start` onto a row whose range was empty — so
            // `<ruby>` belonged to no slot at all, the caret for those six
            // characters collapsed onto a blank square, and with 標點旁置 on it
            // happened to *every* ruby group.
            None if row == 0 => (group.start, group.base.0, String::new()),
            None if row < top => (group.base.0, group.base.0, String::new()),
            None => (base_end, base_end, String::new()),
        };
        slots.push(Slot {
            start,
            end,
            text: rotate(&body),
            ruby: row.checked_sub(reading_top).and_then(|i| reading.get(i)).copied(),
            mark: None,
            ink: None,
        });
    }
    // The very first row owns the whole group's markup, so a cursor stepping
    // over it steps over the tags too rather than into them.
    let first_row = slots.len().checked_sub(rows);
    if let Some(first) = first_row.and_then(|i| slots.get_mut(i)) {
        first.start = group.start;
    }
    // A bracket that was waiting for the character this group annotates hangs
    // against the base's *first* row — the row the reader sees the base on —
    // rather than being left behind as a row of its own before the group.
    if let Some((opened_at, mark)) = opening.take() {
        // The base's first row — or, when the base is empty (`<ruby><rt>…`),
        // the group's first row, which is the one the reader sees. Without the
        // fallback the index landed one past the end, the bracket was put back,
        // and it came out later with a `start` behind rows already pushed:
        // unsorted rows, which `position` binary-searches.
        let base_row = slots
            .len()
            .checked_sub(rows.saturating_sub(top))
            .filter(|&i| i < slots.len())
            .or(first_row);
        match base_row.and_then(|i| slots.get_mut(i)) {
            Some(slot) if slot.mark.is_none() => {
                slot.mark = Some(mark);
                // The bracket's own character belongs to the group's **first**
                // row, not to the base's. Giving it to the base pulled that row
                // behind the padding rows above it, and `position` looks a
                // caret up with a binary search — which needs the rows sorted,
                // and quietly answered with the wrong one when they were not.
                if let Some(first) = first_row.and_then(|i| slots.get_mut(i)) {
                    first.start = opened_at.min(first.start);
                }
            }
            // Its row is already spoken for; put the bracket back to be drawn
            // on its own rather than dropping it.
            _ => *opening = Some((opened_at, mark)),
        }
    }
    if let Some(last) = slots.last_mut() {
        last.end = group.end;
    }
}

/// Substitute the vertical presentation form, if the body is a lone character
/// that has one.
fn rotate(body: &str) -> String {
    match yumete_cjk::vertical_grapheme(body) {
        Some(c) => c.to_string(),
        None => body.to_string(),
    }
}

/// The char offset of every **slot** boundary in `text`, including the end.
///
/// A slot is one row of a 縱. Usually it holds one grapheme, but a short run of
/// half-width characters is packed into one slot 縦中横-style — see
/// [`TATECHUYOKO`]. Everything else in this module is defined in terms of these
/// offsets, so packing here is what makes the cursor, the wrap length, motion
/// and the renderer all agree that `12` is one row.
///
/// The result always has at least one element, so `offsets.len() - 1` is the
/// slot count and `offsets[i]` is where slot `i` begins.
fn slot_offsets(text: &str, tatechuyoko: bool) -> Vec<usize> {
    let all: Vec<&str> = graphemes(text).collect();
    let mut offsets = Vec::with_capacity(all.len() + 1);
    let mut chars = 0usize;
    let mut i = 0;
    // A run of half-width *alphanumerics* may share a slot; anything else —
    // full-width, punctuation, a space — stands on its own. 縦中横 is for
    // numbers and short Latin, and packing a comma in beside a letter would
    // only look like a mistake.
    let narrow = |g: &str| grapheme_width(g) == 1 && g.chars().all(char::is_alphanumeric);
    while i < all.len() {
        if !tatechuyoko || !narrow(all[i]) {
            offsets.push(chars);
            chars += all[i].chars().count();
            i += 1;
            continue;
        }
        // **The whole run, or none of it.** Filling one slot and spilling the
        // rest turned 「1997」 into 「19」 and 「97」 stacked — two numbers, read
        // as two numbers. A run that will not fit is set the way a Japanese
        // book sets a long number in a 縱: one digit to a slot, straight up.
        let mut run = i;
        while run < all.len() && narrow(all[run]) {
            run += 1;
        }
        let pack = run - i <= TATECHUYOKO;
        offsets.push(chars);
        for g in &all[i..run] {
            if !pack && chars > offsets[offsets.len() - 1] {
                offsets.push(chars);
            }
            chars += g.chars().count();
        }
        i = run;
    }
    offsets.push(chars);
    offsets
}

/// Where each 縱 of a line begins, counted in slots.
///
/// **The one place a 縱 boundary is decided.** It used to be decided in five —
/// every one of them `index * zong_len` — which is why 禁則處理 could not be
/// added: any adjustment made in one of them would have disagreed with the
/// other four about which character the cursor was standing on.
///
/// The rule is [`wrap::adjusted_break`]'s, turned ninety degrees: a 縱 may not
/// open with a mark that closes something (。、」）) and may not close with one
/// that opens something (「（), so the offending character is pulled down with
/// its neighbour. The horizontal page has always done this; the vertical page —
/// the reason to choose this editor — did not.
fn zong_breaks(
    chars: &[char],
    slots: &[Slot],
    zong_len: usize,
    groups: &[crate::ruby::Ruby],
    hidden: &[(usize, usize)],
    sentences: bool,
) -> Vec<usize> {
    let total = slots.len();
    // **The character the reader sees**, not the one the range begins with.
    // 禁則 is about what stands at the head and the foot of a 縱, and a slot
    // that swallowed a hidden run begins at the `*` — so with 所見即所得 on,
    // 。 was read as an asterisk and allowed to open a column, which is the one
    // thing 禁則處理 exists to prevent. A row with nothing but markup in it
    // reports a space, which no rule forbids anywhere.
    let is_hidden = |at: usize| hidden.iter().any(|&(a, b)| at >= a && at < b);
    let char_of = |i: usize| {
        slots
            .get(i)
            .and_then(|s: &Slot| (s.start..s.end).find(|&at| !is_hidden(at)))
            .and_then(|at| chars.get(at))
            .copied()
            .unwrap_or(' ')
    };
    // Which slot each ruby group opens on. A group is laid out across several
    // slots — the reading dealt down the ruby column, the base centred against
    // it — so a break inside one leaves the base in this 縱 and the rest of its
    // reading in the next, where it is centred against nothing.
    let group_start = |i: usize| -> Option<usize> {
        let at = slots.get(i)?.start;
        groups
            .iter()
            .find(|g| at > g.start && at < g.end)
            .map(|g| slots.partition_point(|s| s.start < g.start))
    };
    // Where each 句 opens, in slots (`:view-sentence`, Feature #237). The
    // boundaries are `motion`'s, so the page breaks where `(` and `)` jump; the
    // *character* index they come back as is turned into a slot index here,
    // because a slot may have swallowed hidden markup or a ruby group and the
    // two stopped counting alike several features ago.
    let sentence_cuts: Vec<usize> = match sentences {
        false => Vec::new(),
        true => crate::motion::sentence_starts(chars)
            .into_iter()
            .map(|at| slots.partition_point(|s: &Slot| s.start < at))
            .filter(|&i| i > 0 && i < total)
            .collect(),
    };
    let mut breaks = vec![0usize];
    let mut at = 0;
    while at < total {
        // The 縱 ends at whichever comes first: the end of the 句, or the
        // measure. A 句 longer than the column still wraps — one 句 to a 縱 is
        // the *most* a 縱 holds, never a licence to run off the foot of the
        // page.
        //
        // A sentence cut needs no 禁則 adjustment and must not be given one: it
        // already falls after the 。 and after whatever closed the quotation,
        // which is exactly where the rule would have moved it to, and retreating
        // from it would put the 。 at the head of the next 縱 instead.
        if let Some(cut) = sentence_cuts.iter().copied().find(|&c| c > at) {
            if cut <= at + zong_len {
                breaks.push(cut);
                at = cut;
                continue;
            }
        }
        if at + zong_len >= total {
            break;
        }
        let mut cut = at + zong_len;
        // A reading group is not two characters and cannot be pulled down two
        // at a time; it moves whole or not at all. Moving it whole is right
        // while the 縱 keeps most of its length — beyond that the group is
        // simply longer than a column has room to postpone, and it breaks.
        if let Some(opens) = group_start(cut) {
            if opens > at && (opens - at) * 2 >= zong_len {
                cut = opens;
            }
        }
        for _ in 0..crate::wrap::MAX_KINSOKU_RETREAT {
            if cut <= at + 1 {
                break;
            }
            // 分離禁止 (clreq §6.1.2.1): `——` and `……` take two squares and
            // are one mark. Vertically the halves are stroke glyphs, so a
            // split leaves a rule that stops at the foot of one 縱 and starts
            // again at the head of the next.
            let split_a_pair = crate::wrap::half_of_a_pair(char_of(cut))
                && char_of(cut) == char_of(cut - 1);
            if split_a_pair
                || crate::wrap::forbidden_at_row_start(char_of(cut))
                || crate::wrap::forbidden_at_row_end(char_of(cut - 1))
            {
                cut -= 1;
            } else {
                break;
            }
        }
        breaks.push(cut);
        at = cut;
    }
    breaks
}

/// One laid-out paragraph: the hash it is keyed by, its rows, and where its 縱
/// begin.
///
/// Behind `Rc`, because a page asks for the same paragraph once per 縱 and the
/// answer is one `Slot` per character: handing back a copy meant a 500,000-
/// character paragraph was *copied* forty times a frame even when every one of
/// those asks was a cache hit — and merely asking **how many** 縱 it has copied
/// all of them.
type RememberedZongs = (u64, Laid);

/// A paragraph's rows and 縱 boundaries, shared rather than copied.
type Laid = (std::rc::Rc<Vec<Slot>>, std::rc::Rc<Vec<usize>>);

/// How many paragraphs' worth of 縱 are kept.
///
/// A page of 縱書 walks its paragraphs **twice** — `zongs_from` to find the
/// columns, then `zong_slots` for each of them — so a page whose 縱 come from
/// more than this many paragraphs lays every one of them out again on the
/// second pass. Prose with short paragraphs (dialogue, a 論語 chapter) is
/// exactly that page, and eight was fewer than one screen holds. The entries
/// are `Rc`, so the cost of a larger number is one pointer per paragraph plus
/// whatever is still referenced.
const REMEMBERED_PARAGRAPHS: usize = 96;

thread_local! {
    /// Laid-out paragraphs, most recently used first.
    ///
    /// Keyed by a hash of the paragraph's own text and of everything about the
    /// grid that changes the answer — never by a line number or a revision, so
    /// a matching hash is a correct answer whatever else in the document, or in
    /// another document, has moved since. Exactly how [`crate::wrap`] does it,
    /// and for the same reason: the vertical page asks `zongs_from` and
    /// `zong_slots` once per 縱, so a paragraph holding 500,000 characters was
    /// laid out afresh forty times a frame — 565 ms per keystroke.
    static ZONGS: RefCell<Vec<RememberedZongs>> = const { RefCell::new(Vec::new()) };

    /// How many paragraphs have actually been laid out, for the test that
    /// keeps a frame from quietly becoming forty passes over a chapter again.
    static LAID_OUT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How many paragraphs have been laid out since [`reset_layout_count`].
#[cfg(test)]
fn layout_count() -> usize {
    LAID_OUT.with(|n| n.get())
}

/// Start counting laid-out paragraphs again.
#[cfg(test)]
fn reset_layout_count() {
    LAID_OUT.with(|n| n.set(0));
}

/// One line's slots and where its 縱 begin — always asked for together.
fn line_zongs(rope: &Rope, line: usize, grid: Grid) -> Laid {
    let mut hasher = DefaultHasher::new();
    // **Which document, which version, which line** — and everything about the
    // grid that changes the answer. Neither the text nor what is hidden on it
    // is read here: both are answers *about* that version, and the stamp
    // already names it (see [`Grid::stamp`]). Asking the page what is hidden
    // costs a Markdown scan of the paragraph, and asking it before the cache
    // was consulted put that scan back on every one of a page's forty 縱 —
    // which was the whole cost the memo had just removed.
    (grid.stamp, line).hash(&mut hasher);
    (
        grid.zong_len,
        grid.indent,
        grid.hanging,
        grid.sentences,
        grid.tatechuyoko,
        grid.ruby.bits(),
        grid.open_line == line,
    )
        .hash(&mut hasher);
    // **What is drawn is not part of that version.** The stamp names the
    // buffer and its revision, which is why what is hidden need not be hashed;
    // a candidate changes on every keystroke while the buffer does not move at
    // all, so the memo would keep answering with the candidate before last. Asking
    // for it is a filter over a handful of runs, not a Markdown scan, so it can
    // be asked before the cache is consulted.
    grid.drawn_on(line).hash(&mut hasher);
    let hash = hasher.finish();
    if grid.stamp == 0 {
        let hidden = (grid.hidden)(line);
        return lay_out(rope, line, grid, &hidden);
    }
    if let Some(answer) = ZONGS.with(|cache| {
        let mut cache = cache.borrow_mut();
        let i = cache.iter().position(|(h, _)| *h == hash)?;
        let entry = cache.remove(i);
        let answer = entry.1.clone();
        cache.insert(0, entry);
        Some(answer)
    }) {
        return answer;
    }
    let hidden = (grid.hidden)(line);
    let laid = lay_out(rope, line, grid, &hidden);
    ZONGS.with(|cache| {
        let mut cache = cache.borrow_mut();
        cache.insert(0, (hash, laid.clone()));
        cache.truncate(REMEMBERED_PARAGRAPHS);
    });
    laid
}

/// Lay one paragraph out: its rows, and where its 縱 begin.
fn lay_out(rope: &Rope, line: usize, grid: Grid, hidden: &[(usize, usize)]) -> Laid {
    LAID_OUT.with(|n| n.set(n.get() + 1));
    let slots = line_grid_in(rope, line, grid, hidden);
    let chars: Vec<char> = line_text(rope, line).chars().collect();
    let groups = crate::ruby::groups(&chars, grid.ruby);
    let breaks = zong_breaks(
        &chars,
        &slots,
        grid.zong_len.max(1),
        &groups,
        hidden,
        grid.sentences,
    );
    (std::rc::Rc::new(slots), std::rc::Rc::new(breaks))
}

/// Where the `index`-th 縱 of a line begins and ends, in slots.
fn zong_span(breaks: &[usize], total: usize, index: usize) -> (usize, usize) {
    let first = breaks.get(index).copied().unwrap_or(total);
    let last = breaks.get(index + 1).copied().unwrap_or(total);
    (first, last.max(first))
}

/// How many 縱 the logical `line` wraps into (always at least one, so an empty
/// paragraph still occupies a column).
pub fn zong_count_in_line(rope: &Rope, line: usize, grid: Grid) -> usize {
    line_zongs(rope, line, grid).1.len()
}

/// The rows `line` draws as, under `grid`.
fn line_grid_in(rope: &Rope, line: usize, grid: Grid, hidden: &[(usize, usize)]) -> Vec<Slot> {
    let text = line_text(rope, line);
    // The paragraph being typed into is laid out as the file has it: no
    // opening squares. Done here, where the line is known — everything
    // downstream asks this function for a line's slots, so nothing else has
    // to know about it.
    let grid = match line == grid.open_line {
        true => Grid { indent: 0, ..grid },
        false => grid,
    };
    let mut slots = line_slots_in(&text, grid, hidden);
    let drawn = grid.drawn_on(line);
    if !drawn.is_empty() {
        insert_drawn(&mut slots, text.chars().count(), grid, &drawn);
    }
    slots
}

/// Locate the char index `pos` in the 縱 grid.
pub fn position(rope: &Rope, pos: usize, grid: Grid) -> Position {
    let line = rope.char_to_line(pos.min(rope.len_chars()));
    let start = rope.line_to_char(line);
    let (slots, breaks) = line_zongs(rope, line, grid);
    let total = slots.len();

    // Which slot the cursor sits in (or `total`, past the last one). A slot may
    // span several characters — a 縦中横 pair, or a whole ruby group's markup —
    // so this is the last one that *starts* at or before the cursor, not the
    // first one that reaches it.
    let col = pos.saturating_sub(start);
    let g = if slots.last().is_some_and(|s| col >= s.end) {
        total
    } else {
        slots.partition_point(|s| s.start <= col).saturating_sub(1)
    };

    // Which 縱 holds it: the last one that starts at or before this slot. A
    // paragraph whose length is an exact multiple of the wrap length has no
    // further 縱 to hold the end-of-paragraph caret, and this puts it on the
    // spare row under the last full one rather than opening a phantom column.
    let index_in_line = breaks.partition_point(|&b| b <= g).saturating_sub(1);
    let slot = g - breaks.get(index_in_line).copied().unwrap_or(0);
    Position {
        line,
        index_in_line,
        slot,
    }
}

/// The slot the cursor occupies within its 縱 — the "goal slot" preserved when
/// moving between 縱, the way a goal column is preserved between lines.
pub fn slot_of(rope: &Rope, pos: usize, grid: Grid) -> usize {
    position(rope, pos, grid).slot
}

/// The largest slot the caret may occupy in a given 縱. The last 縱 of a
/// paragraph has one extra slot for the end-of-paragraph caret.
fn max_slot(rope: &Rope, line: usize, index_in_line: usize, grid: Grid) -> usize {
    let (slots, breaks) = line_zongs(rope, line, grid);
    let total = slots.len();
    let (first, last) = zong_span(&breaks, total, index_in_line);
    if index_in_line + 1 >= breaks.len() {
        total.saturating_sub(first)
    } else {
        (last - first).saturating_sub(1)
    }
}

/// The char index of `slot` in the `index_in_line`-th 縱 of `line`.
fn char_at(rope: &Rope, line: usize, index_in_line: usize, slot: usize, grid: Grid) -> usize {
    let start = rope.line_to_char(line);
    let (slots, breaks) = line_zongs(rope, line, grid);
    let g = breaks.get(index_in_line).copied().unwrap_or(slots.len()) + slot;
    match slots.get(g) {
        Some(slot) => start + slot.start,
        // Past the last slot: the end-of-paragraph caret.
        None => start + slots.last().map_or(0, |s| s.end),
    }
}

/// Move to the **next** 縱 — the one drawn to the *left*, since 縱 run right to
/// left — keeping `goal_slot` where the new 縱 is long enough. Stays put at the
/// end of the buffer.
pub fn next_zong(rope: &Rope, pos: usize, grid: Grid, goal_slot: usize) -> usize {
    let p = position(rope, pos, grid);
    let (line, index) = if p.index_in_line + 1 < zong_count_in_line(rope, p.line, grid) {
        (p.line, p.index_in_line + 1)
    } else if next_shown(rope, p.line + 1, grid) < line_count(rope) {
        (next_shown(rope, p.line + 1, grid), 0)
    } else {
        return pos;
    };
    let slot = goal_slot.min(max_slot(rope, line, index, grid));
    char_at(rope, line, index, slot, grid)
}

/// Move to the **previous** 縱 — the one drawn to the *right* — keeping
/// `goal_slot`. Stays put at the start of the buffer.
pub fn prev_zong(rope: &Rope, pos: usize, grid: Grid, goal_slot: usize) -> usize {
    let p = position(rope, pos, grid);
    let (line, index) = if p.index_in_line > 0 {
        (p.line, p.index_in_line - 1)
    } else if p.line > 0 {
        match prev_shown(rope, p.line - 1, grid) {
            Some(line) => (line, zong_count_in_line(rope, line, grid) - 1),
            None => return pos,
        }
    } else {
        return pos;
    };
    let slot = goal_slot.min(max_slot(rope, line, index, grid));
    char_at(rope, line, index, slot, grid)
}

/// Which 縱 a page is anchored at: a paragraph, and which piece of it.
///
/// The renderer scrolls in these rather than in a global 縱 number, because a
/// global number cannot be found without walking the document from the top —
/// which, on a novel, is the whole novel on every keystroke.
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

/// What, if anything, is drawn in a paragraph's opening squares.
///
/// **Nothing, by default.** In a printed book those two squares are white, and
/// a mark that appears at the head of *every* paragraph is a mark that says
/// nothing — the eye stops reading it by the third page. It is here because
/// while a draft is being *edited* the question 「這裏原本是不是有個空行」 is
/// a real one, and the answer is otherwise only in the line numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndentHint {
    /// White, the way a book prints it.
    #[default]
    None,
    /// The opening squares carry a band, a shade off the page.
    Colour,
    /// A character in the first square — a `↵`, say, for the line break it
    /// stands in for.
    Symbol,
}

impl IndentHint {
    pub fn parse(value: &str) -> Option<IndentHint> {
        match value.trim().to_ascii_lowercase().as_str() {
            "none" | "off" | "無" | "无" => Some(IndentHint::None),
            "color" | "colour" | "底色" => Some(IndentHint::Colour),
            "symbol" | "mark" | "符號" | "符号" => Some(IndentHint::Symbol),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            IndentHint::None => "none",
            IndentHint::Colour => "color",
            IndentHint::Symbol => "symbol",
        }
    }
}

/// Whether `line` is off the page altogether (Feature #159).
///
/// The single blank line between two written ones, once an indent is marking
/// the paragraphs instead. Two blanks in a row are a scene break and both
/// stay; the cursor's own line always stays.
pub fn folded(_rope: &Rope, line: usize, grid: Grid) -> bool {
    // **One rule, asked — not a second copy of it.** This function used to
    // decide for itself, with a different test from the horizontal page's:
    // no block knowledge, a span instead of a map, and a `cursor_line` of its
    // own. Same document, two answers, and flipping `:layout` changed which
    // lines of the file were on the page.
    grid.folded(line)
}

/// The first line at or after `line` that is on the page.
fn next_shown(rope: &Rope, line: usize, grid: Grid) -> usize {
    let lines = line_count(rope);
    let mut line = line;
    while line < lines && folded(rope, line, grid) {
        line += 1;
    }
    line
}

/// The last line at or before `line` that is on the page, if there is one.
fn prev_shown(rope: &Rope, line: usize, grid: Grid) -> Option<usize> {
    let mut line = line;
    while folded(rope, line, grid) {
        line = line.checked_sub(1)?;
    }
    Some(line)
}

/// The next `n` 縱 starting at `anchor`, stopping early at the end of the
/// buffer.
///
/// Costs one pass over each *paragraph the page touches*, not over the
/// document: a 縱 that continues the previous one reuses the segmentation
/// already done for its paragraph.
pub fn zongs_from(rope: &Rope, anchor: Anchor, grid: Grid, n: usize) -> Vec<Zong> {
    let lines = line_count(rope);
    let mut zongs = Vec::with_capacity(n);
    let mut line = anchor.line;
    let mut index = anchor.index_in_line;
    while zongs.len() < n && line < lines {
        if folded(rope, line, grid) {
            line += 1;
            index = 0;
            continue;
        }
        let start = rope.line_to_char(line);
        let (slots, breaks) = line_zongs(rope, line, grid);
        let total = slots.len();
        while index < breaks.len() && zongs.len() < n {
            let (first, last) = zong_span(&breaks, total, index);
            zongs.push(Zong {
                line,
                index_in_line: index,
                start: start + slots.get(first).map_or(0, |s| s.start),
                end: start
                    + last
                        .checked_sub(1)
                        .and_then(|i| slots.get(i))
                        .map_or(0, |s| s.end),
                slots: last - first,
            });
            index += 1;
        }
        line += 1;
        index = 0;
    }
    zongs
}

/// The anchor `n` 縱 *before* `anchor` — that is, `n` to the right — clamped to
/// the first 縱 of the buffer.
pub fn retreat(rope: &Rope, anchor: Anchor, grid: Grid, mut n: usize) -> Anchor {
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
        // Step past this paragraph's remaining pieces and land on the last 縱
        // of the one before it.
        n -= index + 1;
        line = match prev_shown(rope, line - 1, grid) {
            Some(line) => line,
            None => return Anchor::default(),
        };
        index = zong_count_in_line(rope, line, grid) - 1;
    }
}

/// How many 縱 forward it is from `from` to `to`, or `None` when `to` is behind
/// `from` or further ahead than `limit`.
///
/// Bounded on purpose: the renderer only ever needs to know where the cursor
/// sits *within the page*, and giving up past the page keeps this proportional
/// to the screen rather than to the document.
pub fn distance(rope: &Rope, from: Anchor, to: Anchor, grid: Grid, limit: usize) -> Option<usize> {
    let lines = line_count(rope);
    if from.line >= lines || to < from {
        return None;
    }
    let mut line = from.line;
    let mut index = from.index_in_line;
    let mut count = zong_count_in_line(rope, line, grid);
    for step in 0..=limit {
        if line == to.line && index == to.index_in_line {
            return Some(step);
        }
        index += 1;
        if index >= count {
            line = next_shown(rope, line + 1, grid);
            if line >= lines {
                return None;
            }
            index = 0;
            count = zong_count_in_line(rope, line, grid);
        }
    }
    None
}

/// Every 縱 in the buffer, in reading order: index `0` is the rightmost.
///
/// Only the renderer needs this; motion answers its questions from the cursor's
/// own line. It walks the whole document, which is why the renderer calls it
/// once per frame rather than per query.
pub fn layout(rope: &Rope, grid: Grid) -> Vec<Zong> {
    let mut zongs = Vec::new();
    for line in 0..line_count(rope) {
        let start = rope.line_to_char(line);
        let (slots, breaks) = line_zongs(rope, line, grid);
        let total = slots.len();
        for index_in_line in 0..breaks.len() {
            let (first, last) = zong_span(&breaks, total, index_in_line);
            zongs.push(Zong {
                line,
                index_in_line,
                start: start + slots.get(first).map_or(0, |s| s.start),
                end: start
                    + last
                        .checked_sub(1)
                        .and_then(|i| slots.get(i))
                        .map_or(0, |s| s.end),
                slots: last - first,
            });
        }
    }
    zongs
}

/// The rows one 縱 draws, ready to paint.
///
/// Taken from the whole paragraph's layout rather than from the 縱's own text,
/// because a ruby group must not be re-parsed from a slice that might cut it in
/// half at a 縱 boundary.
pub fn zong_slots(rope: &Rope, zong: &Zong, grid: Grid) -> Vec<Slot> {
    let (slots, breaks) = line_zongs(rope, zong.line, grid);
    let (first, last) = zong_span(&breaks, slots.len(), zong.index_in_line);
    if first >= last {
        return Vec::new();
    }
    // One 縱's worth, not the paragraph's: what the caller draws is a column,
    // and the paragraph behind it stays shared.
    slots[first..last].to_vec()
}

/// The index into [`layout`] of the 縱 holding char index `pos`.
///
/// A cursor resting on a line break belongs to the last 縱 of the line it ends,
/// which is what "the last 縱 that starts at or before `pos`" gives.
pub fn zong_index(zongs: &[Zong], pos: usize) -> usize {
    match zongs.binary_search_by(|z| z.start.cmp(&pos)) {
        Ok(i) => i,
        Err(0) => 0,
        Err(i) => i - 1,
    }
}

/// Split `text` into what each slot draws, punctuation already rotated.
///
/// The renderer walks this rather than the graphemes, so a 縦中横 pair arrives
/// as one two-cell string and lands in one row.
///
/// Takes the one setting it uses rather than a whole [`Grid`]: it used to take
/// the page and read only `tatechuyoko` from it, which is API that contradicts
/// the value it is handed — the same thing `line_slots` was renamed for.
pub fn slot_text(text: &str, tatechuyoko: bool) -> Vec<String> {
    let offsets = slot_offsets(text, tatechuyoko);
    let chars: Vec<char> = text.chars().collect();
    offsets
        .windows(2)
        .map(|w| {
            let slice: String = chars[w[0]..w[1]].iter().collect();
            match yumete_cjk::vertical_grapheme(&slice) {
                Some(c) => c.to_string(),
                None => slice,
            }
        })
        .collect()
}

/// How many cells one slot of a 縱 occupies.
const SLOT_CELLS: usize = 2;

/// Render the whole buffer as a plain-text vertical page: a grid of lines,
/// each holding one slot from every 縱, with the 縱 running right to left.
///
/// This is what the terminal draws, minus colour and the cursor — it exists so
/// `yumete --preview --vertical` can show the layout on stdout, and so the
/// vertical page can be diffed in a test without a terminal backend.
pub fn render_page(rope: &Rope, grid: Grid, gap: usize) -> Vec<String> {
    let mut zongs = layout(rope, grid);
    // A file's trailing newline opens an empty line that the editor needs (the
    // caret has to have somewhere to go) but a printed page does not, exactly as
    // the horizontal preview drops the same phantom line.
    if rope.len_chars() > 0 && rope.char(rope.len_chars() - 1) == '\n' {
        zongs.pop();
    }
    let rows = zongs.iter().map(|z| z.slots).max().unwrap_or(0);
    // Every 縱's slots, top to bottom; the page is then read across.
    // Each 縱 contributes its bodies and, beside them, its readings — the same
    // two columns the terminal draws.
    let columns: Vec<(Vec<String>, Vec<Option<char>>)> = zongs
        .iter()
        .map(|z| {
            let rows = zong_slots(rope, z, grid);
            (
                rows.iter().map(|s| s.text.clone()).collect(),
                // A hung mark shares the margin with a reading and wins it.
                rows.iter().map(|s| s.mark.or(s.ruby)).collect(),
            )
        })
        .collect();

    // Which 縱 carry anything in their margin, decided one 縱 at a time exactly
    // as the terminal decides it: a 縱 with no reading and no hung mark needs no
    // margin, and with the gap set to zero it sits flush against its neighbour.
    let margins: Vec<usize> = columns
        .iter()
        .map(|(_, margin)| usize::from(margin.iter().any(Option::is_some)))
        .collect();

    (0..rows)
        .map(|row| {
            let mut line = String::new();
            // 縱 0 is the rightmost, so the page is written in reverse order.
            for (i, (bodies, margin)) in columns.iter().enumerate().rev() {
                match bodies.get(row) {
                    // Pad a short grapheme out to the full slot so the columns
                    // stay aligned. An *empty* body — a mark-only row, or a row
                    // of a ruby group's padding — is two cells of nothing, not
                    // one; padding it to one walked the rest of the row left.
                    Some(g) => {
                        line.push_str(g);
                        for _ in yumete_cjk::str_width(g)..SLOT_CELLS {
                            line.push(' ');
                        }
                    }
                    None => line.push_str("  "),
                }
                // The margin sits to the *right* of its base — after it in the
                // written line — and it *is* the gap rather than sitting beside
                // one: two 縱 are `max(gap, margin)` apart, which is how the
                // terminal places them.
                let region = gap.max(margins[i]);
                for cell in 0..region {
                    let glyph = margin.get(row).copied().flatten();
                    match (cell, glyph) {
                        (0, Some(c)) if margins[i] == 1 => line.push(c),
                        _ => line.push(' '),
                    }
                }
            }
            // Only the padding is trimmed, never the text: an ideographic
            // space is whitespace to `char::is_whitespace`, and trimming it
            // would eat the 全角 indent every paragraph opens with.
            line.trim_end_matches(' ').to_string()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The grid these tests read against: the default wrap length, ruby layout
    /// off, so they exercise the slot model alone. The ruby tests build their
    /// own grid.
    /// Readings written in HTML, the dialect the Yuhao documents use.
    const HTML_ONLY: Dialects = Dialects(1 << (crate::ruby::Dialect::Html as u8));

    const G: Grid = Grid {
        // No stamp: a grid built by hand remembers nothing, because without one
        // two different documents look the same to the memo's key.
        stamp: 0,
        zong_len: 32,
        ruby: Dialects::NONE,
        hanging: false,
        sentences: false,
        tatechuyoko: false,
        hidden: NOTHING_HIDDEN,
        folded: NOTHING_FOLDED,
        drawn: NOTHING_DRAWN,
        indent: 0,
        open_line: usize::MAX,
    };

    /// Readings laid out, so the ruby tests exercise the layout.
    const RUBY: Grid = Grid {
        ruby: HTML_ONLY,
        ..G
    };

    /// Half-width pairs packed, for the 縦中横 tests.
    const PACKED: Grid = Grid {
        tatechuyoko: true,
        ..G
    };

    fn rope(text: &str) -> Rope {
        Rope::from_str(text)
    }

    #[test]
    fn a_short_paragraph_is_one_zong() {
        let r = rope("春江潮水連海平");
        let zongs = layout(&r, G);
        assert_eq!(zongs.len(), 1);
        assert_eq!(zongs[0].slots, 7);
        assert!(zongs[0].starts_line());
    }

    #[test]
    fn a_zong_does_not_open_with_a_full_stop() {
        // 行頭禁則, down the column. The horizontal page has always done this;
        // the vertical page — the reason to choose this editor — did not, and
        // a 縱 opening with 。 is the one typographic error a Chinese reader
        // cannot fail to see.
        //
        // Ten slots to a 縱, and the eleventh character is the stop, so the
        // unadjusted break puts it at the head of the second column.
        let rope = Rope::from_str("一二三四五六七八九十。十一十二\n");
        let grid = Grid {
            zong_len: 10,
            hanging: false,
            ..Grid::default()
        };
        let zongs = layout(&rope, grid);
        assert!(zongs.len() >= 2);
        let second: String = zong_slots(&rope, &zongs[1], grid)
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert!(
            !second.starts_with('。') && !second.starts_with('︒'),
            "a 縱 opened with a full stop: {second:?}"
        );
        // The stop went down with the character it belongs to — 追い出し, the
        // same strategy the horizontal wrap uses, so the two pages never
        // disagree about where a paragraph breaks.
        assert!(second.starts_with('十'), "{second:?}");
        let first: String = zong_slots(&rope, &zongs[0], grid)
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(first.chars().count(), 9, "{first:?}");
    }

    #[test]
    fn a_zong_does_not_close_with_an_opening_bracket() {
        // 行末禁則: 「 introduces what follows it, so it goes down with it.
        let rope = Rope::from_str("一二三四五六七八九「十」十一\n");
        let grid = Grid {
            zong_len: 10,
            hanging: false,
            ..Grid::default()
        };
        let zongs = layout(&rope, grid);
        assert!(zongs.len() >= 2);
        let first: String = zong_slots(&rope, &zongs[0], grid)
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert!(
            !first.ends_with('「') && !first.ends_with('﹁'),
            "a 縱 closed with an opening bracket: {first:?}"
        );
    }

    #[test]
    fn every_slot_of_a_line_is_in_exactly_one_zong() {
        // The invariant a break table has to keep, and the reason there is now
        // one of them rather than five: the cursor's 縱 and the drawn 縱 must
        // agree about which character it is standing on.
        let rope = Rope::from_str("一二三。四五六「七八」九十。十一十二十三、十四\n");
        for zong_len in 2..12 {
            let grid = Grid {
                zong_len,
                hanging: false,
                ..Grid::default()
            };
            let zongs = layout(&rope, grid);
            let mut seen = 0;
            for (i, zong) in zongs.iter().enumerate() {
                let drawn = zong_slots(&rope, zong, grid).len();
                assert_eq!(drawn, zong.slots, "縱 {i} at width {zong_len}");
                seen += drawn;
            }
            let total = line_slots_plain(rope.line(0).to_string().trim_end(), grid).len();
            assert_eq!(seen, total, "width {zong_len}: {} 縱", zongs.len());
            // And every character's own position agrees with the layout.
            for pos in 0..rope.len_chars() {
                let p = position(&rope, pos, grid);
                if p.line != 0 {
                    continue;
                }
                assert!(
                    p.index_in_line < zongs.len(),
                    "char {pos} at width {zong_len} claims 縱 {}",
                    p.index_in_line
                );
                assert!(
                    p.slot <= zongs[p.index_in_line].slots,
                    "char {pos} at width {zong_len}: slot {} of a {}-slot 縱",
                    p.slot,
                    zongs[p.index_in_line].slots
                );
            }
        }
    }

    #[test]
    fn a_paragraph_opens_two_squares_in() {
        // A Chinese paragraph is marked by an indent of two 字, not by a blank
        // line — and the blank line costs a whole 縱 of the page.
        let grid = Grid { indent: 2, ..G };
        let slots = line_slots_plain("那年冬天", grid);
        assert_eq!(slots.len(), 6, "two empty squares, then four characters");
        assert!(slots[0].text.is_empty() && slots[0].start == slots[0].end);
        assert!(slots[1].text.is_empty());
        assert_eq!(slots[2].text, "那");

        // The file is untouched: this is a view, so the document still exports
        // as the paragraphs it is.
        let rope = Rope::from_str("那年冬天\n");
        assert_eq!(rope.to_string(), "那年冬天\n");

        // And the cursor never sits in the indent: at the line's start it is on
        // the first real character.
        let p = position(&rope, 0, grid);
        assert_eq!(p.slot, 2, "past the padding");
        assert_eq!(char_at(&rope, 0, 0, p.slot, grid), 0, "…and on 那");
    }

    #[test]
    fn only_prose_is_indented() {
        let grid = Grid { indent: 2, ..G };
        for line in [
            "# 第一章", "- 一項", "+ 一項", "> 引文", "| a | b |", "```", "~~~",
            "[^1]: 一條腳註", "  已經縮進了",
        ] {
            let slots = line_slots_plain(line, grid);
            assert!(
                slots.first().is_some_and(|s| !s.text.is_empty()),
                "{line:?} should not be indented"
            );
        }
        // A blank line stays blank.
        assert!(line_slots_plain("", grid).is_empty());
        // …but a paragraph may perfectly well open with a link.
        let slots = line_slots_plain("[書名](a.md) 是這樣寫的。", grid);
        assert!(slots[0].text.is_empty() && slots[1].text.is_empty(), "{slots:?}");
    }

    #[test]
    fn the_indent_costs_the_first_zong_its_squares() {
        // It is made of slots, so it is paid for where a reader sees it paid:
        // the paragraph's first 縱 holds two characters fewer.
        let rope = Rope::from_str("一二三四五六七八九十\n");
        let grid = Grid {
            zong_len: 5,
            indent: 2,
            ..G
        };
        let zongs = layout(&rope, grid);
        assert_eq!(zongs[0].slots, 5, "two of them empty");
        let first: String = zong_slots(&rope, &zongs[0], grid)
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(first, "一二三");
    }

    #[test]
    fn a_long_paragraph_wraps_into_several_zong() {
        let r = rope(&"字".repeat(70));
        let zongs = layout(&r, G);
        assert_eq!(zongs.len(), 3);
        assert_eq!(zongs[0].slots, 32);
        assert_eq!(zongs[1].slots, 32);
        assert_eq!(zongs[2].slots, 6);
        // The pieces are consecutive and cover the paragraph.
        assert_eq!(zongs[0].end, zongs[1].start);
        assert_eq!(zongs[2].end, 70);
    }

    #[test]
    fn an_empty_paragraph_still_takes_a_zong() {
        let r = rope("上\n\n下");
        let zongs = layout(&r, G);
        assert_eq!(zongs.len(), 3);
        assert_eq!(zongs[1].slots, 0);
        assert_eq!(zongs[1].line, 1);
    }

    #[test]
    fn position_reports_the_piece_and_slot() {
        let r = rope(&"字".repeat(70));
        assert_eq!(position(&r, 0, G).slot, 0);
        assert_eq!(position(&r, 31, G).index_in_line, 0);
        assert_eq!(position(&r, 31, G).slot, 31);
        assert_eq!(position(&r, 32, G).index_in_line, 1);
        assert_eq!(position(&r, 32, G).slot, 0);
        assert_eq!(position(&r, 70, G).index_in_line, 2);
        assert_eq!(position(&r, 70, G).slot, 6);
    }

    /// A paragraph that fills its last 縱 exactly parks the caret on the spare
    /// row rather than opening an empty column.
    #[test]
    fn end_of_an_exactly_full_paragraph_uses_the_spare_slot() {
        let r = rope(&"字".repeat(32));
        assert_eq!(layout(&r, G).len(), 1);
        let p = position(&r, 32, G);
        assert_eq!(p.index_in_line, 0);
        assert_eq!(p.slot, 32);
    }

    #[test]
    fn next_zong_walks_left_within_and_across_paragraphs() {
        let r = rope(&format!("{}\n{}", "字".repeat(70), "文".repeat(5)));
        // Slot 3 of the first 縱 → slot 3 of the second.
        let pos = next_zong(&r, 3, G, 3);
        assert_eq!(position(&r, pos, G).index_in_line, 1);
        assert_eq!(position(&r, pos, G).slot, 3);
        // From the paragraph's last 縱 into the next paragraph, whose 縱 is
        // shorter — the slot is clamped to its end.
        let pos = next_zong(&r, 64 + 3, G, 3);
        assert_eq!(position(&r, pos, G).line, 1);
        assert_eq!(position(&r, pos, G).slot, 3);
        let pos = next_zong(&r, 64 + 30, G, 30);
        assert_eq!(position(&r, pos, G).line, 1);
        assert_eq!(position(&r, pos, G).slot, 5);
    }

    #[test]
    fn prev_zong_walks_right_and_stops_at_the_start() {
        let r = rope(&"字".repeat(70));
        let pos = prev_zong(&r, 40, G, 8);
        assert_eq!(position(&r, pos, G).index_in_line, 0);
        assert_eq!(position(&r, pos, G).slot, 8);
        // Already in the first 縱 of the first paragraph: stay put.
        assert_eq!(prev_zong(&r, 5, G, 5), 5);
    }

    #[test]
    fn next_zong_stops_at_the_end_of_the_buffer() {
        let r = rope("終");
        assert_eq!(next_zong(&r, 0, G, 0), 0);
    }

    /// A file's trailing newline opens one more 縱, and the caret must land on
    /// *that* one — not back on top of the last character of the paragraph
    /// before it. `layout` and `position` have to agree about it.
    #[test]
    fn a_trailing_newline_opens_a_zong_for_the_caret() {
        let r = rope("上下\n");
        let zongs = layout(&r, G);
        assert_eq!(zongs.len(), 2);
        assert_eq!(zongs[1].line, 1);
        assert_eq!(zongs[1].slots, 0);

        let end = r.len_chars();
        assert_eq!(zong_index(&zongs, end), 1);
        let p = position(&r, end, G);
        assert_eq!((p.line, p.index_in_line, p.slot), (1, 0, 0));

        // …and `h` can still walk onto it, rather than being dead there.
        let onto = next_zong(&r, 0, G, 0);
        assert_eq!(position(&r, onto, G).line, 1);
        assert_eq!(next_zong(&r, end, G, 0), end, "nothing past the last 縱");
    }

    #[test]
    fn a_page_lays_each_paragraph_out_once() {
        // A Chinese chapter is one paragraph, and the vertical page asks
        // `zongs_from` and then `zong_slots` for every 縱 on it — so an
        // unmemoised layout ran over the whole chapter once per column: 565 ms
        // a keystroke on 500,000 characters.
        let r = rope(&"那年冬天，山下起了大雪。".repeat(4_000));
        // Stamped, as the editor stamps it: which document and which version.
        // Without one nothing is remembered — see `Grid::stamp` — which is the
        // right answer for a grid built by hand and the wrong one for a page.
        let grid = Grid { zong_len: 24, ..G }.with_stamp(1);
        reset_layout_count();
        let page = zongs_from(&r, Anchor { line: 0, index_in_line: 0 }, grid, 40);
        for zong in &page {
            let _ = zong_slots(&r, zong, grid);
        }
        assert_eq!(page.len(), 40, "a page of 縱");
        assert_eq!(
            layout_count(),
            1,
            "the paragraph is laid out once for the whole page"
        );
    }

    #[test]
    fn zongs_from_walks_a_page_without_reading_the_whole_document() {
        let r = rope(&format!(
            "{}\n{}\n{}",
            "字".repeat(70),
            "文",
            "句".repeat(40)
        ));
        let page = zongs_from(
            &r,
            Anchor {
                line: 0,
                index_in_line: 1,
            },
            G,
            4,
        );
        assert_eq!(page.len(), 4);
        assert_eq!(
            page.iter()
                .map(|z| (z.line, z.index_in_line, z.slots))
                .collect::<Vec<_>>(),
            [(0, 1, 32), (0, 2, 6), (1, 0, 1), (2, 0, 32)]
        );
        // Asking past the end simply returns fewer.
        assert_eq!(
            zongs_from(
                &r,
                Anchor {
                    line: 2,
                    index_in_line: 1
                },
                G,
                5
            )
            .len(),
            1
        );
    }

    #[test]
    fn retreat_and_distance_are_inverses_across_paragraphs() {
        let r = rope(&format!(
            "{}\n{}\n{}",
            "字".repeat(70),
            "文",
            "句".repeat(40)
        ));
        let last = Anchor {
            line: 2,
            index_in_line: 1,
        };
        // 0,0 · 0,1 · 0,2 · 1,0 · 2,0 · 2,1 — six 縱 in all.
        assert_eq!(retreat(&r, last, G, 5), Anchor::default());
        assert_eq!(
            retreat(&r, last, G, 2),
            Anchor {
                line: 1,
                index_in_line: 0
            }
        );
        // Clamps rather than wrapping.
        assert_eq!(retreat(&r, last, G, 99), Anchor::default());

        assert_eq!(distance(&r, Anchor::default(), last, G, 10), Some(5));
        assert_eq!(
            distance(&r, Anchor::default(), last, G, 4),
            None,
            "past the limit"
        );
        assert_eq!(distance(&r, last, Anchor::default(), G, 10), None, "behind");
        assert_eq!(distance(&r, last, last, G, 10), Some(0));
    }

    /// The windowed walk and the whole-document layout must agree, or the page
    /// would drift from what the motions think is on it.
    #[test]
    fn zongs_from_agrees_with_the_full_layout() {
        let r = rope(&format!(
            "{}\n\n{}\n{}",
            "字".repeat(70),
            "文",
            "句".repeat(33)
        ));
        let all = layout(&r, G);
        for skip in 0..all.len() {
            let anchor = Anchor {
                line: all[skip].line,
                index_in_line: all[skip].index_in_line,
            };
            assert_eq!(
                zongs_from(&r, anchor, G, all.len()),
                all[skip..],
                "from {skip}"
            );
            assert_eq!(
                distance(&r, Anchor::default(), anchor, G, all.len()),
                Some(skip)
            );
            assert_eq!(retreat(&r, anchor, G, skip), Anchor::default());
        }
    }

    #[test]
    fn zong_index_finds_the_column_holding_a_position() {
        let r = rope(&format!("{}\n{}", "字".repeat(70), "文".repeat(5)));
        let zongs = layout(&r, G);
        assert_eq!(zong_index(&zongs, 0), 0);
        assert_eq!(zong_index(&zongs, 35), 1);
        assert_eq!(zong_index(&zongs, 65), 2);
        // The line break at char 70 belongs to the paragraph it ends.
        assert_eq!(zong_index(&zongs, 70), 2);
        assert_eq!(zong_index(&zongs, 71), 3);
    }

    #[test]
    fn render_page_lays_columns_out_right_to_left() {
        let r = rope("上下\n左右");
        let page = render_page(&r, G, 1);
        // Two 縱, the first paragraph on the right.
        assert_eq!(page, vec!["左 上".to_string(), "右 下".to_string()]);
    }

    #[test]
    fn render_page_drops_the_phantom_trailing_zong() {
        // The editor shows a column for the trailing newline; a printed page
        // must not open with a blank one.
        assert_eq!(render_page(&rope("上下\n"), G, 1), vec!["上", "下"]);
    }

    #[test]
    fn render_page_rotates_punctuation() {
        let r = rope("「甲」。");
        let page = render_page(&r, G, 1);
        assert_eq!(page, vec!["﹁", "甲", "﹂", "︒"]);
    }

    /// The reading is longer than the base, so the base is spaced out to make
    /// room — mono-ruby with spacing, which is what keeps two adjacent readings
    /// from colliding.
    #[test]
    fn a_ruby_group_spaces_its_base_against_the_reading() {
        let ruby = RUBY;
        let slots = line_slots_plain("他<ruby>口<rt>kǒu</rt></ruby>很", RUBY);
        let bodies: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(
            bodies,
            ["他", "", "口", "", "很"],
            "口 centred in three rows"
        );
        let readings: Vec<Option<char>> = slots.iter().map(|s| s.ruby).collect();
        assert_eq!(
            readings,
            [None, Some('k'), Some('ǒ'), Some('u'), None],
            "the reading runs beside the space it opened"
        );

        // The markup itself never takes a row of its own.
        let r = rope("他<ruby>口<rt>kǒu</rt></ruby>很");
        assert_eq!(layout(&r, ruby)[0].slots, 5);
    }

    /// A reading shorter than the word it reads sits against the middle of it
    /// — JLREQ §3.3.6, the same rule the base follows when the reading is the
    /// longer of the two.
    #[test]
    fn a_group_reading_is_centred_against_its_word() {
        let slots = line_slots_plain("<ruby>上海話<rt>zon</rt></ruby>好", RUBY);
        let bodies: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(bodies, ["上", "海", "話", "好"]);
        let readings: Vec<Option<char>> = slots.iter().map(|s| s.ruby).collect();
        assert_eq!(readings, [Some('z'), Some('o'), Some('n'), None]);

        // Two rows of reading over four of base: one row of space above it and
        // one below, rather than the reading starting at the top and pointing
        // at 上.
        let slots = line_slots_plain("<ruby>上海話劇<rt>zo</rt></ruby>好", RUBY);
        let readings: Vec<Option<char>> = slots.iter().map(|s| s.ruby).collect();
        assert_eq!(readings, [None, Some('z'), Some('o'), None, None]);

        // An odd row left over goes below, so a reading never sits lower than
        // the middle of its word.
        let slots = line_slots_plain("<ruby>上海話<rt>zo</rt></ruby>好", RUBY);
        let readings: Vec<Option<char>> = slots.iter().map(|s| s.ruby).collect();
        assert_eq!(readings, [Some('z'), Some('o'), None, None]);
    }

    #[test]
    fn two_adjacent_readings_do_not_collide() {
        let slots = line_slots_plain(
            "<ruby>口<rt>kǒu</rt></ruby><ruby>囗<rt>wéi</rt></ruby>",
            RUBY,
        );
        assert_eq!(slots.len(), 6, "three rows each");
        let readings: String = slots.iter().filter_map(|s| s.ruby).collect();
        assert_eq!(readings, "kǒuwéi");
    }

    #[test]
    fn a_reading_group_is_not_cut_in_half_by_a_zong_boundary() {
        // A group is laid out across several slots — the reading dealt down
        // the ruby column, the base centred against it. Cut it, and the base
        // stays in this 縱 while the rest of its reading runs down the next
        // one, centred against nothing.
        let text = "一二三四五六<ruby>漢字<rt>kanji</rt></ruby>七八九十\n";
        let rope = Rope::from_str(text);
        let grid = Grid {
            zong_len: 8,
            ruby: HTML_ONLY,
            hanging: false,
            ..Grid::default()
        };
        let chars: Vec<char> = text.trim_end().chars().collect();
        let groups = crate::ruby::groups(&chars, HTML_ONLY);
        assert_eq!(groups.len(), 1);
        let zongs = layout(&rope, grid);
        // Every 縱 boundary falls outside the group, or on its very first slot.
        for zong in &zongs {
            let at = zong.start;
            assert!(
                !(at > groups[0].start && at < groups[0].end),
                "a 縱 begins inside the group at char {at}",
            );
        }
        // And the adjustment really fired: the group straddles slot 8, so the
        // first 縱 gave up its last slots rather than cut it.
        assert!(zongs[0].slots < 8, "{} slots", zongs[0].slots);
    }

    #[test]
    fn a_reading_shorter_than_its_base_does_not_shrink_it() {
        let slots = line_slots_plain("<ruby>漢字<rt>hz</rt></ruby>", RUBY);
        let bodies: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(bodies, ["漢", "字"]);
        assert_eq!(
            slots.iter().map(|s| s.ruby).collect::<Vec<_>>(),
            [Some('h'), Some('z')]
        );
    }

    /// With ruby layout off the markup is exactly the text it is, so it can be
    /// read and edited.
    #[test]
    fn ruby_off_shows_the_markup() {
        let slots = line_slots_plain("<ruby>口<rt>kǒu</rt></ruby>", G);
        let bodies: String = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(bodies, "<ruby>口<rt>kǒu</rt></ruby>");
        assert!(slots.iter().all(|s| s.ruby.is_none()));
    }

    /// The cursor has to land inside the group, and stepping over it must not
    /// walk through the tags one character at a time.
    #[test]
    fn the_cursor_steps_over_a_ruby_group_by_row() {
        let ruby = RUBY;
        let r = rope("他<ruby>口<rt>kǒu</rt></ruby>很");
        assert_eq!(position(&r, 0, ruby).slot, 0, "他");
        // Anywhere inside the group maps to one of its three rows.
        for pos in 1..r.len_chars() - 1 {
            let slot = position(&r, pos, ruby).slot;
            assert!(
                (1..=3).contains(&slot) || slot == 4,
                "pos {pos} → slot {slot}"
            );
        }
        assert_eq!(position(&r, r.len_chars() - 1, ruby).slot, 4, "很");
    }

    /// Slots are grapheme clusters, so an ideographic variation sequence takes
    /// one row, not two.
    /// 縦中横: a short run of half-width characters is turned sideways into one
    /// slot, so a year or a chapter number reads as a number.
    #[test]
    fn digits_pack_sideways_into_one_slot_when_asked() {
        let r = rope("第12章");
        let zongs = layout(&r, PACKED);
        assert_eq!(zongs[0].slots, 3, "第 / 12 / 章");
        assert_eq!(slot_text("第12章", true), ["第", "12", "章"]);

        // The cursor agrees: the character after the pair is slot 2, not 3.
        assert_eq!(position(&r, 1, PACKED).slot, 1, "on the 1");
        assert_eq!(position(&r, 2, PACKED).slot, 1, "still inside the pair");
        assert_eq!(position(&r, 3, PACKED).slot, 2, "on 章");
    }

    #[test]
    fn packing_is_off_by_default() {
        // One letter to a row: turned sideways a pair reads as a syllable that
        // is not there.
        assert_eq!(slot_text("yume", false), ["y", "u", "m", "e"]);
        assert_eq!(slot_text("第12章", false), ["第", "1", "2", "章"]);
    }

    #[test]
    fn a_run_too_long_to_pack_is_not_packed_at_all() {
        // **The whole run, or none of it.** Two slots of two turned 「2026」
        // into 「20」 over 「26」 — two numbers, and read as two numbers. A run
        // that will not fit is set the way a Japanese book sets a long number
        // in a 縱: one character to a slot, straight up.
        assert_eq!(slot_text("2026年", true), ["2", "0", "2", "6", "年"]);
        assert_eq!(slot_text("abcde", true), ["a", "b", "c", "d", "e"]);
        // A run that *does* fit still packs, which is the whole point: 第12章
        // reads as a number rather than a stack of loose digits.
        assert_eq!(slot_text("第12章", true), ["第", "12", "章"]);
        assert_eq!(slot_text("第7章", true), ["第", "7", "章"]);
    }

    /// A 句讀 mark stops being a row of its own and hangs beside the character
    /// it follows, so the text runs unbroken down the 縱.
    #[test]
    fn a_mark_hangs_beside_the_character_it_follows() {
        let hanging = G.with_hanging(true);
        let slots = line_slots_plain("春江。潮水，", hanging);
        let bodies: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(bodies, ["春", "江", "潮", "水"], "four rows, not six");
        assert_eq!(
            slots.iter().map(|s| s.mark).collect::<Vec<_>>(),
            [None, Some('｡'), None, Some(',')],
            "and the marks are rotated, in the margin"
        );

        // Off, they take a square each, as they did.
        assert_eq!(line_slots_plain("春江。", G).len(), 3);
    }

    /// An opening bracket introduces what follows it, so it hangs beside *that*
    /// character — which is also what keeps the text column unbroken.
    #[test]
    fn an_opener_hangs_on_the_character_it_introduces() {
        let slots = line_slots_plain("曰「春江", G.with_hanging(true));
        assert_eq!(
            slots.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(),
            ["曰", "春", "江"],
            "three rows: the bracket costs none"
        );
        assert_eq!(
            slots.iter().map(|s| s.mark).collect::<Vec<_>>(),
            [None, Some('｢'), None],
            "and it sits beside 春, not 曰"
        );
    }

    /// 「。」 ends a line of speech, and is common enough that the second mark
    /// must not fall back into the text column.
    #[test]
    fn markup_taken_off_the_page_joins_the_slot_beside_it() {
        // 所見即所得: the `**` is not a row of its own. It belongs to the slot
        // beside it, so `l` steps over it in one press and a 縱 holds writing
        // rather than asterisks.
        // The page says what is off it — this test *is* the editor, so it
        // says so itself: `**` at 1..3 and at 4..6.
        let slots = line_slots_in("那**年**冬", G, &[(1, 3), (4, 6)]);
        let texts: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["那", "年", "冬"]);
        // …and the ranges cover every character of the line between them, so
        // there is nowhere to step that is not on the screen. Hidden markup
        // joins the slot *after* it, which is why 年 carries the opening `**`
        // and 冬 carries the closing one.
        assert_eq!((slots[0].start, slots[0].end), (0, 1), "那");
        assert_eq!((slots[1].start, slots[1].end), (1, 4), "**年");
        assert_eq!((slots[2].start, slots[2].end), (4, 7), "**冬");

        // The construct the cursor is in is shown whole — which is a decision
        // the *editor* makes, and it makes it by handing over nothing hidden.
        let texts: Vec<String> = line_slots_in("那**年**冬", G, &[])
            .into_iter()
            .map(|s| s.text)
            .collect();
        assert_eq!(texts, ["那", "*", "*", "年", "*", "*", "冬"]);
    }

    #[test]
    fn the_cursors_own_construct_is_shown_in_the_vertical_page_too() {
        // The whole invariant depends on where the selection is: without it the
        // construct under the cursor is hidden like any other, and `j` steps
        // onto an asterisk that is not on the screen.
        let rope = Rope::from_str("那**年**冬\n");
        // Nothing hidden on this line: the selection is holding the construct
        // open, and it is the editor that decides that.
        let grid = G;
        let texts: Vec<String> = zong_slots(
            &rope,
            &zongs_from(&rope, Anchor::default(), grid, 1)[0],
            grid,
        )
        .into_iter()
        .map(|s| s.text)
        .collect();
        assert_eq!(texts, ["那", "*", "*", "年", "*", "*", "冬"]);

        // With the cursor elsewhere on the line it comes off again — which
        // the page says by hiding it.
        let off = |_: usize| vec![(1usize, 3usize), (4, 6)];
        let grid = G.with_hidden(&off);
        let texts: Vec<String> = zong_slots(
            &rope,
            &zongs_from(&rope, Anchor::default(), grid, 1)[0],
            grid,
        )
        .into_iter()
        .map(|s| s.text)
        .collect();
        assert_eq!(texts, ["那", "年", "冬"]);
    }

    #[test]
    fn every_character_of_a_line_belongs_to_some_slot() {
        // Nothing may be steppable-onto that is not on the screen — so the
        // slots have to tile the line, whatever is hidden and wherever the
        // markup sits next to a ruby group.
        for line in [
            "那**年**<ruby>漢<rt>h</rt></ruby>冬",
            "**年**<ruby>漢<rt>h</rt></ruby>",
            "%%整行都是批注%%",
            "`碼`",
            "[](x)",
            "那**年",
            // A reading of **more than one character**, which is what every
            // real one is: the base then needs padding rows above it, and the
            // group's own tags used to be written onto a row whose range was
            // empty — so `<ruby>` belonged to no slot at all. The line this
            // test had always used, `<rt>h</rt>`, is the one length at which
            // that cannot happen.
            "他<ruby>漢<rt>hàn</rt></ruby>字。",
            "曰「<ruby>漢<rt>hàn</rt></ruby>字",
            "「<ruby>口<rt>kǒu</rt></ruby>」和「<ruby>囗<rt>wéi</rt></ruby>」。",
            // Markup inside the base: the base is text like any other.
            "<ruby>**永和**<rt>えいわ</rt></ruby>九年。",
            // A mark that arrives while an opening bracket is still waiting.
            "條目（；，。￥）不改",
        ] {
            // Whatever the editor says is off the page — here, the markup
            // Markdown itself would take off.
            let hidden = crate::markdown::hidden(&crate::markdown::spans(line), None);
            // 標點旁置 makes the reading take the rows *above* the base
            // outright, so every ruby group has padding rows — with it on, a
            // one-character reading is no longer the safe case either.
            for grid in [RUBY, Grid { hanging: true, ..RUBY }] {
                let slots = line_slots_in(line, grid, &hidden);
                let n = line.chars().count();
                for at in 0..n {
                    assert!(
                        slots.iter().any(|s| at >= s.start && at < s.end),
                        "char {at} of {line:?} is in no slot (hanging={})",
                        grid.hanging
                    );
                }
                // …and in document order, because `position` looks a caret up
                // in them with a binary search, which on unsorted rows does not
                // fail — it answers with the wrong row.
                assert!(
                    slots.windows(2).all(|w| w[0].start <= w[1].start),
                    "{line:?} (hanging={}) is out of order: {:?}",
                    grid.hanging,
                    slots.iter().map(|s| s.start).collect::<Vec<_>>()
                );
                assert!(!slots.is_empty(), "{line:?} has nowhere to put the cursor");
            }
        }
    }

    /// 所見即所得 does not turn 標點旁置 off, and does not turn 禁則處理 off.
    ///
    /// Both used to read the character at a slot's *start*, which after a
    /// hidden run is an asterisk: so 那年**冬天**。 gave its 。 a whole square
    /// while 那年冬天。 hung it, and a 。 at a column's head was let through
    /// because the rule was asked about `*`.
    #[test]
    fn what_comes_off_the_page_does_not_change_the_other_settings() {
        let hang = Grid { hanging: true, ..G };
        let mark_of = |line: &str| -> Vec<Option<char>> {
            let hidden = crate::markdown::hidden(&crate::markdown::spans(line), None);
            line_slots_in(line, hang, &hidden)
                .iter()
                .map(|s| s.mark)
                .collect()
        };
        assert!(
            mark_of("那年冬天。").contains(&Some('｡')),
            "the 。 hangs"
        );
        assert!(
            mark_of("那年**冬天**。").contains(&Some('｡')),
            "…and it still hangs when there is markup on the page"
        );

        // 禁則: a 。 may not open a 縱, markup or no markup.
        let opens_with = |line: &str, len: usize| -> Vec<char> {
            let hidden = crate::markdown::hidden(&crate::markdown::spans(line), None);
            let chars: Vec<char> = line.chars().collect();
            let grid = Grid { zong_len: len, ..G };
            let slots = line_slots_in(line, grid, &hidden);
            let groups = crate::ruby::groups(&chars, grid.ruby);
            zong_breaks(&chars, &slots, len, &groups, &hidden, false)
                .iter()
                .filter_map(|&b| slots.get(b))
                .filter_map(|s| s.text.chars().next())
                .collect()
        };
        for line in ["一二三四五六七八九十甲乙。丙丁戊己", "一二三四五六七八九十甲**乙**。丙丁戊己"] {
            let heads = opens_with(line, 12);
            assert!(
                !heads.contains(&'︒') && !heads.contains(&'。'),
                "{line:?} opens a 縱 with 。: {heads:?}"
            );
        }
    }

    /// `:view-sentence` (Feature #237): a 縱 ends where the 句 does — and the file
    /// is not touched, which is what the manual's `:%s/。/。\n/g` could never
    /// say.
    #[test]
    fn one_sentence_to_a_zong() {
        // Each 縱's opening character, at a measure far longer than any of the
        // sentences: without `sentences` the whole line is one 縱.
        let heads = |line: &str, len: usize, sentences: bool| -> Vec<String> {
            let grid = Grid {
                zong_len: len,
                sentences,
                ..G
            };
            let chars: Vec<char> = line.chars().collect();
            let slots = line_slots_in(line, grid, &[]);
            let groups = crate::ruby::groups(&chars, grid.ruby);
            zong_breaks(&chars, &slots, len, &groups, &[], sentences)
                .iter()
                .filter_map(|&b| slots.get(b))
                .map(|s| s.text.to_string())
                .collect()
        };

        let line = "雪下得極早。她說：「你回來了。」他沒有答話。";
        assert_eq!(heads(line, 40, false).len(), 1, "one 縱 without it");
        // Three 句 — and the third opens at 他, not at 」: a quotation closes
        // the sentence it belongs to.
        assert_eq!(heads(line, 40, true), ["雪", "她", "他"]);

        // A 句 longer than the measure still wraps: one 句 to a 縱 is the most a
        // 縱 holds, not a licence to run off the foot of the page.
        let long = "一二三四五六七八九十。甲乙丙";
        assert_eq!(heads(long, 6, true), ["一", "七", "甲"]);

        // 禁則 still holds where the measure does the breaking, and a sentence
        // break never leaves a 。 at the head of a 縱.
        for zong in heads("一二三四五。六七八九十。", 4, true) {
            assert_ne!(zong, "︒");
            assert_ne!(zong, "。");
        }
    }

    /// **Every character in exactly one row, and the rows in document order** —
    /// over every short line that can be built from the characters that fight.
    ///
    /// Enumerated rather than chosen: each of these defects was found by
    /// somebody reading the code and none by the tests, because the test that
    /// existed asked whether every character was in *at least* one slot. Two
    /// rows holding one character is the same defect as none holding it —
    /// `position` binary-searches the rows, and a caret that resolves into the
    /// wrong one of two overlapping rows is drawn a row off the mark.
    /// 禁則處理 against the marks a Chinese manuscript actually uses.
    ///
    /// The set used to be derived from what fits in a one-cell margin, which is
    /// a rendering question — so `》〉』】〕` and the 簡體 quotes `”’` were free
    /// to open a column, and did, thirteen times on one page of 論語集解.
    #[test]
    fn the_marks_that_may_not_open_a_line_are_the_ones_clreq_names() {
        for c in ['、', '，', '。', '：', '；', '！', '？', '」', '』', '”', '’',
                  '）', '〕', '】', '》', '〉', '·', '—', '…', '～'] {
            assert!(
                crate::wrap::forbidden_at_row_start(c),
                "{c} may not open a line (clreq §6.1.1)"
            );
        }
        for c in ['「', '『', '“', '‘', '（', '〔', '【', '《', '〈'] {
            assert!(
                crate::wrap::forbidden_at_row_end(c),
                "{c} may not end a line (clreq §6.1.1)"
            );
        }
        // …and a 漢字 is free at either end, which is the whole point.
        assert!(!crate::wrap::forbidden_at_row_start('雪'));
        assert!(!crate::wrap::forbidden_at_row_end('雪'));

        // 分離禁止: `——` and `……` are one mark of two squares.
        let text = "一二三四五六七八九十甲——乙丙";
        for len in [11usize, 12] {
            let grid = Grid { zong_len: len, ..G };
            let slots = line_slots_in(text, grid, &[]);
            let chars: Vec<char> = text.chars().collect();
            let groups = crate::ruby::groups(&chars, grid.ruby);
            let breaks = zong_breaks(&chars, &slots, len, &groups, &[], false);
            for &at in &breaks {
                if let Some(slot) = slots.get(at) {
                    let head = slot.start;
                    assert!(
                        !(chars.get(head) == Some(&'—') && chars.get(head + 1) == Some(&'—'))
                            || head == 0,
                        "a 縱 opens with half a 破折號 at {head}, zong_len {len}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_rows_of_a_line_tile_it_exactly_and_in_order() {
        const ATOMS: &[&str] = &[
            "文",
            "。",
            "、",
            "「",
            "」",
            "（",
            "）",
            "**",
            "*",
            "<ruby>永<rt>えい</rt></ruby>",
            // An empty base: not a thing anybody writes, and the shape that
            // made the rows overlap.
            "<ruby><rt>えい</rt></ruby>",
        ];
        let grids = [
            G,
            Grid { hanging: true, ..G },
            RUBY,
            Grid { hanging: true, ..RUBY },
            Grid {
                hanging: true,
                tatechuyoko: true,
                indent: 2,
                ..RUBY
            },
        ];
        let mut lines: Vec<String> = vec![String::new()];
        // Four atoms deep: the families that break need a bracket, a run of
        // markup, a mark and *another* run of markup — `「**。**` is the
        // shortest of them — and at four this is a second of testing.
        for _ in 0..4 {
            let mut next = Vec::new();
            for line in &lines {
                for atom in ATOMS {
                    next.push(format!("{line}{atom}"));
                }
            }
            lines.extend(next);
        }
        let mut broken: Vec<String> = Vec::new();
        for line in &lines {
            if line.is_empty() {
                continue;
            }
            let markup = crate::markdown::hidden(&crate::markdown::spans(line), None);
            for hidden in [Vec::new(), markup] {
                for grid in grids {
                    let slots = line_slots_in(line, grid, &hidden);
                    if !slots.windows(2).all(|w| w[0].start <= w[1].start) {
                        broken.push(format!(
                            "{line:?} hanging={} wysiwyg={}: out of order {:?}",
                            grid.hanging,
                            !hidden.is_empty(),
                            slots.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>()
                        ));
                        continue;
                    }
                    for at in 0..line.chars().count() {
                        let holding = slots.iter().filter(|s| at >= s.start && at < s.end).count();
                        if holding != 1 {
                            broken.push(format!(
                                "{line:?} hanging={} wysiwyg={}: char {at} is in {holding} rows {:?}",
                                grid.hanging,
                                !hidden.is_empty(),
                                slots.iter().map(|s| (s.start, s.end)).collect::<Vec<_>>()
                            ));
                            break;
                        }
                    }
                }
            }
        }
        assert!(
            broken.is_empty(),
            "{} of {} lines:\n{}",
            broken.len(),
            lines.len(),
            broken.iter().take(8).cloned().collect::<Vec<_>>().join("\n")
        );
    }

    /// The markup inside a ruby base comes off the page like any other.
    ///
    /// `push_ruby` was the one function never handed `hidden`: it laid the base
    /// out from the raw characters, so 縱書 drew the `**` that 橫排 hid — the
    /// same divergence the page-as-data refactor closed everywhere else.
    #[test]
    fn markup_inside_a_ruby_base_comes_off_the_page() {
        let line = "<ruby>**永和**<rt>えいわ</rt></ruby>九年。";
        let hidden = crate::markdown::hidden(&crate::markdown::spans(line), None);
        let drawn: String = line_slots_in(line, RUBY, &hidden)
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        // 。 is drawn in its vertical form; the asterisks are drawn not at all.
        assert_eq!(drawn, "永和九年︒", "the asterisks are markup");
    }

    #[test]
    fn markup_at_the_end_of_a_line_joins_the_slot_before_it() {
        let hidden = |line: &str| crate::markdown::hidden(&crate::markdown::spans(line), None);
        let slots = line_slots_in("那年**冬**", G, &hidden("那年**冬**"));
        let texts: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["那", "年", "冬"]);
        // The last slot reaches the end of the line, so the caret past it sits
        // past the whole thing rather than between two asterisks.
        assert_eq!(slots.last().unwrap().end, 7);
    }

    #[test]
    fn a_bracket_waiting_for_a_ruby_group_hangs_on_its_base() {
        // The bracket introduces the character the group annotates, and that
        // character is inside the group. Losing the wait at the group's edge
        // left the bracket as a row of its own — the square hanging exists to
        // save.
        let grid = Grid::plain(8, Dialects::only(crate::ruby::Dialect::Html)).with_hanging(true);
        let slots = line_slots_plain("曰「<ruby>漢<rt>hàn</rt></ruby>字", grid);
        let texts: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        let marks: Vec<Option<char>> = slots.iter().map(|s| s.mark).collect();
        // Three rows of reading above, then 漢 carrying the bracket, then 字.
        assert_eq!(texts, ["曰", "", "", "", "漢", "字"]);
        assert_eq!(marks[4], Some('｢'), "「 hangs on the base it introduces");
        assert!(marks[1..4].iter().all(Option::is_none));
        assert!(
            !texts[1..4].iter().any(|t| !t.is_empty()),
            "and it costs no text row"
        );
    }

    #[test]
    fn marks_hang_in_their_narrow_forms() {
        // The margin is one cell. A full-width mark in it spills onto the 縱 to
        // the right; a narrow one is what the margin was sized for.
        let grid = Grid::plain(8, Dialects::NONE).with_hanging(true);
        let slots = line_slots_plain("春。夏、秋「冬」", grid);
        let marks: Vec<Option<char>> = slots.iter().map(|s| s.mark).collect();
        // 秋 carries nothing: the 「 after it waits for 冬, which it introduces.
        // ⚠️ The closing 」 finds 冬's margin already taken, and **keeps a
        // square of its own** rather than a margin row with a blank square
        // beside it (#231) — a row is a row either way, and one of the two
        // leaves a hole in the column.
        assert_eq!(marks, [Some('｡'), Some('､'), None, Some('｢'), None]);
        assert_eq!(slots[4].text, "﹂", "the bracket is in the square, not the margin");
        for mark in marks.into_iter().flatten() {
            assert_eq!(yumete_cjk::char_width(mark), 1, "{mark} must be one cell");
        }

        // A mark with no narrow form keeps its square rather than making the
        // margin two cells wide for every 縱 on the page.
        let slots = line_slots_plain("讀《詩》", grid);
        let texts: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["讀", "︽", "詩", "︾"]);
        assert!(slots.iter().all(|s| s.mark.is_none()));
    }

    #[test]
    fn two_marks_running_share_a_square_instead_of_hanging() {
        // JLREQ §3.1.4① and clreq §6.3.2.2: a 句點 followed by a closing
        // bracket is set solid, and since each is a half-em glyph the pair
        // fills exactly one em — one terminal square, one cell each. clreq
        // §6.1.3 says it from the other side: 連續標點不作懸掛.
        //
        // Before this, the second mark took a margin row of its own and the
        // text square beside it was left empty — a hole in the middle of the
        // column, at the end of almost every line of Chinese dialogue.
        let slots = line_slots_plain("春。」", G.with_hanging(true));
        assert_eq!(slots.len(), 2, "a row for the pair, not one each");
        assert_eq!(slots[0].text, "春");
        assert_eq!(slots[0].mark, None, "the 。 came back out of the margin");
        assert_eq!(slots[1].text, "｡｣", "both marks, one square, one cell each");
        assert_eq!(slots[1].mark, None);
        assert_eq!(yumete_cjk::str_width(&slots[1].text), 2, "a full square");
        // The square covers both characters, so the cursor crosses it in one
        // step and 、`d` takes the pair.
        assert_eq!(slots[1].start + 2, slots[1].end);

        // One mark still hangs — that is where the space is actually saved.
        let slots = line_slots_plain("春。夏", G.with_hanging(true));
        assert_eq!(slots[0].mark, Some('｡'));

        // …and a closing bracket after a base that is carrying its own opener
        // is not a cluster: 「 belongs *before* 冬, and pulling it out would
        // put it after the character it opens.
        let slots = line_slots_plain("秋「冬」", G.with_hanging(true));
        let marks: Vec<Option<char>> = slots.iter().map(|s| s.mark).collect();
        // …and the closing bracket keeps a square rather than a hole (#231).
        assert_eq!(marks, [None, Some('｢'), None]);
        assert_eq!(slots[1].text, "冬");
        assert_eq!(slots[2].text, "﹂");
    }

    /// **A mark that cannot hang keeps its own square, never an empty one**
    /// — Feature #231.
    ///
    /// The hole this closes is not rare. A base already carrying an opener has
    /// no margin left to give, so every mark after it used to take a margin
    /// row with the text square beside it **blank**: 秋「冬」」 made two of
    /// those and 春（。）」 made four. A row is a row either way — the margin
    /// is beside the square, not instead of it — so putting the glyph in the
    /// square costs nothing and leaves nothing blank.
    #[test]
    fn a_mark_that_cannot_hang_takes_a_square_rather_than_leaving_a_hole() {
        for line in [
            "秋「冬」」",
            "春（。）」",
            "「春」。」",
            "（（春",
            "春。」」」",
            "春？」」",
        ] {
            let slots = line_slots_plain(line, G.with_hanging(true));
            assert!(
                slots.iter().all(|s| !s.text.is_empty()),
                "{line}: a blank square in the column: {slots:?}"
            );
        }
        // What each of them actually comes to, so a change here has to be
        // meant rather than merely allowed.
        let shape = |line: &str| -> Vec<(String, Option<char>)> {
            line_slots_plain(line, G.with_hanging(true))
                .iter()
                .map(|s| (s.text.clone(), s.mark))
                .collect()
        };
        assert_eq!(
            shape("秋「冬」」"),
            vec![
                ("秋".to_string(), None),
                ("冬".to_string(), Some('｢')),
                ("﹂".to_string(), Some('｣')),
            ]
        );
        // ⚠️ An opener that never reached a base is still a mark.
        assert_eq!(
            shape("（（春"),
            vec![("︵".to_string(), None), ("春".to_string(), Some('('))]
        );
    }

    /// **Only 。 and 、 squeeze; ？ ！ ， do not** — Feature #230.
    ///
    /// clreq §6.3.2 separates the 問號／嘆號 from the 句號 group, and the
    /// reason carries straight into a terminal: 。 and 、 are half-em glyphs
    /// whose right half is blank, and Unicode gives them a **true** narrow
    /// form (`｡` `､`). ？ and ！ and ， fill their em, and the only narrow
    /// twin they have is the **ASCII** mark — so squeezing them wrote `?」`
    /// and `,」` into a manuscript set in Chinese.
    #[test]
    fn only_the_marks_with_a_true_narrow_form_share_a_square() {
        for (line, want) in [
            ("春。」", vec!["春", "｡｣"]),
            ("春、」", vec!["春", "､｣"]),
        ] {
            let slots = line_slots_plain(line, G.with_hanging(true));
            let bodies: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
            assert_eq!(bodies, want, "{line}");
        }
        // The other three: neither mark hangs, each takes a square, and the
        // glyphs are the **vertical** forms — not an ASCII twin in sight.
        for (line, second) in [("春？」", '︖'), ("春！」", '︕'), ("春，」", '︐')] {
            let slots = line_slots_plain(line, G.with_hanging(true));
            assert_eq!(slots.len(), 3, "{line}: a square each");
            assert_eq!(slots[0].text, "春");
            assert_eq!(slots[0].mark, None, "{line}: it came back out of the margin");
            assert_eq!(slots[1].text, second.to_string(), "{line}");
            assert_eq!(slots[2].text, "﹂", "{line}");
            assert!(
                slots.iter().all(|s| s.mark.is_none()),
                "{line}: 連續標點不作懸掛"
            );
            assert!(
                !slots.iter().any(|s| s.text.chars().any(|c| c.is_ascii_punctuation())),
                "{line}: no ASCII mark in a Chinese manuscript: {slots:?}"
            );
        }
        // ⚠️ And one of them alone still hangs — that is where the space is
        // saved, and a lone mark in a half-cell margin is what the ASCII twin
        // is *for*.
        let slots = line_slots_plain("春？夏", G.with_hanging(true));
        assert_eq!(slots[0].mark, Some('?'));
    }

    /// The reading gives way upward, leaving the base's own row for a mark.
    #[test]
    fn a_reading_moves_above_its_base_when_marks_hang() {
        let slots = line_slots_plain("<ruby>漢<rt>hàn</rt></ruby>。", RUBY.with_hanging(true));
        let bodies: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(bodies, ["", "", "", "漢"], "three rows of space, then 漢");
        assert_eq!(
            slots.iter().map(|s| s.ruby).collect::<Vec<_>>(),
            [Some('h'), Some('à'), Some('n'), None],
            "the reading is wholly above"
        );
        assert_eq!(slots[3].mark, Some('｡'), "and the mark has the base's row");
    }

    #[test]
    fn the_blank_line_an_indent_replaces_is_not_a_zong() {
        // 縱書 pays more for it than 橫排 does: a blank line is a whole empty
        // column down the page, in the one layout where columns are the page.
        let rope = Rope::from_str("第一段\n\n第二段\n\n\n第三段\n");
        let plain = Grid { indent: 2, ..G };
        // **The page decides, and it decides once** — this is the editor's own
        // rule (`Editor::line_is_folded`), which the horizontal page asks too.
        // The 縱書 side used to keep a second copy of it and the two disagreed.
        let folds = |line: usize| line == 1;
        let folded_grid = plain.with_folds(&folds);
        assert!(!folded(&rope, 1, plain), "…unless the page says to fold");
        assert!(folded(&rope, 1, folded_grid), "the one between two paragraphs");
        assert!(!folded(&rope, 3, folded_grid), "two blanks are a scene break");
        assert!(!folded(&rope, 4, folded_grid));
        // …and the page skips it: the 縱 run 0, 2, … with no column for line
        // 1, while the scene break keeps both of its.
        let page = zongs_from(&rope, Anchor::default(), folded_grid, 8);
        let lines: Vec<usize> = page.iter().map(|z| z.line).collect();
        assert!(!lines.contains(&1), "{lines:?}");
        assert_eq!(lines, [0, 2, 3, 4, 5, 6], "{page:?}");
    }

    #[test]
    fn a_variation_sequence_fills_one_slot() {
        let r = rope("葛\u{E0100}城");
        let zongs = layout(&r, G);
        assert_eq!(zongs[0].slots, 2);
        assert_eq!(position(&r, 2, G).slot, 1);
    }
    /// Feature #210: text on the page the file has no bytes for.
    ///
    /// Down the column it takes rows of its own — a candidate is as much on
    /// the page as anything else — while standing for no characters, so the
    /// cursor steps past it exactly as it steps past the indent's padding.
    #[test]
    fn drawn_text_stands_in_rows_of_its_own() {
        let r = rope("春夏秋冬\n");
        let runs = |_: usize| vec![crate::drawn::Run::new(2, "候", crate::drawn::Ink::Typed)];
        let grid = G.with_drawn(&runs);
        let slots = line_grid_in(&r, 0, grid, &[]);
        assert_eq!(slots.len(), 5, "four 字 and the candidate");
        assert_eq!(slots[2].text, "候");
        assert!(slots[2].is_drawn());
        assert_eq!((slots[2].start, slots[2].end), (2, 2), "stands for no char");
        // …and nothing else on the 縱 is drawn, least of all the character it
        // stands before.
        assert_eq!(slots[3].text, "秋");
        assert!(!slots[3].is_drawn());
    }

    #[test]
    fn a_multi_character_run_takes_a_row_each() {
        let r = rope("春夏\n");
        let runs = |_: usize| vec![crate::drawn::Run::new(1, "候補", crate::drawn::Ink::Typed)];
        let slots = line_grid_in(&r, 0, G.with_drawn(&runs), &[]);
        let drawn: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(drawn, vec!["春", "候", "補", "夏"]);
    }

    #[test]
    fn the_cursor_steps_past_a_drawn_row() {
        // The candidate is drawn between 夏 and 秋, so 秋 is one row further
        // down the column — but the cursor on it is still on 秋.
        let r = rope("春夏秋冬\n");
        let runs = |_: usize| vec![crate::drawn::Run::new(2, "候", crate::drawn::Ink::Typed)];
        let grid = G.with_drawn(&runs);
        assert_eq!(position(&r, 1, grid).slot, 1, "夏, before the candidate");
        assert_eq!(position(&r, 2, grid).slot, 3, "秋, one row past it");
        assert_eq!(char_at(&r, 0, 0, 3, grid), 2, "and that row is 秋's");
    }

    #[test]
    fn a_candidate_at_the_head_of_a_paragraph_stands_after_the_indent() {
        // 首行縮進 is two empty squares, and a candidate typed at the head of
        // the paragraph belongs after them: the indent is the shape of the
        // paragraph, not something the candidate was typed in front of.
        let r = rope("春夏\n");
        let runs = |_: usize| vec![crate::drawn::Run::new(0, "候", crate::drawn::Ink::Typed)];
        let grid = Grid {
            indent: 2,
            ..G.with_drawn(&runs)
        };
        let slots = line_grid_in(&r, 0, grid, &[]);
        let drawn: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(drawn, vec!["", "", "候", "春", "夏"]);
        assert!(!slots[1].is_drawn(), "padding draws nothing, so it is not");
    }

    #[test]
    fn a_line_is_laid_out_again_when_only_the_candidate_changed() {
        // The layout memo is keyed on the buffer's revision, and a candidate
        // moves while the buffer does not move at all. Keyed without it, the
        // 縱 would keep being drawn with the candidate before last.
        let r = rope("春夏\n");
        let stamped = Grid { stamp: 7, ..G };
        let one = |_: usize| vec![crate::drawn::Run::new(1, "候", crate::drawn::Ink::Typed)];
        let two = |_: usize| vec![crate::drawn::Run::new(1, "補", crate::drawn::Ink::Typed)];
        let drawn = |grid: Grid| -> Vec<String> {
            line_grid_in(&r, 0, grid, &[])
                .iter()
                .map(|s| s.text.clone())
                .collect()
        };
        assert_eq!(drawn(stamped.with_drawn(&one)), ["春", "候", "夏"]);
        assert_eq!(drawn(stamped.with_drawn(&two)), ["春", "補", "夏"]);
    }
}

