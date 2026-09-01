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
use yumete_core::{Editor, Mode, TextStore};
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
}

impl Metrics {
    /// Work out the geometry for a text area `height` rows tall in a buffer of
    /// `total_lines` paragraphs.
    ///
    /// The configured 縱 length is a typographic choice and is never *raised* to
    /// fill a tall terminal, only lowered when the terminal cannot hold it. One
    /// row beyond the 縱 is kept spare so the end-of-paragraph caret has
    /// somewhere to sit below a full 縱.
    pub fn new(config: &Config, height: u16, total_lines: usize) -> Metrics {
        let head_rows = number_rows(config.editor.line_numbers, total_lines);
        let rows = height.saturating_sub(head_rows) as usize;
        let zong_len = config.editor.zong_length.min(rows.saturating_sub(1)).max(1);
        Metrics {
            zong_len,
            pitch: SLOT_WIDTH + config.editor.zong_gap as u16,
            head_rows,
        }
    }

    /// How many 縱 fit across an area `width` cells wide. Only the gaps
    /// *between* 縱 count, so the leftmost one may sit flush against the edge.
    pub fn visible(&self, width: u16) -> usize {
        if width < SLOT_WIDTH {
            0
        } else {
            1 + ((width - SLOT_WIDTH) / self.pitch) as usize
        }
    }

    /// The left cell of the `k`-th visible 縱, counting from the right edge —
    /// `k = 0` is the rightmost, which is where reading starts.
    ///
    /// Saturating, so a pane too narrow to hold a single 縱 clamps to its left
    /// edge instead of wrapping around; callers still check [`visible`] before
    /// drawing there.
    ///
    /// [`visible`]: Metrics::visible
    pub fn x_of(&self, area: Rect, k: usize) -> u16 {
        (area.x + area.width)
            .saturating_sub(SLOT_WIDTH + k as u16 * self.pitch)
            .max(area.x)
    }
}

/// How many rows the paragraph-number header needs: two digits stack into one
/// row 縦中横-style, so a four-digit novel needs two rows.
fn number_rows(mode: LineNumbers, total_lines: usize) -> u16 {
    match mode {
        LineNumbers::None => 0,
        _ => (total_lines.max(1).to_string().len().div_ceil(2)).clamp(1, 3) as u16,
    }
}

/// The 縱 length in force for a terminal `height` rows tall (including the
/// status line), so the event loop can tell the editor where 縱 break before the
/// motions that depend on it run.
pub fn zong_length_for(config: &Config, height: u16, total_lines: usize) -> usize {
    Metrics::new(config, height.saturating_sub(1), total_lines).zong_len
}

/// Paint one 縱 slot: the grapheme in the left cell, the style across both, so
/// a selection or the cursor covers the whole square.
fn put_slot(buf: &mut Buffer, x: u16, y: u16, symbol: &str, style: Style) {
    // The trailing cell first: a wide symbol makes the renderer skip it, and a
    // half-width one leaves it as the styled other half of the slot.
    if let Some(cell) = buf.cell_mut((x + 1, y)) {
        cell.set_symbol(" ").set_style(style);
    }
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(symbol).set_style(style);
    }
}

/// Stack `text` into slots, pairing consecutive half-width characters into one
/// slot — 縦中横, the treatment vertical typesetting gives a short Latin run.
///
/// A slot is two cells wide, so two ASCII characters fit side by side exactly.
/// Without this a three-letter code hint would be three rows tall and the whole
/// panel would grow with it; with it, `jvy` reads as `jv` over `y`.
fn pack_slots(text: &str) -> Vec<String> {
    let mut slots: Vec<String> = Vec::new();
    for g in graphemes(text) {
        // Pair up only with a half-width neighbour that is not already paired.
        let pairable = str_width(g) == 1
            && slots
                .last()
                .is_some_and(|last| str_width(last) == 1 && !last.chars().any(|c| c == ' '));
        if pairable {
            slots.last_mut().expect("checked above").push_str(g);
        } else {
            slots.push(g.to_string());
        }
    }
    slots
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
    let metrics = Metrics::new(config, area.height, total_lines);
    let rope = buffer.rope();

    let cursor_pos = zong::position(rope, editor.cursor(), metrics.zong_len);
    let cursor_anchor = Anchor::from(cursor_pos);

    // Scroll leftward/rightward so the cursor's 縱 stays on the page, keeping
    // `scrolloff` 縱 of context on either side — the horizontal view's rule,
    // counted in 縱 instead of lines.
    //
    // The page is anchored at a paragraph rather than at a 縱 *number*: numbering
    // the cursor's 縱 would mean walking the document from the top on every
    // keystroke, which on a novel-length buffer is the whole novel. Everything
    // here is bounded by the width of the page instead.
    let visible = metrics.visible(area.width);
    let scrolloff = config.editor.scrolloff.min(visible.saturating_sub(1) / 2);
    let last_column = visible.saturating_sub(1);
    let cursor_column = match zong::distance(
        rope,
        *viewport,
        cursor_anchor,
        metrics.zong_len,
        last_column,
    ) {
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
            *viewport = zong::retreat(rope, cursor_anchor, metrics.zong_len, inset);
            zong::distance(
                rope,
                *viewport,
                cursor_anchor,
                metrics.zong_len,
                last_column,
            )
            .unwrap_or(0)
        }
    };
    let zongs = zong::zongs_from(rope, *viewport, metrics.zong_len, visible);

    let text_top = area.y + metrics.head_rows;
    let (sel_start, sel_end) = editor.selection();
    let has_selection = sel_start != sel_end;
    let (sr, sg, sb) = config.theme.selection;
    let sel_style = Style::default().bg(Color::Rgb(sr, sg, sb)).fg(Color::White);
    let show_segmentation = editor.segmentation_visible();
    let seg_colors = config.theme.segmentation;
    let cursor_line = editor.cursor_line();
    let numbers = config.editor.line_numbers;

    // Word ranges are per paragraph, and consecutive 縱 usually share one, so
    // segment each paragraph once as the page is walked.
    let mut segmented: Option<(usize, Vec<(usize, usize)>)> = None;

    let buf = frame.buffer_mut();
    for (k, zong) in zongs.iter().enumerate() {
        let x = metrics.x_of(area, k);

        if numbers != LineNumbers::None && zong.starts_line() {
            let n = match numbers {
                LineNumbers::Relative if zong.line != cursor_line => {
                    zong.line.abs_diff(cursor_line)
                }
                _ => zong.line + 1,
            };
            // The cursor's own paragraph keeps its number bright, so the eye can
            // find where it is on a dense page.
            let style = if zong.line == cursor_line {
                Style::default()
            } else {
                Style::default().add_modifier(Modifier::DIM)
            };
            put_number(buf, x, area.y, metrics.head_rows, n, style);
        }

        let line_start = rope.line_to_char(zong.line);
        let text = rope.slice(zong.start..zong.end).to_string();
        let mut offset = 0usize;
        // Slots, not graphemes: a 縦中横 pair is one row holding two characters,
        // and the punctuation is already rotated.
        for (slot, symbol) in zong::slot_text(&text).into_iter().enumerate() {
            let at = zong.start + offset;
            let len = symbol.chars().count();
            offset += len;

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

            put_slot(buf, x, text_top + slot as u16, &symbol, style);
        }
    }

    // The cursor, drawn last so it wins over a selection or a word tint.
    let cursor_x = metrics.x_of(area, cursor_column.min(last_column));
    let cursor_y =
        (text_top + cursor_pos.slot as u16).min((area.y + area.height).saturating_sub(1));
    if cursor_column < visible {
        if editor.mode() == Mode::Insert {
            // Insert: a caret at the boundary text will be pushed into. Turned a
            // quarter turn with the text, the bar of a horizontal editor becomes
            // a rule lying *across* the 縱 — drawn as an underline on the slot
            // above, so the character it sits between stays readable.
            let rule = Style::default()
                .add_modifier(Modifier::UNDERLINED)
                .fg(Color::Rgb(0, 89, 209));
            let above = cursor_y.saturating_sub(1);
            if cursor_y > area.y {
                for dx in 0..SLOT_WIDTH {
                    if let Some(cell) = buf.cell_mut((cursor_x + dx, above)) {
                        let style = cell.style().patch(rule);
                        cell.set_style(style);
                    }
                }
            } else {
                // Nothing above to underline at the head of a 縱: mark the slot
                // itself instead, still distinct from the Normal-mode block.
                for dx in 0..SLOT_WIDTH {
                    if let Some(cell) = buf.cell_mut((cursor_x + dx, cursor_y)) {
                        let style = cell.style().patch(rule);
                        cell.set_style(style);
                    }
                }
            }
        } else {
            // Normal: a solid block over the whole two-cell slot.
            let under = buf
                .cell((cursor_x, cursor_y))
                .map(|c| c.symbol().to_string())
                .unwrap_or_else(|| " ".to_string());
            let symbol = if under.trim().is_empty() { " " } else { &under };
            put_slot(
                buf,
                cursor_x,
                cursor_y,
                symbol,
                Style::default().add_modifier(Modifier::REVERSED),
            );
        }
    }
    (cursor_x, cursor_y)
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
mod ink {
    use ratatui::style::Color;

    /// 墨 — the dark theme's text colour.
    const STICK: (u8, u8, u8) = (0xCF, 0xC6, 0xA9);
    /// 紙 — the dark theme's ground.
    const PAPER: (u8, u8, u8) = (0x26, 0x2A, 0x27);

    /// One rung of the ladder: `0.0` is pure ink, `1.0` pure paper. Mixed in
    /// sRGB, not linear light, because the hand-tuned original was picked by eye
    /// in sRGB and mixing linearly comes out far lighter.
    const fn mix(a: u8, b: u8, t: u32) -> u8 {
        // Fixed point in thousandths, rounded — no floats, so the whole ladder
        // is a compile-time constant.
        let (a, b) = (a as i64, b as i64);
        ((a * 1000 + (b - a) * t as i64 + 500) / 1000) as u8
    }

    const fn step(t: u32) -> Color {
        Color::Rgb(
            mix(STICK.0, PAPER.0, t),
            mix(STICK.1, PAPER.1, t),
            mix(STICK.2, PAPER.2, t),
        )
    }

    /// The panel's ground.
    pub fn paper() -> Color {
        step(1000)
    }
    /// The ring around the panel.
    pub fn border() -> Color {
        step(750)
    }
    /// A candidate.
    pub fn text() -> Color {
        step(0)
    }
    /// Selection digits and the preedit — one shade back from the candidates.
    pub fn helper() -> Color {
        step(300)
    }
    /// The remaining-code hint, a shade back again.
    pub fn footer() -> Color {
        step(400)
    }
    /// The ground of the highlighted candidate, and the text on it.
    pub fn highlight() -> Color {
        step(0)
    }
    pub fn on_highlight() -> Color {
        step(1000)
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
    area: Rect,
    cursor_x: u16,
    cursor_y: u16,
) {
    let candidates = ime.page_candidates();
    if candidates.is_empty() {
        return;
    }
    let highlight = ime.highlight();
    // The panel packs its columns edge to edge. The text gets a gap between 縱
    // because the eye has to track down a long column and back up the next one;
    // a candidate is one or two characters, and the gap would only make an
    // already wide panel wider.
    let pitch = SLOT_WIDTH;

    // Each column is a stack of slots: the selection digit, the candidate, then
    // its remaining-code hint packed 縦中横 — one letter to a row would make the
    // panel as deep as the longest code. The 拆分 comment is left to the
    // horizontal panel, where it costs a line rather than a whole column.
    // The rightmost column is "what you typed, and what you would get": the
    // preedit, then — after a blank slot — the 拆分 of the highlighted
    // candidate, when the engine is annotating. Keeping it here rather than
    // beside each candidate is what lets the panel stay one row per character
    // instead of one column per decomposition.
    let mut preedit = pack_slots(&ime.display_buffer());
    if let Some(chaifen) = candidates
        .get(highlight)
        .map(|c| c.comment.as_str())
        .filter(|c| !c.is_empty())
    {
        preedit.push(String::new());
        preedit.extend(pack_slots(chaifen));
    }
    let columns: Vec<Vec<String>> = candidates
        .iter()
        .enumerate()
        .map(|(i, cand)| {
            let mut column = vec![(i + 1).to_string()];
            column.extend(graphemes(&cand.text).map(String::from));
            column.extend(pack_slots(&cand.completion));
            column
        })
        .collect();

    let depth = columns
        .iter()
        .map(|c| c.len())
        .chain(std::iter::once(preedit.len()))
        .max()
        .unwrap_or(1);
    let count = columns.len() + 1; // the candidates plus the preedit column
    let inner_w = count as u16 * SLOT_WIDTH + (count as u16 - 1) * (pitch - SLOT_WIDTH);
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
    let ground = Style::default().bg(ink::paper()).fg(ink::text());
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ink::border()).bg(ink::paper()))
        .style(ground);
    let inner = block.inner(panel);
    block.render(panel, frame.buffer_mut());

    let buf = frame.buffer_mut();
    let dim = ground.fg(ink::helper());
    let hint = ground.fg(ink::footer());
    let chosen = Style::default()
        .bg(ink::highlight())
        .fg(ink::on_highlight());

    // Column 0 (rightmost) is the preedit; the candidates follow leftward.
    // `None` means the column would run past the panel's left border, which is
    // what a pane too narrow for the whole page produces.
    let column_x = |k: usize| -> Option<u16> {
        let offset = (k as u16).checked_mul(pitch)?.checked_add(SLOT_WIDTH)?;
        let x = (inner.x + inner.width).checked_sub(offset)?;
        (x >= inner.x).then_some(x)
    };
    if let Some(x) = column_x(0) {
        for (slot, ch) in preedit.iter().enumerate() {
            if (slot as u16) < inner.height {
                put_slot(buf, x, inner.y + slot as u16, ch, dim);
            }
        }
    }
    for (i, column) in columns.iter().enumerate() {
        let Some(x) = column_x(i + 1) else {
            break;
        };
        for (slot, symbol) in column.iter().enumerate() {
            if slot as u16 >= inner.height {
                break;
            }
            // Digit on top, then the candidate, then the code still owed —
            // three rungs of the same ink so they separate by weight alone.
            let style = if i == highlight {
                chosen
            } else if slot == 0 {
                dim
            } else if slot <= graphemes(&candidates[i].text).count() {
                ground
            } else {
                hint
            };
            put_slot(buf, x, inner.y + slot as u16, symbol, style);
        }
    }
}
