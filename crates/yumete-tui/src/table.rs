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
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::Clear;
use ratatui::Frame;
use yumete_config::Config;
use yumete_core::editor::Editor;
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
fn widths(editor: &Editor, first: usize, rows: usize) -> Vec<usize> {
    let Some(view) = editor.table() else {
        return Vec::new();
    };
    let mut widths: Vec<usize> = view
        .schema
        .columns
        .iter()
        .map(|c| yumete_cjk::str_width(c.heading()).clamp(MIN_COLUMN, MAX_COLUMN))
        .collect();
    let lines = editor.current_buffer().line_count();
    for line in first..(first + rows).min(lines) {
        for (i, span) in editor.row_cells(line).into_iter().enumerate() {
            let Some(want) = widths.get_mut(i) else {
                // A ragged row has cells the schema does not know about. They
                // are drawn, because hiding them would hide the damage.
                continue;
            };
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
pub fn draw(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    area: Rect,
    viewport: &mut Viewport,
) -> (u16, u16) {
    let Some(view) = editor.table() else {
        return (area.x, area.y);
    };
    let lines = editor.current_buffer().line_count();
    let (cursor_row, cursor_cell) = editor.cell_position().unwrap_or((0, 0));

    // The header takes the top row and never scrolls, so a page of rows is one
    // shorter than the area.
    let head = u16::from(view.schema.header);
    let rows = (area.height.saturating_sub(head)) as usize;
    if rows == 0 {
        return (area.x, area.y);
    }

    // Vertical scroll: the ordinary rule, keeping the cursor's row on screen.
    let scrolloff = config.editor.scrolloff.min(rows / 2);
    let first_data = usize::from(view.schema.header);
    if cursor_row < viewport.top + scrolloff {
        viewport.top = cursor_row.saturating_sub(scrolloff);
    }
    if cursor_row + scrolloff >= viewport.top + rows {
        viewport.top = (cursor_row + scrolloff + 1).saturating_sub(rows);
    }
    viewport.top = viewport
        .top
        .min(lines.saturating_sub(1))
        .max(first_data.min(lines.saturating_sub(1)));

    let gutter = gutter_width(lines, config.editor.line_numbers) as u16;
    let room = area.width.saturating_sub(gutter) as usize;
    let widths = widths(editor, viewport.top, rows);
    scroll_columns(&widths, room, cursor_cell, &mut viewport.left);

    let (gr, gg, gb) = config.theme.gutter;
    let gutter_style = Style::default().bg(Color::Rgb(gr, gg, gb));
    let head_style = gutter_style
        .fg(Color::Rgb(0xd8, 0xc9, 0x9a))
        .add_modifier(Modifier::BOLD);
    let (sr, sg, sb) = config.theme.selection;
    let on = Style::default().bg(Color::Rgb(sr, sg, sb)).fg(Color::White);
    let text = Style::default();
    // The row the cursor is on, banded. Across twenty-eight columns the eye
    // loses which row it was reading the moment it looks sideways, and this is
    // the cheapest possible answer to that.
    let band = Style::default().bg(Color::Rgb(0x2c, 0x2e, 0x34));
    let quiet = Style::default().add_modifier(Modifier::DIM);
    // A row the schema cannot account for. Not an error to be refused — this
    // is the tool for mending it — so it is marked, not blocked.
    let torn = Style::default().fg(Color::Rgb(0xd8, 0x9a, 0x9a));

    frame.render_widget(Clear, area);
    let right = area.x + area.width;
    let buf = frame.buffer_mut();

    // The header: the column names, in the gutter's own colour so it reads as
    // furniture rather than as the first row of data.
    if head == 1 {
        for x in area.x..right {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_symbol(" ").set_style(gutter_style);
            }
        }
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
            put_text(buf, x, area.y, (x + w).min(right), column.heading(), style);
            x += w + GAP as u16;
        }
    }

    let mut caret = (area.x + gutter, area.y + head);
    for slot in 0..rows {
        let line = viewport.top + slot;
        if line >= lines {
            break;
        }
        let y = area.y + head + slot as u16;
        let ragged = editor.row_is_ragged(line);

        // The row number, frozen at the left: with the columns scrolled away
        // it is the only thing left that says which row this is.
        if gutter > 0 {
            let n = crate::gutter_text(line, cursor_row, gutter as usize, config.editor.line_numbers);
            let style = if ragged { gutter_style.patch(torn) } else { gutter_style };
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
            let here = line == cursor_row && i == cursor_cell;
            let style = if here {
                on
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
                // The caret sits where typing would land: the cell's start.
                caret = (x, y);
            }
            x += w + GAP as u16;
        }
        // A row with fewer cells than the schema says leaves the rest blank
        // rather than drawing columns that are not there.
        if ragged && cells.len() < view.schema.columns.len() {
            put_text(buf, x.min(right), y, right, "⟨缺⟩", quiet.patch(torn));
        }
    }
    caret
}
