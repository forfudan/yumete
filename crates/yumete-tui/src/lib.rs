//! `yumete-tui` — the terminal user interface for yumete.
//!
//! This is the `Renderer` layer: it drives the interactive loop, translates
//! terminal key events into `yumete-core`'s backend-agnostic [`Key`]s, and
//! draws the buffer with a line-number gutter (Feature #20) and a status line
//! (Feature #19). It is a thin shell over the tested editor core — all editing
//! logic lives in `yumete-core`.
//!
//! The terminal stack is `ratatui` (the maintained `tui-rs` fork) over its
//! bundled `crossterm` backend, so no ANSI escapes are hand-written here.

use std::io;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use yumete_core::{Editor, Key, KeyOutcome, Mode, TextStore};

/// Run the interactive editor until the user quits.
///
/// Sets up the alternate screen and raw mode (via `ratatui::init`), runs the
/// draw/read loop, and always restores the terminal on the way out.
pub fn run(editor: &mut Editor) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let mut viewport_top = 0usize;

    let result = loop {
        if let Err(err) = terminal.draw(|frame| draw(frame, editor, &mut viewport_top)) {
            break Err(err);
        }
        match event::read() {
            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                if let Some(k) = map_key(key.code, key.modifiers) {
                    if editor.on_key(k) == KeyOutcome::Quit {
                        break Ok(());
                    }
                }
            }
            Ok(_) => {}
            Err(err) => break Err(err),
        }
    };

    ratatui::restore();
    result
}

/// Translate a terminal key event into a core [`Key`], or `None` to ignore it.
fn map_key(code: KeyCode, modifiers: KeyModifiers) -> Option<Key> {
    match code {
        // Drop control-modified characters so control codes aren't inserted.
        KeyCode::Char(_) if modifiers.contains(KeyModifiers::CONTROL) => None,
        KeyCode::Char(c) => Some(Key::Char(c)),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        _ => None,
    }
}

/// The width of the line-number gutter (digits + one trailing space).
fn gutter_width(total_lines: usize) -> usize {
    total_lines.max(1).to_string().len() + 1
}

fn draw(frame: &mut Frame, editor: &Editor, viewport_top: &mut usize) {
    let area = frame.area();
    let regions = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    let text_area = regions[0];
    let status_area = regions[1];

    let buffer = editor.current_buffer();
    let total_lines = buffer.line_count();
    let height = text_area.height as usize;

    // Scroll so the cursor line stays within the viewport.
    let cursor_line = editor.cursor_line();
    if cursor_line < *viewport_top {
        *viewport_top = cursor_line;
    } else if height > 0 && cursor_line >= *viewport_top + height {
        *viewport_top = cursor_line + 1 - height;
    }

    let gutter = gutter_width(total_lines);

    // Visible lines with a right-aligned line-number gutter.
    let mut lines: Vec<Line> = Vec::new();
    let last = (*viewport_top + height).min(total_lines);
    for i in *viewport_top..last {
        let number = format!("{:>width$} ", i + 1, width = gutter - 1);
        let text = buffer.line(i).unwrap_or_default();
        lines.push(Line::from(vec![
            Span::styled(number, Style::default().add_modifier(Modifier::DIM)),
            Span::raw(text),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), text_area);

    // Status line, or the command line while in Command mode.
    let status = if editor.mode() == Mode::Command {
        format!(":{}", editor.command_line())
    } else {
        let dirty = if buffer.is_modified() { " [+]" } else { "" };
        let left = format!(
            "-- {} --  {}{}",
            editor.mode().label(),
            buffer.display_name(),
            dirty
        );
        if editor.status().is_empty() {
            format!(
                "{left}   Ln {}, Col {}",
                editor.cursor_line() + 1,
                editor.cursor_visual_column() + 1,
            )
        } else {
            format!("{left}   {}", editor.status())
        }
    };
    frame.render_widget(
        Paragraph::new(status).style(Style::default().add_modifier(Modifier::REVERSED)),
        status_area,
    );

    // Place the terminal cursor.
    if editor.mode() == Mode::Command {
        let col = 1 + editor.command_line().chars().count();
        frame.set_cursor_position(Position::new(status_area.x + col as u16, status_area.y));
    } else {
        let x = text_area.x + gutter as u16 + editor.cursor_visual_column() as u16;
        let y = text_area.y + (cursor_line - *viewport_top) as u16;
        frame.set_cursor_position(Position::new(x, y));
    }
}
