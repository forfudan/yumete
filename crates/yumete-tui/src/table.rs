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
use yumete_core::say;
use yumete_core::sidebar::Side;
use yumete_core::table::Rules;

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
///
/// **The ceiling is `t w`'s to lift** (#283, 2026-09-07). It used to be the
/// law here, with no key against it: a cell wider than 32 was cut off with
/// nothing to say it had been, no width the reader could ask for, and — since
/// the grid draws a cell and not a line — no way to walk into the tail even in
/// Insert. So the cap is what folding *means* in the grid, `t w` turns it off,
/// and the cell the caret stands in is measured whole whichever way the switch
/// is set. That is [`yumete_core::editor::Editor::cell_folds`]'s other half:
/// the prose page folds by hiding characters, the grid by drawing narrow.
const MIN_COLUMN: usize = 3;
const MAX_COLUMN: usize = 32;

/// The gap between two columns.
const GAP: usize = 1;

/// Measure the visible rows and say how wide each column should be drawn.
///
/// Warning: **The visible rows, on purpose — not a bug to fix**
/// (2026-09-17). Measuring the whole table lets one very long cell a thousand
/// rows away widen its column on every screen, and push the other columns off
/// the right edge. The price — columns narrowing and widening as that row
/// scrolls in and out — is the one chosen. A 2026-09-15 review
/// logged it as 「列寬跟着可見範圍變，滾動時整表橫跳」; it was answered, and
/// the answer is this.
///
/// `last` is the table's last row, not the file's: since 2026-09-05 the widget
/// is given `|` tables that are three lines of a chapter, and measuring the
/// chapter under them would make every column as wide as the prose.
fn widths(
    editor: &Editor,
    anchor: usize,
    first: usize,
    rows: usize,
    last: usize,
    room: usize,
    caret: (usize, usize),
) -> Vec<usize> {
    let Some(view) = editor.table() else {
        return Vec::new();
    };
    // How wide a column may be drawn. Folding on, that is the cap; folding
    // off, it is the window — a column wider than the window buys nothing,
    // since the grid scrolls by whole columns and the rest could never be
    // reached.
    let ceiling = match editor.cell_folds() {
        true => MAX_COLUMN,
        false => room.max(MIN_COLUMN),
    };
    // A hidden column is drawn at no width at all — which is what `hidden`
    // buys: two of the 拆分表's twenty-eight are empty in all 123,380 rows and
    // were costing eight cells each across the whole page.
    let headings = editor.table_headings_at(anchor);
    // **This** table's columns, not the schema's: `t ]` into the next table of
    // a document lands in one with a different number of them.
    let mut widths: Vec<usize> = (0..editor.table_column_count_at(anchor))
        .map(|i| match view.schema.columns.get(i).is_some_and(|c| c.hidden) {
            true => 0,
            false => {
                let named = view.schema.columns.get(i).map(|c| c.heading());
                let text = headings.get(i).map(String::as_str).or(named).unwrap_or("");
                yumete_cjk::str_width(text).clamp(MIN_COLUMN, ceiling)
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
            let text = editor.row_source(line);
            // **Measured as drawn**: the quotes are off and a line break shows
            // as `␊`, so the column is as wide as what is on the page.
            let cell = editor.cell_shown(&text, span);
            // **The cell being typed in is not folded** — the prose page's own
            // rule (`folds_open_at`), and now kept the same way here: measure
            // that one cell against the window instead of the cap.
            //
            // It used to open for the caret in Normal too, and that was the
            // page's rule read one word too widely. 2026-09-11:
            // 「我觉得这个有一个问题就是列宽容易跳。所以我建议这个和 tf 保持
            // 一致（光标不自动展开，insert 模式展开这个格子），这样我们的逻辑
            // 更简洁、一致，而且不用每个单元格都重算这一列的列宽。」Right on
            // all three counts: every `l` across a row of long cells was a new
            // width for that column, so the grid shifted sideways under a key
            // that only meant 「next cell」 — and reading a cell whole is what
            // the panel down the right is for, which needs no width at all.
            //
            // **And not under 折行** even then, where nothing is cut in the
            // first place: widening the column would swell it to the window to
            // show what the rows underneath already show.
            let typing = editor.mode() == yumete_core::input::Mode::Insert;
            let ceiling = match typing && (line, i) == caret && !editor.cell_wrap() {
                true => room.max(MIN_COLUMN),
                false => ceiling,
            };
            // A cell the writer broke into lines is as wide as its widest
            // line, not as wide as all of them run together.
            let wide = cell
                .split('\n')
                .map(yumete_cjk::str_width)
                .max()
                .unwrap_or(0);
            *want = (*want).max(wide.min(ceiling));
        }
    }
    widths
}

/// **A cell's drawn lines** (2026-10-20 作者定).
///
/// One per line the writer wrote. The break that ended each of them is drawn
/// as `␊` and then honoured — a soft wrap is a consequence of a narrow window
/// and a hard newline is the author's own, so `t a` governs the first and has
/// no say over the second. Each written line is then soft-wrapped on its own
/// when `t a` is on.
fn cell_lines(shown: &str, width: usize, wrap: bool) -> Vec<String> {
    let hard: Vec<&str> = shown.split('\n').collect();
    let last = hard.len() - 1;
    hard.into_iter()
        .enumerate()
        .flat_map(|(i, piece)| {
            let piece = match i == last {
                true => piece.to_string(),
                false => format!("{piece}{}", yumete_cjk::LINE_FEED_PICTURE),
            };
            match wrap && width > 0 {
                true => wrapped(piece.trim_end(), width),
                false => vec![piece],
            }
        })
        .collect()
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
    let (_, bottom) = editor.table_row_span()?;
    let head = u16::from(view.schema.header) + u16::from(editor.table_numbers());
    let rows = area.height.saturating_sub(head) as usize;
    if rows == 0 || mouse.row < area.y {
        return None;
    }
    // 表格視圖没有診斷那一欄：一張 CSV 不會有語言服務器。
    let gutter = crate::gutter_width(
        bottom.saturating_sub(editor.table_row_base()) + 1,
        config.editor.line_numbers,
        false,
    ) as u16;
    // The same numbers `draw` measured with, the caret's own cell included:
    // that cell is drawn whole whatever the cap says, so a hit test that did
    // not know it lands one column out for every column to its right.
    let room = area.width.saturating_sub(gutter) as usize;
    let widths = widths(
        editor,
        editor.cursor_line(),
        viewport.top,
        rows,
        bottom,
        room,
        editor.cell_position().unwrap_or((0, 0)),
    );
    let right = area.x + area.width;
    let rope = editor.current_buffer().rope();
    // **Which row, counting the lines each one is drawn on** (`t a`). Under
    // 折行 a row of the file is as many rows of the window as its tallest
    // cell needs, so a click resolved by dividing the page into equal rows
    // lands further and further off with every wrapped row above it.
    let wrap = editor.cell_wrap();
    let lines_of = |line: usize| -> Vec<Vec<String>> {
        let source = editor.row_source(line);
        editor
            .row_cells(line)
            .iter()
            .enumerate()
            .map(|(i, span)| {
                let w = widths.get(i).copied().unwrap_or(MIN_COLUMN);
                cell_lines(&editor.cell_shown(&source, *span), w, wrap)
            })
            .collect()
    };
    let tall_of = |parts: &[Vec<String>]| -> usize {
        // **A hard break counts whether `t a` is on or not** (2026-10-20): the
        // soft wrap is a consequence of a narrow window and `t a` governs it,
        // while a line break the writer typed is the writer's. With `t a` off
        // a cell written over two lines still takes two rows — dropping the
        // second was the one answer worse than either of the others, since the
        // text was then nowhere on the page.
        let mut x = area.x + gutter;
        let mut tall = 1;
        for (i, part) in parts.iter().enumerate().skip(viewport.left) {
            if x >= right {
                break;
            }
            tall = tall.max(part.len());
            let w = widths.get(i).copied().unwrap_or(MIN_COLUMN) as u16;
            x += w + if w == 0 { 0 } else { GAP as u16 };
        }
        tall
    };
    // The header row is not a row of the table: a click on it means the first
    // row under it, which is the one thing it could sensibly mean. Past the
    // last row of the table is the last row of the table — a click on the
    // empty page under a three-line table must not land in the chapter.
    let mut line = viewport.top;
    let mut top = area.y + head;
    let mut parts = lines_of(line);
    let mut on = 0usize;
    while line < bottom {
        let tall = tall_of(&parts) as u16;
        if mouse.row < top + tall {
            on = mouse.row.saturating_sub(top) as usize;
            break;
        }
        top += tall;
        line += editor.record_height(line).max(1);
        parts = lines_of(line);
    }
    let start = rope.line_to_char(line);
    let cells = editor.row_cells(line);
    if cells.is_empty() {
        return Some(start);
    }
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
            // because that is what was drawn, and from the line of the cell
            // the pointer is on rather than from its first.
            let want = mouse.column.saturating_sub(x) as usize;
            let held = parts.get(i).cloned().unwrap_or_default();
            // **…plus what the cell scrolled off its own front** ([`slid_by`]).
            let want = want + slid_by(editor, line, i, on, &held, w as usize);
            let before: usize = held.iter().take(on).map(|l| l.chars().count()).sum();
            let from = span.0 + before;
            let to = match held.get(on) {
                Some(text) => (from + text.chars().count()).min(span.1),
                // A cell with nothing on this line: the click means where its
                // writing stopped.
                None => span.1,
            };
            // Warning: **格數是字簇的，位置是 `char` 的**（#422）。走的是檔裏的字符位置
            // （點到哪就回答哪一個 `char`），而「這個位置佔幾格」只有整個字簇答得
            // 出來：`cells_per_char` 把字簇的格數記在它第一個 `char` 上。逐字加的
            // 話，一格裏有一個 `Warning: `，它後面點哪裏都偏一個字。
            let text: Vec<char> = (from..to).map(|at| rope.char(start + at)).collect();
            let cells = yumete_cjk::cells_per_char(&text);
            let mut column = 0;
            for (k, at) in (from..to).enumerate() {
                let cw = cells[k];
                if want < column + cw {
                    return Some(start + at);
                }
                column += cw;
            }
            // Past the text: the cell's last character, or its start when the
            // cell is empty — and never past the end of the document, which the
            // last cell of the last line is one character short of.
            let at = start + to.saturating_sub(1).max(span.0);
            return Some(at.min(rope.len_chars().saturating_sub(1)));
        }
        x = end + if w == 0 { 0 } else { GAP as u16 };
    }
    Some((start + cells.last().map_or(0, |&(a, _)| a)).min(rope.len_chars().saturating_sub(1)))
}

// Warning: **`for i in 視窗左邊..widths.len()` 不改成 `enumerate`**（2026-10-01）：
// 走的是**一段**欄（從橫向捲到的那一欄起），而 `i` 本身就是欄號——標尺與欄名
// 都要拿它去對 `cursor_cell`。`enumerate` 只會多一次 `skip` 再多一個偏移量。
#[allow(clippy::needless_range_loop)]
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
    // **The table's own lines, not the file's** (2026-09-05). `t t` now hands
    // this widget a `|` table that is three lines of a chapter, and the rows
    // below are the chapter — measured, scrolled through and clicked on, they
    // would be the chapter drawn as a grid.
    // **Whose table**: a pane that is only being read is parked on a line of
    // its own, and every one of these questions is about the table *that* line
    // is in — its rows, its headings, how many columns it has.
    let anchor = match peek {
        None => editor.cursor_line(),
        Some(pane) => {
            let rope = editor.current_buffer().rope();
            rope.char_to_line(pane.cursor().min(rope.len_chars()))
        }
    };
    let Some((top_row, bottom_row)) = editor.table_row_span_at(anchor) else {
        return (area.x, area.y);
    };
    // A pane that is only being read is scrolled around the row it was opened
    // at, and marks it — it has no cursor and no cell of its own.
    let (cursor_row, cursor_cell) = match peek {
        None => {
            let (row, cell) = editor.cell_position().unwrap_or((0, 0));
            (editor.table_row_start(row), cell)
        }
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
    // every numeric key here (`t3/`, `t20,20g`, `t1s2S4s`) names a column by
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
    // that rule, and it was the copy that never heard of `:view-typewriter`.
    let distance = (cursor_row >= viewport.top && cursor_row < viewport.top + rows)
        .then(|| cursor_row - viewport.top);
    if let Some(inset) = editor.page_inset(distance, rows.saturating_sub(1), scrolloff) {
        viewport.top = cursor_row.saturating_sub(inset);
    }
    viewport.top = viewport
        .top
        .min(bottom_row)
        .max(first_data.min(bottom_row));
    // **The page starts at a record, never inside one** (2026-10-20 作者報的).
    // The scroll above counts lines, so stepping down past a record that holds
    // a line break landed `top` on its continuation line — and every row from
    // there was read from the middle of a record: the cell's second line became
    // column 1, the detail panel showed it as the row, and the first column's
    // width was measured against it.
    viewport.top = editor.table_row_start(viewport.top);

    // **The gutter numbers this table's rows**, not the file's lines: the
    // window is bounded (`Editor::hold_the_pane`), so inside it every number
    // — the gutter's, the status line's, `t20g`'s — means the same thing.
    let base = editor.table_row_base();
    let gutter =
        gutter_width(bottom_row.saturating_sub(base) + 1, config.editor.line_numbers, false) as u16;
    let room = area.width.saturating_sub(gutter) as usize;
    let widths = widths(
        editor,
        anchor,
        viewport.top,
        rows,
        bottom_row,
        room,
        (cursor_row, cursor_cell),
    );
    // How many columns this table has — what a row is measured against when
    // asking whether it is ragged, and where the drawn extras start.
    let columns = editor.table_column_count_at(anchor);
    scroll_columns(&widths, room, cursor_cell, &mut viewport.left);

    let ink = match peek {
        None => crate::theme::Palette::of(config),
        // A rung back, all of it: the half that is only being read.
        Some(_) => crate::theme::Palette::of(config).faded(),
    };
    let gutter_style = ink.ground(yumete_config::rung::CHROME);
    let head_style = gutter_style.fg(ink.gold()).add_modifier(Modifier::BOLD);
    // A ground, and only a ground: the ink under it is left alone, so a torn
    // cell is still torn while you stand on it to mend it. It used to be
    // `fg(White)`, which is brighter than the ink and flattened every colour
    // underneath at the moment the writer was looking hardest.
    //
    // Warning: **This is the selection now, not the cell** (2026-09-12). It used to
    // paint the whole cell the caret was in — and since it painted it with the
    // *selection's* rung, a selection inside that cell had no colour left to
    // be drawn in: press `w` in a grid and the word you just took looked
    // exactly like the cell around it. There is no room on the ladder for a
    // third ground either (`HEAD` 815 and `SELECTION` 700 are 5% apart at the
    // paper end, and the ladder's own note says 1.11–1.20 is invisible).
    //
    // The cell is still a box; the box just moved off the ground and into the
    // seam beside it — two 金 rules, drawn further down where `here` is known.
    // Which cell the caret is in was already said twice besides (the column's
    // number and its heading are both lit at the top of the grid), while *what
    // is selected* was said nowhere.
    let on = Style::default().bg(ink.selection());
    // **A column is a band, and the page shows between them** — the seam is
    // the page itself, one cell wide, and that is the whole of the ruling.
    // Twenty-eight columns of one or two characters need it to read as a grid;
    // six wide ones do not, and there the seams are noise between the words —
    // so `:table-rules off` gives every cell the page and the columns run
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
    // Warning: **光標那一行沒有底色了**（2026-10-17 作者報的）。從前它鋪一層
    // `rung::HEAD`，而選中的那一塊鋪的是它隔壁那一鬥——量出來：選中在平常那幾
    // 行上是 1.98:1，落在這一條帶子上只剩 1.37:1，而這個檔自己的註釋寫着
    // 1.11–1.20 就是看不見。正在編輯的那一行恰恰是看不清選中的那一行。
    //
    // 梯子上擠不出第三層（`HEAD` 815 與 `SELECTION` 700 在紙那一頭只差 5%），
    // 所以照 2026-09-12 給**格**的那一刀再切一次：底色歸選中一個人，「在哪一
    // 行」挪到邊上去說——行號點成金的，和欄的標題同一種說法。試算表都是這麼
    // 分的（Excel／Numbers／Sheets：選中吃底色，當前格靠兩頭的表頭點亮）。
    // **Every other row** (#449). The columns are banded, and banding the
    // columns alone leaves nothing saying where one row ends — worst exactly
    // where this mode is most useful: with 折行 on, a cell that wraps to three
    // lines has to be tellable from the row under it.
    //
    // Warning: **Which row this is, and which ground it gets, are both somebody
    // else's to say** (#458, #478). A `|` table read in a chapter and the same
    // table read in this window are the same table, and 「to tf tb tt 的隔行底
    // 色应该是统一的参数」.
    //
    // The row number comes from the block scanner — the same count the page
    // renderer bands by — rather than from `line - base + 2`, which assumed
    // every table has a `---` rule under its header. A pipe table written
    // without one had its parity inverted here and nowhere else.
    let banded = |line: usize, plain: Style| {
        let nth = match editor.block_of(line) {
            yumete_core::markdown::Block::Table { nth, .. } => nth,
            // Not a `|` table at all — a delimited file has no header rule and
            // no scanner to ask, so the base row is row 0 of its body.
            //
            // Warning: **the record's number, not the line's** (2026-10-21 作者報的:
            // 「the background color of this row and the next two rows are
            // indifferent」). A record that runs over three lines moves every
            // row after it three lines down and only one row down, so counting
            // lines put the stripes out of step from there to the end of the
            // file — and two rows in a row came out the same colour.
            _ => editor.table_row_index(line) + 2,
        };
        match crate::table_row_depth(nth) {
            // Over whatever this row sits on — in this window that is the page,
            // but the arithmetic is the same one the chapter's tables use.
            Some(depth) => {
                let beneath = plain.bg.unwrap_or(ink.paper());
                plain.bg(ink.over(beneath, depth))
            }
            None => plain,
        }
    };
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
    // can be *named*. Every numeric key in a grid — `t3/` for a column, `t20,20g`
    // for a cell, `t1s2S4s` for a sort — asks the reader to count columns, and
    // on the 拆分表 that is counting to seventeen by eye.
    //
    // Right-aligned in each column and a rung quieter than the heading: they
    // are a ruler, not a row — but a rung **darker than the furniture**,
    // because a ruler is read rather than glanced at (#232).
    if numbers == 1 {
        for x in area.x..right {
            if let Some(cell) = buf.cell_mut((x, area.y)) {
                cell.set_symbol(" ").set_style(gutter_style);
            }
        }
        let quiet = gutter_style.fg(ink.ruler());
        let mut x = area.x + gutter;
        for i in viewport.left..widths.len() {
            let w = widths[i] as u16;
            // **Stop where the rows stop**, not one column earlier. A column
            // wider than the room left over is still drawn — the rows clip it
            // and `t w` makes that the ordinary case — so breaking on
            // `x + w > right` took the ruler and the name off the one column
            // the reader had just asked to see whole.
            if x >= right {
                break;
            }
            if w > 0 {
                let n = (i + 1).to_string();
                let wide = n.chars().count() as u16;
                // Right-aligned in the column, or at the edge of what is left
                // of it: a number past the window is no number at all.
                let at = (x + w.saturating_sub(wide)).min(right.saturating_sub(wide)).max(x);
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
        let headings = editor.table_headings_at(anchor);
        let mut x = area.x + gutter;
        for i in viewport.left..widths.len() {
            let w = widths[i] as u16;
            // The rows' own stop, for the reason on the number row above.
            if x >= right {
                break;
            }
            let style = if i == cursor_cell {
                head_style.add_modifier(Modifier::REVERSED)
            } else {
                head_style
            };
            let named = view.schema.columns.get(i).map(|c| c.heading());
            let text = headings.get(i).map(String::as_str).or(named).unwrap_or("");
            put_text(buf, x, head_y, (x + w).min(right), text, style);
            // A hidden column takes no gap either — a column of
            // nothing is not a column with a space beside it.
            x += w + if w == 0 { 0 } else { GAP as u16 };
        }
    }

    // **A row is as tall as its tallest cell** (`t a`, 2026-09-07). 摺起 takes
    // the tail off the page and stands a `>` where it stopped; 折行 draws it
    // **underneath**, inside the cell's own column, so a row grows downwards
    // and the rows under it move down with it. Only the grid can offer this:
    // the prose page's rows come from the shared wrap layer, which has no
    // hanging indent to draw a cell's second line under the cell.
    let wrap = editor.cell_wrap();
    // One row's cells, each as the lines it is drawn on — one line each
    // unless 折行 is on.
    let lines_of = |line: usize| -> (Vec<(usize, usize)>, Vec<Vec<String>>) {
        let source = editor.row_source(line);
        let cells = editor.row_cells(line);
        let parts = cells
            .iter()
            .enumerate()
            .map(|(i, span)| {
                let w = widths.get(i).copied().unwrap_or(MIN_COLUMN);
                // **Trailing spaces do not buy a line.** A cell of a table
                // squared up in the file carries its column's whole width as
                // padding, so `短` plus thirty spaces is thirty-three cells
                // wide and wrapped to a second line holding nothing. Handled
                // inside `cell_lines`, per line the writer wrote.
                cell_lines(&editor.cell_shown(&source, *span), w, wrap)
            })
            .collect();
        (cells, parts)
    };
    // How tall a row is drawn — counting only the columns that are on screen,
    // since a cell scrolled off the right edge must not buy rows nobody sees.
    let height_of = |parts: &[Vec<String>]| -> usize {
        // The same rule as `tall_of` in the click map: a hard break counts
        // whether `t a` is on or not, so the two surfaces agree about how many
        // rows a record takes.
        let mut x = area.x + gutter;
        let mut tall = 1;
        for (i, part) in parts.iter().enumerate().skip(viewport.left) {
            if x >= right {
                break;
            }
            tall = tall.max(part.len());
            let w = widths.get(i).copied().unwrap_or(MIN_COLUMN) as u16;
            x += w + if w == 0 { 0 } else { GAP as u16 };
        }
        tall
    };
    // **The cursor's row is kept whole on the page.** The ordinary scroll rule
    // counts rows of the file; with 折行 on, one row of the file can be five
    // rows of the window, so the page starts low enough that the row the
    // cursor is in — all of it — still fits.
    if wrap {
        while viewport.top < cursor_row {
            let mut used = 0usize;
            let mut line = viewport.top;
            let fits = loop {
                if line > bottom_row {
                    break true;
                }
                let (_, parts) = lines_of(line);
                let tall = height_of(&parts);
                if line == cursor_row {
                    break used + tall <= rows;
                }
                used += tall;
                if used >= rows {
                    break false;
                }
                line += editor.record_height(line).max(1);
            };
            if fits {
                break;
            }
            viewport.top += 1;
        }
    }

    let mut caret = (area.x + gutter, area.y + head);
    let floor = area.y + area.height;
    let mut y = area.y + head;
    let mut line = viewport.top;
    while y < floor && line <= bottom_row {
        let ragged = editor.row_is_ragged(line);
        let (cells, parts) = lines_of(line);
        let tall = height_of(&parts).min((floor - y) as usize).max(1);
        // **A record's continuation lines are not rows of their own**
        // (2026-10-20 作者定): a field holding a line break is one record over
        // several lines, so the step is the record's height and the gutter
        // numbers it once — the same rule a 折行 row already follows above.
        let step = editor.record_height(line).max(1);
        for k in 0..tall {
            let y = y + k as u16;
            // The row number, frozen at the left: with the columns scrolled
            // away it is the only thing left that says which row this is. A
            // wrapped row is numbered **once**, on the line it starts on —
            // the lines under it are the same row, and a second number there
            // would say they were not.
            if gutter > 0 {
                let n = match k {
                    // **Counted in records, not in lines** (2026-10-20): a
                    // field holding a line break is one row, so the row after
                    // it is the next number and not the next line.
                    0 => crate::gutter_text(
                        editor.table_row_index(line),
                        editor.table_row_index(cursor_row),
                        gutter as usize,
                        config.editor.line_numbers,
                        false,
                    ),
                    _ => String::new(),
                };
                let style = match (ragged, peek.is_some(), line == cursor_row) {
                    (true, ..) => gutter_style.patch(torn),
                    // The row the hit is on, so 「在哪一行」 is answered before
                    // the eye has found the cell.
                    (false, true, true) => {
                        gutter_style.fg(ink.mark()).add_modifier(Modifier::BOLD)
                    }
                    // **哪一行，和哪一欄同一種說法**（2026-10-17）。欄早就這麽
                    // 說了——它的號和它的標題都點成金的；行從前只有底色，而底
                    // 色是選中的那一塊的（見 `band` 那一段的註釋）。
                    (false, false, true) => head_style,
                    _ => gutter_style,
                };
                for x in area.x..area.x + gutter {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_symbol(" ").set_style(style);
                    }
                }
                put_text(buf, area.x, y, area.x + gutter, &n, style);
            }

            let mut x = area.x + gutter;
            for (i, _) in cells.iter().enumerate().skip(viewport.left) {
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
                        // The row's own band: the selection is painted over it
                        // below, and the lit column number says which cell.
                        None => banded(line, text),
                        // 朱's own wash, the same mark the prose page gives the
                        // hit you are standing on.
                        Some(_) => Style::default().bg(ink.wash()),
                    }
                } else if ragged && i >= columns {
                    banded(line, text).patch(torn)
                } else {
                    banded(line, text)
                };
                // The cell's ground runs the column's full width, so the
                // highlight is a *cell* — a box you are inside — and not just
                // its letters. Every line of it, when it is drawn on several.
                for cx in x..(x + w).min(right) {
                    if let Some(cell) = buf.cell_mut((cx, y)) {
                        cell.set_symbol(" ").set_style(style);
                    }
                }
                // **Where the caret stands in this cell, before the cell is
                // drawn** — because it decides *which part of it* is drawn.
                // `None` unless the caret is in this cell and on this one of
                // its lines.
                //
                // Warning: **Clamped into the cell** (2026-10-17). The caret's file
                // offset is not always inside the cell the row says it is in:
                // `x` selects the whole line and leaves the head on the row's
                // last `|`, which is past the last cell's content. Counting
                // from the cell's start then gave a step beyond its text, and
                // `window_at` read that as 「the caret has been carried past
                // the column」 and scrolled the cell — `dog` drew as `<g` for
                // a row nobody was typing in. 全窗 is a view of the content,
                // so a caret anywhere else is drawn at the nearest place in
                // the content.
                let step_here = here
                    .then(|| caret_step_in(editor, line, i, &parts[i]))
                    .filter(|(at, _)| *at == k)
                    .map(|(_, step)| step);
                let whole = parts[i].get(k).map(String::as_str).unwrap_or("");
                // The caret's own cell scrolls inside itself ([`window_at`]).
                // A cell that merely holds the other pane's search hit does
                // not: nothing is being typed there, and its head is what a
                // reader needs to recognise the row by.
                // `slid` is how many columns the cell scrolled off its own
                // front. The caret is translated by `window_at`; the selection
                // band has to be translated by the same amount or it paints
                // the columns the selected characters *would* have been in
                // (2026-10-17, found in review: with the caret at the end of a
                // 36-character cell, `v` + nine `h` lit `01234` and the `>`
                // while the nine selected characters were off screen).
                let mut slid = 0u16;
                let (shown, step_here) = match step_here.filter(|_| peek.is_none()) {
                    Some(step) => {
                        let (shown, moved, _) = window_at(whole, step, w as usize);
                        slid = (step as u16).saturating_sub(moved as u16);
                        (shown, Some(moved))
                    }
                    None => (whole.to_string(), step_here),
                };
                let content = shown.as_str();
                // **A cut cell says so**, in the same `>` the prose page marks
                // a folded tail with — one symbol, one meaning, whichever
                // surface the table is drawn on. Nothing is cut under 折行:
                // what would have been folded away is on the lines below.
                let mark = yumete_core::mdtable::FOLD_MARK;
                let cut = w > 0 && yumete_cjk::str_width(content) > w as usize;
                let stop = match cut {
                    true => (x + w).saturating_sub(yumete_cjk::str_width(mark) as u16),
                    false => x + w,
                };
                put_text(buf, x, y, stop.min(right), content, style);
                if cut && stop < right {
                    // 金 and bold, the ink the prose page gives it: the
                    // furniture's grey is the colour of the rules and the
                    // line numbers, which set the one mark on the grid that
                    // is *not* furniture back further than the writing it
                    // stands after.
                    put_text(
                        buf,
                        stop,
                        y,
                        (x + w).min(right),
                        mark,
                        style.fg(ink.gold()).add_modifier(Modifier::BOLD),
                    );
                }
                // **Every cell the selection reaches, not just the caret's**
                // (2026-10-17). The test used to be `here`, so a selection
                // wider than one cell was painted on one cell: `x` lit the
                // last cell of the row and left the rest of the record
                // looking untouched, and a selection standing on a wall was
                // drawn nowhere at all (§5.12.69).
                // **A caret is a point, not a selection** (2026-10-20 作者報的:
                // 「the selection is wrong when the cursor is on an empty
                // cell」). An empty cell is a single offset, so a zero-width
                // selection standing on it satisfies 「covers this cell
                // whole」 — and so does the cell on the **other side of the
                // wall**, which shares that offset. Both lit, and 「whole」
                // then lights every line of the row: beside a 折行 cell that
                // is twelve rows of grey, two columns wide, over a cell the
                // reader had merely walked into.
                let bare = {
                    let (a, b) = editor.selection();
                    // Warning: **not `a == b`.** The caret here *covers* the character
                    // it is on, so standing still is a selection one character
                    // wide — and on an empty cell that one character is the
                    // wall, which is exactly how both sides of it lit up.
                    b <= a + 1
                };
                let lit = editor.cell_span(line, i).filter(|_| peek.is_none()).and_then(
                    |(from, to)| {
                        // Standing somewhere is one cell's business, and which
                        // one is `cell_position`'s answer, not an arithmetic
                        // coincidence at a wall.
                        if bare && (line, i) != (cursor_row, cursor_cell) {
                            return None;
                        }
                        let (a, b) = editor.selection();
                        // Clip the selection to this cell. An empty cell is a
                        // single offset, so it lights when the selection
                        // reaches across it rather than when it covers width.
                        // `b` is clipped below; `past` keeps the raw end,
                        // because 「does the selection go **beyond** this
                        // cell」 is a different question from 「does it reach
                        // the end of it」 — see `runs_on`.
                        let past = b > to;
                        (a <= to && from <= b).then(|| (from, to, a.max(from), b.min(to), past))
                    },
                );
                if let Some((from, to, a, b, past)) = lit {
                    // **The selection, over the cell's own ground.** Only the
                    // background is touched: a heading inside a selection is
                    // still a heading, which is the rule everywhere else too.
                    let (la, ca) = caret_in(&parts[i], a - from);
                    let (lb, cb) = caret_in(&parts[i], b - from);
                    // **A cell the selection covers whole is lit on every
                    // line of the row** (2026-10-17, found in review). The
                    // line test below asks which of *this cell's* lines the
                    // selection touches, and a one-line cell has no second
                    // line — so on a wrapped row the short cells went dark
                    // while the seams between them stayed lit, and the band
                    // came out as holes with bridges. A record is one thing on
                    // every line it occupies.
                    // …and a caret lights the cell's box on **its own line**,
                    // not on all of them: 「covers it whole」 is about a
                    // selection, which a point is not.
                    let whole_cell = a <= from && to <= b && !bare;
                    let only_line = usize::from(bare) * la;
                    if (whole_cell || (la <= k && k <= lb)) && (!bare || k == only_line) {
                        let c0 = match (la == k, whole_cell) {
                            (true, false) => (ca as u16).saturating_sub(slid),
                            _ => 0,
                        };
                        // **A cell the selection runs past is lit to its box**,
                        // not to the end of its letters — the comment above
                        // says the highlight is a cell you are inside, and a
                        // row selection that stopped at each cell's last 字
                        // came out as stripes with gaps where the padding is.
                        //
                        // Warning: **Past it, not up to it** (2026-10-17 作者報的:
                        // 「I press `w` on 拼, the word is selected but also
                        // the empty region after it」). The test was `b >= to`
                        // against the *clipped* end, which every selection
                        // that stops at a cell's last character satisfies —
                        // so `w` on the tail of a cell lit the padding behind
                        // it as though the selection ran on into the wall.
                        // Warning: **a caret runs past nothing** (2026-10-21). An
                        // empty cell is a single offset, and the caret covers
                        // the character it stands on, so `b` is always one
                        // beyond it — which read as 「the selection carries on
                        // into the next cell」 and lit the whole box.
                        let runs_on = past && !bare;
                        let c1 = match (lb == k, runs_on, whole_cell) {
                            // **An empty cell under a caret is lit one column
                            // wide** — the width of the caret, not the width of
                            // the cell. It has no character to stand on, so
                            // something must be drawn; painting its whole box
                            // says 「this cell is selected」 about a cell the
                            // reader has merely walked into, and beside a
                            // second empty cell the two bands read as one
                            // (2026-10-21 作者報的: 「a cursor can still
                            // highlight two cells if it is empty」).
                            (true, false, false) if bare && from == to => c0 + 1,
                            (true, false, false) => (cb as u16).saturating_sub(slid),
                            _ => w,
                        };
                        let stop = (x + c1).min(x + w).min(right);
                        for cx in (x + c0).min(stop)..stop {
                            if let Some(cell) = buf.cell_mut((cx, y)) {
                                cell.set_style(cell.style().patch(on));
                            }
                        }
                    }
                }
                if let Some(step) = step_here {
                    // Where typing would land — which is *inside* the cell, not
                    // at its start. Reading by character (`Tab`), and typing in
                    // Insert, both move the cursor within the cell, and a caret
                    // pinned to the cell's first 字 says they did not. Under
                    // 折行 it is on the line the caret's own character is on,
                    // and past the column's width it is wherever the cell has
                    // scrolled to.
                    caret = ((x + (step as u16).min(w)).min(right.saturating_sub(1)), y);
                }
                // The seam after the cell: the page's own ground, except on the
                // row the cursor is on, where the band runs unbroken so the row
                // reads as one thing. A drawn rule goes in it — and keeps the
                // row's ground, so the cursor's band is not cut into pieces.
                // Warning: **The seam takes the row's own band too** (#473). It was
                // the page, which is right when the columns are the only thing
                // banded — and wrong since the rows are: every body row came
                // out striped with page-coloured gaps where the walls are,
                // while the same table read in a chapter is banded straight
                // across. The seam is *inside* the table.
                let seam = if w == 0 { 0 } else { GAP as u16 };
                let ground = banded(line, page);
                // **And it takes the selection when the selection crosses it**
                // (2026-10-17), for the same reason the band does: a record
                // the reader selected is one thing, and a gap of page colour
                // every few columns says it is several.
                let crossed = editor
                    .cell_span(line, i)
                    .zip(editor.cell_span(line, i + 1))
                    .is_some_and(|((_, to), (next, _))| {
                        let (a, b) = editor.selection();
                        // Warning: **a caret crosses nothing** (2026-10-20 作者報的).
                        // Two empty cells share the offset their wall sits on,
                        // so a caret standing on one of them read as a
                        // selection reaching across the seam — and the seam
                        // then took the lit ground down **every** line of a
                        // 折行 row, which is the grey ribbon that was left
                        // after the cells themselves stopped lighting.
                        peek.is_none() && !bare && a <= to && next <= b
                    });
                let ground = match crossed {
                    true => ground.patch(on),
                    false => ground,
                };
                for cx in (x + w)..(x + w + seam).min(right) {
                    if let Some(cell) = buf.cell_mut((cx, y)) {
                        match stroke {
                            Some((glyph, rule)) => cell.set_symbol(glyph).set_style(ground.fg(rule)),
                            None => cell.set_symbol(" ").set_style(ground),
                        };
                    }
                }
                // **The caret's cell, bracketed** (#407). The box used to be
                // the cell's whole ground, painted in the *selection's* rung —
                // which left a selection inside it no colour to be drawn in.
                // The ladder has no room for a third ground (HEAD 815 and
                // SELECTION 700 are 5% apart at the paper end), so the box
                // moved off the ground and into the seam the page already
                // keeps between columns: two thin rules in 金, the one colour
                // on the grid that is not a quantity of ink. A spreadsheet's
                // active cell — and it costs the selection nothing.
                if here {
                    // **A rule that is already there is only recoloured.** A
                    // drawn seam (`:table-rules line`) turns 金 and the grid
                    // reads exactly as before; only a *blank* seam is given a
                    // mark of its own, hugging the cell from the far side of
                    // the gap — a right eighth-block before, a left one after.
                    let mut wall = |bx: u16, glyph: &str| {
                        if let Some(cell) = buf.cell_mut((bx, y)) {
                            if stroke.is_none() {
                                cell.set_symbol(glyph);
                            }
                            cell.set_style(cell.style().fg(ink.gold()));
                        }
                    };
                    if x > 0 {
                        wall(x - 1, "\u{2595}");
                    }
                    if x + w < right {
                        wall(x + w, "\u{258f}");
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
                    cell.set_symbol(" ").set_style(banded(line, page));
                }
            }
            if ragged && cells.len() < columns && k == 0 {
                put_text(buf, x.min(right), y, right, "⟨缺⟩", quiet.patch(torn));
            }
        }
        y += tall as u16;
        line += step;
    }
    caret
}

/// The panel down the right: every field of the row the cursor is in.
///
/// The grid can only show what fits across the window; this is where the rest
/// of a twenty-eight-column row goes, along with the fields that are worked
/// out rather than stored. It is a *reading* surface — nothing here is edited,
/// and nothing here scrolls out from under you as you move along the row.
pub fn draw_detail(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    side: Side,
    area: Rect,
) {
    let Some(detail) = editor.detail() else {
        return;
    };
    let ink = crate::theme::Palette::of(config);
    let ground = ink.ground(yumete_config::rung::CHROME);
    let name = ground.fg(ink.quiet());
    let value = ground.fg(ink.text());
    let here = ground.fg(ink.gold()).add_modifier(Modifier::BOLD);
    // A component with no row of its own — for a 拆分表 that is a finding, not
    // a blank.
    let missing = ground.fg(ink.mark());

    frame.render_widget(Clear, area);
    // **兩堵牆與底色，和常駐面板同一支**，這樣一欄不管怎麼分，讀起來都是一欄
    // （#293）；有焦點時兩邊都是塗滿的金——見 `crate::sidebar_shell`。
    let whose = yumete_core::messages::say(yumete_core::sidebar::Info::Record.tag(), &[]);
    let shell = crate::sidebar_shell(frame, editor, ink, ground, side, area, &whose);
    let (right, area) = (shell.to, shell.area);
    let buf = frame.buffer_mut();
    // 兩邊都是「離窗口邊 2 格」：左邊那一堵牆在 `area.x`，右邊那一堵在對面，
    // 而牆與字之間留一格空氣。
    let left = area.x + 2;
    // **標題由框畫，這裏只接自己那一句**（同文件樹接根目錄名）：那張表叫什麼。
    //
    // Warning: **用 `shell.head`，別自己調一個**（2026-09-30 報的：「line
    // index in the title has a wrong background colour」）。有焦點的時候那一行
    // 整條是金底，而自己調的那一個帶着 CHROME 的底色——寫上去就是金條上一塊
    // 別的顏色。框交出 `head` 正是為了這件事，文件樹那一支一直用的就是它。
    put_text(buf, shell.head_at + 2, area.y, right, &detail.title, shell.head);
    let mut y = area.y + 2;
    // The 部件 list first, because it is what the panel is *read for* — and it
    // used to be drawn last, under twenty-eight mostly-blank fields, which on
    // any real window meant not drawn at all.
    if !detail.links.is_empty() {
        put_text(buf, left, y, right, &say!("detail.components"), name);
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
    // **The values line up past the longest name**, rather than at a fixed
    // ten cells: the names carry their column number now (「12 pinyin」),
    // and a name longer than the guess ran straight into its own value.
    // Capped, so one long name does not push every value off the panel.
    let column = detail
        .rows
        .iter()
        .map(|(field, _)| yumete_cjk::str_width(field) as u16 + 1)
        .max()
        .unwrap_or(10)
        .clamp(10, 20);
    let indent = left + column;
    // **A value too long for the panel is wrapped, not cut** (#283,
    // 2026-09-07). This panel is the way to read a cell the grid folded, so a
    // panel that folds it again answers nothing: 「信息面板也没有换行功能来显
    // 示这个单元格的全部信息」. Wrapped by the editor's own line breaker, so
    // 禁則 holds here as it does on the page.
    let width = right.saturating_sub(indent) as usize;
    let drawn: Vec<Vec<String>> = detail
        .rows
        .iter()
        .map(|(_, text)| match text {
            Some(text) => wrapped(text, width),
            None => vec!["⟨缺⟩".to_string()],
        })
        .collect();
    // Far enough up that the *last* row of the field the cursor is in is still
    // on the panel — the fields above it are as many as the rest of the room
    // holds, counted in drawn rows rather than in fields.
    // A field takes at least its own row, even when there is no room beside
    // the name to draw a value in.
    let height = |i: usize| drawn.get(i).map_or(1, |rows: &Vec<String>| rows.len().max(1));
    let mut first = at;
    let mut used = height(at);
    while first > 0 && used + height(first - 1) <= room {
        first -= 1;
        used += height(first);
    }
    // **鍵進來了就換成鍵說了算**（2026-09-30 補）。跟着光標那一套是 2026-09-07
    // 定的，理由沒錯——「a reading surface with a scrollbar is a surface with a
    // mode」——可那是**鍵在正文裏**的時候：那時光標一走，這一格自己跟上。
    //
    // Warning: **鍵一進來，光標就不動了**，於是跟着光標那一套當場凍住，而底邊
    // 提示行還寫着 `j k J K g G 上下`——一扇拿走了鍵卻不認那幾個鍵的面板。
    // 「拿走鍵的那一半有義務」。
    if editor.panel_focus() == Some(side) {
        // Warning: **最多翻到「最後一屏」**（2026-09-30 報的：「pressing will
        // scroll down even though it is not necessary」）。夾在 `rows.len()-1`
        // 上的話，`j` 一路按得到只剩最後一格，下面全是空的——那不是翻頁，那是
        // 把東西推出去。
        // 從最後一格往回數，數到這一屏裝不下為止——那就是翻得到的最遠一格。
        let mut fits = 0usize;
        let mut used = 0usize;
        for i in (0..detail.rows.len()).rev() {
            used += height(i);
            if used > room && fits > 0 {
                break;
            }
            fits += 1;
        }
        let last = detail.rows.len().saturating_sub(fits);
        first = editor.panel_scroll().min(last);
    }
    // 讀到第幾行 / 共幾行，寫在底邊——翻得動了纔配有這個數。
    let mut seen = 0usize;
    let mut upto = first;
    while upto < detail.rows.len() && seen + height(upto) <= room {
        seen += height(upto);
        upto += 1;
    }
    crate::count_on_the_floor(frame, &shell, upto.max(first + 1), detail.rows.len());
    let buf = frame.buffer_mut();
    for (i, (field, text)) in detail.rows.iter().enumerate().skip(first) {
        if y >= area.y + area.height {
            return;
        }
        let style = if *field == detail.here { here } else { name };
        put_text(buf, left, y, right, field, style);
        // **A field the row does not have**, said in 朱 and in words: a blank
        // here used to mean either 「this cell is empty」 or 「this row is short
        // by twenty-four columns」, and telling those apart is most of what
        // this panel is for.
        let style = match (text.is_some(), *field == detail.here) {
            (false, _) => missing,
            (true, true) => here,
            (true, false) => value,
        };
        if indent < right {
            for line in &drawn[i] {
                if y >= area.y + area.height {
                    return;
                }
                put_text(buf, indent, y, right, line, style);
                y += 1;
            }
        } else {
            y += 1;
        }
    }
}


/// **The run of `line` to draw when the caret has been carried past `room`**,
/// where the caret then stands, and whether a head was left behind.
///
/// A column is only as wide as the page can give it, and a cell can be longer.
/// Drawn always from its first character, the tail — which is where the typing
/// is — falls off the right edge and the caret with it: it stopped at the
/// column's last cell and stayed there however much more was typed, so neither
/// the characters nor the cursor moved (reported 2026-10-09: 「The cursor
/// location in the table is incorrect (insert mode)」). So the cell scrolls
/// inside itself, which is what a spreadsheet's active cell does — and what
/// this editor's own query boxes already do ([`crate::window_on`]).
///
/// A cell is reserved at each end: [`HEAD_MARK`] in front, and the room
/// [`FOLD_MARK`] needs behind, so the caret is never standing on a mark.
///
/// Warning: **Both marks earn their column** — see [`HEAD_MARK`], which was taken
/// out on 2026-10-17 and put back the same day once a real frame showed what
/// `>` alone cannot say.
/// **Where the caret stands inside a cell** — which of the cell's drawn lines
/// it is on, and how far into that line, in the cell's own characters.
///
/// One answer, because two things need it and they have to agree: the page
/// scrolls the caret's own cell by it ([`window_at`]), and [`char_at`] has to
/// take the same amount back off a click. They did not agree — the hit test
/// had never heard of the scroll, so a click in a long cell the caret had
/// scrolled landed on a character that was off the left edge (2026-10-19,
/// caught by `the_caret_reads_back` the hour it was armed).
///
/// Warning: **Clamped into the cell** (2026-10-17). The caret's file offset is not
/// always inside the cell the row says it is in: `x` selects the whole line
/// and leaves the head on the row's last `|`, which is past the last cell's
/// content. Counting from the cell's start then gave a step beyond its text,
/// and `window_at` read that as 「the caret has been carried past the column」
/// and scrolled the cell — `dog` drew as `<g` for a row nobody was typing in.
/// 全窗 is a view of the content, so a caret anywhere else is drawn at the
/// nearest place in the content.
///
/// Warning: **`to` is inside only while appending** (2026-10-17, found in review).
/// It is the content's *exclusive* end, so in Normal a caret resting there is
/// one past the last character — `x` in a `.csv` drew the cursor on the
/// padding after the last cell's `1` instead of on the `1`. In Insert it is
/// the place appending happens and must be drawn.
fn caret_step_in(editor: &Editor, line: usize, cell: usize, parts: &[String]) -> (usize, usize) {
    let (from, to) = editor.cell_span(line, cell).unwrap_or((usize::MAX, usize::MAX));
    let at = editor.cursor();
    let appending = editor.mode() == yumete_core::input::Mode::Insert;
    let content = match appending || to == from {
        true => from..=to,
        false => from..=to.saturating_sub(1).max(from),
    };
    let into = match content.contains(&at) {
        true => at - from,
        false => to.saturating_sub(1).max(from) - from,
    };
    caret_in(parts, into)
}

/// **How far the caret's own cell has scrolled off its own front**, in cells.
///
/// Zero everywhere else: a cell nobody is typing in keeps its head, which is
/// what a reader recognises the row by.
fn slid_by(editor: &Editor, line: usize, cell: usize, on: usize, parts: &[String], room: usize) -> usize {
    if editor.cell_position() != Some((line, cell)) {
        return 0;
    }
    let (k, step) = caret_step_in(editor, line, cell, parts);
    if k != on {
        return 0;
    }
    let whole = parts.get(k).map(String::as_str).unwrap_or("");
    let (_, moved, _) = window_at(whole, step, room);
    step.saturating_sub(moved)
}

fn window_at(line: &str, caret: usize, room: usize) -> (String, usize, bool) {
    let head = yumete_cjk::str_width(yumete_core::mdtable::HEAD_MARK);
    let fold = yumete_cjk::str_width(yumete_core::mdtable::FOLD_MARK);
    // **A cell that fits is never scrolled** (2026-10-17). The two tests
    // below are about the caret, and both say yes for a caret on the last
    // character of a cell exactly as wide as its column — so `reads` in a
    // five-wide column scrolled to `<ads` the moment the caret stood on its
    // `s`, with no tail to fold and nothing gained. Ask the cell first.
    if yumete_cjk::str_width(line) <= room {
        return (line.to_string(), caret, false);
    }
    // **The caret's own glyph is one column or two** (2026-10-17, found in
    // review). `caret` is where it *starts*, so a 漢字 reaches `caret + 1`;
    // measured against `caret` alone, a wide glyph sitting in the last column
    // before the mark's reserved one counted as 「still inside」 and the caret
    // was then drawn **on** the `>` — which this function's own promise says
    // can never happen. ASCII was right all along, which is why it took a
    // 漢字 column to show.
    let caret_end = caret
        + yumete_cjk::graphemes(line)
            .scan(0usize, |at, g| {
                let was = *at;
                *at += yumete_cjk::grapheme_width(g).max(1);
                Some((was, yumete_cjk::grapheme_width(g).max(1)))
            })
            .find(|&(at, _)| at == caret)
            .map(|(_, w)| w)
            .unwrap_or(1)
            .saturating_sub(1);
    // Nothing to do while the caret is still inside the room — and nothing
    // that *can* be done once the two marks would leave it no cell to stand in.
    if caret_end + fold < room || room <= head + fold {
        return (line.to_string(), caret, false);
    }
    let want = caret_end + head + fold + 1 - room;
    let (mut dropped, mut from) = (0usize, 0usize);
    for g in yumete_cjk::graphemes(line) {
        if dropped >= want {
            break;
        }
        // **A wide glyph goes whole or not at all**: half a 漢字 is not a cell
        // the terminal can draw, and charging one for two walks every column
        // after it out of line.
        dropped += yumete_cjk::grapheme_width(g).max(1);
        from += g.len();
    }
    (
        format!("{}{}", yumete_core::mdtable::HEAD_MARK, &line[from..]),
        caret + head - dropped,
        true,
    )
}

/// Which line of a wrapped cell the caret is on, and how far into it.
///
/// `into` counts the cell's own characters. A cell drawn on one line answers
/// `(0, …)`, which is what every un-wrapped grid asks for.
fn caret_in(parts: &[String], into: usize) -> (usize, usize) {
    let mut seen = 0;
    for (k, part) in parts.iter().enumerate() {
        let held = part.chars().count();
        if into < seen + held || k + 1 == parts.len() {
            let text: String = part.chars().take(into.saturating_sub(seen)).collect();
            return (k, yumete_cjk::str_width(&text));
        }
        seen += held;
    }
    (0, 0)
}

/// A value broken across `width` cells, as the rows it is drawn on.
///
/// [`yumete_core::wrap::line_rows`] is the page's own breaker — the one that
/// keeps 禁則 and does not split a Latin word — so a wrapped cell reads the
/// way the same words would read in the document they came from.
fn wrapped(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = yumete_core::wrap::line_rows(text, width)
        .into_iter()
        .map(|(from, to)| chars[from.min(chars.len())..to.min(chars.len())].iter().collect())
        .collect();
    // **A row that ends exactly on the measure leaves an empty one after it**
    // — the page wants somewhere to put the caret when a line fills its width
    // to the cell. Nothing is drawn there, so a cell would be a line taller
    // than it has anything to show, and every row under it would move down.
    while out.len() > 1 && out.last().is_some_and(String::is_empty) {
        out.pop();
    }
    out
}
