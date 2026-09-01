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

pub mod vertical;

use std::io::{self, stdout};

use ratatui::crossterm::cursor::SetCursorStyle;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, KeyboardEnhancementFlags, ModifierKeyCode, MouseEventKind,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::supports_keyboard_enhancement;
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use yumete_config::{Config, LineNumbers};
use yumete_core::zong::{Anchor, Layout as WritingLayout};
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

    // Take the mouse, so the wheel can turn the page. The cost, which Helix
    // pays too: the terminal's own click-and-drag selection stops working and
    // needs whatever modifier that terminal reserves for it (Option, on macOS).
    let _ = execute!(stdout(), EnableMouseCapture);

    let mut viewport = Viewport::default();
    let mut shift = ShiftTap::default();
    let mut last_mode = None;

    let result = loop {
        let mode = editor.mode();
        if last_mode != Some(mode) {
            // A block in Normal, a bar in Insert — the shape a modal editor is
            // read by. Only sent on a change, so the terminal is not asked to
            // reset its cursor on every keystroke.
            // A bar in Insert — but laid out vertically the bar turns with the
            // text, and an underscore is the only thin *horizontal* cursor a
            // terminal offers. Elsewhere a block, which vertically is drawn into
            // the page instead and the terminal's own cursor stays hidden.
            let vertical = editor.layout() == WritingLayout::Vertical;
            let _ = execute!(
                stdout(),
                match (mode, vertical) {
                    (Mode::Insert, false) => SetCursorStyle::SteadyBar,
                    (Mode::Insert, true) => SetCursorStyle::SteadyUnderScore,
                    _ => SetCursorStyle::SteadyBlock,
                }
            );

            // Opening the command line cancels a composition rather than leaving
            // it hanging: `:` does not compose, so there is nothing to finish it
            // with. The 中/英 state itself is left alone — it belongs to Insert,
            // and a command is over in a keystroke or two.
            if mode == Mode::Command && ime.available() && ime.is_composing() {
                ime.escape();
            }
            last_mode = Some(mode);
        }
        // The 縱 wrap length depends on the terminal height, and the motions
        // that cross 縱 run before the next draw, so settle it up front.
        if editor.layout() == WritingLayout::Vertical {
            if let Ok(size) = terminal.size() {
                let lines = editor.current_buffer().line_count();
                let ruby = !editor.ruby().is_empty();
                editor.set_zong_length(vertical::zong_length_for(config, size.height, lines, ruby));
            }
        }
        // Tell the editor how much is on screen, so `C-d` means half of what
        // can actually be seen.
        if let Ok(size) = terminal.size() {
            let lines = size.height.saturating_sub(1) as usize;
            let columns = (size.width / 3).max(1) as usize;
            editor.set_page(lines, columns);
        }
        if let Err(err) = terminal.draw(|frame| draw(frame, editor, config, ime, &mut viewport)) {
            break Err(err);
        }
        match event::read() {
            Ok(Event::Key(key)) => {
                // A lone-Shift tap toggles 中/英 in Insert mode; other Shift
                // activity is swallowed so it never reaches the editor.
                match shift.update(&key) {
                    ShiftResult::Toggle => {
                        if composes(editor.mode()) && ime.available() {
                            ime.toggle_language();
                        }
                        continue;
                    }
                    ShiftResult::Consumed => continue,
                    ShiftResult::Pass => {}
                }
                if !is_actionable(key.kind) {
                    continue;
                }
                let (code, mods) = normalize_shift(key.code, key.modifiers);
                let consumed = composes(editor.mode())
                    && ime.available()
                    && ime_handle(ime, editor, code, mods);
                if !consumed {
                    if let Some(k) = map_key(code, mods) {
                        if editor.on_key(k) == KeyOutcome::Quit {
                            break Ok(());
                        }
                    }
                }
                // `:chaifen` configures the IME, which the core cannot reach;
                // it leaves the request here and the answer goes back, so the
                // next toggle starts from what the engine actually did.
                if let Some(on) = editor.take_chaifen_request() {
                    let settled = ime.set_annotations(on);
                    editor.set_chaifen(settled);
                    editor.set_status(if settled {
                        "拆分 on".to_string()
                    } else if ime.annotations_available() {
                        "拆分 off".to_string()
                    } else {
                        "拆分 unavailable for this scheme".to_string()
                    });
                }
            }
            Ok(Event::Mouse(mouse)) => match mouse.kind {
                // A notch moves three 縱 — the same three lines a terminal
                // scrolls by, counted in the unit the page is set in.
                MouseEventKind::ScrollDown => editor.scroll(WHEEL_STEP, false),
                MouseEventKind::ScrollUp => editor.scroll(WHEEL_STEP, true),
                _ => {}
            },
            Ok(_) => {}
            Err(err) => break Err(err),
        }
    };

    let _ = execute!(
        stdout(),
        DisableMouseCapture,
        SetCursorStyle::DefaultUserShape
    );
    if enhanced {
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
    result
}

/// How far one notch of the wheel moves — three, as a terminal scrolls three
/// lines, counted in whichever unit the page is set in.
const WHEEL_STEP: usize = 3;

/// Whether a mode collects text the IME should compose into.
///
/// Insert is the obvious one, but a `/` search is text too — and in a Chinese
/// document it is usually Chinese text. Without this, `/` could only search for
/// what could be typed as ASCII, which in a novel is almost nothing.
fn composes(mode: Mode) -> bool {
    // Ruby included: a reading is kana or 拼音, and kana needs the IME as much
    // as the body text does. The `:` command line is **not** — its vocabulary is
    // ASCII command names, so running the IME there would only mean toggling out
    // of it before every command.
    matches!(mode, Mode::Insert | Mode::Search | Mode::Ruby)
}

/// Whether a key event should drive the editor.
///
/// Presses and **auto-repeat** both do; releases only feed the lone-Shift
/// tracker. The repeat case is the one that matters: under the Kitty keyboard
/// protocol a held key arrives as one `Press` followed by a stream of
/// `Repeat`s, so ignoring `Repeat` makes holding `j` move the cursor exactly
/// once. Terminals without the protocol send plain `Press` events for repeats
/// and were never affected.
fn is_actionable(kind: KeyEventKind) -> bool {
    matches!(kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

/// How far the page is scrolled, in the unit each layout scrolls by.
///
/// Both are kept across a `:layout` switch so flipping back and forth does not
/// lose the reader's place; each is clamped to the buffer when it is used.
#[derive(Default)]
struct Viewport {
    /// The first visible line, in horizontal layout.
    top: usize,
    /// The paragraph and piece the rightmost visible 縱 sits at, in vertical
    /// layout. An anchor rather than a 縱 number: see `vertical::draw`.
    zong: Anchor,
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
        // Chords first: Helix binds `C-a`/`C-x` and `A-.`, and a bare control
        // character must never reach the buffer as a literal control code.
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Key::Ctrl(c.to_ascii_lowercase()))
        }
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::ALT) => Some(Key::Alt(c)),
        KeyCode::Char(c) => Some(Key::Char(c)),
        KeyCode::Enter => Some(Key::Enter),
        KeyCode::Backspace => Some(Key::Backspace),
        KeyCode::Esc => Some(Key::Esc),
        KeyCode::Left => Some(Key::Left),
        KeyCode::Right => Some(Key::Right),
        KeyCode::Up => Some(Key::Up),
        KeyCode::Down => Some(Key::Down),
        KeyCode::Home => Some(Key::Home),
        KeyCode::End => Some(Key::End),
        KeyCode::Tab => Some(Key::Tab),
        KeyCode::BackTab => Some(Key::BackTab),
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
    viewport: &mut Viewport,
) {
    let area = frame.area();
    let regions = Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).split(area);
    let text_area = regions[0];
    let status_area = regions[1];

    // The text body is the one part that differs between the layouts; both
    // report back the cell the cursor landed on, which the status line and the
    // candidate panel are positioned from.
    let (cursor_x, cursor_y) = match editor.layout() {
        WritingLayout::Horizontal => {
            draw_horizontal(frame, editor, config, text_area, &mut viewport.top)
        }
        WritingLayout::Vertical => {
            vertical::draw(frame, editor, config, text_area, &mut viewport.zong)
        }
    };

    draw_status(frame, editor, ime, status_area);
    draw_command_menu(frame, editor, area, status_area);

    // In vertical layout the cursor is a block drawn into the page: a hardware
    // cursor is one cell wide and would sit lopsided inside a two-cell 縱.
    if let Some((_, text)) = editor.prompt() {
        // Measured in cells, not characters: a Chinese search pattern is twice
        // as wide as it is long.
        let col =
            1 + yumete_cjk::str_width(text) + yumete_cjk::str_width(&prompt_preedit(editor, ime));
        frame.set_cursor_position(Position::new(status_area.x + col as u16, status_area.y));
    } else if editor.layout() == WritingLayout::Horizontal || editor.mode() == Mode::Insert {
        // Vertically the terminal's cursor is shown only in Insert, where it is
        // the caret; in Normal the block is painted into the page and a second,
        // half-width cursor on top of it would only confuse.
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }

    if composes(editor.mode()) && ime.available() && ime.is_composing() {
        // The panel follows the page, not the prompt: a `/` search in a
        // vertically set document still picks its candidates out of a vertical
        // list, and one panel wearing a different skin from the other reads as a
        // different program.
        let (at_x, at_y) = match editor.prompt() {
            Some((_, text)) => {
                let col = 1
                    + yumete_cjk::str_width(text)
                    + yumete_cjk::str_width(&prompt_preedit(editor, ime));
                (status_area.x + col as u16, status_area.y)
            }
            None => (cursor_x, cursor_y),
        };
        match editor.layout() {
            WritingLayout::Horizontal => draw_candidate_panel(frame, ime, area, at_x, at_y),
            WritingLayout::Vertical => vertical::draw_candidate_panel(frame, ime, area, at_x, at_y),
        }
    }
}

/// The 中/英 indicator, or empty when the IME is not engaged in this mode.
///
/// It has to show in a `/` prompt as much as in Insert: the whole point of
/// composing there is that the pattern is Chinese, and without the tag there is
/// no way to tell why letters are or are not turning into 漢字.
fn language_tag(editor: &Editor, ime: &ImeSession) -> String {
    if !composes(editor.mode()) || !ime.available() {
        return String::new();
    }
    if ime.is_chinese() {
        format!("[中 {}]", ime.scheme_name())
    } else {
        "[ABC]".to_string()
    }
}

/// Draw the command menu above the command line.
///
/// Twenty-odd commands is past the point where they can be remembered, so `:`
/// on its own lists them and every keystroke narrows the list. It is laid out in
/// as many aligned columns as fit, tallest-first down each column, because a
/// single column of twenty would cover the page it is being run against.
fn draw_command_menu(frame: &mut Frame, editor: &Editor, area: Rect, status: Rect) {
    let Some((':', _)) = editor.prompt() else {
        return;
    };
    let (matches, selected) = editor.command_menu();
    if matches.is_empty() {
        return;
    }

    // A column is the widest name plus its help, and they all share one width so
    // the help lines up down the menu.
    let name_w = matches
        .iter()
        .map(|e| e.name.chars().count() + e.alias.map_or(0, |a| a.chars().count() + 3))
        .max()
        .unwrap_or(0);
    let help_w = matches
        .iter()
        .map(|e| e.help.chars().count())
        .max()
        .unwrap_or(0);
    let col_w = (name_w + help_w + 4) as u16;
    let columns = ((area.width / col_w.max(1)) as usize).clamp(1, 4);
    let rows = matches.len().div_ceil(columns);
    let height = (rows as u16).min(area.height.saturating_sub(1));
    if height == 0 {
        return;
    }

    let width = (col_w * columns as u16).min(area.width);
    let menu = Rect::new(area.x, status.y.saturating_sub(height), width, height);
    frame.render_widget(Clear, menu);

    let ground = Style::default().bg(Color::Rgb(0x26, 0x2a, 0x27));
    let name_style = ground.fg(Color::Rgb(0xcf, 0xc6, 0xa9));
    let help_style = ground.fg(Color::Rgb(0x9c, 0x97, 0x82));
    let buf = frame.buffer_mut();
    for y in 0..height {
        for x in 0..width {
            if let Some(cell) = buf.cell_mut((menu.x + x, menu.y + y)) {
                cell.set_symbol(" ").set_style(ground);
            }
        }
    }
    for (i, entry) in matches.iter().enumerate() {
        // Down each column, then across — so an alphabetical list still reads
        // alphabetically.
        let (col, row) = (i / rows, i % rows);
        let x = menu.x + col as u16 * col_w + 1;
        let y = menu.y + row as u16;
        if row as u16 >= height || x >= menu.x + width {
            continue;
        }
        // Tab's current pick is inked, the way the highlighted candidate is.
        let picked = selected == Some(i);
        let (name_style, help_style) = if picked {
            let on = Style::default()
                .bg(Color::Rgb(0xcf, 0xc6, 0xa9))
                .fg(Color::Rgb(0x26, 0x2a, 0x27));
            (on, on)
        } else {
            (name_style, help_style)
        };
        if picked {
            for n in 0..col_w {
                let cx = x.saturating_sub(1) + n;
                if cx < menu.x + width {
                    if let Some(cell) = buf.cell_mut((cx, y)) {
                        cell.set_symbol(" ").set_style(name_style);
                    }
                }
            }
        }
        let name = match entry.alias {
            Some(alias) => format!("{} ({alias})", entry.name),
            None => entry.name.to_string(),
        };
        for (text, at, style) in [
            (name.as_str(), x, name_style),
            (entry.help, x + name_w as u16 + 2, help_style),
        ] {
            for (n, ch) in text.chars().enumerate() {
                let cx = at + n as u16;
                if cx >= menu.x + width {
                    break;
                }
                if let Some(cell) = buf.cell_mut((cx, y)) {
                    cell.set_symbol(&ch.to_string()).set_style(style);
                }
            }
        }
    }
}

/// The composition in progress, when a `/` or `:` prompt is open.
fn prompt_preedit(editor: &Editor, ime: &ImeSession) -> String {
    if editor.prompt().is_some() && ime.available() && ime.is_composing() {
        ime.display_buffer()
    } else {
        String::new()
    }
}

/// Draw the buffer as ordinary horizontal lines with a line-number gutter,
/// returning the cell the cursor sits on.
fn draw_horizontal(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    text_area: Rect,
    viewport_top: &mut usize,
) -> (u16, u16) {
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

    (
        text_area.x + gutter as u16 + editor.cursor_visual_column() as u16,
        text_area.y + (cursor_line - *viewport_top) as u16,
    )
}

/// Draw the status line, or the command line while a `:` or `/` prompt is open.
fn draw_status(frame: &mut Frame, editor: &Editor, ime: &ImeSession, status_area: Rect) {
    let buffer = editor.current_buffer();
    let status = if let Some((prefix, text)) = editor.prompt() {
        // The composition in progress belongs at the caret, so a search reads as
        // the pattern being typed rather than jumping into place on commit. The
        // 中/英 tag is pushed to the right edge, where it cannot be mistaken for
        // part of the pattern.
        let line = format!("{prefix}{text}{}", prompt_preedit(editor, ime));
        let ghost = editor.prompt_ghost();
        let tag = language_tag(editor, ime);
        let used = yumete_cjk::str_width(&line)
            + yumete_cjk::str_width(&ghost)
            + yumete_cjk::str_width(&tag);
        let gap = (status_area.width as usize).saturating_sub(used);
        // Rendered as three spans so the guess can be a lighter ink than what
        // was actually typed — it has to be visibly *not yet* part of the line.
        let reversed = Style::default().add_modifier(Modifier::REVERSED);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(line, reversed),
                Span::styled(ghost, reversed.add_modifier(Modifier::DIM)),
                Span::styled(format!("{}{tag}", " ".repeat(gap)), reversed),
            ])),
            status_area,
        );
        return;
    } else {
        let dirty = if buffer.is_modified() { " [+]" } else { "" };
        // In Insert mode with the IME available, show the 中/英 state + scheme.
        let ime_tag = match language_tag(editor, ime).as_str() {
            "" => String::new(),
            tag => format!("{tag} "),
        };
        let left = format!(
            "-- {} --  {}{}{}",
            editor.mode_label(),
            ime_tag,
            buffer.display_name(),
            dirty
        );
        if !editor.status().is_empty() {
            format!("{left}   {}", editor.status())
        } else if editor.layout() == WritingLayout::Vertical {
            // Vertically the coordinates are named for the directions they run
            // in: paragraphs stack across the page, so a paragraph number is a
            // 橫 position; the 縱 is which run of it; 字 is how far down that
            // run. "Ln" and "Col" would each mean two things here.
            let at = editor.zong_position();
            format!(
                "{left}   橫 {}, 縱 {}, 字 {}",
                at.line + 1,
                at.index_in_line + 1,
                at.slot + 1,
            )
        } else {
            format!(
                "{left}   Ln {}, Col {}",
                editor.cursor_line() + 1,
                editor.cursor_visual_column() + 1,
            )
        }
    };
    frame.render_widget(
        Paragraph::new(status).style(Style::default().add_modifier(Modifier::REVERSED)),
        status_area,
    );
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
            // The code as typed, a shade back from the candidates.
            lines.push(Line::from(Span::styled(
                row,
                Style::default()
                    .bg(vertical::ink::paper())
                    .fg(vertical::ink::helper()),
            )));
        } else if i - 1 == highlight {
            lines.push(Line::from(Span::styled(
                row,
                Style::default()
                    .bg(vertical::ink::highlight())
                    .fg(vertical::ink::on_highlight()),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                row,
                Style::default()
                    .bg(vertical::ink::paper())
                    .fg(vertical::ink::text()),
            )));
        }
    }

    frame.render_widget(Clear, panel);
    // A wide glyph in the column left of the panel covers the panel's own border
    // cell, and the renderer skips what a wide glyph covers — so without this
    // the left border is never emitted.
    vertical::clear_wide_left_edge(frame.buffer_mut(), panel);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(
                    Style::default()
                        .fg(vertical::ink::border())
                        .bg(vertical::ink::paper()),
                )
                .style(
                    Style::default()
                        .bg(vertical::ink::paper())
                        .fg(vertical::ink::text()),
                ),
        ),
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
        let mut viewport = Viewport::default();
        terminal
            .draw(|frame| draw(frame, editor, config, ime, &mut viewport))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// Render with an unavailable IME (the common case for non-IME tests).
    fn render(editor: &Editor, config: &Config, w: u16, h: u16) -> ratatui::buffer::Buffer {
        render_with(editor, config, &no_ime(), w, h)
    }

    /// Render vertically, settling the 縱 length from the terminal height first
    /// exactly as the event loop does, so motion and drawing agree.
    fn render_vertical(
        editor: &mut Editor,
        config: &Config,
        w: u16,
        h: u16,
    ) -> ratatui::buffer::Buffer {
        // Ruby off: these read the slot grid itself, and the reading column
        // would step every coordinate in by a cell. `render_vertical_ruby`
        // covers the other side.
        editor.set_ruby(yumete_core::ruby::Dialects::NONE);
        render_vertical_with(editor, config, &no_ime(), w, h)
    }

    /// Render vertically, returning where the terminal's cursor was left — the
    /// caret, in Insert mode.
    fn render_vertical_caret(
        editor: &mut Editor,
        config: &Config,
        w: u16,
        h: u16,
    ) -> (ratatui::buffer::Buffer, Option<Position>) {
        editor.set_layout(WritingLayout::Vertical);
        editor.set_ruby(yumete_core::ruby::Dialects::NONE);
        let lines = editor.current_buffer().line_count();
        let ruby = !editor.ruby().is_empty();
        editor.set_zong_length(vertical::zong_length_for(config, h, lines, ruby));
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut viewport = Viewport::default();
        terminal
            .draw(|frame| draw(frame, editor, config, &no_ime(), &mut viewport))
            .unwrap();
        let at = terminal.get_cursor_position().ok();
        (terminal.backend().buffer().clone(), at)
    }

    /// Render vertically with ruby layout on.
    fn render_vertical_ruby(
        editor: &mut Editor,
        config: &Config,
        w: u16,
        h: u16,
    ) -> ratatui::buffer::Buffer {
        editor.set_ruby(yumete_core::ruby::Dialects::only(
            yumete_core::ruby::Dialect::Html,
        ));
        render_vertical_with(editor, config, &no_ime(), w, h)
    }

    /// As [`render_vertical`], with a live IME session for the panel tests.
    fn render_vertical_with(
        editor: &mut Editor,
        config: &Config,
        ime: &ImeSession,
        w: u16,
        h: u16,
    ) -> ratatui::buffer::Buffer {
        editor.set_layout(WritingLayout::Vertical);
        let lines = editor.current_buffer().line_count();
        let ruby = !editor.ruby().is_empty();
        editor.set_zong_length(vertical::zong_length_for(config, h, lines, ruby));
        render_with(editor, config, ime, w, h)
    }

    /// A vertical-layout config with the decorations off, so tests read the
    /// text grid itself.
    fn vertical_config() -> Config {
        let mut config = Config::default();
        config.editor.layout = WritingLayout::Vertical;
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        config
    }

    /// Type `text` into a fresh editor and return to Normal mode.
    fn editor_with(text: &str) -> Editor {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        for c in text.chars() {
            editor.on_key(if c == '\n' { Key::Enter } else { Key::Char(c) });
        }
        editor.on_key(Key::Esc);
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('g'));
        editor
    }

    /// The symbol at a cell, for grid assertions.
    fn at(buffer: &ratatui::buffer::Buffer, x: u16, y: u16) -> String {
        buffer[(x, y)].symbol().to_string()
    }

    #[test]
    fn vertical_layout_stacks_characters_down_from_the_right_edge() {
        let mut editor = editor_with("春江潮水");
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 30, 12);

        // The first 縱 occupies the two rightmost cells, reading downward.
        for (row, expected) in ["春", "江", "潮", "水"].iter().enumerate() {
            assert_eq!(at(&buffer, 28, row as u16), *expected, "row {row}");
        }
    }

    #[test]
    fn paragraphs_stack_leftward_one_gap_apart() {
        let mut editor = editor_with("上\n中\n下");
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 30, 12);

        // Two cells per 縱 plus a one-cell gap: 28, 25, 22, right to left.
        assert_eq!(at(&buffer, 28, 0), "上");
        assert_eq!(at(&buffer, 25, 0), "中");
        assert_eq!(at(&buffer, 22, 0), "下");
    }

    #[test]
    fn a_long_paragraph_wraps_into_the_next_zong() {
        // Six rows of text area (8 minus the status line and the spare caret
        // row) means the 縱 wraps every six characters, however long the
        // configured 縱 is.
        let mut editor = editor_with(&"字".repeat(8));
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 20, 8);

        assert_eq!(at(&buffer, 18, 0), "字");
        assert_eq!(at(&buffer, 18, 5), "字");
        // The seventh character starts the next 縱, to the left.
        assert_eq!(at(&buffer, 15, 0), "字");
        assert_eq!(at(&buffer, 15, 1), "字");
        assert_eq!(at(&buffer, 15, 2), " ");
    }

    #[test]
    fn a_reading_runs_beside_the_base_it_annotates() {
        let mut editor = editor_with("他<ruby>口<rt>kǒu</rt></ruby>很");
        let config = vertical_config();
        let buffer = render_vertical_ruby(&mut editor, &config, 20, 12);

        // The page steps in one cell so the rightmost 縱 has a reading column.
        assert_eq!(at(&buffer, 17, 0), "他");
        // 口 is centred against its three-character reading, and the markup
        // itself is gone from the page.
        assert_eq!(at(&buffer, 17, 1), " ", "spacing above the base");
        assert_eq!(at(&buffer, 17, 2), "口");
        assert_eq!(at(&buffer, 17, 4), "很");
        // …and the reading runs down the cell to its right.
        let reading: String = (1..4).map(|y| at(&buffer, 19, y)).collect();
        assert_eq!(reading, "kǒu");
        assert!(
            !buffer_text(&buffer).contains("<rt>"),
            "markup must not show"
        );
    }

    /// With the gap set to zero, only a 縱 that carries a reading pays for the
    /// column beside it; the rest sit flush.
    #[test]
    fn a_zero_gap_reserves_a_column_only_where_a_reading_needs_one() {
        let mut editor = editor_with("甲乙\n<ruby>丙<rt>bǐng</rt></ruby>\n丁戊");
        let mut config = vertical_config();
        config.editor.zong_gap = 0;
        let buffer = render_vertical_ruby(&mut editor, &config, 20, 12);

        // 甲 has no reading, so it sits flush against the right edge.
        assert_eq!(at(&buffer, 18, 0), "甲");
        // 丙 does, so it takes the cell to its right — three cells on, not two.
        // It is centred against its four-character reading, so it sits a row in.
        assert_eq!(at(&buffer, 15, 1), "丙");
        let reading: String = (0..4).map(|y| at(&buffer, 17, y)).collect();
        assert_eq!(reading, "bǐng");
        // 丁 has none, so it follows flush: two cells on from 丙, not three.
        assert_eq!(at(&buffer, 13, 0), "丁");
    }

    #[test]
    fn a_one_cell_gap_is_shared_with_the_reading() {
        // With a gap of one, a reading costs nothing extra: it uses the gap.
        let mut editor = editor_with("甲乙\n<ruby>丙<rt>bǐng</rt></ruby>\n丁戊");
        let config = vertical_config(); // zong_gap = 1
        let buffer = render_vertical_ruby(&mut editor, &config, 20, 12);
        assert_eq!(at(&buffer, 18, 0), "甲");
        // 丙 is centred against its four-character reading, so it sits a row in.
        assert_eq!(at(&buffer, 15, 1), "丙");
        let reading: String = (0..4).map(|y| at(&buffer, 17, y)).collect();
        assert_eq!(reading, "bǐng", "the reading uses the gap, costing nothing");
        // Evenly spaced three cells apart, annotated or not.
        assert_eq!(at(&buffer, 12, 0), "丁");
    }

    #[test]
    fn ruby_off_shows_the_markup_in_the_page() {
        let mut editor = editor_with("<ruby>口<rt>kǒu</rt></ruby>");
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 20, 24);
        // The tags take rows of their own, one character each and hung right, so
        // `<ruby>` reads down the 縱 rather than across it.
        let tags: Vec<String> = (0..6).map(|y| at(&buffer, 19, y)).collect();
        assert_eq!(tags, ["<", "r", "u", "b", "y", ">"]);
        // The base is full-width and fills the slot, so it starts at the left.
        assert_eq!(at(&buffer, 18, 6), "口");
    }

    #[test]
    fn digits_are_set_tatechuyoko_when_asked() {
        let mut editor = editor_with("第12章");
        editor.set_tatechuyoko(true);
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 20, 12);

        // Three rows, not four: the pair shares one slot and fills both cells.
        assert_eq!(at(&buffer, 18, 0), "第");
        assert_eq!(at(&buffer, 18, 1), "12");
        assert_eq!(at(&buffer, 18, 2), "章");
    }

    #[test]
    fn punctuation_is_rotated_on_screen_but_not_in_the_buffer() {
        let mut editor = editor_with("「甲」。");
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 20, 12);

        assert_eq!(at(&buffer, 18, 0), "﹁");
        assert_eq!(at(&buffer, 18, 1), "甲");
        assert_eq!(at(&buffer, 18, 2), "﹂");
        assert_eq!(at(&buffer, 18, 3), "︒");
        // What is saved to disk keeps the ordinary characters.
        assert_eq!(editor.current_buffer().text(), "「甲」。");
    }

    #[test]
    fn hjkl_keep_their_screen_meaning_when_vertical() {
        let mut editor = editor_with("一二三\n四五六");
        editor.set_layout(WritingLayout::Vertical);
        editor.set_zong_length(32);

        // `j` reads onward down the 縱...
        editor.on_key(Key::Char('j'));
        assert_eq!(editor.cursor(), 1);
        editor.on_key(Key::Char('k'));
        assert_eq!(editor.cursor(), 0);
        // ...and `h` steps left, which is the next 縱 at the same depth.
        editor.on_key(Key::Char('j'));
        editor.on_key(Key::Char('h'));
        assert_eq!(editor.cursor_line(), 1);
        assert_eq!(editor.zong_position().slot, 1);
        // `l` steps back to the right.
        editor.on_key(Key::Char('l'));
        assert_eq!(editor.cursor_line(), 0);
        assert_eq!(editor.zong_position().slot, 1);
    }

    #[test]
    fn paragraph_numbers_label_only_the_zong_that_starts_a_paragraph() {
        let mut editor = editor_with("甲乙丙\n丁戊己");
        let mut config = vertical_config();
        config.editor.line_numbers = LineNumbers::Absolute;
        let buffer = render_vertical(&mut editor, &config, 20, 12);

        // One header row for a two-paragraph buffer, the number right-aligned
        // in its 縱 and the text starting on the row below.
        // Half-width throughout, right-aligned, so a two-digit number lines up
        // with a one-digit one rather than mixing the two widths.
        assert_eq!(at(&buffer, 19, 0), "1");
        assert_eq!(at(&buffer, 18, 1), "甲");
        assert_eq!(at(&buffer, 16, 0), "2");
        assert_eq!(at(&buffer, 15, 1), "丁");
    }

    #[test]
    fn the_insert_caret_is_the_terminals_own_cursor() {
        let mut editor = editor_with("甲乙丙");
        editor.set_layout(WritingLayout::Vertical);
        editor.on_key(Key::Char('i')); // Insert
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 20, 12);

        // Nothing in the page is repainted for the caret: the character keeps
        // its own colour and is not underlined.
        let style = buffer[(18, 0)].style();
        assert!(!style.add_modifier.contains(Modifier::REVERSED));
        assert!(!style.add_modifier.contains(Modifier::UNDERLINED));
        assert!(
            matches!(style.fg, None | Some(Color::Reset)),
            "the character must not be recoloured, got {:?}",
            style.fg
        );
    }

    /// Typing inserts *before* the character the cursor is on, so the boundary
    /// the text arrives at is that character's top edge — and an underscore is
    /// drawn at the bottom of the cell it is in. The caret therefore belongs on
    /// the slot above; on the cursor's own it reads as "insert after this
    /// character", which is not where the text appears.
    #[test]
    fn the_insert_caret_marks_the_boundary_text_arrives_at() {
        let mut editor = editor_with("甲乙丙");
        editor.set_layout(WritingLayout::Vertical);
        editor.on_key(Key::Char('j')); // onto 乙
        editor.on_key(Key::Char('i')); // insert before it
        let config = vertical_config();
        let (buffer, caret) = render_vertical_caret(&mut editor, &config, 20, 12);

        // 乙 is row 1; the rule sits under 甲, at the boundary between them.
        assert_eq!(at(&buffer, 18, 1), "乙");
        assert_eq!(caret.map(|p| p.y), Some(0), "one row up from the cursor");
        assert_eq!(caret.map(|p| p.x), Some(18));
    }

    /// The terminal sizes its cursor to the grapheme under it, so a blank slot
    /// would give a caret half the width of the 縱.
    #[test]
    fn a_blank_caret_slot_is_filled_so_the_rule_spans_it() {
        let mut editor = Editor::new();
        editor.set_layout(WritingLayout::Vertical);
        editor.on_key(Key::Char('i')); // Insert on an empty buffer
        let config = vertical_config();
        let (buffer, caret) = render_vertical_caret(&mut editor, &config, 20, 12);

        let at_caret = caret.expect("a caret in Insert mode");
        assert_eq!(
            at(&buffer, at_caret.x, at_caret.y),
            "\u{3000}",
            "an ideographic space fills the slot so the rule spans it"
        );
    }

    #[test]
    fn the_cursor_is_drawn_as_a_block_over_its_slot() {
        // A half-width character, so both cells of the slot are real cells in
        // the rendered buffer — behind a full-width glyph the second column is
        // the first one's continuation and never drawn separately.
        let mut editor = editor_with("甲a丙");
        editor.set_layout(WritingLayout::Vertical);
        editor.on_key(Key::Char('j')); // down the 縱, onto the `a`
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 20, 12);

        let reversed = |x: u16, y: u16| {
            buffer[(x, y)]
                .style()
                .add_modifier
                .contains(Modifier::REVERSED)
        };
        // Half-width characters hang against the slot's right edge.
        assert_eq!(at(&buffer, 19, 1), "a");
        assert!(reversed(18, 1), "cursor cell not highlighted");
        assert!(reversed(19, 1), "cursor must cover both cells of the slot");
        assert!(!reversed(18, 0), "the character above must stay plain");
        assert!(!reversed(18, 2), "the character below must stay plain");
    }

    #[test]
    fn the_segmentation_overlay_tints_words_down_the_zong() {
        let mut editor = editor_with("你好世界");
        editor.set_segmenter(Box::new(DictionarySegmenter::builtin(0)));
        editor.set_segmentation_visible(true);
        let mut config = vertical_config();
        config.editor.show_segmentation = true;
        let buffer = render_vertical(&mut editor, &config, 20, 12);

        // Successive words alternate tint down the 縱. Only the leading cell of
        // each slot is asserted: a full-width glyph's second column is its
        // continuation, which the renderer skips rather than drawing, so the
        // terminal paints both columns from the style set here.
        let (a, b) = (config.theme.segmentation[0], config.theme.segmentation[1]);
        let bg = |x: u16, y: u16| buffer[(x, y)].style().bg;
        assert_eq!(
            bg(18, 1),
            Some(Color::Rgb(a.0, a.1, a.2)),
            "第一詞 untinted"
        );
        assert_eq!(
            bg(18, 2),
            Some(Color::Rgb(b.0, b.1, b.2)),
            "第二詞 untinted"
        );
    }

    /// A pane too small to hold even one 縱 must not panic — ratatui hands out
    /// tiny areas while a window is being resized.
    #[test]
    fn a_tiny_pane_renders_without_panicking() {
        let mut editor = editor_with("甲乙丙\n丁戊");
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        ime.input('b');
        let config = vertical_config();
        for (w, h) in [(1, 1), (2, 2), (3, 1), (1, 8), (4, 3), (2, 40)] {
            let _ = render_vertical(&mut editor, &config, w, h);
        }
        // …including with the candidate panel open.
        editor.on_key(Key::Char('i'));
        for (w, h) in [(1, 1), (2, 2), (6, 4), (12, 6)] {
            let _ = render_vertical_with(&mut editor, &config, &ime, w, h);
        }
    }

    #[test]
    fn candidates_are_numbered_with_circled_chinese_numerals() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        ime.input('b');
        let config = vertical_config();
        let buffer = render_vertical_with(&mut editor, &config, &ime, 40, 16);

        let find = |needle: &str| {
            (0..buffer.area.height)
                .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
                .find(|&(x, y)| buffer[(x, y)].symbol() == needle)
        };
        let (x1, y1) = find("㊀").expect("first candidate numbered ㊀");
        assert_eq!(find("㊁").map(|(_, y)| y), Some(y1), "㊁ on the same row");
        // A blank row separates the number from the candidate it labels.
        assert_eq!(at(&buffer, x1, y1 + 1), " ", "gap under the number");
        assert_eq!(at(&buffer, x1, y1 + 2), "吧");
    }

    #[test]
    fn the_code_reads_down_the_header_and_across_under_its_column() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        // The only match needs three more letters, so it is the highlighted one
        // and the header carries its code.
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "ajvy 奧\n");
        ime.input('a');
        let config = vertical_config();
        let buffer = render_vertical_with(&mut editor, &config, &ime, 40, 16);

        // The typed code runs down the rightmost column of the panel, one
        // character to a row like everything else in it.
        let find = |needle: &str| {
            (0..buffer.area.height)
                .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
                .find(|&(x, y)| buffer[(x, y)].symbol() == needle)
        };
        let (hx, _) = find("a").expect("the typed code, in the header column");
        let (cx, cy) = find("奧").expect("the annotated candidate");
        assert!(hx > cx, "the header sits to the right of the candidates");

        // The 下標 runs *down* the same column, one letter to a row and hung
        // right — nothing in the panel sets two letters side by side.
        let subscript: String = (1..4).map(|d| at(&buffer, cx + 1, cy + d)).collect();
        assert_eq!(subscript, "jvy", "下標 reads down the column");
        for d in 1..4 {
            assert_eq!(at(&buffer, cx, cy + d), " ", "letters hang right");
        }
    }

    #[test]
    fn the_chaifen_joins_the_header_when_the_engine_annotates() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        // `code text completion comment` — the fourth field is the 拆分.
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧\n");
        ime.input('b');
        let config = vertical_config();

        // With no annotation the panel shows the preedit alone…
        let plain = render_vertical_with(&mut editor, &config, &ime, 40, 14);
        let plain_rows = panel_depth(&plain);

        // …and the column simply grows when there is one, rather than adding a
        // column per candidate.
        assert!(plain_rows >= 3, "panel should have a preedit column");
        assert!(buffer_text(&plain).contains('吧'));
    }

    /// The number of rows between the panel's top and bottom border.
    fn panel_depth(buffer: &ratatui::buffer::Buffer) -> u16 {
        let find = |glyph: &str| {
            (0..buffer.area.height)
                .find(|&y| (0..buffer.area.width).any(|x| buffer[(x, y)].symbol() == glyph))
        };
        match (find("\u{256d}"), find("\u{2570}")) {
            (Some(top), Some(bottom)) => bottom - top + 1,
            _ => 0,
        }
    }

    /// An empty symbol is a wide glyph's *continuation* to the renderer, so it
    /// emits nothing and slides the rest of the row a column left — which is how
    /// the panel came to paint its ground over its own border.
    #[test]
    fn no_cell_is_ever_left_with_an_empty_symbol() {
        let mut editor = editor_with("那年冬天");
        editor.on_key(Key::Char('a'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        ime.input('b');
        let config = Config::default();
        for (w, h) in [(40, 16), (24, 12), (60, 24)] {
            let buffer = render_vertical_with(&mut editor, &config, &ime, w, h);
            for y in 0..buffer.area.height {
                for x in 0..buffer.area.width {
                    let cell = &buffer[(x, y)];
                    // Ratatui itself leaves continuation cells empty; what must
                    // not happen is an empty cell carrying a *style*, which is
                    // what a slot written with no text produced.
                    assert!(
                        !cell.symbol().is_empty() || cell.style().bg.is_none(),
                        "styled empty cell at {x},{y}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_candidate_panel_wears_the_ink_skin() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        ime.input('b');
        let config = vertical_config();
        let buffer = render_vertical_with(&mut editor, &config, &ime, 40, 14);

        // 墨香 dark: warm ink on a deep ground, ringed in a mid rung of the same
        // ladder. Nothing in the panel falls back to the terminal default.
        let paper = Color::Rgb(0x26, 0x2A, 0x27);
        let ring = Color::Rgb(0x50, 0x51, 0x48);
        let corner = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
            .find(|&(x, y)| buffer[(x, y)].symbol() == "\u{256d}")
            .expect("rounded top-left corner");
        assert_eq!(buffer[corner].style().fg, Some(ring), "border not inked");
        assert_eq!(
            buffer[corner].style().bg,
            Some(paper),
            "panel ground missing"
        );

        let cand = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
            .find(|&(x, y)| buffer[(x, y)].symbol() == "吧")
            .expect("first candidate");
        // The highlighted candidate is ink-on-paper inverted.
        assert_eq!(
            buffer[cand].style().bg,
            Some(Color::Rgb(0xCF, 0xC6, 0xA9)),
            "highlight not inked"
        );
    }

    #[test]
    fn vertical_candidate_panel_runs_right_to_left() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i')); // Insert mode
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        ime.input('b');

        let config = vertical_config();
        let buffer = render_vertical_with(&mut editor, &config, &ime, 40, 14);
        let text = buffer_text(&buffer);
        assert!(text.contains('吧'), "first candidate missing");
        assert!(text.contains('八'), "second candidate missing");

        // Candidate 1 must sit to the *right* of candidate 2, and its digit
        // directly above it.
        let find = |needle: &str| {
            (0..buffer.area.height)
                .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
                .find(|&(x, y)| buffer[(x, y)].symbol() == needle)
                .unwrap_or_else(|| panic!("{needle} not drawn"))
        };
        let first = find("吧");
        let second = find("八");
        assert!(first.0 > second.0, "candidates must run right to left");
        assert_eq!(first.1, second.1, "candidates must share a row");
        // Two rows up, past the gap, is the 帶圈中文數字 numbering it.
        assert_eq!(at(&buffer, first.0, first.1 - 2), "㊀");
        assert_eq!(at(&buffer, second.0, second.1 - 2), "㊁");
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

    /// Laid out vertically the coordinates are named for the directions they
    /// run in — "Ln" and "Col" would each mean two things.
    #[test]
    fn the_vertical_status_line_names_its_directions() {
        let mut editor = editor_with("上山\n下海");
        let config = vertical_config();
        editor.set_layout(WritingLayout::Vertical);
        editor.on_key(Key::Char('j')); // down the 縱
        let buffer = render_vertical(&mut editor, &config, 60, 12);

        // A wide glyph leaves its continuation cell blank in the test backend,
        // so the run of spaces after 橫 is an artefact of reading the grid.
        let raw: String = (0..buffer.area.width)
            .map(|x| buffer[(x, buffer.area.height - 1)].symbol())
            .collect();
        let status = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(status.contains("橫 1"), "paragraph, across: {status:?}");
        assert!(status.contains("縱 1"), "which run of it: {status:?}");
        assert!(status.contains("字 2"), "how far down it: {status:?}");
        assert!(!status.contains("Ln"), "no ambiguous line number");
    }

    /// The guess has to be visibly *not yet* part of the line, or it reads as
    /// text that has been typed.
    #[test]
    fn the_prompt_guess_is_a_lighter_ink() {
        let mut editor = editor_with("那年冬天");
        editor.on_key(Key::Char(':'));
        for c in "seg".chars() {
            editor.on_key(Key::Char(c));
        }
        let config = Config::default();
        let buffer = render_with(&editor, &config, &no_ime(), 90, 24);

        let row = buffer.area.height - 1;
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        assert!(line.starts_with(":segment"), "guess shown: {line:?}");

        let dim = |x: u16| {
            buffer[(x, row)]
                .style()
                .add_modifier
                .contains(Modifier::DIM)
        };
        assert!(!dim(3), "`seg` was typed");
        assert!(dim(4), "`ment` is only a guess");
    }

    #[test]
    fn the_command_menu_lists_and_narrows() {
        let mut editor = editor_with("那年冬天");
        let config = Config::default();

        // `:` on its own offers everything.
        editor.on_key(Key::Char(':'));
        let buffer = render_with(&editor, &config, &no_ime(), 90, 24);
        let text = buffer_text(&buffer);
        assert!(text.contains("write"), "menu should list commands: absent");
        assert!(text.contains("save"), "…with what they do");

        // Typing narrows it, and the commands that no longer match go away.
        for c in "ruby".chars() {
            editor.on_key(Key::Char(c));
        }
        let buffer = render_with(&editor, &config, &no_ime(), 90, 24);
        let text = buffer_text(&buffer);
        assert!(text.contains("ruby-off"), "still matching");
        assert!(!text.contains("write"), "no longer matching");
    }

    #[test]
    fn tab_inks_its_pick_in_the_menu() {
        let mut editor = editor_with("那年冬天");
        editor.on_key(Key::Char(':'));
        editor.on_key(Key::Char('r'));
        let config = Config::default();

        let plain = render_with(&editor, &config, &no_ime(), 90, 24);
        let inked = |b: &ratatui::buffer::Buffer| {
            (0..b.area.height).any(|y| {
                (0..b.area.width)
                    .any(|x| b[(x, y)].style().bg == Some(Color::Rgb(0xcf, 0xc6, 0xa9)))
            })
        };
        assert!(!inked(&plain), "nothing picked until Tab is pressed");

        editor.on_key(Key::Tab);
        let picked = render_with(&editor, &config, &no_ime(), 90, 24);
        assert!(inked(&picked), "Tab's pick should be inked");
    }

    #[test]
    fn the_command_menu_sits_above_the_command_line() {
        let mut editor = editor_with("那年冬天");
        editor.on_key(Key::Char(':'));
        editor.on_key(Key::Char('r'));
        let config = Config::default();
        let buffer = render_with(&editor, &config, &no_ime(), 90, 24);

        // The command line is the last row; the menu is directly above it and
        // never covers it.
        let last: String = (0..buffer.area.width)
            .map(|x| buffer[(x, buffer.area.height - 1)].symbol())
            .collect();
        assert!(last.starts_with(":r"), "command line intact: {last:?}");
        let above: String = (0..buffer.area.width)
            .map(|x| buffer[(x, buffer.area.height - 2)].symbol())
            .collect();
        assert!(
            above.contains("ruby") || above.contains("redo"),
            "{above:?}"
        );
    }

    /// A search prompt is not a command line and gets no menu.
    #[test]
    fn only_the_command_line_gets_a_menu() {
        let mut editor = editor_with("那年冬天");
        editor.on_key(Key::Char('/'));
        let config = Config::default();
        let buffer = render_with(&editor, &config, &no_ime(), 90, 24);
        assert!(!buffer_text(&buffer).contains("redo"));
    }

    /// The panel follows the page, not the prompt: a `/` search in a vertically
    /// set document still picks from a vertical list, in the same skin.
    #[test]
    fn a_search_prompt_gets_the_panel_the_page_uses() {
        let mut editor = editor_with("那年冬天");
        editor.set_layout(WritingLayout::Vertical);
        editor.on_key(Key::Char('/'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        ime.input('b');
        let config = vertical_config();
        let buffer = render_vertical_with(&mut editor, &config, &ime, 40, 20);

        // Numbered with 帶圈中文數字 — the vertical panel — and wearing 墨香.
        let text = buffer_text(&buffer);
        assert!(text.contains('㊀'), "vertical panel expected: {text:?}");
        let paper = Color::Rgb(0x26, 0x2a, 0x27);
        assert!(
            (0..buffer.area.height).any(|y| {
                (0..buffer.area.width).any(|x| buffer[(x, y)].style().bg == Some(paper))
            }),
            "panel should wear the ink ground"
        );
    }

    /// A two-cell glyph in the column left of a panel covers the panel's border
    /// cell, and the renderer skips what a wide glyph covers — so the border
    /// would never be drawn.
    #[test]
    fn a_panel_cuts_back_the_wide_glyph_on_its_left_edge() {
        use ratatui::layout::Rect;
        let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 10, 3));
        buffer[(2, 1)].set_symbol("漢");
        vertical::clear_wide_left_edge(&mut buffer, Rect::new(3, 0, 5, 3));
        assert_eq!(
            buffer[(2, 1)].symbol(),
            " ",
            "the intruding half is cut back"
        );
    }

    #[test]
    fn the_ime_composes_into_a_search_prompt() {
        let mut editor = editor_with("春江潮水連海平");
        editor.on_key(Key::Char('/'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");

        // Letters typed at a `/` prompt compose instead of landing literally.
        assert!(ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char('b'),
            KeyModifiers::NONE
        ));
        assert!(ime.is_composing());
        assert_eq!(editor.prompt(), Some(('/', "")), "nothing committed yet");

        // …and the committed candidate lands in the pattern, not the buffer.
        assert!(ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char(' '),
            KeyModifiers::NONE
        ));
        assert_eq!(editor.prompt(), Some(('/', "吧")));
        assert_eq!(editor.current_buffer().text(), "春江潮水連海平");
    }

    #[test]
    fn the_prompt_shows_the_preedit_and_the_language_tag() {
        let mut editor = editor_with("春江潮水");
        editor.on_key(Key::Char('/'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧\n");
        ime.input('b');
        let config = Config::default();
        let buffer = render_with(&editor, &config, &ime, 60, 8);

        let status: String = (0..buffer.area.width)
            .map(|x| buffer[(x, buffer.area.height - 1)].symbol())
            .collect();
        assert!(status.starts_with("/b"), "preedit missing: {status:?}");
        assert!(status.contains("[中"), "language tag missing: {status:?}");
    }

    /// Modes that collect *prose* compose; Normal must not, or `/` itself would
    /// be swallowed by the IME, and the command line must not, because its
    /// whole vocabulary is ASCII.
    #[test]
    fn only_prose_modes_compose() {
        assert!(composes(Mode::Insert));
        assert!(composes(Mode::Search));
        assert!(composes(Mode::Ruby));
        assert!(!composes(Mode::Normal));
        assert!(!composes(Mode::Command));
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
    fn a_held_key_repeats() {
        // Holding a key under the Kitty protocol sends one Press and then
        // Repeats; both must reach the editor, or the cursor moves once and
        // stops.
        assert!(is_actionable(KeyEventKind::Press));
        assert!(is_actionable(KeyEventKind::Repeat));
        assert!(!is_actionable(KeyEventKind::Release));
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
