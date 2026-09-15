//! Vertical-layout rendering — Feature #61.
//!
//! The terminal grid turns out to suit vertical CJK typesetting almost exactly.
//! A cell is about 1:2, so a full-width character — two cells wide, one row
//! tall — is square; stack those downward and you have a 縱, put the next 縱 to
//! its *left*, and the page reads the way a novel does.
//!
//! What the terminal will not do for you is the punctuation. It applies no
//! OpenType `vert` feature, so `。` and `「` are substituted by codepoint on the
//! way to the screen (see [`yumete_cjk::vertical_form`]); the buffer keeps the
//! ordinary characters. Nor will it rotate Latin text: a Latin run stacks letter
//! by letter, which is legible but is the one place where a real typesetter
//! still wins.
//!
//! A **縱** (*zong*) is one such run of text — what a line is in horizontal
//! layout, named separately because "line" and "column" are both ambiguous here.
//! Unlike the horizontal view, this one draws straight into the frame buffer
//! rather than through `Paragraph`: the unit of layout is a slot on a
//! two-dimensional grid, not a line of spans.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};
use ratatui::Frame;

use yumete_cjk::{graphemes, str_width, WordMark};
use yumete_config::{Config, LineNumbers};
use yumete_core::zong::{self, Anchor, IndentHint};
use yumete_core::{Editor, Mode, Rope, TextStore};
use yumete_ime::ImeSession;

/// The width of one 縱 in cells. A full-width character is two cells, and the
/// grid is built around that, not around any particular character's width.
const SLOT_WIDTH: u16 = 2;

/// 着重號 — the mark Chinese typesetting puts beside an emphasised character
/// (Feature #236).
///
/// A 漢字 cannot lean and does not want to: the Chinese setting of `<em>` is a
/// dot beside every character of the run, which 縱書 puts in the margin to the
/// right — the column this page already draws readings and hung 句讀 in. Which
/// is why `*字*` **is** the markup for it rather than something new: 着重號 is
/// what emphasis *means* here.
///
/// It sits in the margin the way a hung mark does, and the layout buys that
/// margin for it, the way it buys one for a reading. **How wide is asked, not
/// assumed**: `·` is East-Asian *ambiguous* — one cell in a Latin font, two in
/// a CJK one — so [`Metrics::ruby_cell`] measures it with
/// [`yumete_cjk::char_width`], the same call the readings go through. Reserving
/// one cell for a glyph the terminal draws in two walks the whole row a column
/// left. See [`Margin`].
const EMPHASIS: char = '·';

/// 平仄 in the margin (Feature #247), in the 詞譜's own notation.
///
/// Hollow is 平 and solid is 仄, which is how every 詞譜 in print draws it; the
/// triangles are the same two at a 韻腳 — the last character of a 句, where a
/// rhyme falls. Four glyphs of one width, so a column of them reads as a
/// pattern rather than as a sentence.
const PING: char = '○';
const ZE: char = '●';
const PING_RHYME: char = '△';
const ZE_RHYME: char = '▲';

/// Which of the four a mark is drawn as.
pub(crate) fn meter_glyph(mark: yumete_core::meter::Mark) -> char {
    use yumete_core::meter::Level;
    match (mark.level, mark.rhyme) {
        (Level::Ping, false) => PING,
        (Level::Ze, false) => ZE,
        (Level::Ping, true) => PING_RHYME,
        (Level::Ze, true) => ZE_RHYME,
    }
}

/// What the cell to the right of one 縱 has to hold — which is what decides how
/// far the next 縱 sits from it.
///
/// Both of these are read off the 縱's own text, unlike the gap and the 稿紙
/// rule, which the page has whether or not anything wants them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Margin {
    /// A reading or a hung 句讀 mark: `ruby_width` cells.
    reading: bool,
    /// A 着重號: as many cells as the terminal draws `·` in.
    dot: bool,
    /// A 平仄 mark: as many cells as the terminal draws `○` in.
    ///
    /// Asked of the *page* rather than of the line — `:view-meter` is on or it is
    /// not — so that the 縱 do not change width as a poem scrolls past a line
    /// with no 漢字 on it.
    tone: bool,
}

/// The screen geometry of a vertically laid-out page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Metrics {
    /// Graphemes per 縱 — the wrap length actually in force, which is the
    /// configured one unless the terminal is too short for it.
    pub zong_len: usize,
    /// Cells from the left edge of one 縱 to the left edge of the next.
    pub pitch: u16,
    /// Rows reserved above the text for paragraph numbers.
    pub head_rows: u16,
    /// Cells between one 縱 and the next, from the config.
    pub gap: u16,
    /// Whether readings are being laid out at all.
    pub ruby: bool,
    /// Whether 句讀 hang in the margin, which needs the same column.
    pub hanging: bool,
    /// How many cells the reading margin takes: one for pinyin, two once any
    /// reading on the page is full-width. Uniform across the page, because a
    /// margin that changed width from 縱 to 縱 would not be a margin.
    pub ruby_width: u16,
    /// Whether the page is ruled like 稿紙, which needs a margin of its own on
    /// every 縱 — including the rightmost, which otherwise sits flush against
    /// the edge and would have nowhere to put a tick.
    pub ticks: bool,
    /// How many bands the page is divided into (段組).
    ///
    /// Japanese vertical typesetting halves a tall page and uses the width
    /// instead: a 縱 of fifty characters is tiring to read, and the traditional
    /// answer is two bands of twenty-five, read top-right to top-left and then
    /// bottom-right to bottom-left. A terminal is a wide, short shape, which is
    /// exactly the shape 段組 is for.
    pub bands: usize,
    /// Rows from the top of one band to the top of the next, numbers included.
    pub band_height: u16,
}

impl Metrics {
    /// Work out the geometry for a text area `height` rows tall in a buffer of
    /// `total_lines` paragraphs.
    ///
    /// The configured 縱 length is a typographic choice and is never *raised* to
    /// fill a tall terminal, only lowered when the terminal cannot hold it. One
    /// row beyond the 縱 is kept spare so the end-of-paragraph caret has
    /// somewhere to sit below a full 縱.
    pub fn new(config: &Config, height: u16, total_lines: usize, look: Look) -> Metrics {
        let Look {
            ruby,
            hanging,
            measure,
            gap,
            dense,
            bands,
        } = look;
        let head_rows = number_rows(config.editor.line_numbers, total_lines);
        // …and never taller than the page it sits on. A novel of ten thousand
        // paragraphs spends five rows on numbers, and on a seven-row page the
        // writing was then laid out *below* the page — over the command row and
        // the status line, which are drawn after it and painted the numbers
        // out. The page showed its numbers and none of its text.
        let head_rows = head_rows.min(height.saturating_sub(1));
        // Bands are equal by construction: the page is divided, not packed, so
        // the second band can never be a row shorter than the first.
        let bands = bands.clamp(1, 4);
        let band_height = height / bands as u16;
        // A band too short to hold a 縱 at all is not a band; fall back to one.
        let bands = if band_height <= head_rows + 1 { 1 } else { bands };
        let band_height = height / bands as u16;
        let rows = band_height.saturating_sub(head_rows) as usize;
        // A 縱 is as long as the writer said, or — by default — as long as the
        // window allows. The window is the default in both directions and for
        // the same reason: a fixed count is a decision about the *book*, and
        // the editor has no business making one on the writer's behalf. `:view-wrap
        // 40` is that decision, in either layout, and vertically the measure
        // *is* the length of a column.
        let want = measure
            .or((config.editor.zong_length > 0).then_some(config.editor.zong_length))
            .unwrap_or(usize::MAX);
        let zong_len = want.min(rows.saturating_sub(1)).max(1);
        // The writer's own gap wins over the config's, the way the measure does.
        let gap = gap.unwrap_or(config.editor.zong_gap) as u16;
        Metrics {
            zong_len,
            pitch: SLOT_WIDTH + gap,
            head_rows,
            gap,
            ruby,
            hanging,
            ruby_width: 1,
            // Ticks cost the column a 縱's reading would have used, so a page
            // packed tight has none: that column is the whole point.
            ticks: config.editor.paper_ticks > 0 && !dense,
            bands,
            band_height,
        }
    }

    /// How wide a margin a 縱 needs on its right.
    ///
    /// `ruby_width` cells for a reading, not one: a reading in 注音符號 (ㄩㄥˇ)
    /// or in kana is full-width, and squeezing it into one cell walks every 縱
    /// after it a column to the left — the page comes apart into a staircase.
    /// Pinyin is half-width and keeps the one cell it always had.
    ///
    /// A 着重號 asks the same way, and asks whether or not the page is showing
    /// readings: it is the text's own emphasis, not an annotation laid over it,
    /// so `:ruby off` has nothing to say about it. What it asks for is measured
    /// rather than assumed — see [`EMPHASIS`].
    fn ruby_cell(&self, margin: Margin) -> u16 {
        let reading = if margin.reading && (self.ruby || self.hanging) {
            self.ruby_width
        } else {
            0
        };
        let dot = match margin.dot {
            true => yumete_cjk::char_width(EMPHASIS) as u16,
            false => 0,
        };
        let tone = match margin.tone {
            true => yumete_cjk::char_width(PING) as u16,
            false => 0,
        };
        reading.max(u16::from(self.ticks)).max(dot).max(tone)
    }

    /// How many 縱 fit across an area `width` cells wide. Only the gaps
    /// *between* 縱 count, so the leftmost one may sit flush against the edge.
    /// The most 縱 that could fit, were every one of them flush.
    ///
    /// An upper bound, used to decide how many to lay out before measuring; the
    /// real count comes out of [`place`], which knows which of them carry a
    /// reading.
    pub fn capacity(&self, width: u16) -> usize {
        (width / SLOT_WIDTH.max(1)) as usize * self.bands
    }
}

/// Which character of the buffer a click landed on, in the vertical page.
///
/// The page is laid out again rather than remembered: the layout depends on
/// which 縱 carry a reading, which depends on where the cursor is, and a
/// remembered one could be a frame out of date — a click that put the cursor
/// somewhere else than where it was pointed.
pub fn char_at(
    editor: &Editor,
    config: &Config,
    area: Rect,
    viewport: Anchor,
    mouse: ratatui::crossterm::event::MouseEvent,
) -> Option<usize> {
    let buffer = editor.current_buffer();
    let metrics = Metrics::new(
        config,
        area.height,
        buffer.line_count(),
        Look::of(editor),
    );
    // The same two answers the horizontal page is drawn from — asked of the
    // editor, not worked out again here.
    let hidden = |line: usize| editor.markup_hidden_on_line(line);
    let folded = |line: usize| editor.line_is_folded(line);
    let drawn = |line: usize| editor.drawn_runs_on_line(line);
    let grid = editor
        .grid_with(&hidden, &folded, &drawn)
        .with_zong_len(metrics.zong_len);
    let capacity = metrics.capacity(area.width);
    // The same page the drawing laid out, 着重號 and all: a click lands on the
    // character it looks like it landed on only if both agree where the 縱 are.
    let markup = Markup::of(editor);
    let page = layout_page(
        buffer.rope(),
        viewport,
        grid,
        &metrics,
        area,
        capacity,
        &|line| markup.dotted(line),
        editor.meter_drawn(),
    );

    // Which band the row fell in, then which 縱 of it the column fell in — or,
    // failing that, the nearest one to its right, since a click in a gap means
    // the 縱 beside it.
    let placed = page
        .iter()
        .filter(|p| mouse.column >= p.x && mouse.row >= p.top)
        .min_by_key(|p| (mouse.row - p.top, mouse.column - p.x))?;
    let slot = (mouse.row - placed.top) as usize;
    let start = buffer.rope().line_to_char(placed.zong.line);
    match placed.slots.get(slot) {
        Some(row) => Some(start + row.start),
        // Past the end of that 縱: the caret sits after its last character.
        None => Some(start + placed.slots.last().map_or(0, |row| row.end)),
    }
}

/// One 縱 as it was placed on the page.
#[derive(Debug, Clone)]
pub struct Placed {
    pub zong: zong::Zong,
    pub slots: Vec<zong::Slot>,
    /// The left edge of its two cells.
    pub x: u16,
    /// The row its first slot is drawn on — which band it landed in.
    pub top: u16,
    /// How many cells this 縱 bought to its right, from [`Metrics::ruby_cell`].
    ///
    /// **Carried rather than re-derived.** The renderer has to know whether the
    /// second margin cell is its to blank, and the answer is per-縱 — a page
    /// where one 縱 carries a 注音 reading and the next only a 着重號 buys two
    /// cells for the first and one for the second. Asking the page instead
    /// (「is any reading on it full-width」) blanks a cell the dotted 縱 never
    /// bought, which is the cell the 縱 to its right is drawn in.
    pub margin_cells: u16,
}

/// The 縱 of one page, in reading order: down a band right to left, then the
/// next band.
type Page = Vec<Placed>;

/// How wide a reading margin the readings on `slots` need.
///
/// Readings only. A hung 句讀 is full-width by nature and has always leaned into
/// the gap beside it; widening the margin for one would move every 縱.
fn ruby_width_of(slots: &[Vec<zong::Slot>]) -> u16 {
    slots
        .iter()
        .flatten()
        .filter_map(|r| r.ruby)
        .map(|c| yumete_cjk::char_width(c) as u16)
        .max()
        .unwrap_or(1)
        .max(1)
}

/// The Markdown of the lines a page is drawn from, asked of the editor once.
///
/// Three callers want the same answers: the **layout**, which has to know
/// before it places anything whether a line's 縱 need a cell for a 着重號, the
/// **drawing**, which needs the runs themselves, and [`char_at`], which has to
/// place the 縱 exactly where the drawing did or a click lands a character off.
struct Markup<'a> {
    editor: &'a Editor,
}

impl<'a> Markup<'a> {
    fn of(editor: &'a Editor) -> Self {
        Markup { editor }
    }

    /// What kind of line this is. Inside a fence or a page's metadata nothing is
    /// markup, and colouring `**` there — let alone dotting beside it — would
    /// misreport what the file says.
    ///
    /// The editor answers this in O(1) from a cache of its own
    /// ([`Editor::block_of`]); the page must not build a second one. Asking for
    /// a run of lines instead — `blocks_through` — hands back a copy of every
    /// line above as well, which is 364 µs a frame at line 19,883 of 資治通鑑.
    fn block(&self, line: usize) -> yumete_core::markdown::Block {
        self.editor.block_of(line)
    }

    /// The Markdown runs of one line — empty when the markup is not being
    /// rendered at all, which is what `:render off` means.
    fn runs(&self, line: usize) -> Vec<yumete_core::markdown::Span> {
        match self.editor.markup_visible() {
            true => self.editor.markup_line_in(line, self.block(line)),
            false => Vec::new(),
        }
    }

    /// Whether this line has a `*emphasis*` on it, and so needs the cell beside
    /// its 縱 for the 着重號 (Feature #236).
    fn dotted(&self, line: usize) -> bool {
        self.runs(line)
            .iter()
            .any(|r| r.kind == yumete_core::markdown::Kind::Emphasis)
    }
}

/// Lay out a page from `anchor`: fetch the 縱, work out their rows, and place
/// them right to left until the width runs out.
fn layout_page(
    rope: &Rope,
    anchor: Anchor,
    grid: zong::Grid,
    metrics: &Metrics,
    area: Rect,
    capacity: usize,
    dotted: &dyn Fn(usize) -> bool,
    metered: bool,
) -> Page {
    let zongs = zong::zongs_from(rope, anchor, grid, capacity);
    let slots: Vec<Vec<zong::Slot>> = zongs
        .iter()
        .map(|z| zong::zong_slots(rope, z, grid))
        .collect();
    // What each 縱 needs the cell to its right for. A 着重號 is asked for by
    // **line** rather than by 縱: a paragraph broken across three 縱 reserves the
    // cell in all three, which costs a cell only on the rightmost 縱 of the page
    // — every other one borrows the gap it already had — and in exchange the
    // page does not change shape as it is scrolled through.
    let margins: Vec<Margin> = zongs
        .iter()
        .zip(&slots)
        .map(|(z, rows)| Margin {
            reading: rows.iter().any(|r| r.ruby.is_some() || r.mark.is_some()),
            dot: dotted(z.line),
            tone: metered,
        })
        .collect();
    // Measured from the page itself: one full-width reading anywhere on it
    // widens the margin for all of them.
    let mut metrics = *metrics;
    metrics.ruby_width = ruby_width_of(&slots);
    // Each band is laid out as its own short page: filled right to left, and
    // when it runs out of width the next one starts again at the right edge.
    // Which is all 段組 is — the reading order is the sequence, and the bands
    // are where the sequence is put.
    let mut spots: Vec<(u16, u16)> = Vec::new();
    for band in 0..metrics.bands {
        let top = area.y + band as u16 * metrics.band_height + metrics.head_rows;
        let xs = place(&metrics, area, &margins[spots.len().min(margins.len())..]);
        if xs.is_empty() {
            break;
        }
        for x in xs {
            spots.push((x, top));
        }
    }
    zongs
        .into_iter()
        .zip(slots)
        .zip(margins)
        .zip(spots)
        .map(|(((zong, slots), margin), (x, top))| Placed {
            zong,
            slots,
            x,
            top,
            margin_cells: metrics.ruby_cell(margin),
        })
        .collect()
}

/// Where each 縱 of a page starts, right to left, and how many of them fit.
///
/// Positions are walked rather than computed, because a 縱's width is no longer
/// the same for all of them: with the gap set to zero, one carrying a reading
/// takes a cell more than one that does not.
fn place(metrics: &Metrics, area: Rect, margins: &[Margin]) -> Vec<u16> {
    let mut xs: Vec<u16> = Vec::with_capacity(margins.len());
    for &margin in margins {
        // A reading sits in the cell to the *right* of its own 縱, while the gap
        // sits *between* two — and they are the same cell. So one column apart
        // costs whichever is larger, and the rightmost 縱 pays for a reading
        // alone, since it has no neighbour to borrow the cell from.
        let ruby = metrics.ruby_cell(margin);
        let x = match xs.last() {
            None => (area.x + area.width).checked_sub(SLOT_WIDTH + ruby),
            Some(&previous) => previous.checked_sub(SLOT_WIDTH + metrics.gap.max(ruby)),
        };
        let Some(x) = x.filter(|&x| x >= area.x) else {
            break;
        };
        xs.push(x);
    }
    xs
}

/// How many rows the paragraph-number header needs: two digits stack into one
/// row 縦中横-style, so a four-digit novel needs two rows.
pub(crate) fn number_rows(mode: LineNumbers, total_lines: usize) -> u16 {
    match mode {
        LineNumbers::None => 0,
        // One row per digit. Two digits to a row packed twice as short, but
        // with the 縱 packed tight (`:view-dense`) there is no gap between them and
        // 「119」「118」 ran together into 「11」「11」 over 「9」「8」 — a wall of
        // digits nobody can read a line number out of. One digit to a row
        // cannot merge with its neighbour, because there is nothing beside it.
        _ => (total_lines.max(1).to_string().len()).clamp(1, 6) as u16,
    }
}

/// The 縱 length in force for a terminal `height` rows tall (including the
/// status line), so the event loop can tell the editor where 縱 break before the
/// motions that depend on it run.
pub fn zong_length_for(config: &Config, height: u16, total_lines: usize, look: Look) -> usize {
    // `height` is the page's own, already free of the status line, the hint
    // row, the tab bar and the detail panel — so nothing is subtracted here.
    // It used to take one off for the status line and know about none of the
    // rest, which made the 縱 the cursor moved on longer than the drawn one.
    Metrics::new(config, height, total_lines, look).zong_len
}

/// What the editor says about how this page is to be set.
///
/// Five answers to one question — how much of the window is writing — and they
/// travel together everywhere: the 縱 length, the pitch, the reading column and
/// the tick column are all worked out from them at once.
#[derive(Debug, Clone, Copy)]
pub struct Look {
    /// Whether readings are laid out beside the 縱.
    pub ruby: bool,
    /// Whether 句讀 hang in the margin.
    pub hanging: bool,
    /// The 縱 length the writer asked for, if any.
    pub measure: Option<usize>,
    /// The gap between 縱 the writer asked for, if any.
    pub gap: Option<usize>,
    /// Whether the page is packed as tight as a terminal allows.
    pub dense: bool,
    /// How many bands the page is divided into (段組).
    pub bands: usize,
}

impl Look {
    /// Ask the editor. The flags come from *it*, not from the config: `:ruby-off`,
    /// `:view-hanging` and `:view-dense` change them at runtime, and a page laid out from
    /// the config would disagree with the grid the cursor moves on.
    pub fn of(editor: &Editor) -> Look {
        Look {
            ruby: !editor.ruby().is_empty(),
            hanging: editor.hanging_punctuation(),
            measure: editor.measure(),
            gap: editor.zong_gap(),
            dense: editor.dense(),
            bands: editor.bands(),
        }
    }
}

/// Blank any wide glyph that reaches *into* `rect` from the column on its left.
///
/// A two-cell character at `rect.x - 1` covers `rect.x` as well, and the
/// renderer skips the cell a wide glyph covers — so the overlay's own left
/// border is computed, stored, and then never emitted. Cutting the glyph back to
/// a space is what lets the border be drawn at all.
pub fn clear_wide_left_edge(buf: &mut Buffer, rect: Rect) {
    if rect.x == 0 {
        return;
    }
    for y in rect.y..rect.y + rect.height {
        let wide = buf
            .cell((rect.x - 1, y))
            .is_some_and(|c| str_width(c.symbol()) > 1);
        if wide {
            if let Some(cell) = buf.cell_mut((rect.x - 1, y)) {
                cell.set_symbol(" ");
            }
        }
    }
}

/// Paint one 縱 slot: the grapheme in the left cell, the style across both, so
/// a selection or the cursor covers the whole square.
fn put_slot(buf: &mut Buffer, x: u16, y: u16, symbol: &str, style: Style) {
    // Never write an empty symbol. To the renderer an empty cell is the
    // *continuation* of a wide glyph, so it emits nothing and everything after
    // it on the row slides a column left — which paints the panel's ground
    // across its own border.
    // A control character in a manuscript is not a glyph, it is an instruction
    // to the terminal — see `crate::drawable`. It measures zero cells, so
    // dropping it moves nothing on the page.
    let clean = crate::drawable(symbol);
    let symbol = if clean.is_empty() { " " } else { clean.as_ref() };
    // The trailing cell first: a wide symbol makes the renderer skip it, and a
    // half-width one leaves it as the styled other half of the slot.
    if let Some(cell) = buf.cell_mut((x + 1, y)) {
        cell.set_symbol(" ").set_style(style);
    }
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(symbol).set_style(style);
    }
}

/// Paint a slot with a half-width symbol pushed against its **right** edge.
///
/// A column of single letters set flush left drifts away from the 漢字 beside
/// it; hung on the right they line up as one edge running down the panel. A
/// full-width symbol fills the slot either way, so this only moves the narrow
/// ones.
fn put_slot_right(buf: &mut Buffer, x: u16, y: u16, symbol: &str, style: Style) {
    if str_width(symbol) >= 2 {
        return put_slot(buf, x, y, symbol, style);
    }
    // A control character in a manuscript is not a glyph, it is an instruction
    // to the terminal — see `crate::drawable`. It measures zero cells, so
    // dropping it moves nothing on the page.
    let clean = crate::drawable(symbol);
    let symbol = if clean.is_empty() { " " } else { clean.as_ref() };
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(" ").set_style(style);
    }
    if let Some(cell) = buf.cell_mut((x + 1, y)) {
        cell.set_symbol(symbol).set_style(style);
    }
}

/// One column of a vertical panel, with half-width runs set 縦中横.
///
/// A 漢字 is a row to itself, as it is anywhere in 縱書. Latin letters and
/// digits are not: set one to a row they read as a column of nothing and cost
/// a row each, so consecutive ones are paired into a single slot — which is
/// what 縦中横 is for, and what the page's own text already does with a year.
fn packed(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut half = String::new();
    for g in graphemes(text) {
        if str_width(g) == 1 {
            half.push_str(g);
            if str_width(&half) == 2 {
                out.push(std::mem::take(&mut half));
            }
            continue;
        }
        if !half.is_empty() {
            out.push(std::mem::take(&mut half));
        }
        out.push(g.to_string());
    }
    if !half.is_empty() {
        out.push(half);
    }
    out
}

/// The character candidate `i` is numbered with.
///
/// Taken from the configured list — 帶圈中文數字 ㊀㊁㊂ by default. Circled
/// *Chinese* numerals, not the circled Arabic ①②③: those are East-Asian
/// *ambiguous* width, so a terminal may draw them one cell or two and the column
/// would come apart. Past the end of the list a plain digit stands in.
pub(crate) fn index_mark(markers: &str, i: usize) -> String {
    match markers.chars().nth(i) {
        Some(c) => c.to_string(),
        None => (i + 1).to_string(),
    }
}

/// Draw a paragraph number above its 縱, two digits to a row (縦中横), so it
/// reads as a number rather than a stack of loose digits.
fn put_number(buf: &mut Buffer, x: u16, top: u16, rows: u16, n: usize, style: Style) {
    let digits = n.to_string();
    let shown = digits.len().min(rows as usize);
    // The last `shown` digits: a number too long for the header loses its
    // leading digits rather than its trailing ones, since it is the units that
    // tell two neighbouring 縱 apart.
    let digits = &digits[digits.len() - shown..];
    for (row, ch) in digits.chars().enumerate() {
        // Bottom-aligned, against the text it labels.
        let y = top + rows - shown as u16 + row as u16;
        // Hung right, on the same edge the 漢字 below it are hung on, so the
        // number reads as belonging to this 縱 and not to the one beside it.
        // Half-width: a full-width digit would fill the slot and centre nicely,
        // but then 1–9 and 10–99 would mix the two widths down one gutter.
        if let Some(cell) = buf.cell_mut((x + 1, y)) {
            cell.set_symbol(&ch.to_string()).set_style(style);
        }
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol(" ").set_style(style);
        }
    }
}

/// Draw the buffer vertically into `area`, returning the cell the cursor is on.
///
/// The caller does not place a terminal cursor over this: a hardware cursor is
/// one cell wide and would sit inside a two-cell slot, so the cursor is drawn
/// here as a block covering the whole slot.
pub fn draw(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    area: Rect,
    viewport: &mut Anchor,
    peek: Option<&yumete_core::editor::Pane>,
) -> (u16, u16) {
    let buffer = editor.current_buffer();
    let total_lines = buffer.line_count();
    // Both flags come from the **editor**, not the config: `:ruby-off` and
    // `:view-hanging` change them at runtime, and a page laid out from the config
    // would disagree with the grid the cursor moves on.
    let metrics = Metrics::new(
        config,
        area.height,
        total_lines,
        Look::of(editor),
    );
    let rope = buffer.rope();

    // The grid the editor navigates by, at the wrap length this page settled on.
    // It must be *the editor's* grid and not a fresh one, or a setting the
    // editor holds — 縦中横, say — would apply to motion and not to drawing, and
    // the cursor would sit a row out from the character it is on.
    // The same two answers the horizontal page is drawn from — asked of the
    // editor, not worked out again here.
    let hidden = |line: usize| editor.markup_hidden_on_line(line);
    let folded = |line: usize| editor.line_is_folded(line);
    let drawn = |line: usize| editor.drawn_runs_on_line(line);
    let grid = editor
        .grid_with(&hidden, &folded, &drawn)
        .with_zong_len(metrics.zong_len);
    // A pane that is only being read has no cursor: the page is laid out
    // around the place it was left at, and the hit is what it marks.
    let at = peek.map_or_else(|| editor.cursor(), |pane| pane.cursor());
    let cursor_pos = zong::position(rope, at, grid);
    let cursor_anchor = Anchor::from(cursor_pos);

    // Scroll leftward/rightward so the cursor's 縱 stays on the page, keeping
    // `scrolloff` 縱 of context on either side — the horizontal view's rule,
    // counted in 縱 instead of lines.
    //
    // The page is anchored at a paragraph rather than at a 縱 *number*: numbering
    // the cursor's 縱 would mean walking the document from the top on every
    // keystroke, which on a novel-length buffer is the whole novel. Everything
    // here is bounded by the width of the page instead.
    // How many 縱 fit depends on which of them carry a reading, and which fit
    // depends on where the page is scrolled to — so it is measured, scrolled,
    // and measured again. Twice is enough: the second measurement is of the
    // page actually being drawn.
    let capacity = metrics.capacity(area.width);
    let markup = Markup::of(editor);
    let mut page = layout_page(
        rope,
        *viewport,
        grid,
        &metrics,
        area,
        capacity,
        &|line| markup.dotted(line),
        editor.meter_drawn(),
    );
    let visible = page.len().max(1);
    let scrolloff = config.editor.scrolloff.min(visible.saturating_sub(1) / 2);
    let last_column = visible.saturating_sub(1);
    // **Where the cursor sits on a page is the editor's answer** — the same
    // one the horizontal page and the grid ask, so a jump lands in the middle
    // whichever way the text runs and `:view-typewriter` means something here too.
    // This used to be a second copy of that rule, and it was the copy that had
    // never heard of typewriter mode.
    //
    // `capacity` is the *bound* — the most 縱 that could fit were every one of
    // them flush. With a gap, or with 稿紙 ticks, the page holds fewer, so
    // centring on the bound pushed the cursor's 縱 off the left edge.
    let found = zong::distance(rope, *viewport, cursor_anchor, grid, last_column);
    let cursor_column = match editor.page_inset(found, last_column, scrolloff) {
        None => found.unwrap_or(0),
        Some(inset) => {
            let inset = inset.min(page.len().max(1) / 2).max(match found {
                Some(d) if d < scrolloff => scrolloff,
                _ => 0,
            });
            *viewport = zong::retreat(rope, cursor_anchor, grid, inset);
            page = layout_page(
                rope,
                *viewport,
                grid,
                &metrics,
                area,
                capacity,
                &|line| markup.dotted(line),
                editor.meter_drawn(),
            );
            zong::distance(rope, *viewport, cursor_anchor, grid, page.len()).unwrap_or(0)
        }
    };
    let visible = page.len();

    // Lay the number band down as a band, before anything is drawn on it. In
    // every other editor a line number is separated from the text by position —
    // a gutter column the text never enters. Here the numbers sit *above* the
    // 縱, in the text's own columns, so position separates nothing and a bare
    // dim digit reads as a digit somebody typed. The colour is the gutter.
    let ink = match peek {
        None => crate::theme::Palette::of(config),
        // A rung back, all of it: the half that is only being read.
        Some(_) => crate::theme::Palette::of(config).faded(),
    };
    // **The page is painted, here too.** A 縱書 page is mostly margin — the
    // squares a 縱 does not reach are the page as much as the ones it does —
    // and until this line every one of them was the terminal's own ground, so
    // the paper stopped wherever the writing did.
    if ink.paints() {
        let ground = ink.page();
        let buf = frame.buffer_mut();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(ground);
                }
            }
        }
    }
    // The number band's own ground, when it is asked for. Off by default and
    // in both layouts: 縱書 painted this band and 橫排 painted nothing, which
    // is one editor giving two answers to one question. What tells the numbers
    // from the writing is their colour and their row, not a fill.
    let band_ground = match editor.number_fill() {
        true => ink.ground(yumete_config::rung::HEAD),
        false => ink.page(),
    };
    if metrics.head_rows > 0 {
        let ground = band_ground;
        let buf = frame.buffer_mut();
        // One band per 段: each is a page of its own and each opens with its
        // own row of paragraph numbers.
        for band in 0..metrics.bands as u16 {
            let top = area.y + band * metrics.band_height;
            for y in top..(top + metrics.head_rows).min(area.y + area.height) {
                for x in area.x..area.x + area.width {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_symbol(" ").set_style(ground);
                    }
                }
            }
        }
    }

    let text_top = area.y + metrics.head_rows;
    let (sel_start, sel_end) = match peek {
        None => editor.selection(),
        Some(pane) => pane.highlight.unwrap_or((at, at)),
    };
    // Asked of the editor, not of the range: the selection always covers the
    // cursor's own grapheme, so a bare cursor would otherwise be drawn as a
    // one-character highlight and the word-tint overlay would never appear.
    // …and a peeked hit is a selection for drawing purposes: it is the one
    // thing that pane is showing you.
    let has_selection = match peek {
        None => editor.has_selection(),
        Some(pane) => pane.highlight.is_some(),
    };
    // A ground, and only a ground: `fg(White)` used to flatten every colour
    // underneath — a heading, a reading, a hung mark — at the moment the writer
    // was looking hardest at them.
    let sel_style = Style::default().bg(ink.selection());
    // **The cell you are standing on** (#229), on the vertical page too. A `|`
    // table inside a 縱書 manuscript is edited where it lies — the page is not
    // turned for it (`turn_for_table`) — so this is the only surface that ever
    // says which cell Insert is confined to. `HEAD`, one rung quieter than a
    // selection, so a selection inside the cell still reads first.
    //
    // The padding of #212 is horizontal-only (`table_padding_on`), so here the
    // box holds nothing the file does not: the spaces around the content are
    // the ones that were typed.
    //
    // Asked of the **region**, the same as the horizontal page: `h`, `G` and a
    // search all walk out of the table without putting `:table` away, and the
    // rule row is drawn rather than edited. Either kind of region (#216).
    let cell = match peek.is_none() && editor.table().is_some_and(|t| !t.takes_the_pane()) {
        true => editor.prose_region().and_then(|region| {
            editor
                .cell_position()
                .filter(|&(line, _)| region.holds(line) && !region.is_rule(line))
                .and_then(|(line, at)| editor.cell_box(line, at))
        }),
        false => None,
    };
    let show_segmentation = editor.segmentation_visible();
    let mark = editor.word_mark();
    let cursor_line = editor.cursor_line();
    let numbers = config.editor.line_numbers;

    let ticks = config.editor.paper_ticks;
    // What marks a paragraph's opening squares, and how many there are.
    let hint = editor.indent_hint();
    let indent = editor.paragraph_indent();

    // 焦點模式 (#246): every 縱 but the one being written stands back a rung —
    // the same `faded()` a peeked pane recedes by, applied a 縱 at a time.
    //
    // **The 段, not the 縱.** A wrap point is not a unit of writing: stand back
    // everything outside the cursor's own visual column and the sentence just
    // finished — which wrapped into the 縱 to the right — goes quiet with the
    // rest of the page. What a focus mode lights horizontally is the line, and
    // in 縱書 the line is the 段.
    //
    // Not in a pane that is only being read: that half is already a rung back,
    // and standing part of it back again would say it has a cursor.
    let focus = peek.is_none() && editor.focus();
    let stood_back = ink.faded();

    // Word ranges are per paragraph, and consecutive 縱 usually share one, so
    // segment each paragraph once as the page is walked. Its Markdown runs are
    // held the same way, for the same reason.
    let mut segmented: Option<(usize, Vec<(usize, usize)>)> = None;
    let mut marked: Option<(usize, Vec<yumete_core::markdown::Span>)> = None;
    // And its 平仄, for the same reason: the editor caches the answer per line
    // too, but a page is forty 縱 and this saves the lookup as well as the walk.
    let mut metered: Option<(usize, Vec<yumete_core::meter::Mark>)> = None;


    let buf = frame.buffer_mut();
    for placed in page.iter() {
        let (zong, slots, x) = (&placed.zong, &placed.slots, placed.x);
        // Which palette this 縱 is drawn off. Everything below asks `ink`, so
        // 焦點模式 is one decision here rather than six dimmings further down.
        // The number *band* is not one of them: it is the page's furniture and
        // stays where it is, or the dimmed 縱 would punch holes in it.
        let stands_back = focus && zong.line != cursor_line;
        // The band is the page's furniture and its ground is painted off the
        // lit palette above; its **digits** have to come off the same one, or
        // a stood-back 縱's number is drawn at 1.39:1 against a band that did
        // not move — a number that recedes from furniture that does not.
        let band_ink = ink;
        let ink = match stands_back {
            true => stood_back,
            false => ink,
        };
        // A reading is set back — **a rung, not `DIM`**. It was DIM with no
        // colour at all, so on a terminal that ignores DIM a reading and the
        // character it annotates were the same colour, in a two-cell margin,
        // down a 縱. That is the flagship of this editor and its only
        // separation was an attribute several terminals drop.
        let reading_style = Style::default().fg(ink.quiet());
        // A rule, on the ladder's own rung for one. It used to be drawn in the
        // ruler's *tint* — a ground colour used as a foreground — and measured
        // 1.11:1 against the page, which is to say it has never been seen.
        let tick_style = Style::default().fg(ink.rule());
        // A hung 句讀 *is* the sentence, set beside the character it follows,
        // so it keeps the writing's own colour and is told apart by position.
        let mark_style = Style::default().fg(ink.text());
        // 平仄 are furniture, not writing: they are the editor talking about
        // the poem, so they sit on the rung the line numbers do rather than in
        // the reading's own shade, which belongs to something the *file* says.
        let tone_style = Style::default().fg(ink.furniture());
        let cell_style = Style::default().bg(ink.at(yumete_config::rung::HEAD));
        // Which band this 縱 landed in decides where its first slot is drawn.
        let text_top = placed.top;
        // Room for a tick: a 縱 that carries a reading or a hung mark has a
        // margin of its own, and with a gap between 縱 the cell to the right is
        // blank anyway. With neither, the next 縱 begins there.
        // Every 縱 has a margin when the page is ruled, so the tick has a cell
        // even on the rightmost one.
        let has_margin = x + SLOT_WIDTH < area.x + area.width;

        if numbers != LineNumbers::None && zong.starts_line() {
            let n = match numbers {
                LineNumbers::Relative if zong.line != cursor_line => {
                    zong.line.abs_diff(cursor_line)
                }
                _ => zong.line + 1,
            };
            // Set vertically the numbers sit *above* the 縱, in the text's own
            // columns — position separates nothing, so colour does the whole
            // job. Both halves of it: the band is a real ground and the digits
            // are a real rung. They used to be the terminal's own foreground on
            // a band at 1.04:1, which made the cursor's number the brightest
            // thing on a page of somebody's novel.
            let style = band_ground;
            let style = match zong.line == cursor_line {
                // 朱 for the one you are in: the one question vertical layout
                // strips position of, answered by the one colour off the ladder.
                true => style.fg(band_ink.mark()),
                false => style.fg(band_ink.furniture()),
            };
            // Above its own band, not above the page.
            let band_top = text_top.saturating_sub(metrics.head_rows);
            put_number(buf, x, band_top, metrics.head_rows, n, style);
        }

        let line_start = rope.line_to_char(zong.line);
        // Rows, not graphemes: a 縦中横 pair is one row holding two characters, a
        // ruby group is however many rows its reading needs, and the punctuation
        // is already rotated.
        for (slot, row) in slots.iter().cloned().enumerate() {
            let y = text_top + slot as u16;
            // This line's markup runs, wanted twice over: here, for the 着重號
            // that goes in the margin, and below for the ink of the character
            // itself. Cached per line, because consecutive 縱 share one — and
            // inside a fence or a page's metadata nothing is markup, so the
            // block decides before the line is scanned at all.
            if marked.as_ref().is_none_or(|(l, _)| *l != zong.line) {
                marked = Some((zong.line, markup.runs(zong.line)));
            }
            let runs: &[yumete_core::markdown::Span] = match &marked {
                Some((l, r)) if *l == zong.line => r.as_slice(),
                _ => &[],
            };
            // Whether a `*emphasis*` covers this slot. **Emphasis, not strong**:
            // `**` is set in another weight, and doubling both marks would put
            // dots down half a page. The cell is there — the layout reserved it
            // for this line — but `has_margin` is still asked, because a page
            // one 縱 wide has run out of width before it could.
            let emphasised = has_margin
                && !row.text.is_empty()
                && runs.iter().any(|r| {
                    r.kind == yumete_core::markdown::Kind::Emphasis
                        && r.end > row.start
                        && r.start < row.end
                });
            // The margin to the right of the 縱 carries both a reading and a
            // hung 句讀 mark. The mark wins the cell — it belongs against the
            // character it follows, and the reading has already given way
            // upward to leave that row free — and it is tinted apart from a
            // reading so the two are never mistaken for one another. A 着重號
            // is last of the three: it says 「this word」, which the reading and
            // the sentence's own punctuation both outrank, and it is the only
            // one of the three that can be read off the page without it.
            // 平仄 (#247) go in the same column, under the reading and over the
            // 着重號: a tone is computed and can be read nowhere else on the
            // page, while the dot repeats what `*` already says in the file.
            // With `:view-meter` off the line is never asked, so a manuscript that
            // is not a poem pays nothing.
            let tone = match editor.meter_drawn() && !row.text.is_empty() {
                false => None,
                true => {
                    if metered.as_ref().is_none_or(|(l, _)| *l != zong.line) {
                        metered = Some((zong.line, editor.meter_on_line(zong.line)));
                    }
                    metered
                        .as_ref()
                        .filter(|(l, _)| *l == zong.line)
                        .and_then(|(_, marks)| {
                            // A slot may hold more than one character — a ruby
                            // group, a 縦中横 pair — and the mark belongs to the
                            // first of them, which is the one the reader sees.
                            marks.iter().find(|m| m.column == row.start).copied()
                        })
                        .map(meter_glyph)
                }
            };
            let margin = row
                .mark
                .map(|m| (m, mark_style))
                .or_else(|| row.ruby.map(|r| (r, reading_style)))
                .or_else(|| tone.map(|g| (g, tone_style)))
                .or_else(|| emphasised.then_some((EMPHASIS, mark_style)));
            // 稿紙 is ruled, and a writer estimates length by it. A tick every
            // `paper_ticks` characters down the 縱 is the vertical page's own
            // version of that, and it costs one dim cell in a margin that is
            // otherwise blank. It yields to a reading and to a hung mark, both
            // of which carry meaning where this only guides the eye — and it is
            // skipped where there is no margin to put it in.
            if ticks > 0 && margin.is_none() && (slot + 1) % ticks == 0 && has_margin {
                if let Some(cell) = buf.cell_mut((x + SLOT_WIDTH, y)) {
                    if cell.symbol().trim().is_empty() {
                        cell.set_symbol(".").set_style(tick_style);
                    }
                }
            }
            if let Some((glyph, style)) = margin {
                let mx = x + SLOT_WIDTH;
                // A full-width glyph covers the cell after it, and that cell is
                // only ours to blank when **this 縱** widened its margin for it.
                // With a one-cell margin the glyph leans into the gap instead —
                // which is what a hung 句讀 has always done — and blanking there
                // would rub out the 縱 to the right.
                if placed.margin_cells >= 2 {
                    if let Some(cell) = buf.cell_mut((mx + 1, y)) {
                        cell.set_symbol(" ").set_style(style);
                    }
                }
                if let Some(cell) = buf.cell_mut((mx, y)) {
                    cell.set_symbol(&glyph.to_string()).set_style(style);
                }
            }
            // A control character gets its picture (#398): handed to the
            // terminal as itself it draws nothing, and a 縱 with a NUL in it
            // looked like a 縱 with a gap. One cell either way, which is the
            // cell the slot was measured for.
            let symbol: String = row
                .text
                .chars()
                .map(|c| yumete_cjk::control_picture(c).unwrap_or(c))
                .collect();
            if symbol.is_empty() {
                // A paragraph's opening squares are empty slots, and what goes
                // in them is the same question the horizontal page answers:
                // white, a band, or a mark in the first one.
                if hint != IndentHint::None && zong.starts_line() && slot < indent {
                    match hint {
                        IndentHint::Colour => {
                            let ground = ink.ground(yumete_config::rung::BAND);
                            put_slot_right(buf, x, y, " ", ground);
                            if let Some(cell) = buf.cell_mut((x, y)) {
                                cell.set_symbol(" ").set_style(ground);
                            }
                        }
                        IndentHint::Symbol if slot == 0 => {
                            let style = ink.page().fg(ink.rule());
                            put_slot_right(buf, x, y, editor.indent_symbol(), style);
                        }
                        _ => {}
                    }
                }
                continue;
            }
            let at = line_start + row.start;
            let len = row.end - row.start;

            // Virtual text (#248) is not the manuscript, and 縱書 is where the
            // author reads: a note set in the writing's own ink would be read
            // as a word of it. It takes the ink it was drawn in — a note in the
            // marker's colour, a candidate or a table's padding in the quiet
            // one — and none of the layers below, every one of which describes
            // characters the file actually holds.
            if let Some(kind) = row.ink {
                let style = match kind {
                    // 金 and bold, the same as across the page: 這不是正文.
                    yumete_core::drawn::Ink::Fold => {
                        ink.page().fg(ink.gold()).add_modifier(Modifier::BOLD)
                    }
                    yumete_core::drawn::Ink::Note => ink.page().fg(ink.marker()),
                    _ => ink.page().fg(ink.quiet()),
                };
                put_slot_right(buf, x, y, &symbol, style);
                continue;
            }

            // The same three layers the horizontal page composes, in the same
            // order: the block's ground, the inline runs patched onto it, then
            // the selection over everything. Vertical had only the last of the
            // three, so a 縱書 draft got the markup taken off the page but
            // never coloured — half of 所見即所得.
            let column = at - line_start;
            let mut style = crate::block_style(markup.block(zong.line), ink).unwrap_or_default();

            // Whether a `==highlight==` covers this slot, so the cell ground
            // steps around it — the horizontal page has done so since #229 and
            // this page had not. A highlight exists *to be* a ground.
            let mut highlighted = false;
            // A slot is one display unit and may hold several characters — a
            // ruby group, or a 縦中横 pair — so it takes the style of any run it
            // overlaps. `runs` is empty when markup is not being rendered.
            for run in runs.iter().filter(|r| r.end > column && r.start < column + len) {
                highlighted |= run.kind == yumete_core::markdown::Kind::Highlight;
                style = style.patch(crate::markup_style(run.kind, ink));
            }

            // ⚠️ **Not `&& !has_selection`** (#450, and again here for #457).
            // The horizontal page had the same gate and the same bug: every
            // paragraph on the screen went out the moment anything was
            // selected, and a motion *is* a selection here, so holding `w`
            // down flashed the whole page once a keystroke. The selection's
            // own ground is patched on further down, **after** this, so the
            // cells it covers were never going to show a word mark anyway.
            //
            // Fixed in one renderer and not the other is worse than not fixed:
            // 「模式不应该影响分词的闪烁」.
            if show_segmentation && (mark == WordMark::Ink || style.bg.is_none()) {
                let ranges = match &segmented {
                    Some((line, ranges)) if *line == zong.line => ranges,
                    _ => {
                        segmented = Some((zong.line, editor.segment_line(zong.line)));
                        &segmented.as_ref().unwrap().1
                    }
                };
                // The quietest layer of the three: only where nothing else has
                // claimed the ground, so it never rubs out a `==highlight==`.
                // **One tint, alternating with the page**, not two tints
                // alternating with each other. Two differed by temperature and
                // not by weight — 1.04 and 1.07 against the ground, which is to
                // say one read as the ground and the other as a stain. The
                // alternation is strict per word, so an untinted word is always
                // between two tinted ones and says exactly as much.
                if let Some(word) = ranges.iter().position(|&(a, b)| column >= a && column < b) {
                    // ⚠️ **線 does not alternate** (#501). Down a column an
                    // underline is drawn under each *character*, so underlining
                    // every other word would give a run of ticks rather than a
                    // boundary. Marking only each word's **last** cell puts one
                    // short rule exactly where the word ends — which is the
                    // whole of what the mark is for, and it is then every word
                    // rather than every second one.
                    if mark == WordMark::Line {
                        let (_, end) = ranges[word];
                        // A link's own underline wins — see the horizontal
                        // renderer's note.
                        let taken = style.add_modifier.contains(Modifier::UNDERLINED);
                        if column + 1 == end && !taken {
                            style = style
                                .add_modifier(Modifier::UNDERLINED)
                                .underline_color(ink.word_rule());
                        }
                    } else if mark == WordMark::Color {
                        // Both halves, warm against cool — see `Ink::word_hue`.
                        let from = style.fg.unwrap_or(ink.text());
                        style = style.fg(ink.word_hue(from, word % 2 != 0));
                    } else if word % 2 == 0 {
                        // 字色 leaves the paper alone and moves the writing
                        // instead (#278) — **whatever colour that writing
                        // already has, stepped 第 15 檔 toward the page**
                        // (#461). Standing back from a coloured run, as this
                        // did, means a link or a heading shows no word
                        // boundaries at all. The horizontal renderer does the
                        // same thing in the same words.
                        match mark {
                            WordMark::Tint => style = style.bg(ink.word()),
                            WordMark::Ink => {
                                let from = style.fg.unwrap_or(ink.text());
                                style = style.fg(ink.marked(from, yumete_config::rung::WORD_INK));
                            }
                            // Both answered above, before the parity test.
                            WordMark::Color | WordMark::Line => {}
                        }
                    }
                }
            }

            // The page's ground was painted off the lit palette, and a
            // character with no colour of its own takes the colour of the cell
            // it lands on — so a 縱 that stands back has to say its ink out
            // loud. Only where nothing else has: markup, a 着重 run and 字色
            // word marking all colour off this 縱's own palette already.
            if stands_back && style.fg.is_none() {
                style = style.fg(ink.text());
            }

            // The cell first, so the selection still goes over it.
            if let Some((from, to)) = cell {
                if !highlighted && at < to && at + len > from {
                    style = style.patch(cell_style);
                }
            }

            if has_selection && at < sel_end && at + len > sel_start {
                style = style.patch(sel_style);
            }

            // Hung right, so half-width characters line up as one edge running
            // down the 縱 beside the 漢字 rather than drifting to its left.
            put_slot_right(buf, x, y, &symbol, style);
        }
    }

    // The cursor, drawn last so it wins over a selection or a word tint.
    let at_cursor = page.get(cursor_column).or_else(|| page.last());
    let cursor_x = at_cursor.map(|p| p.x).unwrap_or(area.x);
    let cursor_y = (at_cursor.map(|p| p.top).unwrap_or(text_top) + cursor_pos.slot as u16)
        .min((area.y + area.height).saturating_sub(1));
    // Insert leaves the page alone: the caret is the terminal's own cursor, set
    // to an underscore — a thin horizontal rule, which is the bar of a
    // horizontal editor turned the quarter turn the text turned. Drawing it into
    // the page instead would have to recolour a character to show it.
    //
    // It goes on the slot **above** the cursor's. Typing inserts *before* the
    // character the cursor is on, pushing it down, so the boundary the text
    // arrives at is that character's top edge — and an underscore is drawn at
    // the bottom of the cell it is in. Put it on the cursor's own slot and it
    // sits one boundary too low, which reads as "insert after this character"
    // and is not where the text appears.
    let mut caret_y = cursor_y;
    if cursor_column < visible && editor.mode() == Mode::Insert {
        caret_y = cursor_y.saturating_sub(1).max(area.y);
        // A terminal sizes its cursor to the grapheme it sits on, so on a blank
        // slot the rule would be one cell — half the 縱 — and read as lopsided.
        // An ideographic space is two cells and shows nothing.
        //
        // *Both* cells decide whether the slot is blank. A half-width character
        // hangs against the slot's right edge, so asking only the left one says
        // "blank" over a digit — and the ideographic space then paints it out.
        // The character comes back the moment the caret moves on, which is what
        // "I typed 1 and got a space" looks like.
        let occupied = |x: u16| {
            buf.cell((x, caret_y))
                .is_some_and(|c| !c.symbol().trim().is_empty())
        };
        if !occupied(cursor_x) && !occupied(cursor_x + 1) {
            put_slot(buf, cursor_x, caret_y, "\u{3000}", Style::default());
        }
    }
    if cursor_column < visible && editor.mode() != Mode::Insert {
        // Normal: a solid block over the whole two-cell slot, keeping whatever
        // is under it — and keeping *where* it is. A half-width character hangs
        // against the slot's right edge, so reading only the left cell would
        // paint the block over a space and lose the character.
        let read = |x: u16| {
            buf.cell((x, cursor_y))
                .map(|c| c.symbol().to_string())
                .filter(|sym| !sym.trim().is_empty())
        };
        let block = Style::default().add_modifier(Modifier::REVERSED);
        match (read(cursor_x), read(cursor_x + 1)) {
            (Some(left), _) => put_slot(buf, cursor_x, cursor_y, &left, block),
            (None, Some(right)) => put_slot_right(buf, cursor_x, cursor_y, &right, block),
            (None, None) => put_slot(buf, cursor_x, cursor_y, " ", block),
        }
    }
    (cursor_x, caret_y)
}

/// The candidate panel's skin, derived from two colours.
///
/// Yume's own themes are defined by **four numbers** — an ink and a paper for
/// each mode — with every other shade interpolated along a ladder between them
/// (`yume_core::themes::ink_ladder`). Reproducing the ladder rather than storing
/// the resulting shades is what lets a skin be changed by editing a pair of
/// values: the relationships between the shades stay right by construction, and
/// yumete's panel is the same skin as the GUI frontends' when the endpoints
/// match.
///
/// The default is 墨香 dark. Its green is deliberate and slight — R and G differ
/// by about 5, so it reads as ink with a hint of pine rather than grey-green —
/// and the paper is warm rather than white. Dark mode is not the light pair
/// swapped: the ground goes deeper and the ink dimmer, or the panel glows at
/// night.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Skin {
    ink: (u8, u8, u8),
    paper: (u8, u8, u8),
}

impl Skin {
    pub fn new(ink: (u8, u8, u8), paper: (u8, u8, u8)) -> Skin {
        Skin { ink, paper }
    }

    /// One rung of the ladder: `0` is pure ink, [`yumete_config::rung::PAPER`]
    /// pure paper — **the same scale the page uses**, so a rung can be named
    /// here by the same constant it is named by everywhere else.
    ///
    /// ⚠️ It used to be its own 0–1000 while taking `rung::RULE` as an
    /// argument; the day the page's ladder was restretched to 0–10000 that
    /// argument became five times the scale and the mix overflowed.
    ///
    /// Mixed in sRGB, not linear light, because the hand-tuned original was
    /// picked by eye in sRGB and mixing linearly comes out far lighter.
    fn step(self, t: u32) -> Color {
        let full = yumete_config::rung::PAPER as i64;
        let t = (t as i64).min(yumete_config::rung::DEEP as i64);
        let mix = |a: u8, b: u8| -> u8 {
            let (a, b) = (a as i64, b as i64);
            ((a * full + (b - a) * t + full / 2) / full).clamp(0, 255) as u8
        };
        Color::Rgb(
            mix(self.ink.0, self.paper.0),
            mix(self.ink.1, self.paper.1),
            mix(self.ink.2, self.paper.2),
        )
    }

    /// The panel's ground.
    pub fn paper(self) -> Color {
        self.step(yumete_config::rung::PAPER as u32)
    }
    /// The ring around the panel.
    ///
    /// [`yumete_config::rung::RULE`], which is where every other ring on the
    /// screen is drawn — the which-key panel, the sidebar's edge, the ruler's
    /// line. Four panels used to ring themselves at four different rungs, and
    /// on one screen that reads as four different kinds of thing.
    pub fn border(self) -> Color {
        self.step(yumete_config::rung::RULE as u32)
    }
    /// A candidate.
    pub fn text(self) -> Color {
        self.step(yumete_config::rung::TEXT as u32)
    }
    /// Numbers and the code — one shade back from the candidates.
    pub fn helper(self) -> Color {
        self.step(yumete_config::rung::ASIDE as u32)
    }
    /// The ground of the highlighted candidate, and the text on it.
    pub fn highlight(self) -> Color {
        self.step(yumete_config::rung::TEXT as u32)
    }
    pub fn on_highlight(self) -> Color {
        self.step(yumete_config::rung::PAPER as u32)
    }
}

impl From<&Config> for Skin {
    fn from(config: &Config) -> Skin {
        Skin::new(config.panel.ink, config.panel.paper)
    }
}

/// Draw the candidate panel for vertical layout (Feature #61).
///
/// It is the horizontal panel turned a quarter turn: the preedit occupies the
/// rightmost column, and the candidates run **right to left** after it, each in
/// its own column with its selection digit on top. Reading order therefore
/// matches the text it is about to be committed into.
pub fn draw_candidate_panel(
    frame: &mut Frame,
    ime: &ImeSession,
    config: &Config,
    area: Rect,
    cursor_x: u16,
    cursor_y: u16,
) {
    let skin = Skin::from(config);
    let candidates = ime.page_candidates();
    // A code with no candidates still gets a panel — with nothing but the code
    // in it. In 形碼 a dead code is the ordinary way to mistype, the code lives
    // only in this panel (there is no inline preedit), and a panel that vanishes
    // leaves the writer nothing to see and nothing to know to backspace.
    if candidates.is_empty() && ime.display_buffer().is_empty() {
        return;
    }
    let highlight = ime.highlight();

    // Everything in the panel is a 縱, the 下標 included: one character to a
    // row, hung against the slot's right edge so the letters line up as a single
    // edge running down beside the 漢字. Two letters side by side read as a
    // syllable that is not there.
    let pitch = SLOT_WIDTH;

    // The code as typed runs down the rightmost column — but **packed**: a run
    // of half-width letters goes two to a row (縦中横), the way a year does in
    // 縱書. `Dyu_Do_Ne` set one letter to a row is nine rows of nothing, and
    // with 拆分 on it made the panel taller than the page it was covering.
    let header = packed(&ime.display_buffer());
    // Each column is the number, a blank row, then the candidate. The gap is
    // what stops the number reading as the first character of the word.
    // Column 0 is the header; the candidates run leftward from column 1, the
    // direction the text they are joining runs.
    //
    // **Each candidate carries its own 拆分**, under it and set back. It used
    // to be one extra column showing the *highlighted* candidate's — which is
    // the one thing 拆分 is not for: you turn it on to see why 相 and 想 want
    // different codes, and that means seeing both at once. (That column also
    // shifted every candidate one place right of the number the style code
    // thought it was, so with annotations on the highlight was off by one.)
    let mut columns: Vec<Vec<String>> = vec![header];
    let mut comment_from: Vec<usize> = vec![usize::MAX];
    for (i, cand) in candidates.iter().enumerate() {
        // Number, a blank row, the candidate, then the keys still owed.
        let mut column = vec![index_mark(&config.panel.markers, i), String::new()];
        column.extend(graphemes(&cand.text).map(String::from));
        column.extend(graphemes(&cand.completion).map(String::from));
        let at = if cand.comment.is_empty() {
            usize::MAX
        } else {
            let at = column.len() + 1;
            column.push(String::new());
            column.extend(packed(&cand.comment));
            at
        };
        columns.push(column);
        comment_from.push(at);
    }

    let depth = columns.iter().map(|c| c.len()).max().unwrap_or(1);
    let count = columns.len();
    // The header column is only ever a slot wide, whatever the candidates need.
    let inner_w = SLOT_WIDTH + (count as u16).saturating_sub(1) * pitch;
    let panel_w = (inner_w + 2).min(area.width.max(1));
    let panel_h = (depth as u16 + 2).min(area.height.max(1));

    // The text reads leftward, so the panel opens to the left of the cursor's
    // 縱 — the direction the text is going — and flips right only when there is
    // no room for it there.
    let x = if cursor_x >= area.x + panel_w {
        cursor_x - panel_w
    } else {
        (cursor_x + SLOT_WIDTH).min(area.x + area.width.saturating_sub(panel_w))
    };
    let y = cursor_y.min(area.y + area.height.saturating_sub(panel_h));
    let panel = Rect::new(x, y, panel_w, panel_h);

    frame.render_widget(Clear, panel);
    clear_wide_left_edge(frame.buffer_mut(), panel);
    let ground = Style::default().bg(skin.paper()).fg(skin.text());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if config.panel.rounded {
            BorderType::Rounded
        } else {
            BorderType::Plain
        })
        .border_style(Style::default().fg(skin.border()).bg(skin.paper()))
        .style(ground);
    let inner = block.inner(panel);
    block.render(panel, frame.buffer_mut());

    let buf = frame.buffer_mut();
    let dim = ground.fg(skin.helper());
    let chosen = Style::default()
        .bg(skin.highlight())
        .fg(skin.on_highlight());

    let top = inner.y;

    // Candidate ㊀ is rightmost and they run leftward, the direction the text
    // they are joining runs. `None` means the column would fall past the panel's
    // left border, which is what a pane too narrow for them all produces.
    // Column 0 (the header) sits flush at the right in a plain slot; the
    // candidate columns step leftward by `pitch`, which widens only to fit a
    // 下標.
    let column_x = |k: usize| -> Option<u16> {
        let offset = match k {
            0 => SLOT_WIDTH,
            _ => SLOT_WIDTH.checked_add((k as u16).checked_mul(pitch)?)?,
        };
        let x = (inner.x + inner.width).checked_sub(offset)?;
        (x >= inner.x).then_some(x)
    };
    for (i, column) in columns.iter().enumerate() {
        let Some(x) = column_x(i) else {
            break;
        };
        for (slot, symbol) in column.iter().enumerate() {
            let cy = top + slot as u16;
            if cy >= inner.y + inner.height {
                break;
            }
            // Column 0 is the header, which is never highlighted; the
            // candidates start at 1, so their number is `i - 1`. Within a
            // column the number and the 下標 sit a shade back from the
            // candidate itself, so the three separate by weight alone.
            let text_rows = candidates
                .get(i.wrapping_sub(1))
                .map_or(0, |c| 2 + graphemes(&c.text).count());
            let annotated = comment_from.get(i).copied().unwrap_or(usize::MAX);
            let style = if i == 0 || slot >= annotated {
                // The 拆分 is an aside, so it stays quiet even under the
                // candidate that is chosen.
                dim
            } else if i - 1 == highlight {
                chosen
            } else if slot == 0 || slot >= text_rows {
                dim
            } else {
                ground
            };
            put_slot_right(buf, x, cy, symbol, style);
        }
    }
}
