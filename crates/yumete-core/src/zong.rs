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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    /// Whether Markdown's markup comes off the page (所見即所得, Feature #104).
    ///
    /// A hidden run joins the slot beside it rather than taking one of its own,
    /// so the cursor steps over `**` in one press and the wrap length counts
    /// writing rather than asterisks — the same thing a ruby group's tags have
    /// always done.
    pub hide_markup: bool,
    /// What the selection covers, as char indices in the buffer.
    ///
    /// Every construct it touches is shown whole, so this is part of the grid:
    /// it changes which characters occupy a slot, and everything that asks the
    /// grid a question has to be asking about the same page.
    pub selection: Option<(usize, usize)>,
    /// Graphemes per 縱.
    pub zong_len: usize,
    /// Which ruby dialects are laid out as readings. Empty shows the markup as
    /// the text it is.
    pub ruby: Dialects,
    /// Whether 句讀 hang in the margin rather than taking a square each
    /// (標點旁置).
    pub hanging: bool,
    /// Whether a pair of half-width characters shares one slot (縦中横).
    ///
    /// Off by default. Turned sideways a pair reads as a syllable — `yume` set
    /// as `yu` over `me` invites the eye to read two of them — and one character
    /// to a row, hung right, is what a reader of vertical text expects. It stays
    /// available because a two-digit year genuinely does read better packed.
    pub tatechuyoko: bool,
}

impl Grid {
    /// Take the markup off the page, showing whole whatever `selection` (char
    /// indices in the buffer) touches.
    pub fn with_markup_hidden(self, on: bool, selection: Option<(usize, usize)>) -> Grid {
        Grid {
            hide_markup: on,
            selection,
            ..self
        }
    }

    pub fn new(zong_len: usize, ruby: Dialects) -> Grid {
        Grid {
            zong_len: zong_len.max(1),
            ruby,
            hanging: false,
            tatechuyoko: false,
            hide_markup: false,
            selection: None,
        }
    }

    /// The same grid, hanging 句讀 in the margin.
    pub fn with_hanging(self, on: bool) -> Grid {
        Grid {
            hanging: on,
            ..self
        }
    }

    /// The same grid, packing half-width pairs into one slot.
    pub fn with_tatechuyoko(self, on: bool) -> Grid {
        Grid {
            tatechuyoko: on,
            ..self
        }
    }

    /// The same grid at a different wrap length — what the renderer does once
    /// the terminal's height is known.
    pub fn with_zong_len(self, zong_len: usize) -> Grid {
        Grid {
            zong_len: zong_len.max(1),
            ..self
        }
    }
}

impl Default for Grid {
    fn default() -> Grid {
        Grid::new(
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
}

/// Split a line into the rows a 縱 draws it as.
///
/// With `ruby` off this is just the slot run: one grapheme per row, half-width
/// alphanumerics paired 縦中横. With it on, a `<ruby>` group is *laid out*: the
/// markup disappears, the reading is dealt out down the ruby column, and the
/// base is centred over however many rows the reading needs. That spacing is
/// what real typesetting does and is why two adjacent readings never collide.
pub fn line_slots(text: &str, grid: Grid) -> Vec<Slot> {
    line_slots_in(text, grid, None)
}

/// [`line_slots`] with the part of this line the selection covers, as columns —
/// which is what decides which constructs are shown whole.
pub fn line_slots_in(text: &str, grid: Grid, selected: Option<(usize, usize)>) -> Vec<Slot> {
    let chars: Vec<char> = text.chars().collect();
    let groups = crate::ruby::groups(&chars, grid.ruby);
    let mut slots = Vec::new();
    let mut at = 0usize;
    // An opening bracket waits for the character it introduces, and that
    // character may be inside the next ruby group — 「<ruby>漢…. The wait has to
    // outlive the plain run, or the bracket is left behind as a row of its own
    // and the reader loses the very square hanging it was meant to save.
    let mut opening = None;
    // Which characters are markup rather than writing. A hidden run takes no
    // slot of its own; it joins the slot beside it, so the cursor steps over
    // `**` in one press and the wrap length counts writing.
    let hidden = if grid.hide_markup {
        crate::markdown::hidden(&crate::markdown::spans(text), selected)
    } else {
        Vec::new()
    };
    for group in &groups {
        push_plain(
            &mut slots,
            &chars,
            at,
            group.start,
            grid,
            &mut opening,
            &hidden,
        );
        push_ruby(&mut slots, &chars, group, grid, &mut opening);
        at = group.end;
    }
    push_plain(
        &mut slots,
        &chars,
        at,
        chars.len(),
        grid,
        &mut opening,
        &hidden,
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
        });
    }
    slots
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
        if grid.hanging && body.chars().count() == 1 && swallowed.is_none() {
            let mark = body.chars().next().expect("one character");
            // The half-width form is what hangs, and a mark that has none does
            // not hang at all — it keeps its square. So this is one question,
            // not two, and the two can no longer disagree.
            if let Some(hung) = yumete_cjk::margin_form(mark) {
                if yumete_cjk::opens_a_pair(mark) {
                    // A second opener while one is already waiting — `（「` —
                    // must not fall through to the branch below, which hangs a
                    // mark on the character *before* it: the one side an opener
                    // never belongs on. The one already waiting takes a margin
                    // row of its own, above the character, and the new one waits
                    // in its place, so they read down the margin in the order
                    // they were written.
                    if let Some((opened_at, earlier)) = opening.take() {
                        slots.push(Slot {
                            start: opened_at,
                            end: at,
                            text: String::new(),
                            ruby: None,
                            mark: Some(earlier),
                        });
                    }
                    *opening = Some((at, hung));
                    continue;
                }
                match slots.last_mut() {
                    // The usual case: it joins the character it follows.
                    Some(previous) if previous.mark.is_none() && !previous.text.is_empty() => {
                        previous.end = from + w[1];
                        previous.mark = Some(hung);
                        continue;
                    }
                    // A second mark running — 「？」」 ends a quoted question,
                    // and it is common. It takes a row of its own, but stays in
                    // the *margin*: the text column keeps only text, which is
                    // the whole point of hanging them.
                    Some(_) => {
                        slots.push(Slot {
                            start: at,
                            end: from + w[1],
                            text: String::new(),
                            ruby: None,
                            mark: Some(hung),
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
        });
    }

    // A hidden run still pending when the run ends — the markup before a ruby
    // group, say — has no slot after it to join, so it joins the one before.
    // Left dropped, its characters would belong to no slot at all, and the
    // cursor could be put on one of them.
    if let Some(at) = swallowed {
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
) {
    let base = group.base_text(chars);
    let reading: Vec<char> = group.reading_text(chars).to_vec();
    // The base's own rows, then as many more as the reading needs.
    let base_rows: Vec<(usize, usize)> = {
        let text: String = base.iter().collect();
        slot_offsets(&text, grid.tatechuyoko)
            .windows(2)
            .map(|w| (group.base.0 + w[0], group.base.0 + w[1]))
            .collect()
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

    let base_end = base_rows.last().map_or(group.base.0, |&(_, b)| b);
    for row in 0..rows {
        let (start, end, body) = match row.checked_sub(top).and_then(|i| base_rows.get(i)) {
            Some(&(a, b)) => (a, b, chars[a..b].iter().collect::<String>()),
            // A padding row stands for no characters of its own. It still has to
            // sit in document order — above the base it reports the group's
            // start, below it the base's end — or the rows stop being sorted and
            // nothing can look the cursor up in them.
            None if row < top => (group.start, group.start, String::new()),
            None => (base_end, base_end, String::new()),
        };
        slots.push(Slot {
            start,
            end,
            text: rotate(&body),
            ruby: reading.get(row).copied(),
            mark: None,
        });
    }
    // The very first row owns the whole group's markup, so a cursor stepping
    // over it steps over the tags too rather than into them.
    if let Some(first) = slots.len().checked_sub(rows).and_then(|i| slots.get_mut(i)) {
        first.start = group.start;
    }
    // A bracket that was waiting for the character this group annotates hangs
    // against the base's *first* row — the row the reader sees the base on —
    // rather than being left behind as a row of its own before the group.
    if let Some((opened_at, mark)) = opening.take() {
        let base_row = slots.len().checked_sub(rows.saturating_sub(top));
        match base_row.and_then(|i| slots.get_mut(i)) {
            Some(slot) if slot.mark.is_none() => {
                slot.start = opened_at.min(slot.start);
                slot.mark = Some(mark);
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
    let mut offsets = Vec::with_capacity(text.len() / 3 + 1);
    let mut chars = 0usize;
    let mut run = 0usize;
    for g in graphemes(text) {
        if !tatechuyoko {
            offsets.push(chars);
            chars += g.chars().count();
            continue;
        }
        // A run of half-width *alphanumerics* fills the slot it started, up to
        // the limit; anything else — full-width, punctuation, a space — opens a
        // new one. 縦中横 is for numbers and short Latin, and packing a comma in
        // beside a letter would only look like a mistake.
        let narrow = grapheme_width(g) == 1 && g.chars().all(char::is_alphanumeric);
        if narrow && run > 0 && run < TATECHUYOKO {
            run += 1;
        } else {
            offsets.push(chars);
            run = if narrow { 1 } else { 0 };
        }
        chars += g.chars().count();
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
fn zong_breaks(chars: &[char], slots: &[Slot], zong_len: usize) -> Vec<usize> {
    let total = slots.len();
    let char_of = |i: usize| {
        slots
            .get(i)
            .and_then(|s: &Slot| chars.get(s.start))
            .copied()
            .unwrap_or(' ')
    };
    let mut breaks = vec![0usize];
    let mut at = 0;
    while at + zong_len < total {
        let mut cut = at + zong_len;
        for _ in 0..crate::wrap::MAX_KINSOKU_RETREAT {
            if cut <= at + 1 {
                break;
            }
            if crate::wrap::forbidden_at_row_start(char_of(cut))
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

/// One line's slots and where its 縱 begin — always asked for together.
fn line_zongs(rope: &Rope, line: usize, grid: Grid) -> (Vec<Slot>, Vec<usize>) {
    let slots = line_grid(rope, line, grid);
    let chars: Vec<char> = line_text(rope, line).chars().collect();
    let breaks = zong_breaks(&chars, &slots, grid.zong_len.max(1));
    (slots, breaks)
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
fn line_grid(rope: &Rope, line: usize, grid: Grid) -> Vec<Slot> {
    // Where the selection falls on *this* line is what decides which constructs
    // are shown whole, so it is worked out here, where the rope is.
    let text = line_text(rope, line);
    let selected = grid.selection.and_then(|(from, to)| {
        let start = rope.line_to_char(line);
        let end = start + text.chars().count();
        (to >= start && from <= end).then(|| (from.max(start) - start, to.min(end) - start))
    });
    line_slots_in(&text, grid, selected)
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
    } else if p.line + 1 < line_count(rope) {
        (p.line + 1, 0)
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
        let line = p.line - 1;
        (line, zong_count_in_line(rope, line, grid) - 1)
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
        line -= 1;
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
            line += 1;
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
    let (mut slots, breaks) = line_zongs(rope, zong.line, grid);
    let (first, last) = zong_span(&breaks, slots.len(), zong.index_in_line);
    if first >= last {
        return Vec::new();
    }
    slots.drain(..first);
    slots.truncate(last - first);
    slots
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
pub fn slot_text(text: &str, grid: Grid) -> Vec<String> {
    let offsets = slot_offsets(text, grid.tatechuyoko);
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
        zong_len: 32,
        ruby: Dialects::NONE,
        hanging: false,
        tatechuyoko: false,
        hide_markup: false,
        selection: None,
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
            let total = line_slots(rope.line(0).to_string().trim_end(), grid).len();
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
        let slots = line_slots("他<ruby>口<rt>kǒu</rt></ruby>很", RUBY);
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

    #[test]
    fn two_adjacent_readings_do_not_collide() {
        let slots = line_slots(
            "<ruby>口<rt>kǒu</rt></ruby><ruby>囗<rt>wéi</rt></ruby>",
            RUBY,
        );
        assert_eq!(slots.len(), 6, "three rows each");
        let readings: String = slots.iter().filter_map(|s| s.ruby).collect();
        assert_eq!(readings, "kǒuwéi");
    }

    #[test]
    fn a_reading_shorter_than_its_base_does_not_shrink_it() {
        let slots = line_slots("<ruby>漢字<rt>hz</rt></ruby>", RUBY);
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
        let slots = line_slots("<ruby>口<rt>kǒu</rt></ruby>", G);
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
        assert_eq!(slot_text("第12章", PACKED), ["第", "12", "章"]);

        // The cursor agrees: the character after the pair is slot 2, not 3.
        assert_eq!(position(&r, 1, PACKED).slot, 1, "on the 1");
        assert_eq!(position(&r, 2, PACKED).slot, 1, "still inside the pair");
        assert_eq!(position(&r, 3, PACKED).slot, 2, "on 章");
    }

    #[test]
    fn packing_is_off_by_default() {
        // One letter to a row: turned sideways a pair reads as a syllable that
        // is not there.
        assert_eq!(slot_text("yume", G), ["y", "u", "m", "e"]);
        assert_eq!(slot_text("第12章", G), ["第", "1", "2", "章"]);
    }

    #[test]
    fn a_longer_latin_run_packs_two_at_a_time() {
        // Beyond a pair there is nothing to rotate into, so it stacks — legibly,
        // but it is the one thing a terminal cannot set properly.
        assert_eq!(slot_text("abcde", PACKED), ["ab", "cd", "e"]);
        assert_eq!(slot_text("2026年", PACKED), ["20", "26", "年"]);
    }

    /// A 句讀 mark stops being a row of its own and hangs beside the character
    /// it follows, so the text runs unbroken down the 縱.
    #[test]
    fn a_mark_hangs_beside_the_character_it_follows() {
        let hanging = G.with_hanging(true);
        let slots = line_slots("春江。潮水，", hanging);
        let bodies: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(bodies, ["春", "江", "潮", "水"], "four rows, not six");
        assert_eq!(
            slots.iter().map(|s| s.mark).collect::<Vec<_>>(),
            [None, Some('｡'), None, Some(',')],
            "and the marks are rotated, in the margin"
        );

        // Off, they take a square each, as they did.
        assert_eq!(line_slots("春江。", G).len(), 3);
    }

    /// An opening bracket introduces what follows it, so it hangs beside *that*
    /// character — which is also what keeps the text column unbroken.
    #[test]
    fn an_opener_hangs_on_the_character_it_introduces() {
        let slots = line_slots("曰「春江", G.with_hanging(true));
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
        let grid = G.with_markup_hidden(true, None);
        let slots = line_slots_in("那**年**冬", grid, None);
        let texts: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["那", "年", "冬"]);
        // …and the ranges cover every character of the line between them, so
        // there is nowhere to step that is not on the screen. Hidden markup
        // joins the slot *after* it, which is why 年 carries the opening `**`
        // and 冬 carries the closing one.
        assert_eq!((slots[0].start, slots[0].end), (0, 1), "那");
        assert_eq!((slots[1].start, slots[1].end), (1, 4), "**年");
        assert_eq!((slots[2].start, slots[2].end), (4, 7), "**冬");

        // The construct the cursor is in is shown whole.
        let open = G.with_markup_hidden(true, Some((4, 4)));
        let texts: Vec<String> = line_slots_in("那**年**冬", open, Some((4, 4)))
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
        let grid = G.with_markup_hidden(true, Some((4, 4)));
        let texts: Vec<String> = zong_slots(
            &rope,
            &zongs_from(&rope, Anchor::default(), grid, 1)[0],
            grid,
        )
        .into_iter()
        .map(|s| s.text)
        .collect();
        assert_eq!(texts, ["那", "*", "*", "年", "*", "*", "冬"]);

        // With the cursor elsewhere on the line it comes off again.
        let grid = G.with_markup_hidden(true, Some((0, 0)));
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
        let grid = RUBY.with_markup_hidden(true, None);
        for line in [
            "那**年**<ruby>漢<rt>h</rt></ruby>冬",
            "**年**<ruby>漢<rt>h</rt></ruby>",
            "%%整行都是批注%%",
            "`碼`",
            "[](x)",
            "那**年",
        ] {
            let slots = line_slots_in(line, grid, None);
            let n = line.chars().count();
            for at in 0..n {
                assert!(
                    slots.iter().any(|s| at >= s.start && at < s.end),
                    "char {at} of {line:?} is in no slot"
                );
            }
            assert!(!slots.is_empty(), "{line:?} has nowhere to put the cursor");
        }
    }

    #[test]
    fn markup_at_the_end_of_a_line_joins_the_slot_before_it() {
        let grid = G.with_markup_hidden(true, None);
        let slots = line_slots_in("那年**冬**", grid, None);
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
        let grid = Grid::new(8, Dialects::only(crate::ruby::Dialect::Html)).with_hanging(true);
        let slots = line_slots("曰「<ruby>漢<rt>hàn</rt></ruby>字", grid);
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
        let grid = Grid::new(8, Dialects::NONE).with_hanging(true);
        let slots = line_slots("春。夏、秋「冬」", grid);
        let marks: Vec<Option<char>> = slots.iter().map(|s| s.mark).collect();
        // 秋 carries nothing: the 「 after it waits for 冬, which it introduces.
        // The closing 」 finds 冬's margin already taken and gets a margin row
        // of its own — one cell holds one mark.
        assert_eq!(marks, [Some('｡'), Some('､'), None, Some('｢'), Some('｣')]);
        for mark in marks.into_iter().flatten() {
            assert_eq!(yumete_cjk::char_width(mark), 1, "{mark} must be one cell");
        }

        // A mark with no narrow form keeps its square rather than making the
        // margin two cells wide for every 縱 on the page.
        let slots = line_slots("讀《詩》", grid);
        let texts: Vec<&str> = slots.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(texts, ["讀", "︽", "詩", "︾"]);
        assert!(slots.iter().all(|s| s.mark.is_none()));
    }

    #[test]
    fn a_second_mark_running_stays_in_the_margin() {
        let slots = line_slots("春。」", G.with_hanging(true));
        assert_eq!(slots.len(), 2, "a row for the pair, not one each");
        assert_eq!(slots[0].text, "春");
        assert_eq!(slots[0].mark, Some('｡'));
        // The second takes a row, but in the margin: the text column stays text.
        assert_eq!(slots[1].text, "", "nothing in the text column");
        assert_eq!(slots[1].mark, Some('｣'));
    }

    /// The reading gives way upward, leaving the base's own row for a mark.
    #[test]
    fn a_reading_moves_above_its_base_when_marks_hang() {
        let slots = line_slots("<ruby>漢<rt>hàn</rt></ruby>。", RUBY.with_hanging(true));
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
    fn a_variation_sequence_fills_one_slot() {
        let r = rope("葛\u{E0100}城");
        let zongs = layout(&r, G);
        assert_eq!(zongs[0].slots, 2);
        assert_eq!(position(&r, 2, G).slot, 1);
    }
}
