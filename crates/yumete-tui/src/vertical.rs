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

use yumete_cjk::{graphemes, str_width};
use yumete_config::{Config, LineNumbers};
use yumete_core::zong::{self, Anchor};
use yumete_core::{Editor, Mode, Rope, TextStore};
use yumete_ime::ImeSession;

/// The width of one 縱 in cells. A full-width character is two cells, and the
/// grid is built around that, not around any particular character's width.
const SLOT_WIDTH: u16 = 2;

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
}

impl Metrics {
    /// Work out the geometry for a text area `height` rows tall in a buffer of
    /// `total_lines` paragraphs.
    ///
    /// The configured 縱 length is a typographic choice and is never *raised* to
    /// fill a tall terminal, only lowered when the terminal cannot hold it. One
    /// row beyond the 縱 is kept spare so the end-of-paragraph caret has
    /// somewhere to sit below a full 縱.
    pub fn new(
        config: &Config,
        height: u16,
        total_lines: usize,
        ruby: bool,
        hanging: bool,
    ) -> Metrics {
        let head_rows = number_rows(config.editor.line_numbers, total_lines);
        let rows = height.saturating_sub(head_rows) as usize;
        let zong_len = config.editor.zong_length.min(rows.saturating_sub(1)).max(1);
        Metrics {
            zong_len,
            pitch: SLOT_WIDTH + config.editor.zong_gap as u16,
            head_rows,
            gap: config.editor.zong_gap as u16,
            ruby,
            hanging,
            ruby_width: 1,
        }
    }

    /// How wide a margin a 縱 needs on its right for a reading.
    ///
    /// `ruby_width` cells, not one: a reading in 注音符號 (ㄩㄥˇ) or in kana is
    /// full-width, and squeezing it into one cell walks every 縱 after it a
    /// column to the left — the page comes apart into a staircase. Pinyin is
    /// half-width and keeps the one cell it always had.
    fn ruby_cell(&self, annotated: bool) -> u16 {
        if annotated && (self.ruby || self.hanging) {
            self.ruby_width
        } else {
            0
        }
    }

    /// How many 縱 fit across an area `width` cells wide. Only the gaps
    /// *between* 縱 count, so the leftmost one may sit flush against the edge.
    /// The most 縱 that could fit, were every one of them flush.
    ///
    /// An upper bound, used to decide how many to lay out before measuring; the
    /// real count comes out of [`place`], which knows which of them carry a
    /// reading.
    pub fn capacity(&self, width: u16) -> usize {
        (width / SLOT_WIDTH.max(1)) as usize
    }
}

/// The 縱 of one page: each with the rows it draws and where it starts.
type Page = Vec<(zong::Zong, Vec<zong::Slot>, u16)>;

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

/// Lay out a page from `anchor`: fetch the 縱, work out their rows, and place
/// them right to left until the width runs out.
fn layout_page(
    rope: &Rope,
    anchor: Anchor,
    grid: zong::Grid,
    metrics: &Metrics,
    area: Rect,
    capacity: usize,
) -> Page {
    let zongs = zong::zongs_from(rope, anchor, grid, capacity);
    let slots: Vec<Vec<zong::Slot>> = zongs
        .iter()
        .map(|z| zong::zong_slots(rope, z, grid))
        .collect();
    let annotated: Vec<bool> = slots
        .iter()
        .map(|rows| rows.iter().any(|r| r.ruby.is_some() || r.mark.is_some()))
        .collect();
    // Measured from the page itself: one full-width reading anywhere on it
    // widens the margin for all of them.
    let mut metrics = *metrics;
    metrics.ruby_width = ruby_width_of(&slots);
    let xs = place(&metrics, area, &annotated);
    zongs
        .into_iter()
        .zip(slots)
        .zip(xs)
        .map(|((zong, rows), x)| (zong, rows, x))
        .collect()
}

/// Where each 縱 of a page starts, right to left, and how many of them fit.
///
/// Positions are walked rather than computed, because a 縱's width is no longer
/// the same for all of them: with the gap set to zero, one carrying a reading
/// takes a cell more than one that does not.
fn place(metrics: &Metrics, area: Rect, annotated: &[bool]) -> Vec<u16> {
    let mut xs: Vec<u16> = Vec::with_capacity(annotated.len());
    for &annotated in annotated {
        // A reading sits in the cell to the *right* of its own 縱, while the gap
        // sits *between* two — and they are the same cell. So one column apart
        // costs whichever is larger, and the rightmost 縱 pays for a reading
        // alone, since it has no neighbour to borrow the cell from.
        let ruby = metrics.ruby_cell(annotated);
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
        _ => (total_lines.max(1).to_string().len().div_ceil(2)).clamp(1, 3) as u16,
    }
}

/// The 縱 length in force for a terminal `height` rows tall (including the
/// status line), so the event loop can tell the editor where 縱 break before the
/// motions that depend on it run.
pub fn zong_length_for(
    config: &Config,
    height: u16,
    total_lines: usize,
    ruby: bool,
    hanging: bool,
) -> usize {
    Metrics::new(config, height.saturating_sub(1), total_lines, ruby, hanging).zong_len
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
    let symbol = if symbol.is_empty() { " " } else { symbol };
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
    let symbol = if symbol.is_empty() { " " } else { symbol };
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(" ").set_style(style);
    }
    if let Some(cell) = buf.cell_mut((x + 1, y)) {
        cell.set_symbol(symbol).set_style(style);
    }
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
    let bytes = digits.as_bytes();
    // Pair the digits from the right, so "123" becomes "1" then "23".
    let pairs = bytes.len().div_ceil(2);
    let first = bytes.len() - (pairs - 1) * 2;
    let mut at = 0usize;
    for row in 0..pairs.min(rows as usize) {
        let take = if row == 0 { first } else { 2 };
        let text = &digits[at..at + take];
        at += take;
        // Bottom-align the number against the text it labels.
        let y = top + rows - pairs.min(rows as usize) as u16 + row as u16;
        // Half-width throughout, right-aligned. A lone digit could be centred by
        // using its full-width form — that is two cells and fills the slot — but
        // then a gutter of 1–9 and 10–99 would mix the two widths, and the
        // mixture reads worse than the offset it fixes.
        for (i, ch) in format!("{text:>2}").chars().enumerate() {
            if let Some(cell) = buf.cell_mut((x + i as u16, y)) {
                cell.set_symbol(&ch.to_string()).set_style(style);
            }
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
) -> (u16, u16) {
    let buffer = editor.current_buffer();
    let total_lines = buffer.line_count();
    // Both flags come from the **editor**, not the config: `:ruby-off` and
    // `:hanging` change them at runtime, and a page laid out from the config
    // would disagree with the grid the cursor moves on.
    let metrics = Metrics::new(
        config,
        area.height,
        total_lines,
        !editor.ruby().is_empty(),
        editor.hanging_punctuation(),
    );
    let rope = buffer.rope();

    // The grid the editor navigates by, at the wrap length this page settled on.
    // It must be *the editor's* grid and not a fresh one, or a setting the
    // editor holds — 縦中横, say — would apply to motion and not to drawing, and
    // the cursor would sit a row out from the character it is on.
    let grid = editor.grid().with_zong_len(metrics.zong_len);
    let cursor_pos = zong::position(rope, editor.cursor(), grid);
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
    let mut page = layout_page(rope, *viewport, grid, &metrics, area, capacity);
    let visible = page.len().max(1);
    let scrolloff = config.editor.scrolloff.min(visible.saturating_sub(1) / 2);
    let last_column = visible.saturating_sub(1);
    let cursor_column = match zong::distance(rope, *viewport, cursor_anchor, grid, last_column) {
        Some(d) if d >= scrolloff && d + scrolloff <= last_column => d,
        // Off the page, or too close to an edge: re-anchor so the cursor sits
        // `scrolloff` in from whichever side it left by.
        found => {
            let inset = if found.is_some() || cursor_anchor >= *viewport {
                last_column.saturating_sub(scrolloff)
            } else {
                scrolloff
            };
            let inset = if found.is_some_and(|d| d < scrolloff) {
                scrolloff
            } else {
                inset
            };
            *viewport = zong::retreat(rope, cursor_anchor, grid, inset);
            page = layout_page(rope, *viewport, grid, &metrics, area, capacity);
            zong::distance(rope, *viewport, cursor_anchor, grid, page.len()).unwrap_or(0)
        }
    };
    let visible = page.len();

    // The margin the page settled on, so the renderer blanks the cell a
    // full-width reading covers only when that cell was reserved for it.
    let ruby_width = ruby_width_of(&page.iter().map(|(_, s, _)| s.clone()).collect::<Vec<_>>());

    // Lay the number band down as a band, before anything is drawn on it. In
    // every other editor a line number is separated from the text by position —
    // a gutter column the text never enters. Here the numbers sit *above* the
    // 縱, in the text's own columns, so position separates nothing and a bare
    // dim digit reads as a digit somebody typed. The colour is the gutter.
    if metrics.head_rows > 0 {
        let (gr, gg, gb) = config.theme.gutter;
        let ground = Style::default().bg(Color::Rgb(gr, gg, gb));
        let buf = frame.buffer_mut();
        for y in area.y..area.y + metrics.head_rows.min(area.height) {
            for x in area.x..area.x + area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(ground);
                }
            }
        }
    }

    let text_top = area.y + metrics.head_rows;
    let (sel_start, sel_end) = editor.selection();
    // Asked of the editor, not of the range: the selection always covers the
    // cursor's own grapheme, so a bare cursor would otherwise be drawn as a
    // one-character highlight and the word-tint overlay would never appear.
    let has_selection = editor.has_selection();
    let (sr, sg, sb) = config.theme.selection;
    let sel_style = Style::default().bg(Color::Rgb(sr, sg, sb)).fg(Color::White);
    let show_segmentation = editor.segmentation_visible();
    let seg_colors = config.theme.segmentation;
    let cursor_line = editor.cursor_line();
    let numbers = config.editor.line_numbers;

    // A reading is set back; a hung mark is punctuation and reads as the text's
    // own, so it keeps the text colour and is told apart by weight instead.
    let reading_style = Style::default().add_modifier(Modifier::DIM);
    let mark_style = Style::default().fg(Color::Rgb(0xb0, 0x8a, 0x6a));

    // Word ranges are per paragraph, and consecutive 縱 usually share one, so
    // segment each paragraph once as the page is walked.
    let mut segmented: Option<(usize, Vec<(usize, usize)>)> = None;

    let buf = frame.buffer_mut();
    for (zong, slots, x) in page.iter() {
        let x = *x;

        if numbers != LineNumbers::None && zong.starts_line() {
            let n = match numbers {
                LineNumbers::Relative if zong.line != cursor_line => {
                    zong.line.abs_diff(cursor_line)
                }
                _ => zong.line + 1,
            };
            // The cursor's own paragraph keeps its number bright, so the eye can
            // find where it is on a dense page.
            let (gr, gg, gb) = config.theme.gutter;
            let style = Style::default().bg(Color::Rgb(gr, gg, gb));
            // The cursor's own paragraph keeps its number bright against the
            // band, so the eye can find where it is on a dense page.
            let style = if zong.line == cursor_line {
                style
            } else {
                style.add_modifier(Modifier::DIM)
            };
            put_number(buf, x, area.y, metrics.head_rows, n, style);
        }

        let line_start = rope.line_to_char(zong.line);
        // Rows, not graphemes: a 縦中横 pair is one row holding two characters, a
        // ruby group is however many rows its reading needs, and the punctuation
        // is already rotated.
        for (slot, row) in slots.iter().cloned().enumerate() {
            let y = text_top + slot as u16;
            // The margin to the right of the 縱 carries both a reading and a
            // hung 句讀 mark. The mark wins the cell — it belongs against the
            // character it follows, and the reading has already given way
            // upward to leave that row free — and it is tinted apart from a
            // reading so the two are never mistaken for one another.
            let margin = row
                .mark
                .map(|m| (m, mark_style))
                .or_else(|| row.ruby.map(|r| (r, reading_style)));
            if let Some((glyph, style)) = margin {
                let mx = x + SLOT_WIDTH;
                // A full-width reading covers the cell after it, and that cell
                // is only ours to blank when the margin was widened for it.
                // With a one-cell margin the glyph leans into the gap instead —
                // which is what a hung 句讀 has always done — and blanking there
                // would rub out the 縱 to the right.
                if ruby_width >= 2 {
                    if let Some(cell) = buf.cell_mut((mx + 1, y)) {
                        cell.set_symbol(" ").set_style(style);
                    }
                }
                if let Some(cell) = buf.cell_mut((mx, y)) {
                    cell.set_symbol(&glyph.to_string()).set_style(style);
                }
            }
            let symbol = row.text;
            if symbol.is_empty() {
                continue;
            }
            let at = line_start + row.start;
            let len = row.end - row.start;

            let style = if has_selection && at < sel_end && at + len > sel_start {
                sel_style
            } else if show_segmentation {
                let column = at - line_start;
                let ranges = match &segmented {
                    Some((line, ranges)) if *line == zong.line => ranges,
                    _ => {
                        segmented = Some((zong.line, editor.segment_line(zong.line)));
                        &segmented.as_ref().unwrap().1
                    }
                };
                match ranges.iter().position(|&(a, b)| column >= a && column < b) {
                    Some(word) => {
                        let (r, g, b) = seg_colors[word % seg_colors.len()];
                        Style::default().bg(Color::Rgb(r, g, b))
                    }
                    None => Style::default(),
                }
            } else {
                Style::default()
            };

            // Hung right, so half-width characters line up as one edge running
            // down the 縱 beside the 漢字 rather than drifting to its left.
            put_slot_right(buf, x, y, &symbol, style);
        }
    }

    // The cursor, drawn last so it wins over a selection or a word tint.
    let cursor_x = page
        .get(cursor_column)
        .map(|(_, _, x)| *x)
        .or_else(|| page.last().map(|(_, _, x)| *x))
        .unwrap_or(area.x);
    let cursor_y =
        (text_top + cursor_pos.slot as u16).min((area.y + area.height).saturating_sub(1));
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
        let blank = buf
            .cell((cursor_x, caret_y))
            .is_none_or(|c| c.symbol().trim().is_empty());
        if blank {
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

/// The candidate panel's skin: Yume's 墨香 (Ink) theme, dark.
///
/// 墨香 is defined by **four numbers** — an ink and a paper colour for each
/// mode — with the other slots interpolated along a ladder between them
/// (`yume_core::themes::ink_ladder`). Reproducing the ladder rather than
/// pasting the resulting hexes keeps yumete's panel the same skin as the GUI
/// frontends' if either endpoint is ever retuned.
///
/// The green in the ink is deliberate and slight: R and G differ by about 5, so
/// it reads as ink with a hint of pine rather than grey-green. The paper is warm
/// rather than white. Dark mode is not the light pair swapped — the ground goes
/// deeper and the ink dimmer, or the panel glows at night.
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

    /// One rung of the ladder: `0` is pure ink, `1000` pure paper.
    ///
    /// Mixed in sRGB, not linear light, because the hand-tuned original was
    /// picked by eye in sRGB and mixing linearly comes out far lighter.
    fn step(self, t: u32) -> Color {
        let mix = |a: u8, b: u8| -> u8 {
            let (a, b) = (a as i64, b as i64);
            ((a * 1000 + (b - a) * t as i64 + 500) / 1000) as u8
        };
        Color::Rgb(
            mix(self.ink.0, self.paper.0),
            mix(self.ink.1, self.paper.1),
            mix(self.ink.2, self.paper.2),
        )
    }

    /// The panel's ground.
    pub fn paper(self) -> Color {
        self.step(1000)
    }
    /// The ring around the panel.
    pub fn border(self) -> Color {
        self.step(750)
    }
    /// A candidate.
    pub fn text(self) -> Color {
        self.step(0)
    }
    /// Numbers and the code — one shade back from the candidates.
    pub fn helper(self) -> Color {
        self.step(300)
    }
    /// The ground of the highlighted candidate, and the text on it.
    pub fn highlight(self) -> Color {
        self.step(0)
    }
    pub fn on_highlight(self) -> Color {
        self.step(1000)
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

    // The header is a 縱 like everything else in the panel: the code as typed
    // runs down the rightmost column, one character to a row and hung right, and
    // the 拆分 of the highlighted candidate follows it after a blank.
    let mut header: Vec<String> = graphemes(&ime.display_buffer()).map(String::from).collect();
    if let Some(comment) = candidates
        .get(highlight)
        .map(|c| c.comment.as_str())
        .filter(|c| !c.is_empty())
    {
        header.push(String::new());
        header.extend(graphemes(comment).map(String::from));
    }

    // Each column is the number, a blank row, then the candidate. The gap is
    // what stops the number reading as the first character of the word.
    // Column 0 is the header; the candidates run leftward from column 1, the
    // direction the text they are joining runs.
    let mut columns: Vec<Vec<String>> = vec![header];
    columns.extend(candidates.iter().enumerate().map(|(i, cand)| {
        // Number, a blank row, the candidate, then the keys still owed.
        let mut column = vec![index_mark(&config.panel.markers, i), String::new()];
        column.extend(graphemes(&cand.text).map(String::from));
        column.extend(graphemes(&cand.completion).map(String::from));
        column
    }));

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
            let style = if i == 0 {
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
