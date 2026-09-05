//! Drawing a file as a grid — Feature #118.
//!
//! What makes twenty-eight columns of 拆分 readable is not colour, it is
//! **alignment**: the same field of every row starting in the same terminal
//! column, so the eye can run down one of them. Nothing else here matters as
//! much, and everything else here exists to serve it — the frozen header so a
//! column can be named, the frozen row numbers so a row can be, the column
//! scroll so a table wider than the window is still a grid rather than a
//! ragged edge.
//!
//! **Only what is on screen is measured.** A column is as wide as its widest
//! *visible* cell, which means a page of the file costs one pass over a page
//! of the file — not over 123,380 rows. The columns therefore breathe as you
//! scroll, which sounds worse than it is: a table of 漢字 is nearly uniform,
//! and a column that suddenly needs more room is telling you something true
//! about the rows you just reached.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use ratatui::Frame;
use yumete_config::Config;
use yumete_core::editor::Editor;
use yumete_core::table::Rules;
use yumete_core::TextStore;

use crate::{gutter_width, put_text};

/// Where the grid is scrolled to.
#[derive(Debug, Clone, Copy, Default)]
pub struct Viewport {
    /// The first row on screen.
    pub top: usize,
    /// The first column on screen. The header and the row numbers do not
    /// scroll with it — they are what tells you where you are.
    pub left: usize,
}

/// The narrowest a column is ever drawn, and the widest.
///
/// A floor because a column of one character still needs its heading to be
/// recognisable; a ceiling because one long cell must not push every column
/// after it off the window.
const MIN_COLUMN: usize = 3;
const MAX_COLUMN: usize = 32;

/// The gap between two columns.
const GAP: usize = 1;

/// Measure the visible rows and say how wide each column should be drawn.
///
/// `last` is the table's last row, not the file's: since 2026-09-05 the widget
/// is given `|` tables that are three lines of a chapter, and measuring the
/// chapter under them would make every column as wide as the prose.
fn widths(editor: &Editor, first: usize, rows: usize, last: usize) -> Vec<usize> {
    let Some(view) = editor.table() else {
        return Vec::new();
    };
    // A hidden column is drawn at no width at all — which is what `hidden`
    // buys: two of the 拆分表's twenty-eight are empty in all 123,380 rows and
    // were costing eight cells each across the whole page.
    let headings = editor.table_headings();
    let mut widths: Vec<usize> = view
        .schema
        .columns
        .iter()
        .enumerate()
        .map(|(i, c)| match c.hidden {
            true => 0,
            false => {
                let text = headings.get(i).map(String::as_str).unwrap_or(c.heading());
                yumete_cjk::str_width(text).clamp(MIN_COLUMN, MAX_COLUMN)
            }
        })
        .collect();
    for line in first..(first + rows).min(last + 1) {
        for (i, span) in editor.row_cells(line).into_iter().enumerate() {
            let Some(want) = widths.get_mut(i) else {
                // A ragged row has cells the schema does not know about. They
                // are drawn, because hiding them would hide the damage.
                continue;
            };
            if !view.schema.shows(i) {
                continue;
            }
            let text = editor.current_buffer().rope().line(line).to_string();
            let cell = yumete_core::table::cell_text(&text, span);
            *want = (*want).max(yumete_cjk::str_width(&cell).min(MAX_COLUMN));
        }
    }
    widths
}

/// Scroll the columns just enough to keep the cursor's cell whole on screen.
fn scroll_columns(widths: &[usize], room: usize, cell: usize, left: &mut usize) {
    if cell < *left {
        *left = cell;
        return;
    }
    // Widen the window leftwards from the cursor's column until one more would
    // not fit; that first column is where the page starts.
    let mut used = 0;
    let mut first = cell;
    loop {
        let w = widths.get(first).copied().unwrap_or(MIN_COLUMN) + GAP;
        if used + w > room && first != cell {
            first += 1;
            break;
        }
        used += w;
        if first == 0 || used >= room {
            break;
        }
        first -= 1;
    }
    *left = (*left).max(first);
}

/// The banded style on the cursor's row, the plain one elsewhere.
fn band_if(on: bool, band: Style, plain: Style) -> Style {
    if on {
        band
    } else {
        plain
    }
}

/// Draw the grid, returning where the terminal's caret belongs.
/// **Where a click landed, in a grid.**
///
/// A grid is not drawn the way prose is: its columns are padded to line up on
/// screen while the file behind them is ragged, and it scrolls sideways by
/// whole columns. So a click resolved as if the page were prose lands
/// somewhere else entirely — which is what it did, on every table, in a way
/// that looked random because the offset is the padding of every column to the
/// left of the pointer.
///
/// Laid out here from the same numbers `draw` lays it out from, and nothing is
/// remembered between frames: a click answered from last frame's layout is a
/// click that lands where the page *was*.
pub fn char_at(
    editor: &Editor,
    config: &Config,
    area: Rect,
    viewport: &Viewport,
    mouse: ratatui::crossterm::event::MouseEvent,
) -> Option<usize> {
    let view = editor.table()?;
    let lines = editor.current_buffer().line_count();
    let (_, bottom) = editor.table_row_span()?;
    let head = u16::from(view.schema.header) + u16::from(editor.table_numbers());
    let rows = area.height.saturating_sub(head) as usize;
    if rows == 0 || mouse.row < area.y {
        return None;
    }
    // The header row is not a row of the table: a click on it means the first
    // row under it, which is the one thing it could sensibly mean.
    let slot = (mouse.row.saturating_sub(area.y + head)) as usize;
    // Past the last row of the table is the last row of the table — a click on
    // the empty page under a three-line table must not land in the chapter.
    let line = (viewport.top + slot).min(bottom);
    let rope = editor.current_buffer().rope();
    let start = rope.line_to_char(line);
    let cells = editor.row_cells(line);
    if cells.is_empty() {
        return Some(start);
    }
    let gutter = crate::gutter_width(lines, config.editor.line_numbers) as u16;
    let widths = widths(editor, viewport.top, rows, bottom);
    let right = area.x + area.width;
    // Walk the columns the way they were drawn, and stop at the one the
    // pointer is in.
    let mut x = area.x + gutter;
    for (i, span) in cells.iter().enumerate().skip(viewport.left) {
        let w = widths.get(i).copied().unwrap_or(MIN_COLUMN) as u16;
        if x >= right {
            break;
        }
        let end = (x + w).min(right);
        if mouse.column < end || i + 1 == cells.len() {
            // Which character of the cell — counted in cells of the terminal,
            // because that is what was drawn.
            let want = mouse.column.saturating_sub(x) as usize;
            let mut column = 0;
            for at in span.0..span.1 {
                let c = rope.char(start + at);
                let cw = yumete_cjk::char_width(c);
                if want < column + cw {
                    return Some(start + at);
                }
                column += cw;
            }
            // Past the text: the cell's last character, or its start when the
            // cell is empty — and never past the end of the document, which the
            // last cell of the last line is one character short of.
            let at = start + span.1.saturating_sub(1).max(span.0);
            return Some(at.min(rope.len_chars().saturating_sub(1)));
        }
        x = end + if w == 0 { 0 } else { GAP as u16 };
    }
    Some((start + cells.last().map_or(0, |&(a, _)| a)).min(rope.len_chars().saturating_sub(1)))
}

pub fn draw(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    area: Rect,
    viewport: &mut Viewport,
    peek: Option<&yumete_core::editor::Pane>,
) -> (u16, u16) {
    let Some(view) = editor.table() else {
        return (area.x, area.y);
    };
    let lines = editor.current_buffer().line_count();
    // **The table's own lines, not the file's** (2026-09-05). `t t` now hands
    // this widget a `|` table that is three lines of a chapter, and the rows
    // below are the chapter — measured, scrolled through and clicked on, they
    // would be the chapter drawn as a grid.
    let Some((top_row, bottom_row)) = editor.table_row_span() else {
        return (area.x, area.y);
    };
    // A pane that is only being read is scrolled around the row it was opened
    // at, and marks it — it has no cursor and no cell of its own.
    let (cursor_row, cursor_cell) = match peek {
        None => editor.cell_position().unwrap_or((0, 0)),
        Some(pane) => {
            let rope = editor.current_buffer().rope();
            let line = rope.char_to_line(pane.cursor().min(rope.len_chars()));
            let at = pane.cursor() - rope.line_to_char(line);
            let cell = editor
                .row_cells(line)
                .iter()
                .position(|&(from, to)| at >= from && at <= to)
                .unwrap_or(0);
            (line, cell)
        }
    };

    // The header takes the top row and never scrolls, so a page of rows is one
    // shorter than the area — and the column numbers take one more, because
    // every numeric key here (`3gd`, `t20-20g`, `t1s2S4s`) names a column by
    // number and a 28-column 拆分表 gives no other way to count to 17.
    let numbers = u16::from(editor.table_numbers());
    let head = u16::from(view.schema.header) + numbers;
    let rows = (area.height.saturating_sub(head)) as usize;
    if rows == 0 {
        return (area.x, area.y);
    }

    // Vertical scroll: the ordinary rule, keeping the cursor's row on screen.
    let scrolloff = config.editor.scrolloff.min(rows / 2);
    let first_data = top_row;
    // **Where the cursor sits on a page is the editor's answer**, not this
    // surface's: a row off the page is a jump and lands in the middle, a step
    // off an edge scrolls by as little as it takes, and typewriter mode keeps
    // the row in the middle whatever it was. This used to be a third copy of
    // that rule, and it was the copy that never heard of `:typewriter`.
    let distance = (cursor_row >= viewport.top && cursor_row < viewport.top + rows)
        .then(|| cursor_row - viewport.top);
    if let Some(inset) = editor.page_inset(distance, rows.saturating_sub(1), scrolloff) {
        viewport.top = cursor_row.saturating_sub(inset);
    }
    viewport.top = viewport
        .top
        .min(bottom_row)
        .max(first_data.min(bottom_row));

    let gutter = gutter_width(lines, config.editor.line_numbers) as u16;
    let room = area.width.saturating_sub(gutter) as usize;
    let widths = widths(editor, viewport.top, rows, bottom_row);
    scroll_columns(&widths, room, cursor_cell, &mut viewport.left);

    let ink = match peek {
        None => crate::theme::Palette::of(config),
        // A rung back, all of it: the half that is only being read.
        Some(_) => crate::theme::Palette::of(config).faded(),
    };
    let gutter_style = ink.ground(yumete_config::rung::CHROME);
    let head_style = gutter_style.fg(ink.gold()).add_modifier(Modifier::BOLD);
    // A ground, and only a ground: the ink on a selected cell is left alone, so
    // a torn cell is still torn while you stand on it to mend it. It used to be
    // `fg(White)`, which is brighter than the ink and flattened every colour
    // underneath at the moment the writer was looking hardest.
    let on = Style::default().bg(ink.selection());
    // **A column is a band, and the page shows between them** — the seam is
    // the page itself, one cell wide, and that is the whole of the ruling.
    // Twenty-eight columns of one or two characters need it to read as a grid;
    // six wide ones do not, and there the seams are noise between the words —
    // so `:table rules off` gives every cell the page and the columns run
    // together.
    let rules = editor.table_rules();
    let text = match rules {
        Rules::Colour => ink.ground(yumete_config::rung::BAND),
        Rules::Off | Rules::Line(_) => ink.page(),
    };
    // A drawn rule is a *rule*: the ladder's own rung for one, the same one
    // the ruler's line and the 稿紙 ticks are drawn in.
    let stroke = match rules {
        Rules::Line(stroke) => Some((stroke.glyph(), ink.rule())),
        _ => None,
    };
    // The seam, and the ground of anything that is not a cell.
    let page = ink.page();
    // The row the cursor is on. Across twenty-eight columns the eye loses which
    // row it was reading the moment it looks sideways, and this is the cheapest
    // possible answer to that — a rung above the columns, so it still reads as
    // one row when every column is a band.
    let band = ink.ground(yumete_config::rung::HEAD);
    let quiet = Style::default().fg(ink.furniture());
    // A row the schema cannot account for. Not an error to be refused — this
    // is the tool for mending it — so it is marked, not blocked. 朱: the one
    // colour that is not a quantity of ink, because "wrong" is not one.
    let torn = Style::default().fg(ink.mark());

    frame.render_widget(Clear, area);
    let right = area.x + area.width;
    let buf = frame.buffer_mut();
    // The whole area, before anything is put on it. `Clear` gives the cells
    // back to the *terminal*, and a table rarely fills its area to the last
    // cell — the seams between columns, the rows past the end of the file —
    // so without this the page shows through in whatever colour the terminal
    // happens to be. Painting 墨香's light page onto a dark terminal made
    // every one of those gaps a black bar.
    for y in area.y..area.y + area.height {
        for x in area.x..right {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(page);
            }
        }
    }

    // **The column numbers**, above the header: one row of indices, so a column
    // can be *named*. Every numeric key in a grid — `3gd` for a column, `t20-20g`
    // for a cell, `t1s2S4s` for a sort — asks the reader to count columns, and
    // on the 拆分表 that is counting to seventeen by eye.
    //
    // Right-aligned in each column and a rung quieter than the heading: they
    // are a ruler, not a row.
    if numbers == 1 {
        for x in area.x..right {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_symbol(" ").set_style(gutter_style);
            }
        }
        let quiet = gutter_style.fg(ink.furniture());
        let mut x = area.x + gutter;
        for (i, _) in view.schema.columns.iter().enumerate().skip(viewport.left) {
            let w = widths[i] as u16;
            if x + w > right {
                break;
            }
            if w > 0 {
                let n = (i + 1).to_string();
                let at = x + w.saturating_sub(n.chars().count() as u16);
                let style = match i == cursor_cell {
                    true => quiet.fg(ink.gold()),
                    false => quiet,
                };
                put_text(buf, at, area.y, (x + w).min(right), &n, style);
            }
            x += w + if w == 0 { 0 } else { GAP as u16 };
        }
    }

    // The header: the column names, in the gutter's own colour so it reads as
    // furniture rather than as the first row of data.
    if view.schema.header {
        let head_y = area.y + numbers;
        for x in area.x..right {
            if let Some(cell) = buf.cell_mut((x, head_y)) {
                cell.set_symbol(" ").set_style(gutter_style);
            }
        }
        let headings = editor.table_headings();
        let mut x = area.x + gutter;
        for (i, column) in view.schema.columns.iter().enumerate().skip(viewport.left) {
            let w = widths[i] as u16;
            if x + w > right {
                break;
            }
            let style = if i == cursor_cell {
                head_style.add_modifier(Modifier::REVERSED)
            } else {
                head_style
            };
            let text = headings.get(i).map(String::as_str).unwrap_or(column.heading());
            put_text(buf, x, head_y, (x + w).min(right), text, style);
            // A hidden column takes no gap either — a column of
            // nothing is not a column with a space beside it.
            x += w + if w == 0 { 0 } else { GAP as u16 };
        }
    }

    let mut caret = (area.x + gutter, area.y + head);
    for slot in 0..rows {
        let line = viewport.top + slot;
        if line > bottom_row {
            break;
        }
        let y = area.y + head + slot as u16;
        let ragged = editor.row_is_ragged(line);

        // The row number, frozen at the left: with the columns scrolled away
        // it is the only thing left that says which row this is.
        if gutter > 0 {
            let n = crate::gutter_text(line, cursor_row, gutter as usize, config.editor.line_numbers);
            let style = match (ragged, peek.is_some() && line == cursor_row) {
                (true, _) => gutter_style.patch(torn),
                // The row the hit is on, so 「在哪一行」 is answered before the
                // eye has found the cell.
                (false, true) => gutter_style.fg(ink.mark()).add_modifier(Modifier::BOLD),
                (false, false) => gutter_style,
            };
            for x in area.x..area.x + gutter {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(style);
                }
            }
            put_text(buf, area.x, y, area.x + gutter, &n, style);
        }

        let source = editor.current_buffer().rope().line(line).to_string();
        let cells = editor.row_cells(line);
        let mut x = area.x + gutter;
        for (i, span) in cells.iter().enumerate().skip(viewport.left) {
            let w = widths.get(i).copied().unwrap_or(MIN_COLUMN) as u16;
            if x >= right {
                break;
            }
            // The cell the cursor is in — or, in a pane that is only being
            // read, the cell the hit is in. A grid marked *nothing* when it
            // was the one showing a search hit, which is the one case the
            // second work area exists for.
            let here = line == cursor_row && i == cursor_cell;
            let style = if here {
                match peek {
                    None => on,
                    // 朱's own wash, the same mark the prose page gives the
                    // hit you are standing on.
                    Some(_) => Style::default().bg(ink.wash()),
                }
            } else if ragged && i >= view.schema.columns.len() {
                band_if(line == cursor_row, band, text).patch(torn)
            } else {
                band_if(line == cursor_row, band, text)
            };
            // The cell's ground runs the column's full width, so the highlight
            // is a *cell* — a box you are inside — and not just its letters.
            for cx in x..(x + w).min(right) {
                if let Some(cell) = buf.cell_mut((cx, y)) {
                    cell.set_symbol(" ").set_style(style);
                }
            }
            let content = yumete_core::table::cell_text(&source, *span);
            put_text(buf, x, y, (x + w).min(right), &content, style);
            if here {
                // Where typing would land — which is *inside* the cell, not at
                // its start. Reading by character (`Tab`), and typing in Insert,
                // both move the cursor within the cell, and a caret pinned to
                // the cell's first 字 says they did not.
                let from = editor
                    .cell_span(line, i)
                    .map(|(a, _)| a)
                    .unwrap_or(usize::MAX);
                let into: String = content
                    .chars()
                    .take(editor.cursor().saturating_sub(from))
                    .collect();
                let step = (yumete_cjk::str_width(&into) as u16).min(w);
                caret = ((x + step).min(right.saturating_sub(1)), y);
            }
            // The seam after the cell: the page's own ground, except on the
            // row the cursor is on, where the band runs unbroken so the row
            // reads as one thing. A drawn rule goes in it — and keeps the
            // row's ground, so the cursor's band is not cut into pieces.
            let seam = if w == 0 { 0 } else { GAP as u16 };
            let ground = band_if(line == cursor_row, band, page);
            for cx in (x + w)..(x + w + seam).min(right) {
                if let Some(cell) = buf.cell_mut((cx, y)) {
                    match stroke {
                        Some((glyph, rule)) => cell.set_symbol(glyph).set_style(ground.fg(rule)),
                        None => cell.set_symbol(" ").set_style(ground),
                    };
                }
            }
            // A hidden column takes no gap either — a column of
            // nothing is not a column with a space beside it.
            x += w + seam;
        }
        // A row with fewer cells than the schema says leaves the rest blank
        // rather than drawing columns that are not there.
        for cx in x.min(right)..right {
            if let Some(cell) = buf.cell_mut((cx, y)) {
                cell.set_symbol(" ")
                    .set_style(band_if(line == cursor_row, band, page));
            }
        }
        if ragged && cells.len() < view.schema.columns.len() {
            put_text(buf, x.min(right), y, right, "⟨缺⟩", quiet.patch(torn));
        }
    }
    caret
}

/// How wide the detail panel is drawn.
///
/// Wide enough for a heading and a 拆分 sequence side by side, and no wider:
/// the grid is what the window is for.
const DETAIL_WIDTH: u16 = 30;

/// Split the panel off the right of an area, if there is one and it fits.
///
/// A table's panel goes down the *right*, because a row has twenty-eight
/// fields and that is a tall thing. Prose gets the same panel along the
/// **bottom** instead: a footnote is one short paragraph, and taking thirty
/// columns off a page of writing to show it would be paying the wrong price.
pub fn split_detail(editor: &Editor, config: &Config, area: Rect) -> (Rect, Option<Rect>) {
    if !editor.detail_visible() {
        return (area, None);
    }
    if editor.table().is_some_and(|t| t.takes_the_pane()) {
        if area.width < DETAIL_WIDTH {
            return (area, None);
        }
        let w = (editor.detail_width().unwrap_or(config.editor.detail_width) as u16)
            .min(area.width / 2)
            .max(12);
        let grid = Rect::new(area.x, area.y, area.width - w, area.height);
        let panel = Rect::new(area.x + area.width - w, area.y, w, area.height);
        return (grid, Some(panel));
    }
    let h = NOTE_HEIGHT;
    if area.height < h * 3 {
        return (area, None);
    }
    let page = Rect::new(area.x, area.y, area.width, area.height - h);
    let panel = Rect::new(area.x, area.y + area.height - h, area.width, h);
    (page, Some(panel))
}

/// How many rows a note's panel takes along the bottom.
const NOTE_HEIGHT: u16 = 4;

/// The panel down the right: every field of the row the cursor is in.
///
/// The grid can only show what fits across the window; this is where the rest
/// of a twenty-eight-column row goes, along with the fields that are worked
/// out rather than stored. It is a *reading* surface — nothing here is edited,
/// and nothing here scrolls out from under you as you move along the row.
pub fn draw_detail(frame: &mut Frame, editor: &Editor, config: &Config, area: Rect) {
    let Some(detail) = editor.detail() else {
        return;
    };
    let ink = crate::theme::Palette::of(config);
    let ground = ink.ground(yumete_config::rung::CHROME);
    let title = ground.fg(ink.gold()).add_modifier(Modifier::BOLD);
    let name = ground.fg(ink.quiet());
    let value = ground.fg(ink.text());
    let here = ground.fg(ink.gold()).add_modifier(Modifier::BOLD);
    // A component with no row of its own — for a 拆分表 that is a finding, not
    // a blank.
    let missing = ground.fg(ink.mark());

    frame.render_widget(Clear, area);
    let right = area.x + area.width;
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..right {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(ground);
            }
        }
    }
    // A rule down the edge where a side panel meets the grid. Along the bottom
    // the panel's own ground is already the boundary, and a rule there would
    // cost a row of a four-row panel.
    if editor.table().is_some_and(|t| t.takes_the_pane()) {
        for y in area.y..area.y + area.height {
            if let Some(cell) = buf.cell_mut((area.x, y)) {
                cell.set_symbol("│").set_style(name);
            }
        }
    }

    let left = area.x + 2;
    // A note's panel is short and wide: its title sits on the same row as its
    // text, because there is no room to spend a row on a heading.
    if editor.table().is_none() {
        put_text(buf, left, area.y, right, &detail.title, title);
        let body = detail
            .rows
            .first()
            .and_then(|(_, v)| v.as_deref())
            .unwrap_or("");
        let indent = left + yumete_cjk::str_width(&detail.title) as u16 + 2;
        // Wrapped by hand across the panel's rows — a long note is the case
        // this exists for, so cutting it off would defeat the point.
        let mut x = indent;
        let mut y = area.y;
        for word in yumete_cjk::graphemes(body) {
            let w = yumete_cjk::grapheme_width(word).max(1) as u16;
            if x + w > right {
                x = left;
                y += 1;
                if y >= area.y + area.height {
                    break;
                }
            }
            put_text(buf, x, y, right, word, value);
            x += w;
        }
        if let Some((_, Some(at))) = detail.links.first() {
            let label = format!("第 {} 行", at + 1);
            let x = right.saturating_sub(yumete_cjk::str_width(&label) as u16 + 2);
            put_text(buf, x, area.y, right, &label, name);
        }
        return;
    }
    put_text(buf, left, area.y, right, &detail.title, title);
    let mut y = area.y + 2;
    // The 部件 list first, because it is what the panel is *read for* — and it
    // used to be drawn last, under twenty-eight mostly-blank fields, which on
    // any real window meant not drawn at all.
    if !detail.links.is_empty() {
        put_text(buf, left, y, right, "部件（Enter 跟過去）", name);
        y += 1;
        let mut x = left;
        for (c, line) in &detail.links {
            let label = match line {
                Some(n) => format!("{c} {}", n + 1),
                None => format!("{c} —"),
            };
            let w = yumete_cjk::str_width(&label) as u16 + 2;
            if x + w > right {
                x = left;
                y += 1;
                if y >= area.y + area.height {
                    return;
                }
            }
            put_text(
                buf,
                x,
                y,
                right,
                &label,
                if line.is_some() { value } else { missing },
            );
            x += w;
        }
        y += 2;
    }
    // **Scrolled to the field you are in.** Every column is listed now, empty
    // ones included, so a 28-column row is longer than the panel — and the one
    // field that must never be off the bottom is the one the cursor is in.
    // Scrolled by the panel itself, from where the cursor is, rather than by a
    // key: a reading surface with a scrollbar is a surface with a mode.
    let room = (area.y + area.height).saturating_sub(y) as usize;
    let at = detail
        .rows
        .iter()
        .position(|(field, _)| *field == detail.here)
        .unwrap_or(0);
    let first = at.saturating_sub(room.saturating_sub(1));
    let column = detail
        .rows
        .iter()
        .map(|(field, _)| yumete_cjk::str_width(field) as u16 + 1)
        .max()
        .unwrap_or(10)
        .clamp(10, 20);
    for (field, text) in detail.rows.iter().skip(first) {
        if y >= area.y + area.height {
            return;
        }
        let style = if *field == detail.here { here } else { name };
        put_text(buf, left, y, right, field, style);
        // **A field the row does not have**, said in 朱 and in words: a blank
        // here used to mean either 「this cell is empty」 or 「this row is short
        // by twenty-four columns」, and telling those apart is most of what
        // this panel is for.
        let (text, style) = match text {
            Some(text) => (text.as_str(), if *field == detail.here { here } else { value }),
            None => ("⟨缺⟩", missing),
        };
        // **The values line up past the longest name**, rather than at a fixed
        // ten cells: the names carry their column number now (「12 pinyin」),
        // and a name longer than the guess ran straight into its own value.
        // Capped, so one long name does not push every value off the panel.
        let indent = left + column;
        if indent < right {
            put_text(buf, indent, y, right, text, style);
        }
        y += 1;
    }
}
