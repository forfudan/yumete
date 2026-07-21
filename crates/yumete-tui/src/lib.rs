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

use std::io::{self, stdout};

use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    ModifierKeyCode, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::supports_keyboard_enhancement;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};
use ratatui::Frame;

use yumete_config::{Config, LineNumbers};
use yumete_core::{Editor, Key, KeyOutcome, Mode, TextStore};
use yumete_ime::ImeSession;

/// Run the interactive editor until the user quits.
///
/// Sets up the alternate screen and raw mode (via `ratatui::init`), runs the
/// draw/read loop, and always restores the terminal on the way out. In Insert
/// mode the built-in Yume IME (`ime`) intercepts keystrokes when it is available
/// and engaged; otherwise keys go straight to the editor.
///
/// When the terminal supports the Kitty keyboard protocol it is enabled so a
/// lone-Shift tap toggles 中/英 (mirroring the GUI frontends). Terminals without
/// that support (e.g. Apple Terminal) cannot report a bare Shift, so the toggle
/// is unavailable there — use a Kitty-protocol terminal (kitty, WezTerm, foot,
/// Ghostty, Alacritty, Konsole, …).
pub fn run(editor: &mut Editor, config: &Config, ime: &mut ImeSession) -> io::Result<()> {
    let mut terminal = ratatui::init();

    // Enable the Kitty keyboard protocol (report modifier presses/releases) so a
    // lone-Shift tap can toggle 中/英.
    let enhanced = matches!(supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        let _ = execute!(
            stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                    | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
            )
        );
    }

    let mut viewport_top = 0usize;
    let mut shift = ShiftTap::default();

    let result = loop {
        if let Err(err) = terminal.draw(|frame| draw(frame, editor, config, ime, &mut viewport_top))
        {
            break Err(err);
        }
        match event::read() {
            Ok(Event::Key(key)) => {
                // A lone-Shift tap toggles 中/英 in Insert mode; other Shift
                // activity is swallowed so it never reaches the editor.
                match shift.update(&key) {
                    ShiftResult::Toggle => {
                        if editor.mode() == Mode::Insert && ime.available() {
                            ime.toggle_language();
                        }
                        continue;
                    }
                    ShiftResult::Consumed => continue,
                    ShiftResult::Pass => {}
                }
                // Only act on key presses (releases are tracked above only).
                if key.kind != KeyEventKind::Press {
                    continue;
                }
                let (code, mods) = normalize_shift(key.code, key.modifiers);
                let consumed = editor.mode() == Mode::Insert
                    && ime.available()
                    && ime_handle(ime, editor, code, mods);
                if !consumed {
                    if let Some(k) = map_key(code, mods) {
                        if editor.on_key(k) == KeyOutcome::Quit {
                            break Ok(());
                        }
                    }
                }
            }
            Ok(_) => {}
            Err(err) => break Err(err),
        }
    };

    if enhanced {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
    result
}

/// The result of feeding a key event to the lone-Shift-tap tracker.
enum ShiftResult {
    /// A lone Shift tap completed — toggle the language.
    Toggle,
    /// A Shift key event that isn't a completed tap; swallow it.
    Consumed,
    /// Not a Shift key event; handle it normally.
    Pass,
}

/// Detects a *lone* Shift tap (press then release with no other key in between),
/// used to toggle 中/英. Requires the Kitty keyboard protocol so bare modifier
/// presses/releases are reported.
#[derive(Default)]
struct ShiftTap {
    down: bool,
    clean: bool,
}

impl ShiftTap {
    fn update(&mut self, key: &KeyEvent) -> ShiftResult {
        let is_shift = matches!(
            key.code,
            KeyCode::Modifier(ModifierKeyCode::LeftShift)
                | KeyCode::Modifier(ModifierKeyCode::RightShift)
        );
        match key.kind {
            KeyEventKind::Press if is_shift => {
                if !self.down {
                    self.down = true;
                    self.clean = true;
                }
                ShiftResult::Consumed
            }
            KeyEventKind::Repeat if is_shift => ShiftResult::Consumed,
            KeyEventKind::Release if is_shift => {
                let toggled = self.down && self.clean;
                self.down = false;
                self.clean = false;
                if toggled {
                    ShiftResult::Toggle
                } else {
                    ShiftResult::Consumed
                }
            }
            // Any other key press while Shift is held taints the tap.
            KeyEventKind::Press | KeyEventKind::Repeat => {
                if self.down {
                    self.clean = false;
                }
                ShiftResult::Pass
            }
            KeyEventKind::Release => ShiftResult::Pass,
        }
    }
}

/// Apply a held Shift to a lowercase letter, so `Shift+a` is `A` regardless of
/// how the keyboard protocol reports it (the enhanced protocol may deliver the
/// base key plus a Shift modifier).
fn normalize_shift(code: KeyCode, mods: KeyModifiers) -> (KeyCode, KeyModifiers) {
    if let KeyCode::Char(c) = code {
        if mods.contains(KeyModifiers::SHIFT) && c.is_ascii_lowercase() {
            return (KeyCode::Char(c.to_ascii_uppercase()), mods);
        }
    }
    (code, mods)
}

/// Route one Insert-mode key press to the IME. Returns `true` when the IME
/// consumed it (so the editor must not also see it). Committed text is inserted
/// into the editor at the cursor.
fn ime_handle(
    ime: &mut ImeSession,
    editor: &mut Editor,
    code: KeyCode,
    mods: KeyModifiers,
) -> bool {
    if !ime.is_chinese() {
        // ASCII pass-through: the editor inserts the character normally.
        return false;
    }
    let composing = ime.is_composing();
    match code {
        KeyCode::Enter if composing => ime.enter(),
        KeyCode::Backspace if composing => {
            ime.backspace();
        }
        KeyCode::Esc if composing => ime.escape(),
        KeyCode::Char(' ') if composing => ime.space(),
        KeyCode::Char(c) if composing && ('1'..='9').contains(&c) => {
            ime.select_in_page((c as u8 - b'1') as usize);
        }
        KeyCode::Char('-') if composing => ime.page_up(),
        KeyCode::Char('=') if composing => ime.page_down(),
        // Any other printable character (letters start/continue a composition;
        // punctuation and digits are handled by the engine). A literal space
        // with no composition falls through to the editor.
        KeyCode::Char(c) if !mods.contains(KeyModifiers::CONTROL) && c != ' ' => ime.input(c),
        _ => return false,
    }
    let committed = ime.take_committed();
    if !committed.is_empty() {
        editor.insert_committed(&committed);
    }
    true
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

/// The width of the line-number gutter for a given mode (digits + one space).
fn gutter_width(total_lines: usize, mode: LineNumbers) -> usize {
    match mode {
        LineNumbers::None => 0,
        _ => total_lines.max(1).to_string().len() + 1,
    }
}

/// The gutter text for line `i` (0-based) given the cursor line and mode.
fn gutter_text(i: usize, cursor_line: usize, width: usize, mode: LineNumbers) -> String {
    match mode {
        LineNumbers::None => String::new(),
        LineNumbers::Absolute => format!("{:>w$} ", i + 1, w = width - 1),
        LineNumbers::Relative => {
            if i == cursor_line {
                // Show the absolute number on the cursor line, left-aligned.
                format!("{:<w$} ", i + 1, w = width - 1)
            } else {
                let delta = i.abs_diff(cursor_line);
                format!("{:>w$} ", delta, w = width - 1)
            }
        }
    }
}

/// Append a line's spans with each word tinted by an alternating background
/// (the segmentation overlay, Feature #24). `ranges` are character columns
/// within `text`; gaps between them (whitespace) stay untinted.
fn push_segmented_spans<'a>(
    spans: &mut Vec<Span<'a>>,
    text: &str,
    ranges: &[(usize, usize)],
    colors: [(u8, u8, u8); 2],
) {
    let chars: Vec<char> = text.chars().collect();
    let mut col = 0usize;
    for (word_index, &(start, end)) in ranges.iter().enumerate() {
        let start = start.min(chars.len());
        let end = end.min(chars.len());
        if start >= end {
            continue;
        }
        if start > col {
            spans.push(Span::raw(chars[col..start].iter().collect::<String>()));
        }
        let (r, g, b) = colors[word_index % colors.len()];
        spans.push(Span::styled(
            chars[start..end].iter().collect::<String>(),
            Style::default().bg(Color::Rgb(r, g, b)),
        ));
        col = end;
    }
    if col < chars.len() {
        spans.push(Span::raw(chars[col..].iter().collect::<String>()));
    }
}

fn draw(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    ime: &ImeSession,
    viewport_top: &mut usize,
) {
    let area = frame.area();
    let regions = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    let text_area = regions[0];
    let status_area = regions[1];

    let buffer = editor.current_buffer();
    let total_lines = buffer.line_count();
    let height = text_area.height as usize;

    // Scroll so the cursor line stays within the viewport, honouring scrolloff.
    let cursor_line = editor.cursor_line();
    let scrolloff = config.editor.scrolloff.min(height.saturating_sub(1) / 2);
    if cursor_line < *viewport_top + scrolloff {
        *viewport_top = cursor_line.saturating_sub(scrolloff);
    } else if height > 0 && cursor_line + scrolloff >= *viewport_top + height {
        *viewport_top = (cursor_line + scrolloff + 1).saturating_sub(height);
    }

    let mode = config.editor.line_numbers;
    let gutter = gutter_width(total_lines, mode);
    let (sel_start, sel_end) = editor.selection();
    let has_selection = sel_start != sel_end;
    let (sr, sg, sb) = config.theme.selection;
    let sel_style = Style::default().bg(Color::Rgb(sr, sg, sb)).fg(Color::White);
    let show_segmentation = editor.segmentation_visible();
    let seg_colors = config.theme.segmentation;
    let rope = buffer.rope();

    // Visible lines with a line-number gutter and selection highlight.
    let mut lines: Vec<Line> = Vec::new();
    let last = (*viewport_top + height).min(total_lines);
    for i in *viewport_top..last {
        let text = buffer.line(i).unwrap_or_default();
        let mut spans = Vec::new();
        if gutter > 0 {
            spans.push(Span::styled(
                gutter_text(i, cursor_line, gutter, mode),
                Style::default().add_modifier(Modifier::DIM),
            ));
        }

        // Highlight the portion of this line covered by the selection.
        let line_start = rope.line_to_char(i);
        let line_len = text.chars().count();
        if has_selection && sel_end > line_start && sel_start < line_start + line_len {
            let a = sel_start.saturating_sub(line_start).min(line_len);
            let b = (sel_end - line_start).min(line_len);
            let chars: Vec<char> = text.chars().collect();
            let before: String = chars[..a].iter().collect();
            let selected: String = chars[a..b].iter().collect();
            let after: String = chars[b..].iter().collect();
            spans.push(Span::raw(before));
            spans.push(Span::styled(selected, sel_style));
            spans.push(Span::raw(after));
        } else if show_segmentation {
            // Tint each word with an alternating background (Feature #24). The
            // selection takes precedence, so lines under the selection above
            // keep the plain highlight instead.
            push_segmented_spans(&mut spans, &text, &editor.segment_line(i), seg_colors);
        } else {
            spans.push(Span::raw(text));
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), text_area);

    // Status line, or the command line while in Command mode.
    let status = if let Some((prefix, text)) = editor.prompt() {
        format!("{prefix}{text}")
    } else {
        let dirty = if buffer.is_modified() { " [+]" } else { "" };
        // In Insert mode with the IME available, show the 中/英 state + scheme.
        let ime_tag = if editor.mode() == Mode::Insert && ime.available() {
            if ime.is_chinese() {
                format!("[中 {}] ", ime.scheme_name())
            } else {
                "[ABC] ".to_string()
            }
        } else {
            String::new()
        };
        let left = format!(
            "-- {} --  {}{}{}",
            editor.mode_label(),
            ime_tag,
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
    let (cursor_x, cursor_y) = if let Some((_, text)) = editor.prompt() {
        let col = 1 + text.chars().count();
        (status_area.x + col as u16, status_area.y)
    } else {
        let x = text_area.x + gutter as u16 + editor.cursor_visual_column() as u16;
        let y = text_area.y + (cursor_line - *viewport_top) as u16;
        (x, y)
    };
    frame.set_cursor_position(Position::new(cursor_x, cursor_y));

    // The IME candidate panel floats just below the cursor while composing.
    if editor.mode() == Mode::Insert && ime.available() && ime.is_composing() {
        draw_candidate_panel(frame, ime, text_area, cursor_x, cursor_y);
    }
}

/// Draw the floating candidate panel below the cursor (Feature #28).
///
/// The first line is the preedit (raw / segmented code); the rows below are the
/// current page's candidates as `n. 候選 下標`, with the highlighted one
/// reversed. The panel is clamped to stay within `area`.
fn draw_candidate_panel(
    frame: &mut Frame,
    ime: &ImeSession,
    area: Rect,
    cursor_x: u16,
    cursor_y: u16,
) {
    let candidates = ime.page_candidates();
    let highlight = ime.highlight();

    // Build the content lines: preedit header, then the candidates.
    let preedit = ime.display_buffer();
    let mut rows: Vec<String> = Vec::with_capacity(candidates.len() + 1);
    rows.push(preedit);
    for (i, cand) in candidates.iter().enumerate() {
        let mut row = format!("{}. {}", i + 1, cand.text);
        if !cand.completion.is_empty() {
            row.push(' ');
            row.push_str(&cand.completion);
        }
        if !cand.comment.is_empty() {
            row.push_str("  ");
            row.push_str(&cand.comment);
        }
        rows.push(row);
    }

    // Size the panel to its content (plus borders), clamped to the text area.
    let content_w = rows
        .iter()
        .map(|r| yumete_cjk::str_width(r) as u16)
        .max()
        .unwrap_or(0);
    let inner_w = content_w.max(4);
    let panel_w = (inner_w + 2).min(area.width.max(1));
    let panel_h = (rows.len() as u16 + 2).min(area.height.max(1));

    // Prefer just below the cursor; flip above if it would overflow the bottom.
    let x = cursor_x.min(area.x + area.width.saturating_sub(panel_w));
    let below = cursor_y + 1;
    let y = if below + panel_h <= area.y + area.height {
        below
    } else {
        cursor_y.saturating_sub(panel_h).max(area.y)
    };
    let panel = Rect::new(x, y, panel_w, panel_h);

    let mut lines: Vec<Line> = Vec::with_capacity(rows.len());
    for (i, row) in rows.into_iter().enumerate() {
        if i == 0 {
            // Preedit header, dimmed.
            lines.push(Line::from(Span::styled(
                row,
                Style::default().add_modifier(Modifier::DIM),
            )));
        } else if i - 1 == highlight {
            lines.push(Line::from(Span::styled(
                row,
                Style::default().bg(Color::Rgb(0, 89, 209)).fg(Color::White),
            )));
        } else {
            lines.push(Line::from(Span::raw(row)));
        }
    }

    frame.render_widget(Clear, panel);
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL)),
        panel,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use yumete_core::{DictionarySegmenter, Key};
    use yumete_ime::Scheme;

    /// An unavailable IME (no data), for tests that don't exercise composing.
    fn no_ime() -> ImeSession {
        ImeSession::new(Scheme::Lingming, vec![])
    }

    /// Render `editor` with `config` and `ime` to an in-memory terminal buffer.
    fn render_with(
        editor: &Editor,
        config: &Config,
        ime: &ImeSession,
        w: u16,
        h: u16,
    ) -> ratatui::buffer::Buffer {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut top = 0usize;
        terminal
            .draw(|frame| draw(frame, editor, config, ime, &mut top))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// Render with an unavailable IME (the common case for non-IME tests).
    fn render(editor: &Editor, config: &Config, w: u16, h: u16) -> ratatui::buffer::Buffer {
        render_with(editor, config, &no_ime(), w, h)
    }

    #[test]
    fn segmentation_overlay_tints_words() {
        let mut editor = Editor::new();
        editor.set_segmenter(Box::new(DictionarySegmenter::builtin(0)));
        editor.set_segmentation_visible(true);
        // Type a two-word CJK phrase, then return to Normal mode.
        editor.on_key(Key::Char('i'));
        for c in "你好世界".chars() {
            editor.on_key(Key::Char(c));
        }
        editor.on_key(Key::Esc);

        let config = Config::default();
        let buffer = render(&editor, &config, 40, 6);

        // The two segmentation tints should both appear (one per word).
        let (a, b) = (config.theme.segmentation[0], config.theme.segmentation[1]);
        let tint_a = Color::Rgb(a.0, a.1, a.2);
        let tint_b = Color::Rgb(b.0, b.1, b.2);
        let mut seen_a = false;
        let mut seen_b = false;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                match buffer[(x, y)].style().bg {
                    Some(bg) if bg == tint_a => seen_a = true,
                    Some(bg) if bg == tint_b => seen_b = true,
                    _ => {}
                }
            }
        }
        assert!(seen_a, "first word tint not rendered");
        assert!(seen_b, "second word tint not rendered");
    }

    #[test]
    fn no_overlay_when_disabled() {
        let mut editor = Editor::new();
        editor.set_segmenter(Box::new(DictionarySegmenter::builtin(0)));
        editor.set_segmentation_visible(false);
        editor.on_key(Key::Char('i'));
        for c in "你好世界".chars() {
            editor.on_key(Key::Char(c));
        }
        editor.on_key(Key::Esc);

        let config = Config::default();
        let buffer = render(&editor, &config, 40, 6);
        let (a, _) = (config.theme.segmentation[0], config.theme.segmentation[1]);
        let tint_a = Color::Rgb(a.0, a.1, a.2);
        let any_tint = (0..buffer.area.height)
            .any(|y| (0..buffer.area.width).any(|x| buffer[(x, y)].style().bg == Some(tint_a)));
        assert!(!any_tint, "overlay should be hidden when disabled");
    }

    /// Concatenate all cell symbols of a rendered buffer (for content checks).
    fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
        let mut s = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                s.push_str(buffer[(x, y)].symbol());
            }
        }
        s
    }

    #[test]
    fn candidate_panel_shows_while_composing() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i')); // Insert mode
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        ime.input('b');
        assert!(ime.is_composing());

        let config = Config::default();
        let buffer = render_with(&editor, &config, &ime, 40, 10);
        let text = buffer_text(&buffer);
        assert!(text.contains('吧'), "candidate 吧 not shown in panel");
        assert!(text.contains('八'), "candidate 八 not shown in panel");
    }

    #[test]
    fn typing_then_space_commits_into_the_editor() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");

        assert!(ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char('b'),
            KeyModifiers::NONE
        ));
        assert!(ime.is_composing());
        // Space commits the highlighted candidate into the editor buffer.
        assert!(ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char(' '),
            KeyModifiers::NONE
        ));
        assert_eq!(editor.current_buffer().text(), "吧");
        assert!(!ime.is_composing());
    }

    #[test]
    fn digit_selects_a_candidate_into_the_editor() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char('b'),
            KeyModifiers::NONE,
        );
        // Digit 2 picks the second candidate, 八.
        ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char('2'),
            KeyModifiers::NONE,
        );
        assert_eq!(editor.current_buffer().text(), "八");
    }

    #[test]
    fn ascii_mode_lets_keys_fall_through_to_the_editor() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧\n");
        // Switch to ASCII: the IME no longer consumes letters.
        ime.toggle_language();
        assert!(!ime.is_chinese());
        assert!(!ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char('b'),
            KeyModifiers::NONE
        ));
    }

    fn key(code: KeyCode, kind: KeyEventKind) -> KeyEvent {
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind)
    }

    #[test]
    fn lone_shift_tap_toggles_but_shift_chords_do_not() {
        let shift = || KeyCode::Modifier(ModifierKeyCode::LeftShift);

        // Press then release Shift with nothing in between → a toggle.
        let mut tap = ShiftTap::default();
        assert!(matches!(
            tap.update(&key(shift(), KeyEventKind::Press)),
            ShiftResult::Consumed
        ));
        assert!(matches!(
            tap.update(&key(shift(), KeyEventKind::Release)),
            ShiftResult::Toggle
        ));

        // Shift + a letter (a chord) must NOT toggle.
        let mut tap = ShiftTap::default();
        tap.update(&key(shift(), KeyEventKind::Press));
        assert!(matches!(
            tap.update(&key(KeyCode::Char('a'), KeyEventKind::Press)),
            ShiftResult::Pass
        ));
        assert!(matches!(
            tap.update(&key(shift(), KeyEventKind::Release)),
            ShiftResult::Consumed
        ));
    }

    #[test]
    fn shift_normalizes_lowercase_letters_to_uppercase() {
        let (code, _) = normalize_shift(KeyCode::Char('d'), KeyModifiers::SHIFT);
        assert_eq!(code, KeyCode::Char('D'));
        // Without Shift, unchanged.
        let (code, _) = normalize_shift(KeyCode::Char('d'), KeyModifiers::NONE);
        assert_eq!(code, KeyCode::Char('d'));
    }
}
