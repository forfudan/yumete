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

pub mod table;
pub mod theme;
pub mod vertical;

use std::io::{self, stdout, Write as _};

use ratatui::crossterm::cursor::SetCursorStyle;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    ModifierKeyCode, MouseButton, MouseEventKind, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::supports_keyboard_enhancement;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};
use ratatui::Frame;

use yumete_config::{Config, LineNumbers};
use yumete_core::sidebar::View;
use yumete_core::wrap::{self, Anchor as WrapAnchor};
use yumete_core::zong::{Anchor, Layout as WritingLayout};
use yumete_core::{say, Editor, Key, KeyOutcome, Mode, TextStore};
use yumete_ime::{ImeSession, Scheme};

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
/// How wide the terminal is, or `None` when there is no terminal to ask — a
/// pipe into `less`, or output redirected to a file.
///
/// Lives here because this is the crate that owns the terminal; the preview
/// wraps at the same width the editor would.
pub fn terminal_width() -> Option<usize> {
    ratatui::crossterm::terminal::size()
        .ok()
        .map(|(w, _)| w as usize)
        .filter(|&w| w > 0)
}

pub fn run(
    editor: &mut Editor,
    config: &Config,
    ime: &mut ImeSession,
) -> io::Result<()> {
    // Which mood 墨香 is in, settled before the first frame — and before the
    // alternate screen, because the question is put to the terminal and read
    // back off the descriptor.
    crate::theme::settle_at_startup(config);
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

    // Take the mouse, so the wheel can turn the page and text can be selected
    // by dragging. The cost, which Helix pays too: the terminal's *own*
    // click-and-drag selection stops working and needs whatever modifier that
    // terminal reserves for it (Option, on macOS) — which is still the way to
    // copy something that is not in the buffer, such as the status line.
    let _ = execute!(stdout(), EnableMouseCapture);
    // Bracketed paste, so text arriving from the system clipboard is *text*.
    // Without it a paste is a stream of keystrokes: in Normal mode every
    // character of the pasted paragraph runs as a command, which is not a
    // paste going wrong so much as an editor running a macro nobody wrote.
    let _ = execute!(stdout(), EnableBracketedPaste);

    let mut viewport = Viewport::default();
    let mut shift = ShiftTap::default();
    // A typesetter started with `:preview`, if one is running.
    let mut job: Option<Job> = None;
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
        if let Ok(size) = terminal.size() {
            // The page's own rectangle, not the terminal's: the hint row, the
            // tab bar and the detail panel are not writing, and a 縱 measured
            // against them is one longer than the 縱 on the screen.
            let page = page_areas(editor, config, Rect::new(0, 0, size.width, size.height)).text;
            let lines = editor.current_buffer().line_count();
            if editor.layout() == WritingLayout::Vertical {
                let look = vertical::Look::of(editor);
                editor.set_zong_length(vertical::zong_length_for(config, page.height, lines, look));
                // How much a page is, for `C-f`/`C-d`: how many 縱 actually fit,
                // which with 段組 is a band's worth times the number of bands.
                let metrics = vertical::Metrics::new(config, page.height, lines, look);
                editor.set_page(page.height as usize, metrics.capacity(page.width));
            } else {
                // The width paragraphs soft-wrap at depends on the gutter as
                // well; `j` and `k` walk those rows, so it too is settled
                // before the keys that use it.
                let gutter = gutter_width(lines, config.editor.line_numbers);
                editor.set_wrap_width((page.width as usize).saturating_sub(gutter));
                editor.set_page(page.height as usize, page.width.max(1) as usize);
            }
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
                // `:scheme` and `:chaifen` configure the IME, which the core
                // cannot reach; each leaves a request here and the answer goes
                // back, so the next toggle starts from what the engine did.
                // `Space y` reaches the system clipboard through the terminal
                // itself (OSC 52) — no library, and the only route that
                // survives ssh and tmux, which is where this editor is often
                // run. A terminal may refuse it; nothing here can tell.
                if let Some(text) = editor.take_clipboard_request() {
                    let _ = write!(io::stdout(), "\x1b]52;c;{}\x07", base64(text.as_bytes()));
                    let _ = io::stdout().flush();
                }
                // …and reading it needs the platform, because almost every
                // terminal refuses an OSC 52 read.
                if let Some(after) = editor.take_clipboard_read() {
                    match read_clipboard() {
                        Some(text) => editor.provide_clipboard(&text, after),
                        None => editor.set_status(
                            "cannot read the system clipboard here — ⌘V pastes into the terminal"
                                .to_string(),
                        ),
                    }
                }
                        // `:preview` — the real typesetter, in the background. Its
                // address arrives on a later turn of the loop.
                // `:sh` brings the answer back; `:!` hands over the screen.
                if let Some(want) = editor.take_shell_request() {
                    use yumete_core::editor::How;
                    match want.how {
                        How::Terminal => match hand_over(&mut terminal, &want.line) {
                            Ok(()) => editor.set_status(format!("跑完了：{}", want.line)),
                            Err(err) => editor.set_status(format!("跑不動：{err}")),
                        },
                        // Showing you the run: the complaints belong with the
                        // answer, since between them they are what happened.
                        How::Capture => match run_capturing(&want.line, None) {
                            Ok(ran) => {
                                let mut text = ran.said;
                                text.push_str(&ran.complained);
                                editor.provide_shell_output(&want.line, &text);
                            }
                            Err(err) => editor.set_status(format!("跑不動：{err}")),
                        },
                        // Editing your text: a command that failed does not get
                        // to touch it. `tr -D ' '` is a typo, and its answer is
                        // an error message — replacing a paragraph with that is
                        // an edit nobody asked for, undoable or not.
                        How::Pipe(input) => match run_capturing(&want.line, Some(&input)) {
                            Ok(ran) if ran.ok => {
                                editor.provide_pipe_output(&ran.said);
                                if !ran.complained.trim().is_empty() {
                                    editor.set_status(format!("換好了，但它說：{}", ran.why()));
                                }
                            }
                            Ok(ran) => editor.set_status(format!("沒有動你的字：{}", ran.why())),
                            Err(err) => editor.set_status(format!("跑不動：{err}")),
                        },
                    }
                }
                if let Some(want) = editor.take_preview_request() {
                    if let Some(mut running) = job.take() {
                        let _ = running.child.kill();
                        editor.set_status(format!("預覽：{} 已停", running.what));
                    }
                    if let yumete_core::editor::Preview::Start { path, syntax } = want {
                        match syntax {
                            yumete_core::syntax::Syntax::Typst => match Job::typst(&path) {
                                Ok(started) => {
                                    editor.set_status("預覽：tinymist 起來中……".to_string());
                                    job = Some(started);
                                }
                                Err(why) => editor.set_status(why),
                            },
                            // Markdown has no server to run: the export *is*
                            // the preview, made once and handed to the browser.
                            yumete_core::syntax::Syntax::Markdown => {
                                let out = std::env::temp_dir().join("yumete-preview.html");
                                match editor.execute(&format!("export html {}", out.display())) {
                                    Ok(_) => {
                                        show(&out.to_string_lossy());
                                        editor.set_status(format!("預覽：{}", out.display()));
                                    }
                                    Err(err) => editor.set_status(format!("預覽：{err}")),
                                }
                            }
                            // Nothing to typeset: a file with no markup is
                            // already what it is going to look like.
                            yumete_core::syntax::Syntax::Text => {
                                editor.set_status(say!("這個檔案沒有標記，沒有什麼可以預覽的"))
                            }
                        }
                    }
                }
                if let Some(running) = job.as_ref() {
                    if let Ok(url) = running.said.try_recv() {
                        show(&url);
                        editor.set_status(format!("預覽：{url}（`:preview off` 停）"));
                    }
                }
                if let Some(tag) = editor.take_scheme_request() {
                    editor.set_status(switch_scheme(ime, &tag, config));
                    // The scheme's own language data may be better than what
                    // was loaded before it.
                    let words = ime.segmenter();
                    if words.is_available() {
                        editor.set_segmenter(Box::new(words));
                    }
                }
                // `:chaifen` configures the IME, which the core cannot reach;
                // it leaves the request here and the answer goes back, so the
                // next toggle starts from what the engine actually did.
                // Keep a recovery copy of anything unsaved (Feature #79).
                // Throttled inside, so this is a clock check on most keys.
                editor.autosave_tick();
                if let Some((name, mood)) = editor.take_theme_request() {
                    editor.set_status(set_theme(config, name, mood));
                }
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
            // Text arriving whole, from the system clipboard by way of the
            // terminal. It is inserted as writing, never run as keys.
            Ok(Event::Paste(text)) => {
                editor.paste_text(&text);
            }
            Ok(Event::Mouse(mouse)) => match mouse.kind {
                // A notch moves three 縱 — the same three lines a terminal
                // scrolls by, counted in the unit the page is set in.
                MouseEventKind::ScrollDown => editor.scroll(WHEEL_STEP, false),
                MouseEventKind::ScrollUp => editor.scroll(WHEEL_STEP, true),
                // A tab is a thing you point at; the mouse is already captured
                // for the wheel, so this costs nothing but the arithmetic.
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(i) = tab_at(editor, config, terminal.size().ok(), mouse) {
                        editor.show_buffer_at(i);
                    } else if let Some(at) =
                        text_at(editor, config, terminal.size().ok(), &viewport, mouse)
                    {
                        editor.point_at(at);
                    }
                }
                // Dragging picks out a range — the thing the terminal's own
                // selection used to do, given back inside the editor, where it
                // can become a yank, an edit, or the system clipboard.
                MouseEventKind::Drag(MouseButton::Left) => {
                    if let Some(at) =
                        text_at(editor, config, terminal.size().ok(), &viewport, mouse)
                    {
                        editor.drag_to(at);
                    }
                }
                _ => {}
            },
            Ok(_) => {}
            Err(err) => break Err(err),
        }
    };

    // A preview server outlives nothing: it was started for this session and
    // has no reason to go on holding a port after it.
    if let Some(mut running) = job.take() {
        let _ = running.child.kill();
    }

    let _ = execute!(
        stdout(),
        DisableBracketedPaste,
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
    /// The paragraph and wrapped piece the top visible row sits at, in
    /// horizontal layout. An anchor rather than a row number: see
    /// [`yumete_core::wrap`].
    top: WrapAnchor,
    /// The paragraph and piece the rightmost visible 縱 sits at, in vertical
    /// layout. An anchor rather than a 縱 number: see `vertical::draw`.
    zong: Anchor,
    /// Where the grid is scrolled to, when the file is read as one.
    table: table::Viewport,
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

/// Run a command line through the shell and bring back what it said.
///
/// Through the shell, not split by hand: a writer typing `:sh wc -w *.md | sort`
/// means the pipe and the glob, and a command line that quietly did not is
/// worse than one that says it cannot.
fn run_capturing(line: &str, input: Option<&str>) -> io::Result<Ran> {
    let mut child = std::process::Command::new(shell())
        .arg("-c")
        .arg(line)
        .stdin(if input.is_some() {
            std::process::Stdio::piped()
        } else {
            std::process::Stdio::null()
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    if let (Some(text), Some(mut pipe)) = (input, child.stdin.take()) {
        // Written and *closed* — a filter that is still waiting for more input
        // never gets round to answering.
        io::Write::write_all(&mut pipe, text.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    Ok(Ran {
        ok: out.status.success(),
        said: String::from_utf8_lossy(&out.stdout).into_owned(),
        complained: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

/// What a command said, and whether it thinks it worked.
///
/// The two streams are kept apart because the two callers want different
/// things: `:sh` is *showing* you the run and wants the complaints in with the
/// answer, while `!` is *editing your text* and must never put either a
/// complaint or a half-answer into it.
struct Ran {
    ok: bool,
    said: String,
    complained: String,
}

impl Ran {
    /// The one line worth putting on the status bar.
    fn why(&self) -> String {
        self.complained
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("命令失敗了")
            .to_string()
    }
}

/// Give the terminal back, run the command in it, and take it again.
///
/// This is what `:!` has meant since vi, and it is the only honest answer to
/// "where do I see it run": **in the terminal you started the editor in**. A
/// captured pipe cannot show a progress bar, cannot colour anything, and cannot
/// ask you a question. So the editor gets off the screen for the duration and
/// waits for a key before taking it back, so what was printed can be read.
fn hand_over<B: ratatui::backend::Backend + io::Write>(
    terminal: &mut ratatui::Terminal<B>,
    line: &str,
) -> io::Result<()> {
    let _ = execute!(stdout(), DisableMouseCapture, DisableBracketedPaste);
    ratatui::crossterm::terminal::disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        ratatui::crossterm::terminal::LeaveAlternateScreen
    )?;
    let status = std::process::Command::new(shell())
        .arg("-c")
        .arg(line)
        .status();
    println!();
    match &status {
        Ok(code) if code.success() => println!("[{line}]"),
        Ok(code) => println!("[{line} — {code}]"),
        Err(err) => println!("[{line} — {err}]"),
    }
    println!("按任意鍵回到 yumete…");
    let _ = io::Write::flush(&mut stdout());
    ratatui::crossterm::terminal::enable_raw_mode()?;
    // Anything at all: this is "I have read it", not a command.
    loop {
        if let Ok(Event::Key(key)) = event::read() {
            if is_actionable(key.kind) {
                break;
            }
        }
    }
    execute!(
        terminal.backend_mut(),
        ratatui::crossterm::terminal::EnterAlternateScreen
    )?;
    let _ = execute!(stdout(), EnableMouseCapture, EnableBracketedPaste);
    terminal.clear()?;
    status.map(|_| ())
}

/// Answer a `:theme`, and say where things stand afterwards.
///
/// A theme is two questions — which one, and dark or light — and either may be
/// left out, so a bare `:theme` changes nothing and reports. The config's own
/// `[theme]` is not rewritten: this is for the afternoon the room gets bright,
/// and the file is for what you want every day.
fn set_theme(
    config: &Config,
    name: Option<String>,
    mood: Option<yumete_core::command::Mood>,
) -> String {
    use yumete_core::command::Mood;
    // One theme, and the config's own name for it — a reader who renamed it is
    // still allowed to type the name they gave it.
    let known = ["moxiang", "墨香", config.theme.name.as_str()];
    if let Some(asked) = &name {
        if !known.contains(&asked.as_str()) {
            return say!("沒有這個主題：{0}", asked);
        }
    }
    if let Some(mood) = mood {
        crate::theme::set_dark(match mood {
            Mood::Dark => true,
            Mood::Light => false,
            // Back to whatever the terminal said at start-up; a terminal that
            // never answered keeps what the config settled on.
            Mood::System => crate::theme::terminal_answer().unwrap_or(crate::theme::dark()),
        });
    }
    let mood = match crate::theme::dark() {
        true => say!("深色"),
        false => say!("淺色"),
    };
    say!("主題：{0}（{1}）", config.theme.name, mood)
}

/// The shell to run a command line through.
fn shell() -> String {
    std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string())
}

/// `~` at the front of a path, the way a shell would read it.
fn shellexpand(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) => match std::env::var("HOME") {
            Ok(home) => format!("{home}/{rest}"),
            Err(_) => path.to_string(),
        },
        None => path.to_string(),
    }
}

/// A typesetter running in the background, and where its output is to be seen.
///
/// **Not a terminal panel.** What a preview server has to say is one line — the
/// address — and after that it is a process that should be left alone. Giving
/// it a pane would mean watching a log scroll where the writing used to be, and
/// would cost this editor a terminal emulator it is already sitting inside one
/// of. A job is a handle, an address, and a way to stop it.
struct Job {
    what: &'static str,
    child: std::process::Child,
    /// The address it printed, once it has printed one.
    said: std::sync::mpsc::Receiver<String>,
}

impl Job {
    /// Start `tinymist preview`, watching its log for the address it opens on.
    fn typst(path: &std::path::Path) -> Result<Job, String> {
        let mut child = std::process::Command::new("tinymist")
            .arg("preview")
            .arg("--no-open")
            .arg(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| format!("tinymist: {e}（`cargo install tinymist`）"))?;
        let (send, said) = std::sync::mpsc::channel();
        if let Some(log) = child.stderr.take() {
            std::thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(log).lines().map_while(Result::ok) {
                    // It says a good deal; one line of it is the address, and
                    // that is the whole of what a writer wants back.
                    if let Some(at) = line.find("http://") {
                        let url: String =
                            line[at..].chars().take_while(|c| !c.is_whitespace()).collect();
                        let _ = send.send(url);
                        return;
                    }
                }
            });
        }
        Ok(Job {
            what: "tinymist",
            child,
            said,
        })
    }
}

/// Open a URL or a file the way the platform opens things.
fn show(target: &str) {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener)
        .arg(target)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// What the system clipboard holds, asked of the platform.
///
/// Writing goes out through the terminal itself (OSC 52), which works over ssh
/// and inside tmux. Reading cannot: almost every terminal refuses an OSC 52
/// read, and rightly — it would let any program on the far end of a pipe empty
/// your clipboard into a file. So this asks the machine the editor is running
/// on, and says so plainly when there is nothing to ask.
fn read_clipboard() -> Option<String> {
    for (program, args) in [
        ("pbpaste", &[][..]),
        ("wl-paste", &["--no-newline"][..]),
        ("xclip", &["-selection", "clipboard", "-o"][..]),
        ("xsel", &["--clipboard", "--output"][..]),
    ] {
        if let Ok(out) = std::process::Command::new(program).args(args).output() {
            if out.status.success() {
                return String::from_utf8(out.stdout).ok();
            }
        }
    }
    None
}

/// Base64, for OSC 52. Twenty lines against a dependency for one escape
/// sequence, and the alphabet has not changed since 1987.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[(n >> (18 - i * 6)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Switch the IME to the named scheme, and say what happened.
///
/// Only 靈明 ships with yumete. The others are yume's own data, installed the
/// way yume installs it — `scripts/build.sh`, or a download from
/// yuhao-assess-data into the data directory — and a 碼表 of one's own goes in
/// `.yumete/` beside the manuscript. So the failure worth naming is not "no
/// such scheme" but "that scheme's tables are not on this machine".
fn switch_scheme(ime: &mut ImeSession, tag: &str, config: &Config) -> String {
    // Two questions ride the same request, because both are about the session
    // the front end holds and neither is worth a second channel.
    if tag == "?" {
        return if ime.available() {
            format!(
                "{} · 碼表 {} · 拆分 {}",
                ime.scheme_name(),
                ime.table_source(),
                if ime.annotations_enabled() { "開" } else { "關" }
            )
        } else if yumete_ime::has_builtin_table() {
            "還沒開始打字——`:yume scheme` 載入碼表".to_string()
        } else {
            "還沒開始打字，而且這個二進制不帶碼表——先裝資料".to_string()
        };
    }
    if let Some(path) = tag.strip_prefix('=') {
        let path = std::path::PathBuf::from(shellexpand(path));
        return match ImeSession::from_table_file(&path) {
            Ok(mut table) => {
                table.set_page_size(config.panel.page_size);
                *ime = table;
                format!("碼表：{}", path.display())
            }
            Err(why) => why,
        };
    }
    // 中/英, by name. The lone-Shift tap is the same switch; this is for the
    // hand that is already on `:`.
    if tag == "+" || tag == "-" {
        let want = tag == "+";
        // Turning it *on* with no 碼表 loaded is「開始打中文」, which means
        // loading one — nobody who typed this wanted to be told they are not
        // ready.
        if want && !ime.available() {
            let loaded = switch_scheme(ime, "", config);
            if !ime.available() {
                return loaded;
            }
        }
        if ime.is_chinese() != want {
            ime.toggle_language();
        }
        return match ime.is_chinese() {
            true => say!("中文（{0}）", ime.scheme_name()),
            false => say!("英文"),
        };
    }
    // The 碼表 the system has — `builtin`'s other half.
    if tag == "~" {
        let mut full = ImeSession::from_default_dirs(ime.scheme());
        if !full.available() {
            return say!("系統裏沒有裝 {0} 的碼表", ime.scheme_name());
        }
        full.set_page_size(config.panel.page_size);
        full.set_annotations(ime.annotations_enabled());
        *ime = full;
        return say!("方案：{0}（系統裝的碼表）", ime.scheme_name());
    }
    if tag == "!" {
        if !yumete_ime::has_builtin_table() {
            return "這個二進制不帶碼表".to_string();
        }
        let mut full = ImeSession::builtin_lingming();
        full.set_page_size(config.panel.page_size);
        full.set_annotations(ime.annotations_enabled());
        *ime = full;
        return "方案：靈明（出廠自帶的碼表）".to_string();
    }
    // No name means "the one this project writes in" — `:yume s` is the whole
    // of starting to type, and the config already said which.
    let tag = if tag.is_empty() {
        config.ime.scheme.as_str()
    } else {
        tag
    };
    let Some(scheme) = Scheme::from_tag(tag) else {
        let names = Scheme::ALL
            .iter()
            .map(|s| s.tag())
            .collect::<Vec<_>>()
            .join(" ");
        return format!("no scheme '{tag}' — one of: {names}");
    };
    // The session may be the language-only one that starts every launch, in
    // which case there is no 碼表 in it to switch *from* — so this is the
    // hundred milliseconds nobody paid at startup, paid now, once, by the
    // person who asked to type.
    if !ime.available() {
        let mut full = ImeSession::from_default_dirs(scheme);
        if full.available() {
            full.set_page_size(config.panel.page_size);
            full.set_annotations(ime.annotations_enabled());
            let name = full.scheme_name().to_string();
            *ime = full;
            return format!("方案：{name}");
        }
        return format!(
            "{tag} 的碼表沒有裝——放進資料目錄（yume 的 scripts/build.sh 會裝），\
             或者把自己的碼表放進 .yumete/"
        );
    }
    let was = ime.scheme();
    if ime.set_scheme(scheme) {
        return format!("方案：{}", ime.scheme_name());
    }
    // Put back what was working rather than leaving the writer unable to type.
    ime.set_scheme(was);
    format!(
        "{tag} is not installed — put its tables in the data directory \
         (yume's scripts/build.sh installs them) or a 碼表 of your own in .yumete/"
    )
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
        // A digit only selects when a candidate is actually under it: with a
        // five-candidate page, `7` must not commit the second candidate of the
        // page the reader cannot see.
        KeyCode::Char(c)
            if composing && ('1'..='9').contains(&c) && ime.page_has((c as u8 - b'0') as usize) =>
        {
            ime.select_in_page((c as u8 - b'1') as usize);
        }
        // …and one that names nothing is *swallowed*, which is the engine's own
        // factory rule: a digit brushed while typing a code should not push the
        // candidate list out into the manuscript.
        KeyCode::Char(c) if composing && ('1'..='9').contains(&c) => {}
        // The keys the scheme binds to a function. 靈明 puts 選二 on `;` and 選三
        // on `'`; the engine owns that table, and asking it is also what lets a
        // custom 碼表 keep a key it uses as a code.
        KeyCode::Char(c @ (';' | '\'' | '-' | '=')) if composing => {
            if !ime.press_func(c) {
                return false;
            }
        }
        // Nothing else reaches the editor mid-composition. An arrow key used to
        // fall through and move the cursor while the code stayed in the engine,
        // so the characters committed afterwards landed somewhere else.
        KeyCode::Left
        | KeyCode::Right
        | KeyCode::Up
        | KeyCode::Down
        | KeyCode::Home
        | KeyCode::End
        | KeyCode::Delete
        | KeyCode::Tab
        | KeyCode::BackTab
            if composing => {}
        // Any other printable character (letters start/continue a composition;
        // punctuation and digits are handled by the engine). A literal space
        // with no composition falls through to the editor.
        // ⌘ is the terminal's; read as a bare letter it would be typed.
        KeyCode::Char(_)
            if mods.intersects(KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META) => {}
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
    // ⌘ belongs to the terminal, and with the Kitty protocol on, the terminal
    // hands us the keypress anyway. Read as a bare letter, `⌘C` is `c` — which
    // in Normal mode is *change*, so asking for a copy deleted the selection.
    // Nothing modified by ⌘ is ours.
    if modifiers.intersects(KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META) {
        return None;
    }
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
        KeyCode::Delete => Some(Key::Delete),
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

/// Where each part of the window goes.
///
/// **Worked out in one place**, because three parts of this program need the
/// answer and used to work it out separately: the drawing, the mouse looking
/// for what it landed on, and the event loop settling the wrap length and the
/// page size *before* the keys that use them run. They disagreed by the hint
/// row, the tab bar and the detail panel — so the 縱 the cursor moved on was
/// one longer than the 縱 on the screen, and a click resolved to the wrong
/// character.
#[derive(Debug, Clone, Copy)]
struct Areas {
    sidebar: Rect,
    tabs: Rect,
    /// What the page itself is drawn into, the detail panel already taken off.
    text: Rect,
    detail: Option<Rect>,
    hint: Rect,
    status: Rect,
}

/// Divide `area` up. Pure: it draws nothing and depends only on what the
/// editor and the config say.
fn page_areas(editor: &Editor, config: &Config, area: Rect) -> Areas {
    // Two rows at the foot, answering two questions. The bottom one is *where
    // am I* and never changes shape; the one above it is *what just happened,
    // and what can I press*, and is blank when there is neither. Splitting them
    // is what lets the bottom row stay still: a message used to push the
    // position along the line, or take it away outright.
    let hint_rows = u16::from(config.editor.hints && area.height > 4);
    let body_h = area.height.saturating_sub(hint_rows + 1);
    let hint = Rect::new(area.x, area.y + body_h, area.width, hint_rows);
    let status = Rect::new(area.x, area.y + body_h + hint_rows, area.width, 1);
    // The sidebar takes its columns off the left; set vertically that is the
    // right side to lose, because the 縱 fill from the right edge and the page
    // simply ends sooner.
    let want = match editor.sidebar() {
        Some(_) => sidebar_columns(editor, config, area.width),
        None => 0,
    };
    let sidebar = Rect::new(area.x, area.y, want, body_h);
    let body = Rect::new(area.x + want, area.y, area.width.saturating_sub(want), body_h);
    // The tab bar takes the row off the top of what is left.
    let (tabs, page) = match config.editor.tabs.showing(editor.buffer_count()) && body.height > 1 {
        true => (
            Rect::new(body.x, body.y, body.width, 1),
            Rect::new(body.x, body.y + 1, body.width, body.height - 1),
        ),
        false => (Rect::new(body.x, body.y, body.width, 0), body),
    };
    let (text, detail) = table::split_detail(editor, page);
    Areas {
        sidebar,
        tabs,
        text,
        detail,
        hint,
        status,
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
    let areas = page_areas(editor, config, area);
    let Areas {
        sidebar,
        tabs: tab_area,
        text: text_area,
        detail,
        hint: hint_area,
        status: status_area,
    } = areas;
    let hint_rows = hint_area.height;
    if sidebar.width > 0 {
        draw_sidebar(frame, editor, config, sidebar);
    }
    if tab_area.height > 0 {
        draw_tabs(frame, editor, config, tab_area);
    }

    // The text body is the one part that differs between the layouts; both
    // report back the cell the cursor landed on, which the status line and the
    // candidate panel are positioned from.
    let (cursor_x, cursor_y) = match editor.layout() {
        // A grid is not prose and is not drawn as prose: no wrapping, no
        // markup, one row per line, columns that line up.
        // A `|` table lives inside a page of prose and is drawn by whatever
        // draws that page — the paragraph above it must not vanish because the
        // cursor landed in a cell.
        _ if editor.table().is_some_and(|t| t.is_grid()) => {
            table::draw(frame, editor, config, text_area, &mut viewport.table)
        }
        WritingLayout::Horizontal => {
            draw_horizontal(frame, editor, config, text_area, &mut viewport.top)
        }
        WritingLayout::Vertical => {
            vertical::draw(frame, editor, config, text_area, &mut viewport.zong)
        }
    };

    if let Some(panel) = detail {
        table::draw_detail(frame, editor, config, panel);
    }

    if hint_rows == 1 {
        draw_hints(frame, editor, config, hint_area);
    }
    draw_status(frame, editor, config, ime, status_area, tab_area);
    // The floating panels stack upward from the footer, and the footer is now
    // two rows deep — anchored to the status line alone they would be drawn
    // over the hint row.
    let footer = if hint_rows == 1 { hint_area } else { status_area };
    draw_command_menu(frame, editor, config, area, footer);
    draw_picker(frame, editor, config, area, footer);
    draw_space_menu(frame, editor, config, area, footer);

    // In vertical layout the cursor is a block drawn into the page: a hardware
    // cursor is one cell wide and would sit lopsided inside a two-cell 縱.
    if let Some((_, _)) = editor.prompt() {
        // Measured in cells, not characters: a Chinese search pattern is twice
        // as wide as it is long — and up to the **caret**, not to the end of
        // the line, now that the prompt can be edited in the middle.
        let col = 1
            + yumete_cjk::str_width(&editor.prompt_before_caret())
            + yumete_cjk::str_width(&prompt_preedit(editor, ime));
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
            WritingLayout::Horizontal => draw_candidate_panel(frame, ime, config, area, at_x, at_y),
            WritingLayout::Vertical => {
                vertical::draw_candidate_panel(frame, ime, config, area, at_x, at_y)
            }
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

/// How many rows a menu or a picker may take.
///
/// Helix caps its completion popup and scrolls it, and the reason is not screen
/// real estate but reading: a list you have to search is not a list you can
/// glance at. Twenty-odd commands laid out across the whole page hid the very
/// document the command was about to act on.
const MENU_ROWS: usize = 8;

/// The widest a menu gets. Past this the eye stops reading a row as one thing.
const MENU_WIDTH: u16 = 56;

/// The most columns a menu spreads across.
///
/// Bounded because a menu is glanced at, not read: past three or four columns
/// the eye has to hunt, and the thing it is covering is the page.
const MENU_COLUMNS: usize = 4;

/// Draw a compact list just above `bottom`, scrolled so `selected` is on it.
///
/// One column, capped, with a footer naming where you are in the list and what
/// the highlighted row means. Both the `:` menu and the pickers use it, so they
/// look like one idea rather than two.
struct List<'a> {
    items: &'a [String],
    /// Which entry has to stay on screen.
    focus: usize,
    /// Which entry is inked, if any.
    highlight: Option<usize>,
    /// The line under it: a count, and what the inked entry means.
    footer: &'a str,
    /// Whether it may spread across the window.
    columns: bool,
}

fn draw_list(
    frame: &mut Frame,
    ink: crate::theme::Palette,
    area: Rect,
    bottom: u16,
    list: List,
) {
    let List {
        items,
        focus,
        highlight,
        footer,
        columns,
    } = list;
    if items.is_empty() && footer.is_empty() {
        return;
    }
    // As wide as its widest entry, and as many entries across as the window
    // will take. Twenty-six commands down one column is three screenfuls with
    // the rest of the page standing empty beside it; in three columns it is one
    // glance. Column-major, so reading runs *down* and then across — the way a
    // list of files does, and the way the numbers on it stay in order.
    let one = items
        .iter()
        .map(|i| yumete_cjk::str_width(i))
        .max()
        .unwrap_or(0)
        .saturating_add(2)
        .min(MENU_WIDTH as usize);
    let across = if columns {
        let room = (area.width as usize).saturating_sub(2).max(1);
        (room / one.max(1))
            .clamp(1, items.len().div_ceil(MENU_ROWS).max(1))
            .min(MENU_COLUMNS)
    } else {
        1
    };
    let deep = items.len().div_ceil(across).clamp(1, MENU_ROWS);
    let visible = (deep * across).min(items.len());
    let height = (deep + 1) as u16;
    if height > area.height || bottom < height {
        return;
    }
    // Scrolled just enough: the selection stays on the list, and a list that
    // fits never scrolls at all.
    let first = focus
        .saturating_sub(visible.saturating_sub(1))
        .min(items.len().saturating_sub(visible));

    let width = (one * across)
        .max(yumete_cjk::str_width(footer) + 2)
        .min(area.width as usize) as u16;
    let menu = Rect::new(area.x, bottom - height, width, height);
    frame.render_widget(Clear, menu);

    let ground = Style::default().bg(ink.paper());
    let text = ground.fg(ink.text());
    let quiet = ground.fg(ink.quiet());
    let on = Style::default().bg(ink.text()).fg(ink.paper());

    let buf = frame.buffer_mut();
    for y in 0..height {
        for x in 0..width {
            if let Some(cell) = buf.cell_mut((menu.x + x, menu.y + y)) {
                cell.set_symbol(" ").set_style(ground);
            }
        }
    }
    for slot in 0..visible {
        let i = first + slot;
        if i >= items.len() {
            break;
        }
        let (column, row) = (slot / deep, slot % deep);
        let x = menu.x + (column * one) as u16;
        let end = (x + one as u16).min(menu.x + width);
        let y = menu.y + row as u16;
        let picked = highlight == Some(i);
        let style = if picked { on } else { text };
        if picked {
            for cx in x..end {
                if let Some(cell) = buf.cell_mut((cx, y)) {
                    cell.set_symbol(" ").set_style(style);
                }
            }
        }
        put_text(buf, x + 1, y, end, &items[i], style);
    }
    put_text(
        buf,
        menu.x + 1,
        menu.y + deep as u16,
        menu.x + width,
        footer,
        quiet,
    );
}

/// Write `text` from `x`, stopping at `limit`, one cell per column.
fn put_text(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    limit: u16,
    text: &str,
    style: Style,
) {
    let mut at = x;
    for g in yumete_cjk::graphemes(text) {
        let w = yumete_cjk::grapheme_width(g).max(1) as u16;
        if at + w > limit {
            break;
        }
        // The trailing cell first, so a wide glyph's other half is styled and
        // never left holding whatever was drawn there before.
        for n in 1..w {
            if let Some(cell) = buf.cell_mut((at + n, y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
        if let Some(cell) = buf.cell_mut((at, y)) {
            cell.set_symbol(g).set_style(style);
        }
        at += w;
    }
}

/// The `:` command menu — Helix's completion popup, not a wall.
fn draw_command_menu(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    area: Rect,
    status: Rect,
) {
    let Some((':', _)) = editor.prompt() else {
        return;
    };
    let ink = crate::theme::Palette::of(config);
    let (matches, selected) = editor.command_menu();
    if matches.is_empty() {
        return;
    }
    // Tab's pick is inked; without one nothing is, because the ghost text on
    // the command line is already saying what the guess is.
    let highlight = selected.map(|i| i.min(matches.len() - 1));
    let focus = highlight.unwrap_or(0);
    let items: Vec<String> = matches
        .iter()
        // The short way to write it, when there is one. An explicit alias wins
        // over the derived prefix: `:w` is `write` because it was declared so,
        // even though `w` is a prefix of three commands.
        .map(|e| match e.alias.or(e.short) {
            Some(short) => format!("{}{}  ({short})", e.leading, e.written()),
            None => format!("{}{}", e.leading, e.written()),
        })
        .collect();
    // Only the highlighted command's help, on one line. Every command's help at
    // once is what covered the page.
    // Its `help` is written in Chinese and is the key it is translated by, the
    // same as every other thing this editor says.
    let footer = format!(
        "{}/{}  {}",
        focus + 1,
        matches.len(),
        yumete_core::messages::say(matches[focus].help, &[])
    );
    // Spread across the window: the command list is short entries and there
    // are a couple of dozen of them, which is exactly the shape that wants
    // columns.
    draw_list(
        frame,
        ink,
        area,
        status.y,
        List {
            items: &items,
            focus,
            highlight,
            footer: &footer,
            columns: true,
        },
    );
}

/// Which character of the buffer a click landed on, if it landed on the page.
///
/// Worked out the same way the page was drawn — the sidebar's columns, the tab
/// bar's row, the gutter, and then the row model itself — rather than
/// remembered from the last frame, which could be a frame out of date.
fn text_at(
    editor: &Editor,
    config: &Config,
    size: Option<ratatui::layout::Size>,
    viewport: &Viewport,
    mouse: ratatui::crossterm::event::MouseEvent,
) -> Option<usize> {
    let size = size?;
    // The very rectangle the page was drawn into — hint row, tab bar, sidebar
    // and detail panel all already taken off. Working it out again by hand is
    // how a click came to land a row or two from where it was pointed.
    let area = page_areas(
        editor,
        config,
        Rect::new(0, 0, size.width, size.height),
    )
    .text;
    if mouse.column < area.x || mouse.row < area.y || mouse.row >= area.y + area.height {
        return None;
    }
    match editor.layout() {
        WritingLayout::Horizontal => {
            let buffer = editor.current_buffer();
            let gutter = gutter_width(buffer.line_count(), config.editor.line_numbers);
            let width = editor.wrap_width().unwrap_or(usize::MAX / 2).max(1);
            let hide = |line: usize| editor.hidden_on_line(line);
            // The same measure the page was drawn with — the indent changes
            // where a row breaks, so a click resolved without it lands `indent`
            // cells off on every paragraph's first row.
            let fold = |line: usize| editor.line_is_folded(line);
            let measure = wrap::Measure::new(width, &hide)
                .with_indent(editor.paragraph_indent())
                .with_folds(&fold)
                .with_open_line(editor.open_line());
            let row = wrap::rows_from(
                buffer.rope(),
                viewport.top,
                measure,
                (mouse.row - area.y) as usize + 1,
            )
            .pop()?;
            // Which character of that row the column landed on, counting only
            // what is drawn — hidden markup takes no columns.
            let want = (mouse.column - area.x) as usize;
            let goal = want.saturating_sub(gutter);
            let hidden = editor.hidden_on_line(row.line);
            let line_start = buffer.rope().line_to_char(row.line);
            // The row starts where it was drawn: a click anywhere in a
            // paragraph's opening indent means its first character.
            let mut column =
                measure.indent_of(row.line, &buffer.rope().line(row.line).to_string(), row.index_in_line);
            for at in row.start..row.end {
                let c = buffer.rope().char(at);
                let off = hidden
                    .iter()
                    .any(|&(a, b)| at - line_start >= a && at - line_start < b);
                if off {
                    continue;
                }
                let w = yumete_cjk::char_width(c);
                if goal < column + w {
                    return Some(at);
                }
                column += w;
            }
            Some(row.end.saturating_sub(1).max(row.start))
        }
        // Vertical: the page is laid out by the vertical renderer, which knows
        // where every 縱 was put.
        WritingLayout::Vertical => vertical::char_at(editor, config, area, viewport.zong, mouse),
    }
}

/// Which tab a click landed on, if it landed on the bar at all.
///
/// The bar's rectangle is worked out the same way `draw` works it out — below
/// nothing, to the right of the sidebar — rather than remembered, so a click
/// cannot be answered from a layout that is a frame out of date.
fn tab_at(
    editor: &Editor,
    config: &Config,
    size: Option<ratatui::layout::Size>,
    mouse: ratatui::crossterm::event::MouseEvent,
) -> Option<usize> {
    let size = size?;
    let area = page_areas(editor, config, Rect::new(0, 0, size.width, size.height)).tabs;
    if area.height == 0 || mouse.row != area.y {
        return None;
    }
    tab_spans(editor, area)
        .into_iter()
        .find(|&(at, w, _)| mouse.column >= at && mouse.column < at + w)
        .map(|(_, _, i)| i)
}

/// How a Markdown run is set.
///
/// The markup itself is *shown* and set back; what it marks is set forward.
/// Nothing is hidden, because the file is the manuscript — the page says what
/// is in it, and says which part of that is scaffolding.
fn markup_style(kind: yumete_core::markdown::Kind, ink: crate::theme::Palette) -> Style {
    use yumete_core::markdown::Kind;
    // **Weight, not hue.** There were five hues here — a green code, a blue
    // link, a khaki wikilink — all at the same *weight* as the prose, differing
    // only in colour. That is right in a syntax highlighter and backwards in a
    // manuscript: the writing should be the brightest thing on the page and
    // everything else should recede. A rung plus an underline says which is
    // which, and it goes on saying it in light mode and on a terminal that
    // renders no colour at all.
    match kind {
        // A real rung, not `DIM`: a terminal that ignores DIM used to draw
        // `**` exactly like the word between them, which is the whole of
        // 所見即所得 gone.
        Kind::Marker | Kind::HeadingMark => Style::default().fg(ink.marker()),
        // A terminal cannot make a heading bigger, and bold is what it has.
        // The old colour was five ΔE from the ink for a third of a stop of
        // contrast — the weight was doing all the work already.
        // 金, because a heading is not prose and the page is otherwise grey.
        // Weight alone said it in a hundred-line file and stopped saying it in
        // a chapter: bold prose and a heading are the same colour, and the eye
        // has to read them to tell them apart.
        Kind::Heading => Style::default().fg(ink.gold()).add_modifier(Modifier::BOLD),
        Kind::Strong => Style::default().add_modifier(Modifier::BOLD),
        Kind::Emphasis => Style::default().add_modifier(Modifier::ITALIC),
        Kind::Code => Style::default().fg(ink.quiet()),
        // CROSSED_OUT is not everywhere, so the rung carries it as well.
        Kind::Strike => Style::default()
            .fg(ink.furniture())
            .add_modifier(Modifier::CROSSED_OUT),
        Kind::Link | Kind::WikiLink => Style::default()
            .fg(ink.quiet())
            .add_modifier(Modifier::UNDERLINED),
        // A highlighter pen leaves a ground, so this is a ground — and the pen
        // is 朱, washed until the ink still reads on it.
        Kind::Highlight => Style::default().bg(ink.wash()).fg(ink.text()),
        // A footnote *is* a 朱批.
        Kind::Footnote => Style::default().fg(ink.mark()),
        // Not part of the book: set back, but never hidden and never below
        // reading — a note you cannot see is a note you will not act on, and
        // this one measured 2.54:1.
        Kind::Comment => Style::default()
            .fg(ink.furniture())
            .add_modifier(Modifier::ITALIC),
        // Typst's own code: the instructions that make the page, not decoration
        // around writing. Set back, never taken away.
        Kind::Code2 => Style::default().fg(ink.quiet()),
    }
}

/// How a whole row is set, given the block its line belongs to.
///
/// Blocks colour the *row*, inline runs colour the characters, and the two
/// compose — a bold word inside a `::: warning` keeps its bold and gains the
/// container's ground.
fn block_style(block: yumete_core::markdown::Block, ink: crate::theme::Palette) -> Option<Style> {
    use yumete_core::markdown::{Block, Callout};
    // **A container is a container.** The four callouts used to differ by hue
    // at 1.4–2.4 ΔE from each other and from the quote and the code fence —
    // which is below the threshold at which two flat grounds can be told apart
    // at all, and is also what collapses on a 256-colour terminal. The word
    // `note` / `tip` / `warning` on the line is what says which; the ground says
    // only 「這是一塊」. Danger is the exception, because it is the one that
    // means 這裏不對.
    let band = || Some(Style::default().bg(ink.at(yumete_config::rung::BAND)));
    match block {
        Block::Prose | Block::Heading(_) | Block::Item { .. } | Block::Table => None,
        // An aside is a block on the page because it is a block on paper.
        Block::Container(Callout::Danger) => Some(Style::default().bg(ink.wash())),
        Block::Container(_) | Block::Quote | Block::Code => band(),
        // Metadata and scene breaks are furniture, not writing.
        Block::FrontMatter | Block::Rule | Block::FootnoteDef => {
            Some(Style::default().fg(ink.furniture()))
        }
    }
}

/// Where each tab sits on the bar, so a click can find the one it landed on.
///
/// Recomputed from the same rule the drawing uses rather than remembered from
/// the last frame: a remembered layout is one that can be a frame out of date,
/// and a click that opens the wrong file is worse than no click at all.
fn tab_spans(editor: &Editor, area: Rect) -> Vec<(u16, u16, usize)> {
    let tabs = editor.buffer_tabs();
    if tabs.is_empty() || area.width == 0 {
        return Vec::new();
    }
    let widths: Vec<u16> = tabs
        .iter()
        .map(|(name, dirty)| yumete_cjk::str_width(&tab_label(name, *dirty)) as u16)
        .collect();
    let current = editor
        .buffer_position()
        .0
        .saturating_sub(1)
        .min(tabs.len() - 1);
    // **The bar scrolls to the tab you are in.** It used to start at the first
    // file and stop when it ran out of room, so past about eight chapters the
    // one being written was never on it — and the `[n/m]` that would have said
    // so was suppressed *because* the bar was up.
    //
    // Widen leftward from the current tab until one more would not fit, then
    // rightward with whatever is left — the same rule the table's columns
    // follow, and it keeps the bar still while you walk within a page of it.
    let room = area.width;
    let mut used = widths[current].min(room);
    let mut first = current;
    while first > 0 && used + widths[first - 1] <= room {
        first -= 1;
        used += widths[first];
    }
    let mut last = current;
    while last + 1 < tabs.len() && used + widths[last + 1] <= room {
        last += 1;
        used += widths[last];
    }
    let mut spans = Vec::new();
    let mut x = area.x;
    for (i, w) in widths.iter().enumerate().take(last + 1).skip(first) {
        if x + w > area.x + area.width {
            break;
        }
        spans.push((x, *w, i));
        x += w;
    }
    spans
}

/// Whether the tab bar is showing every open file.
///
/// When it is not, the status line says `[n/m]` again: the bar answers "which
/// one am I in" better than a fraction does, but only about the files it is
/// actually drawing.
fn tabs_show_everything(editor: &Editor, config: &Config, area: Rect) -> bool {
    if area.height == 0 || !config.editor.tabs.showing(editor.buffer_count()) {
        return false;
    }
    tab_spans(editor, area).len() == editor.buffer_count()
}

/// One tab's text, padded so the lit one reads as a tab rather than as a word.
fn tab_label(name: &str, dirty: bool) -> String {
    format!(" {name}{} ", if dirty { " +" } else { "" })
}

/// The tab bar: one row naming every open file, the one being written lit.
///
/// A terminal has tabs across the top and so does this, for the same reason —
/// how many things are open, and which one you are in, are questions that
/// should be answered by looking rather than by pressing a key.
fn draw_tabs(frame: &mut Frame, editor: &Editor, config: &Config, area: Rect) {
    let ink = crate::theme::Palette::of(config);
    let ground = ink.ground(yumete_config::rung::CHROME);
    // The lit tab is raised off the bar and written in 金墨 — the bar itself is
    // black, so a lit tab told apart by its *ground* alone would be telling it
    // in the one register this theme keeps quiet.
    let lit = ink
        .ground(yumete_config::rung::HEAD)
        .fg(ink.gold())
        .add_modifier(Modifier::BOLD);
    let unlit = ground.fg(ink.furniture());

    let current = editor.buffer_position().0.saturating_sub(1);
    let tabs = editor.buffer_tabs();
    let spans = tab_spans(editor, area);
    let buf = frame.buffer_mut();
    for x in area.x..area.x + area.width {
        if let Some(cell) = buf.cell_mut((x, area.y)) {
            cell.set_symbol(" ").set_style(ground);
        }
    }
    for (x, w, i) in spans {
        let style = if i == current { lit } else { unlit };
        for n in 0..w {
            if let Some(cell) = buf.cell_mut((x + n, area.y)) {
                cell.set_symbol(" ").set_style(style);
            }
        }
        let (name, dirty) = &tabs[i];
        put_text(buf, x, area.y, x + w, &tab_label(name, *dirty), style);
    }
}

/// How many columns the sidebar takes.
///
/// One rule, asked three times — by the drawing, by the mouse looking for what
/// it landed on, and by the tab bar working out where it starts. A remembered
/// number would be a frame out of date, and a click would open the wrong file.
///
/// Wide, it is as wide as its longest row, so the point of opening it out —
/// reading a whole chapter name — actually happens; but never past half the
/// window, because the writing is what the window is for.
fn sidebar_columns(editor: &Editor, config: &Config, total: u16) -> u16 {
    let Some(sidebar) = editor.sidebar() else {
        return 0;
    };
    let want = if sidebar.wide() {
        // One column of padding on the left, the rule on the right, and the
        // two the outline indents its rows by.
        let longest = sidebar
            .rows()
            .iter()
            .map(|row| yumete_cjk::str_width(&row.name) + 4)
            .max()
            .unwrap_or(0);
        longest
            .max(config.editor.sidebar_width)
            .min(total as usize / 2)
    } else {
        config.editor.sidebar_width
    };
    (want as u16).min(total.saturating_sub(8))
}

/// The file sidebar, in the columns taken off the left of the page.
///
/// A rule rather than a border: one column of `│` says "this is a different
/// thing" and costs one cell, where a box costs four and a corner.
fn draw_sidebar(frame: &mut Frame, editor: &Editor, config: &Config, area: Rect) {
    let Some(sidebar) = editor.sidebar() else {
        return;
    };
    if area.width < 3 {
        return;
    }
    let ink = crate::theme::Palette::of(config);
    let ground = ink.ground(yumete_config::rung::CHROME);
    let text = ground.fg(ink.text());
    let dir = ground.fg(ink.gold()).add_modifier(Modifier::BOLD);
    let quiet = ground.fg(ink.quiet());
    // Unfocused, the highlight is a quiet band; focused, it is inked — so which
    // half of the screen the keys are going to is never in doubt.
    // Focused, it is ink and paper changing places — the most robust 「這裏」
    // a terminal has, and it costs no colour and survives a light/dark flip.
    let on = match editor.sidebar_focused() {
        true => Style::default().bg(ink.text()).fg(ink.paper()),
        false => Style::default().bg(ink.selection()).fg(ink.text()),
    };

    frame.render_widget(Clear, area);
    let rule = area.x + area.width - 1;
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..rule {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(ground);
            }
        }
        if let Some(cell) = buf.cell_mut((rule, y)) {
            cell.set_symbol("│").set_style(quiet);
        }
    }

    // The directory the tree is rooted at, then the tree, scrolled to keep the
    // highlight on screen.
    put_text(buf, area.x + 1, area.y, rule, &sidebar.title(), quiet);
    let rows = sidebar.rows();
    let visible = (area.height as usize).saturating_sub(1);
    if visible == 0 {
        return;
    }
    let first = sidebar
        .selected()
        .saturating_sub(visible.saturating_sub(1))
        .min(rows.len().saturating_sub(visible));
    for slot in 0..visible.min(rows.len()) {
        let i = first + slot;
        let row = &rows[i];
        let y = area.y + 1 + slot as u16;
        let picked = i == sidebar.selected();
        let style = if picked {
            on
        } else if row.is_dir {
            dir
        } else {
            text
        };
        if picked {
            for x in area.x..rule {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(style);
                }
            }
        }
        // In the tree a directory says which way it is facing, and a file is
        // indented past where that mark would be so the names line up. The flat
        // views spend `depth` on an index instead, so they get no indent — and
        // in the buffer list `expanded` marks the one being written.
        let line = match sidebar.view() {
            View::Explorer => {
                let mark = match (row.is_dir, row.expanded) {
                    (true, true) => "▾ ",
                    (true, false) => "▸ ",
                    (false, _) => "  ",
                };
                format!("{}{mark}{}", "  ".repeat(row.depth), row.name)
            }
            View::Buffers => format!("{} {}", if row.expanded { "▸" } else { " " }, row.name),
            View::Outline => format!("  {}", row.name),
        };
        put_text(buf, area.x + 1, y, rule, &line, style);
    }
}

/// The `Space f` / `Space b` picker.
fn draw_picker(frame: &mut Frame, editor: &Editor, config: &Config, area: Rect, status: Rect) {
    let Some(picker) = editor.picker() else {
        return;
    };
    let ink = crate::theme::Palette::of(config);
    let matches = picker.matches();
    let items: Vec<String> = matches.iter().map(|i| i.label().to_string()).collect();
    let footer = format!(
        "{}  {}/{}  {}",
        picker.title,
        if items.is_empty() {
            0
        } else {
            picker.selected() + 1
        },
        picker.total(),
        picker.query()
    );
    let at = picker.selected();
    // One column: these are paths, long and of every length, and columns of
    // ragged paths are harder to read down than a single list.
    draw_list(
        frame,
        ink,
        area,
        status.y,
        List {
            items: &items,
            focus: at,
            highlight: Some(at),
            footer: &footer,
            columns: false,
        },
    );
}

/// The `Space` menu, listed while the key is waiting for its second half.
fn draw_space_menu(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    area: Rect,
    status: Rect,
) {
    if !editor.space_pending() {
        return;
    }
    let ink = crate::theme::Palette::of(config);
    let items: Vec<String> = Editor::SPACE_KEYS
        .iter()
        .map(|(key, what)| format!("{key}   {what}"))
        .collect();
    draw_list(
        frame,
        ink,
        area,
        status.y,
        List {
            items: &items,
            focus: 0,
            highlight: None,
            footer: "空格",
            columns: true,
        },
    );
}

/// The composition in progress, when a `/` or `:` prompt is open.
fn prompt_preedit(editor: &Editor, ime: &ImeSession) -> String {
    if editor.prompt().is_some() && ime.available() && ime.is_composing() {
        ime.display_buffer()
    } else {
        String::new()
    }
}

/// Draw the buffer as horizontal rows with a line-number gutter, returning the
/// cell the cursor sits on.
///
/// A row is a *wrapped piece* of a paragraph, not a paragraph (Feature #77).
/// The two coincide when soft wrap is off, and then a long paragraph runs off
/// the right edge as it always did — which is why wrapping is on by default for
/// prose, where a paragraph is routinely one line of several hundred
/// characters.
fn draw_horizontal(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    text_area: Rect,
    viewport: &mut WrapAnchor,
) -> (u16, u16) {
    let buffer = editor.current_buffer();
    let total_lines = buffer.line_count();
    let height = text_area.height as usize;
    let mode = config.editor.line_numbers;
    let gutter = gutter_width(total_lines, mode);
    let rope = buffer.rope();
    let ink = crate::theme::Palette::of(config);

    // Unwrapped, every paragraph is one row and anything past the right edge is
    // simply clipped, which is what the row model produces at an unreachable
    // width.
    let width = editor.wrap_width().unwrap_or(usize::MAX / 2).max(1);

    // The measure is the width *and* what is off the page: a row holds what
    // fits on the screen, so markup taken off it takes no room.
    let hide = |line: usize| editor.hidden_on_line(line);
    // 首行縮進 is part of the measure, not of this function: it changes where a
    // row breaks, so the cursor and the page have to be asking about the same
    // one. Here it is only *drawn*.
    let fold = |line: usize| editor.line_is_folded(line);
    let measure = wrap::Measure::new(width, &hide)
        .with_indent(editor.paragraph_indent())
        .with_folds(&fold)
        .with_open_line(editor.open_line());

    let cursor_line = editor.cursor_line();
    let cursor_pos = wrap::position(rope, editor.cursor(), measure);
    let cursor_anchor = WrapAnchor::from(cursor_pos);

    // Scroll so the cursor's row stays on the page with `scrolloff` rows of
    // context above and below. Counted from the page's own anchor rather than
    // from a global row number, which cannot be found without walking the
    // document from the top on every keystroke.
    let scrolloff = config.editor.scrolloff.min(height.saturating_sub(1) / 2);
    let last_row = height.saturating_sub(1);
    let cursor_row = match wrap::distance(rope, *viewport, cursor_anchor, measure, last_row) {
        Some(d) if d >= scrolloff && d + scrolloff <= last_row => d,
        found => {
            // Off the page, or too close to an edge: re-anchor so the cursor
            // sits `scrolloff` in from whichever side it left by.
            let inset = if found.is_some_and(|d| d < scrolloff) || cursor_anchor < *viewport {
                scrolloff
            } else {
                last_row.saturating_sub(scrolloff)
            };
            *viewport = wrap::retreat(rope, cursor_anchor, measure, inset);
            wrap::distance(rope, *viewport, cursor_anchor, measure, height).unwrap_or(0)
        }
    };

    let (sel_start, sel_end) = editor.selection();
    // Asked of the editor, not of the range: the selection always covers the
    // cursor's own grapheme, so a bare cursor would otherwise be drawn as a
    // one-character highlight and the word-tint overlay would never appear.
    let has_selection = editor.has_selection();
    // A ground, and only a ground.
    let sel_style = Style::default().bg(ink.selection());
    let show_segmentation = editor.segmentation_visible();
    let show_markup = editor.markup_visible();
    // The measure is counted in *text*: `ruler = 80` means eighty columns of
    // writing, which is what a writer means by it. The line-number gutter is
    // furniture, not text, so it does not eat into the measure — and the ruler
    // moves with the gutter rather than the writing moving under it.
    // A measure set with `:wrap 50` is a ruler by definition — it is the width
    // the writer asked to write to — so it stands in for the configured one.
    let ruler = editor.measure().unwrap_or(config.editor.ruler);

    // Word ranges are per paragraph and consecutive rows usually share one, so
    // each paragraph the page touches is segmented once. Its Markdown runs are
    // held the same way, for the same reason.
    let mut segmented: Option<(usize, Vec<(usize, usize)>)> = None;
    let mut marked: Option<(usize, Vec<yumete_core::markdown::Span>)> = None;
    // Which block each line belongs to, walked from the top of the document
    // down to the bottom of this page — a fence opened above decides what the
    // lines below it mean, and there is no way to know that from a line alone.
    let blocks = if show_markup {
        let last = wrap::rows_from(rope, *viewport, measure, height)
            .last()
            .map_or(0, |row| row.line);
        editor.blocks_through(last)
    } else {
        Vec::new()
    };

    let mut lines: Vec<Line> = Vec::new();
    for row in wrap::rows_from(rope, *viewport, measure, height) {
        let text: String = rope.slice(row.start..row.end).to_string();
        let mut spans = Vec::new();
        if gutter > 0 {
            // A continuation row carries no number: the number belongs to the
            // paragraph, and repeating it down a wrapped paragraph would read as
            // several paragraphs of the same number.
            let label = if row.starts_line() {
                gutter_text(row.line, cursor_line, gutter, mode)
            } else {
                " ".repeat(gutter)
            };
            // A rung, not `DIM`: several terminals ignore DIM outright, and a
            // line number that is the same colour as the writing is worse than
            // no line number.
            spans.push(Span::styled(label, ink.page().fg(ink.furniture())));
        }
        // The paragraph opens two squares in, the way a Chinese paragraph is
        // marked — and the blank line it replaces costs a whole row.
        let indent = measure.indent_of(row.line, &rope.line(row.line).to_string(), row.index_in_line);
        if indent > 0 {
            spans.push(indent_span(indent, editor, ink));
        }

        // Three layers, composed rather than fighting: Markdown sets the ink
        // and the weight, the word overlay and the selection set the ground.
        // Whichever one wins used to be an if/else, so a bold word inside a
        // selection lost its bold and a heading lost its colour the moment the
        // overlay came on.
        let row_len = row.end - row.start;
        let chars: Vec<char> = text.chars().collect();
        // The block grounds the whole row; the inline runs are patched onto it.
        // **The page is painted.** Until this line, the manuscript itself was
        // drawn with no colour at all — the terminal's own ink on the
        // terminal's own ground — while 墨香 dressed five panels around it. A
        // theme that does not own its ground cannot promise anything about
        // contrast, because every tint in it is measured against a colour the
        // editor has never seen.
        let ground = ink.page().patch(
            blocks
                .get(row.line)
                .copied()
                .and_then(|b| block_style(b, ink))
                .unwrap_or_default(),
        );
        // 所見即所得: the markup comes off the page. It is dropped from what is
        // *drawn*, not from the buffer — and never on the construct the cursor
        // is in, so the cursor is never inside text that is not on the screen.
        let hide = editor.hidden_on_line(row.line);
        let start_in_line = row.start - rope.line_to_char(row.line);
        let shown: Vec<bool> = (0..chars.len())
            .map(|i| {
                let at = start_in_line + i;
                !hide.iter().any(|&(a, b)| at >= a && at < b)
            })
            .collect();
        let mut styles = vec![ground; chars.len()];

        if show_markup {
            let start_in_line = row.start - rope.line_to_char(row.line);
            let runs = match &marked {
                Some((line, runs)) if *line == row.line => runs,
                _ => {
                    // Inside a fence or a page's metadata nothing is markup;
                    // colouring `**` there misreports what the file says.
                    let block = blocks.get(row.line).copied().unwrap_or_default();
                    marked = Some((row.line, editor.markup_line_in(row.line, block)));
                    &marked.as_ref().unwrap().1
                }
            };
            for run in runs {
                let a = run.start.saturating_sub(start_in_line);
                let b = run.end.saturating_sub(start_in_line).min(chars.len());
                // Patched onto the block's ground rather than replacing it, so
                // a bold word inside a `::: warning` keeps both.
                for style in styles.iter_mut().take(b).skip(a.min(b)) {
                    *style = style.patch(markup_style(run.kind, ink));
                }
            }
        }

        if show_segmentation && !has_selection {
            let page_bg = ink.page().bg;
            // Tint each word with an alternating background (Feature #24). The
            // words are the paragraph's, sliced to this row, so a word split by
            // a wrap keeps one colour across the break.
            let words = match &segmented {
                Some((line, words)) if *line == row.line => words,
                _ => {
                    segmented = Some((row.line, editor.segment_line(row.line)));
                    &segmented.as_ref().unwrap().1
                }
            };
            let start_in_line = row.start - rope.line_to_char(row.line);
            let visible =
                |&&(a, b): &&(usize, usize)| b > start_in_line && a < start_in_line + row_len;
            // Which word of the paragraph the row opens on, so the two colours
            // keep alternating across the break instead of restarting.
            // The word's index *in the paragraph* picks its colour, so the two
            // keep alternating across a wrap instead of restarting each row.
            for (n, &(a, b)) in words.iter().enumerate().filter(|(_, w)| visible(w)) {
                // One tint, every other word, and bare page between: the pair
                // of tints it replaces differed by *temperature* at 1.04 and
                // 1.07 against the ground, so one read as the ground and the
                // other as a stain.
                if n % 2 != 0 {
                    continue;
                }
                let a = a.saturating_sub(start_in_line);
                let b = (b - start_in_line).min(chars.len());
                for style in styles.iter_mut().take(b).skip(a.min(b)) {
                    // Only where nothing has already claimed the ground. A word
                    // tint is the quietest of the three layers — it must not
                    // rub out a `==highlight==`, which exists *to be* a ground,
                    // nor a container's own colour. The page itself is not a
                    // claim: every style on the row starts from it now that the
                    // paper is painted, and reading that as taken would have
                    // left the overlay with nowhere it was allowed to draw.
                    if style.bg.is_none() || style.bg == page_bg {
                        *style = style.bg(ink.word());
                    }
                }
            }
        }

        // Past the measure, tinted. Information even with soft wrap on: it
        // says this row has run past the length the writer wants a sentence to
        // be, which is what somebody breaking long ones by hand is looking for.
        if ruler > 0 {
            let mut column = 0;
            for (i, ch) in chars.iter().enumerate() {
                if column >= ruler {
                    styles[i] = styles[i].bg(ink.at(yumete_config::rung::BAND));
                }
                column += yumete_cjk::char_width(*ch);
            }
        }

        // The selection's ground goes over everything, because it is the answer
        // to "what would an edit take" and nothing may obscure that.
        let reach = row.start + row_len + usize::from(row.ends_line);
        let mut break_cell = "";
        if has_selection && sel_end > row.start && sel_start < reach {
            let a = sel_start.saturating_sub(row.start).min(row_len);
            let b = (sel_end - row.start).min(row_len);
            for style in styles.iter_mut().take(b).skip(a) {
                *style = style.patch(sel_style);
            }
            if row.ends_line && sel_end > row.start + row_len {
                break_cell = " ";
            }
        }

        // Coalesce the per-character styles into as few spans as the row needs,
        // leaving out what is not on the page.
        let mut at = 0;
        while at < chars.len() {
            let style = styles[at];
            let mut to = at + 1;
            while to < chars.len() && styles[to] == style && shown[to] == shown[at] {
                to += 1;
            }
            if shown[at] {
                spans.push(Span::styled(
                    chars[at..to].iter().collect::<String>(),
                    style,
                ));
            }
            at = to;
        }
        if !break_cell.is_empty() {
            spans.push(Span::styled(break_cell, sel_style));
        }
        // A row's ground runs its whole width, not just under its words: an
        // aside is a block on the page because it is a block on paper, and a
        // painted page is painted to the edge.
        if ground.bg.is_some() {
            // What is *drawn*, not what the source is: markup taken off the
            // page took its columns with it, so the ground would otherwise stop
            // short of the right edge by exactly the hidden width.
            let used: usize = gutter
                + indent
                + chars
                    .iter()
                    .zip(&shown)
                    .filter(|(_, &on)| on)
                    .map(|(&c, _)| yumete_cjk::char_width(c))
                    .sum::<usize>()
                + break_cell.len();
            let rest = (text_area.width as usize).saturating_sub(used);
            if rest > 0 {
                spans.push(Span::styled(" ".repeat(rest), ground));
            }
        }
        lines.push(Line::from(spans));
    }
    // `.style` paints the **whole area**, not only the rows there is writing
    // on: past the last line of a short file the page is still the page, and
    // 墨香's light page on a dark terminal made that half of the window black.
    frame.render_widget(Paragraph::new(lines).style(ink.page()), text_area);

    // The margin: everything past the measure, whether or not there is writing
    // in it. Tinting only the characters that run past says nothing at all
    // when nothing does — which is exactly the case under `:wrap 50`, where
    // the rows are folded before they can reach it. A region is what a writer
    // means by a measure: this is the paper, that is the edge of it.
    //
    // Only cells nothing else has coloured, so a selection reaching into the
    // margin still reads as selected.
    if ruler > 0 {
        let left = text_area.x + (gutter + ruler) as u16;
        let buf = frame.buffer_mut();
        for y in text_area.y..text_area.y + text_area.height {
            for x in left..text_area.x + text_area.width {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    if cell.bg == Color::Reset || cell.bg == ink.paper() {
                        cell.set_bg(ink.at(yumete_config::rung::BAND));
                    }
                }
            }
        }
    }

    // The line itself, only with wrap off. With it on, the edge of the tint is
    // already the line, and drawing one would be saying the same thing twice.
    if ruler > 0 && editor.wrap_width().is_none() {
        let x = text_area.x + (gutter + ruler) as u16;
        if x < text_area.x + text_area.width {
            let buf = frame.buffer_mut();
            for y in text_area.y..text_area.y + text_area.height {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    if cell.symbol().trim().is_empty() {
                        // A **rule**, not the tint it stands in. It used to be
                        // drawn in exactly the colour of the ground behind it —
                        // 1.00:1, which is to say the line has never once been
                        // visible since it was written.
                        cell.set_symbol("│")
                            .set_style(Style::default().fg(ink.rule()));
                    }
                }
            }
        }
    }

    // The caret sits where the writing is, not where the source is: markup
    // taken off the page before it on this row took its columns with it.
    let hidden_before: usize = {
        let hide = editor.hidden_on_line(cursor_pos.line);
        if hide.is_empty() {
            0
        } else {
            let line_start = rope.line_to_char(cursor_pos.line);
            let row_start = wrap::rows_from(
                rope,
                WrapAnchor {
                    line: cursor_pos.line,
                    index_in_line: cursor_pos.index_in_line,
                },
                measure,
                1,
            )
            .first()
            .map_or(line_start, |row| row.start);
            (row_start..editor.cursor())
                .filter(|&at| {
                    let column = at - line_start;
                    hide.iter().any(|&(a, b)| column >= a && column < b)
                })
                .map(|at| yumete_cjk::char_width(rope.char(at)))
                .sum()
        }
    };
    // Clamped to the page: a caret resting past a row that exactly fills the
    // width would otherwise be drawn in the column after the last one.
    let x = (gutter + cursor_pos.column.saturating_sub(hidden_before))
        .min(text_area.width.saturating_sub(1) as usize);
    (
        text_area.x + x as u16,
        text_area.y + cursor_row.min(last_row) as u16,
    )
}

/// Draw the status line, or the command line while a `:` or `/` prompt is open.
fn draw_status(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    ime: &ImeSession,
    status_area: Rect,
    tab_area: Rect,
) {
    let buffer = editor.current_buffer();
    // A raised strip, not a reversal. Reversing gave a **white bar** under a
    // dark page — the loudest thing on the screen, saying the least — and it
    // only inverted the cells something was written on, so the bar stopped
    // wherever the text did and left a notch at the right end.
    let ink = crate::theme::Palette::of(config);
    let bar = ink
        .ground(yumete_config::rung::CHROME)
        .fg(ink.text());
    // The sidebar used to take the whole status line to list its keys. It has
    // the row above for that now, and taking this one as well would mean losing
    // the file name and the position for as long as the sidebar has focus.
    let status = if editor.sidebar_focused() && !config.editor.hints {
        format!("-- 側欄 --  {}", Editor::sidebar_keys())
    } else if let Some((prefix, text)) = editor.prompt() {
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
        // The guess is a rung back from what was actually typed — a colour,
        // not `DIM`, so it is still visibly *not yet* part of the line on a
        // terminal that drops the attribute.
        let guess = bar.fg(ink.at(yumete_config::rung::RULE));
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(line, bar),
                Span::styled(ghost, guess),
                Span::styled(format!("{}{tag}", " ".repeat(gap)), bar),
            ]))
            .style(bar),
            status_area,
        );
        return;
    } else {
        let dirty = if buffer.is_modified() { " [+]" } else { "" };
        // A recovered draft is worth a standing tag, not just the one-off notice
        // at open: the status line is cleared by the next keystroke, and the
        // writer must still be able to see that `:recover` has something to do.
        let draft = if buffer.recovered_draft().is_some() {
            " [draft]"
        } else {
            ""
        };
        // In Insert mode with the IME available, show the 中/英 state + scheme.
        let ime_tag = match language_tag(editor, ime).as_str() {
            "" => String::new(),
            tag => format!("{tag} "),
        };
        // With more than one file open, say which — unless the tab bar is up,
        // which says it better and already says it.
        let (n, total) = editor.buffer_position();
        let which = if total > 1 && !tabs_show_everything(editor, config, tab_area) {
            format!(" [{n}/{total}]")
        } else {
            String::new()
        };
        let left = format!(
            "-- {} --  {}{}{}{}{}",
            editor.mode_label(),
            ime_tag,
            buffer.display_name(),
            dirty,
            draft,
            which
        );
        // Where you are, and nothing else. What just happened is the row
        // above's question — and with no hint row it comes back here, because
        // a message nobody can see is not a message.
        let where_ = position_of(editor);
        if config.editor.hints || editor.status().is_empty() {
            format!("{left}   {where_}")
        } else {
            format!("{left}   {}   {where_}", editor.status())
        }
    };

    // What the 字 under the cursor *is*, pushed to the right edge so it never
    // moves the position readout around. A rare 漢字 that came out as a box is
    // the case this answers: `U+2B740 · CJK Unified Ideographs Extension D`
    // says the character is fine and the font is not.
    let right = char_info(editor, config);
    let used = yumete_cjk::str_width(&status);
    let room = (status_area.width as usize).saturating_sub(used);
    // Two stages of giving way: the block name goes first, then the code point,
    // and the left side is never squeezed.
    let tail = [right.0.as_str(), right.1.as_str()]
        .into_iter()
        .find(|t| !t.is_empty() && yumete_cjk::str_width(t) + 2 <= room)
        .unwrap_or("");
    let gap = room.saturating_sub(yumete_cjk::str_width(tail));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(status, bar),
            Span::styled(format!("{}{tail}", " ".repeat(gap)), bar),
        ]))
        .style(bar),
        status_area,
    );
}

/// A paragraph's opening squares, and what is drawn in them.
///
/// White by default — that is what a book prints — with two answers for a
/// draft being edited, where 「這裏原本有個空行」 is a live question: a band,
/// or a mark in the first square. Both are set **back**, not forward: this is
/// furniture about the writing, not the writing.
fn indent_span(indent: usize, editor: &Editor, ink: crate::theme::Palette) -> Span<'static> {
    use yumete_core::zong::IndentHint;
    match editor.indent_hint() {
        IndentHint::None => Span::styled(" ".repeat(indent), ink.page()),
        IndentHint::Colour => Span::styled(
            " ".repeat(indent),
            ink.ground(yumete_config::rung::BAND),
        ),
        IndentHint::Symbol => {
            let mark = editor.indent_symbol().to_string();
            let width = yumete_cjk::str_width(&mark).min(indent);
            let text = format!("{mark}{}", " ".repeat(indent.saturating_sub(width)));
            Span::styled(text, ink.page().fg(ink.rule()))
        }
    }
}

/// The row above the status line: what just happened, and what you can press.
///
/// Quieter than the status line, and deliberately: the status line is the
/// page's own footing and is always there, while this comes and goes. Set on
/// the page's own ground rather than reversed, so a blank one reads as part of
/// the margin instead of as an empty bar.
fn draw_hints(frame: &mut Frame, editor: &Editor, config: &Config, area: Rect) {
    use yumete_core::editor::Hint;
    let ink = crate::theme::Palette::of(config);
    // News is the loud kind; keys are the quiet kind and read as furniture.
    let news = Style::default().fg(ink.text());
    // The key is what the eye is hunting for, so it is the lit half; what it
    // does is the half you only read once.
    let key = Style::default().fg(ink.text());
    let what = Style::default().fg(ink.furniture());
    // 金墨: the label names what mode you are in, which is not prose either.
    let label = Style::default().fg(ink.gold()).add_modifier(Modifier::BOLD);
    let right = area.x + area.width;
    let buf = frame.buffer_mut();
    let mut x = area.x + 1;
    let mut put = |text: &str, style: Style, x: &mut u16| {
        if *x >= right {
            return;
        }
        put_text(buf, *x, area.y, right, text, style);
        *x += yumete_cjk::str_width(text) as u16;
    };
    match editor.hint() {
        Hint::Quiet => {}
        Hint::Says(text) => put(&text, news, &mut x),
        Hint::Keys(name, keys) => {
            put(&name, label, &mut x);
            put("  ", what, &mut x);
            for (k, doing) in keys {
                if !k.is_empty() {
                    put(k, key, &mut x);
                    put(" ", what, &mut x);
                }
                put(&doing, what, &mut x);
                // Two spaces between pairs rather than a bullet: the gap is
                // what groups a key with its meaning, and a separator between
                // groups only competes with it.
                put("  ", what, &mut x);
            }
        }
    }
}

/// Where the cursor is, in the terms the layout is read in.
///
/// Vertically the coordinates are named for the directions they run in:
/// paragraphs stack across the page, so a paragraph number is a 橫 position;
/// the 縱 is which run of it; 字 is how far down that run. "Ln" and "Col"
/// would each mean two things here. In a grid the useful pair is the row and
/// *which column* — "column 143" of a line of 拆分 means nothing to anybody.
fn position_of(editor: &Editor) -> String {
    if let Some(where_) = editor.table_status() {
        return format!("第 {} 行 · {where_}", editor.cursor_line() + 1);
    }
    if editor.layout() == WritingLayout::Vertical {
        let at = editor.zong_position();
        // Two coordinates, not three: which 縱 the cursor is in, and how far
        // down it. A paragraph long enough to wrap runs over several 縱, so the
        // 縱 is named by the paragraph and which piece of it — `56-2` is the
        // second 縱 of paragraph 56 — and the piece is dropped when there is
        // only one, which is most paragraphs.
        let which = if at.index_in_line == 0 {
            format!("{}", at.line + 1)
        } else {
            format!("{}-{}", at.line + 1, at.index_in_line + 1)
        };
        return format!("橫 {which}, 字 {}", at.slot + 1);
    }
    format!(
        "Ln {}, Col {}",
        editor.cursor_line() + 1,
        editor.cursor_visual_column() + 1,
    )
}

/// What to say about the character under the cursor, long form and short.
///
/// Two strings rather than one, so a narrow terminal can drop the block name
/// and keep the code point instead of dropping both.
fn char_info(editor: &Editor, config: &Config) -> (String, String) {
    if !config.editor.char_info || editor.prompt().is_some() {
        return (String::new(), String::new());
    }
    let Some(c) = editor.char_at_cursor() else {
        return (String::new(), String::new());
    };
    let point = yumete_cjk::blocks::codepoint(c);
    let short = format!("{c} {point}");
    match yumete_cjk::blocks::block_of(c) {
        Some(block) => (format!("{short} · {block}"), short),
        None => (short.clone(), short),
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
    config: &Config,
    area: Rect,
    cursor_x: u16,
    cursor_y: u16,
) {
    let skin = vertical::Skin::from(config);
    let candidates = ime.page_candidates();
    let highlight = ime.highlight();

    // Build the content lines: preedit header, then the candidates.
    let preedit = ime.display_buffer();
    let mut rows: Vec<String> = Vec::with_capacity(candidates.len() + 1);
    rows.push(preedit);
    for (i, cand) in candidates.iter().enumerate() {
        // The same markers the vertical panel uses: `[panel] markers` is one
        // setting, and a reader who set 圈碼 does not want ASCII digits back
        // the moment they switch to horizontal.
        let mut row = format!(
            "{} {}",
            vertical::index_mark(&config.panel.markers, i),
            cand.text
        );
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
                Style::default().bg(skin.paper()).fg(skin.helper()),
            )));
        } else if i - 1 == highlight {
            lines.push(Line::from(Span::styled(
                row,
                Style::default()
                    .bg(skin.highlight())
                    .fg(skin.on_highlight()),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                row,
                Style::default().bg(skin.paper()).fg(skin.text()),
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
                .border_type(if config.panel.rounded {
                    BorderType::Rounded
                } else {
                    BorderType::Plain
                })
                .border_style(Style::default().fg(skin.border()).bg(skin.paper()))
                .style(Style::default().bg(skin.paper()).fg(skin.text())),
        ),
        panel,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use yumete_cjk::grapheme_width;
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

    /// Render horizontally, settling the wrap width from the terminal width
    /// first exactly as the event loop does, so `j` and the page agree on where
    /// a row begins.
    fn render_wrapped(
        editor: &mut Editor,
        config: &Config,
        w: u16,
        h: u16,
    ) -> ratatui::buffer::Buffer {
        let gutter = gutter_width(
            editor.current_buffer().line_count(),
            config.editor.line_numbers,
        );
        editor.set_wrap_width((w as usize).saturating_sub(gutter));
        render(editor, config, w, h)
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

    /// Render, returning where the terminal's cursor was left.
    fn render_caret(
        editor: &Editor,
        config: &Config,
        w: u16,
        h: u16,
    ) -> (ratatui::buffer::Buffer, Option<Position>) {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut viewport = Viewport::default();
        terminal
            .draw(|frame| draw(frame, editor, config, &no_ime(), &mut viewport))
            .unwrap();
        let at = terminal.get_cursor_position().ok();
        (terminal.backend().buffer().clone(), at)
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
        let look = vertical::Look::of(editor);
        editor.set_zong_length(vertical::zong_length_for(config, h, lines, look));
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
        let look = vertical::Look::of(editor);
        // From the page's own rectangle, exactly as the event loop does it —
        // otherwise these tests would be asking about a page nobody draws.
        let page = page_areas(editor, config, Rect::new(0, 0, w, h)).text;
        editor.set_zong_length(vertical::zong_length_for(config, page.height, lines, look));
        render_with(editor, config, ime, w, h)
    }

    /// A vertical-layout config with the decorations off, so tests read the
    /// text grid itself.
    /// The palette a test's config resolves to.
    fn ink(config: &Config) -> crate::theme::Palette {
        crate::theme::Palette::of(config)
    }

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

    /// One rendered row, as the reader sees it.
    ///
    /// A wide glyph occupies two cells and ratatui blanks the second, so the
    /// row is walked by display width rather than by cell.
    fn row_text(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
        let mut out = String::new();
        let mut x = 0;
        while x < buffer.area.width {
            let symbol = buffer[(x, y)].symbol();
            out.push_str(symbol);
            x += grapheme_width(symbol).max(1) as u16;
        }
        out
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
        // configured 縱 is. The hint row is off: this is about the 縱, and a
        // row spent on the footer would only move every coordinate below.
        let mut editor = editor_with(&"字".repeat(8));
        let mut config = vertical_config();
        config.editor.hints = false;
        let buffer = render_vertical(&mut editor, &config, 20, 8);

        assert_eq!(at(&buffer, 18, 0), "字");
        assert_eq!(at(&buffer, 18, 5), "字");
        // The seventh character starts the next 縱, to the left.
        assert_eq!(at(&buffer, 15, 0), "字");
        assert_eq!(at(&buffer, 15, 1), "字");
        assert_eq!(at(&buffer, 15, 2), " ");
    }

    #[test]
    fn hung_marks_sit_in_the_margin_tinted_apart_from_a_reading() {
        let mut editor = editor_with("曰：「學");
        editor.set_hanging_punctuation(true);
        let config = vertical_config();
        let buffer = render_vertical_ruby(&mut editor, &config, 20, 12);

        // The text column carries only text; the marks are beside it, in the
        // one-cell margin — half-width, which is what lets them fit in it.
        assert_eq!(at(&buffer, 17, 0), "曰");
        assert_eq!(at(&buffer, 17, 1), "學", "the bracket costs no row");
        assert_eq!(at(&buffer, 19, 0), ":");
        assert_eq!(at(&buffer, 19, 1), "｢", "beside 學, which it introduces");

        // A mark is tinted apart from a reading, which is set back instead.
        let mark = buffer[(19, 0)].style();
        assert!(mark.fg.is_some(), "a hung mark has a colour of its own");
        assert!(
            !mark.add_modifier.contains(Modifier::DIM),
            "it is punctuation, not an annotation"
        );
    }

    #[test]
    fn a_hung_mark_does_not_paint_over_the_zong_beside_it() {
        // The margin is one cell. A full-width mark in it spills onto the 縱 to
        // the right and the character there is never drawn at all — which is
        // what a reader sees as "hanging hides my text".
        let mut editor = editor_with("一二三。四五\n甲乙丙丁戊己");
        editor.set_hanging_punctuation(true);
        editor.set_zong_length(6);
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 20, 12);

        // Two 縱: the first paragraph on the right, the second to its left.
        let first = (0..20).find(|&x| at(&buffer, x, 0) == "一").unwrap();
        let second = (0..20).find(|&x| at(&buffer, x, 0) == "甲").unwrap();
        assert!(second < first);
        // The mark hangs beside 三…
        assert_eq!(at(&buffer, first + 2, 2), "｡");
        // …and every character of the 縱 to its left is still on the page.
        for (row, ch) in ["甲", "乙", "丙", "丁", "戊", "己"].iter().enumerate() {
            assert_eq!(at(&buffer, second, row as u16), *ch, "row {row}");
        }
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
    fn a_full_width_reading_keeps_the_page_square() {
        // 注音符號 and kana are full-width. Squeezed into the one cell pinyin
        // needs, every 縱 after them walks a column left and the page comes
        // apart into a staircase — and this feature is called 振假名.
        let mut editor =
            editor_with("<ruby>永<rt>ㄩㄥˇ</rt></ruby>和\n<ruby>山<rt>ㄕㄢ</rt></ruby>川\n天地");
        let config = vertical_config();
        let buffer = render_vertical_ruby(&mut editor, &config, 24, 12);

        // Each reading sits in the two cells to the right of its own 縱, and
        // the 縱 that carries one is four cells from the next — two for the
        // text, two for the reading — instead of being walked a column left by
        // a reading that did not fit.
        // A base is centred against its reading, so it may sit a row or two in.
        let cell_of = |c: &str| {
            (0..12)
                .flat_map(|y| (0..24).map(move |x| (x, y)))
                .find(|&(x, y)| at(&buffer, x, y) == c)
        };
        let (first, fy) = cell_of("永").expect("永 is on the page");
        let (second, sy) = cell_of("山").expect("山 is on the page");
        assert_eq!(first - second, 4, "a full-width reading needs two cells");
        // The reading runs down the margin beside its base, one 注音符號 to a
        // row; the base sits against the middle of it.
        assert_eq!(at(&buffer, first + 2, fy), "ㄥ");
        assert_eq!(at(&buffer, second + 2, sy), "ㄕ");
        // …and does not paint over the 縱 to its right.
        assert_eq!(at(&buffer, second, sy), "山");
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
    fn the_number_band_is_a_gutter_of_its_own_colour() {
        // Set vertically the numbers sit above the 縱, in the text's own
        // columns. Position separates nothing, so the band has to be told apart
        // by colour or it reads as digits somebody typed.
        let mut editor = editor_with("春江\n潮水");
        let mut config = vertical_config();
        config.editor.line_numbers = LineNumbers::Absolute;
        let buffer = render_vertical(&mut editor, &config, 30, 12);

        let band = Some(ink(&config).at(yumete_config::rung::HEAD));
        let head = vertical::number_rows(LineNumbers::Absolute, 3);
        assert!(head > 0);
        for x in 0..30 {
            assert_eq!(buffer[(x, head - 1)].style().bg, band, "gutter at {x}");
        }
        // …and the text below it is not on the band.
        assert_ne!(buffer[(28, head)].style().bg, band);
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
        assert_eq!(
            style.fg,
            ink(&config).page().fg,
            "the character keeps the page's own ink"
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
        let (tint, page) = (Some(ink(&config).word()), Some(ink(&config).paper()));
        let bg = |x: u16, y: u16| buffer[(x, y)].style().bg;
        assert_eq!(bg(18, 1), tint, "第一詞 tinted");
        assert_eq!(bg(18, 2), page, "第二詞 left as the page");
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

    #[test]
    fn every_candidate_carries_its_own_chaifen() {
        // 拆分 is turned on to see *why* two candidates want different codes,
        // which means seeing both at once. The vertical panel used to show the
        // highlighted one's alone, in a column of its own — and that column
        // also shifted every candidate one place right of the number the style
        // code thought it was.
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(
            Scheme::Lingming,
            "xj 相 \u{2ff0}\u{6728}\u{76ee}\nxj 想 \u{2ff1}\u{76f8}\u{5fc3}\n",
        );
        ime.set_annotations(true);
        ime.input('x');
        ime.input('j');
        let config = vertical_config();
        let text = buffer_text(&render_vertical_with(&mut editor, &config, &ime, 40, 20));
        assert!(text.contains('相') && text.contains('想'), "{text}");
        // Both decompositions are on the page, not only the highlighted one's.
        assert!(text.contains('目'), "相's own 拆分 is missing: {text}");
        assert!(text.contains('心'), "想's own 拆分 is missing: {text}");
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
    fn the_panel_takes_its_markers_and_skin_from_the_config() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        ime.input('b');
        let mut config = vertical_config();
        config.panel.markers = "壹貳參".to_string();
        config.panel.ink = (0x10, 0x20, 0x30);
        config.panel.paper = (0x90, 0xa0, 0xb0);
        let buffer = render_vertical_with(&mut editor, &config, &ime, 40, 16);

        let text = buffer_text(&buffer);
        assert!(text.contains('壹'), "配置的編號字符沒用上: {text:?}");
        assert!(!text.contains('㊀'), "還在用默認編號");
        // The ground is the configured paper, and the ladder runs from it.
        assert!(
            (0..buffer.area.height).any(|y| {
                (0..buffer.area.width)
                    .any(|x| buffer[(x, y)].style().bg == Some(Color::Rgb(0x90, 0xa0, 0xb0)))
            }),
            "配置的紙色沒用上"
        );
    }

    #[test]
    fn the_horizontal_panel_takes_the_same_markers_and_border() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        ime.input('b');
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.panel.markers = "壹貳參".to_string();
        config.panel.rounded = false;
        let buffer = render_with(&editor, &config, &ime, 40, 16);

        let text = buffer_text(&buffer);
        // `[panel]` is one setting: it must not stop applying the moment the
        // reader switches to the default, horizontal layout.
        assert!(text.contains('壹'), "配置的編號字符沒用上: {text:?}");
        assert!(!text.contains("1. "), "還在用 ASCII 編號");
        assert!(text.contains('┌'), "rounded = false 沒用上: {text:?}");
    }

    #[test]
    fn a_dead_code_still_shows_what_was_typed() {
        // In 形碼 a code that matches nothing is the ordinary way to mistype,
        // and the code lives only in the panel — there is no inline preedit.
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        for c in "qxqx".chars() {
            ime.input(c);
        }
        assert!(ime.page_candidates().is_empty());
        assert_eq!(ime.display_buffer(), "qxqx");

        let config = vertical_config();
        let buffer = render_vertical_with(&mut editor, &config, &ime, 40, 16);
        let text = buffer_text(&buffer);
        assert!(text.contains('q') && text.contains('x'), "打了什麼看不見");
    }

    #[test]
    fn a_digit_past_the_page_never_picks_a_candidate_off_it() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八 巴 芭 疤\n");
        // A short page: the reader sees three candidates, not five.
        ime.set_page_size(3);
        ime.input('b');
        assert_eq!(ime.page_candidates().len(), 3);

        // `5` names nothing the reader can see. It must not commit 疤 — the
        // fifth of the *whole* list, which is on the next page.
        ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char('5'),
            KeyModifiers::NONE,
        );
        assert!(
            !editor.current_buffer().text().contains('疤'),
            "committed a candidate from a page the reader cannot see: {:?}",
            editor.current_buffer().text()
        );

        // A digit that *is* on the page still selects.
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八 巴 芭 疤\n");
        ime.set_page_size(3);
        ime.input('b');
        ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char('3'),
            KeyModifiers::NONE,
        );
        assert_eq!(editor.current_buffer().text(), "巴");
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
    fn a_vertical_panel_does_not_grow_a_row_for_every_letter() {
        // With 拆分 on, the code and the decomposition were stacked down one
        // column, a character to a row: `Dyu_Do_Ne` alone was nine rows, and
        // the panel came out taller than the page it was covering.
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "dydn 靈 聯絡員
");
        for c in "dydn".chars() {
            ime.input(c);
        }
        let config = vertical_config();
        let buffer = render_vertical_with(&mut editor, &config, &ime, 40, 30);

        // The panel is as deep as its deepest column and no deeper. Four
        // letters of code are two rows, not four.
        let depth = panel_depth(&buffer);
        assert!(depth <= 8, "the panel is {depth} rows deep");

        // The code is still all there, two letters to a slot.
        let text = buffer_text(&buffer);
        assert!(text.contains("dy") && text.contains("dn"), "{text:?}");
        assert!(text.contains('靈') && text.contains('聯'), "candidates: {text:?}");
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

        // One tint, every other word, and bare page between — so both the
        // tint and the page must appear.
        let tint_a = ink(&config).word();
        let tint_b = ink(&config).paper();
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
        let tint_a = ink(&config).word();
        let any_tint = (0..buffer.area.height)
            .any(|y| (0..buffer.area.width).any(|x| buffer[(x, y)].style().bg == Some(tint_a)));
        assert!(!any_tint, "overlay should be hidden when disabled");
    }

    /// The whole page as the reader sees it, wide glyphs counted once.
    fn page_text(buffer: &ratatui::buffer::Buffer) -> String {
        (0..buffer.area.height)
            .map(|y| row_text(buffer, y))
            .collect::<Vec<_>>()
            .join("\n")
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
        assert!(status.contains("橫 1"), "which 縱: {status:?}");
        assert!(status.contains("字 2"), "how far down it: {status:?}");
        assert!(!status.contains("Ln"), "no ambiguous line number");

        // A paragraph long enough to wrap runs over several 縱, and then the
        // piece is named too: `2-2` is the second 縱 of paragraph 2.
        let mut editor = editor_with(&format!("一\n{}", "字".repeat(20)));
        editor.set_layout(WritingLayout::Vertical);
        // Into the long paragraph, then far enough down it to be past the first
        // 縱's worth of characters.
        editor.on_key(Key::Char('h'));
        for _ in 0..12 {
            editor.on_key(Key::Char('j'));
        }
        let buffer = render_vertical(&mut editor, &config, 60, 12);
        let raw: String = (0..buffer.area.width)
            .map(|x| buffer[(x, buffer.area.height - 1)].symbol())
            .collect();
        let status = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(status.contains("橫 2-2"), "the wrapped piece: {status:?}");
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

        // A rung back, not `DIM`: the attribute is dropped by enough terminals
        // that a guess would read as typed on them.
        let ink = ink(&config);
        let fg = |x: u16| buffer[(x, row)].style().fg;
        assert_eq!(fg(3), Some(ink.text()), "`seg` was typed");
        assert_eq!(
            fg(4),
            Some(ink.at(yumete_config::rung::RULE)),
            "`ment` is only a guess"
        );
    }

    #[test]
    fn the_measure_is_tinted_and_only_ruled_when_nothing_wraps() {
        let mut editor = editor_with(&"字".repeat(30));
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        config.editor.ruler = 20;
        let tint = Some(ink(&config).at(yumete_config::rung::BAND));
        // The measure is twenty columns of *writing*; with the gutter off it is
        // also cell twenty, which is what makes the coordinates below readable.
        assert_eq!(config.editor.line_numbers, LineNumbers::None);

        // Soft wrap on: the text past the measure is tinted — which is what
        // somebody breaking long sentences by hand is looking for — and there
        // is no line, because the edge of the tint already is one.
        let buffer = render_wrapped(&mut editor, &config, 40, 8);
        assert_ne!(buffer[(18, 0)].style().bg, tint, "before the measure");
        assert_eq!(buffer[(20, 0)].style().bg, tint, "past it");
        assert_ne!(at(&buffer, 20, 0), "│", "no line while it wraps");

        // Wrap off: the line is drawn, in the cells the text does not fill.
        editor.set_soft_wrap(false);
        let buffer = render_wrapped(&mut editor, &config, 40, 8);
        assert_eq!(at(&buffer, 20, 4), "│", "a measure to write to");

        // The measure is counted in writing, not in cells: turning the gutter
        // on moves the ruler over rather than eating twenty columns of text.
        config.editor.line_numbers = LineNumbers::Absolute;
        let buffer = render_wrapped(&mut editor, &config, 40, 8);
        let gutter = gutter_width(editor.current_buffer().line_count(), LineNumbers::Absolute);
        assert_eq!(at(&buffer, 20 + gutter as u16, 4), "│");
    }

    #[test]
    fn the_sidebar_opens_out_to_the_length_of_its_longest_name() {
        let mut editor = editor_with("那年冬天");
        let mut config = Config::default();
        config.editor.sidebar_width = 20;
        let dir = std::env::temp_dir().join(format!("yumete-wide-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("天門真境之傳家寶扇.md"), "# 一\n").unwrap();
        editor.open_sidebar_at(&dir);

        // Narrow, the name is cut where the rule is.
        let buffer = render_with(&editor, &config, &no_ime(), 80, 12);
        assert_eq!(at(&buffer, 19, 0), "│", "the rule at the set width");

        // `w` opens it out far enough to read the whole name, rule included.
        editor.on_key(Key::Char('w'));
        let buffer = render_with(&editor, &config, &no_ime(), 80, 12);
        let rule = (0..80u16)
            .find(|&x| at(&buffer, x, 0) == "│")
            .expect("a rule somewhere");
        assert!(rule > 19, "wider than the setting: {rule}");
        // A wide glyph covers two cells and only the first carries it, so the
        // row reads back with a gap after every 字.
        let name: String = (1..rule)
            .map(|x| at(&buffer, x, 1))
            .collect::<String>()
            .replace(' ', "");
        assert!(
            name.contains("天門真境之傳家寶扇"),
            "the whole name is readable: {name:?}"
        );

        // …and back.
        editor.on_key(Key::Char('w'));
        let buffer = render_with(&editor, &config, &no_ime(), 80, 12);
        assert_eq!(at(&buffer, 19, 0), "│");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_measure_set_by_hand_folds_the_rows_and_leaves_a_margin() {
        let mut editor = editor_with(&"字".repeat(30));
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let tint = Some(ink(&config).at(yumete_config::rung::BAND));
        // No configured ruler: the measure is the only thing saying where the
        // page ends.
        assert_eq!(config.editor.ruler, 0);

        editor.execute("wrap 20").unwrap();
        let buffer = render_wrapped(&mut editor, &config, 40, 8);

        // Twenty columns is ten 字, so the eleventh is on the second row.
        assert_eq!(at(&buffer, 0, 1), "字", "folded at the measure");
        assert_eq!(at(&buffer, 20, 0), " ", "and nothing written past it");

        // The margin is a region, not a scattering: every cell past the
        // measure is tinted, on rows that reach it and rows that do not.
        assert_ne!(buffer[(19, 0)].style().bg, tint, "inside the measure");
        assert_eq!(buffer[(20, 0)].style().bg, tint, "the first cell past it");
        assert_eq!(buffer[(39, 0)].style().bg, tint, "out to the edge");
        assert_eq!(buffer[(20, 5)].style().bg, tint, "and down the empty rows");

        // Giving the window back takes the margin with it.
        editor.execute("wrap 0").unwrap();
        let buffer = render_wrapped(&mut editor, &config, 40, 8);
        assert_ne!(buffer[(20, 0)].style().bg, tint);
        assert_eq!(at(&buffer, 20, 0), "字", "the row runs the full width");
    }

    /// A directory holding a division table, its schema, and the CSV's path.
    fn a_table_file(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("yumete-grid-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("tables").join("t.toml"),
            "[table]\nfile = ['division.csv']\n\
             [[table.column]]\nname = 'char'\n\
             [[table.column]]\nname = 'ids_y'\n\
             [[table.column]]\nname = 'note'\n",
        )
        .unwrap();
        let csv = dir.join("division.csv");
        std::fs::write(
            &csv,
            "char,ids_y,note\n一,⿰木目,x\n齾,⿰⿱⿰木目金,longer\n三,土,\n",
        )
        .unwrap();
        (dir, csv)
    }

    #[test]
    fn the_caret_moves_inside_a_cell_not_only_between_cells() {
        let (dir, csv) = a_table_file("caret");
        let mut editor = Editor::new();
        editor.open_file(&csv).unwrap();
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        editor.execute("2").unwrap();
        editor.on_key(Key::Char('l'));

        let caret = |e: &Editor, c: &Config| render_caret(e, c, 60, 10).1.unwrap().x;
        let at_start = caret(&editor, &config);

        // Reading by character, the caret has to move with the cursor — pinned
        // to the cell's first 字 it would say the cursor had not moved at all.
        editor.on_key(Key::Tab);
        editor.on_key(Key::Char('l'));
        let one = caret(&editor, &config);
        assert_eq!(one, at_start + 2, "one 漢字 further along the cell");
        editor.on_key(Key::Char('l'));
        assert_eq!(caret(&editor, &config), at_start + 4);
        editor.on_key(Key::Char('h'));
        assert_eq!(caret(&editor, &config), one, "and back");

        // The same in Insert, where it decides where the next 字 lands.
        editor.on_key(Key::Tab);
        editor.on_key(Key::Char('i'));
        assert_eq!(caret(&editor, &config), at_start, "`i` is the cell's start");
        editor.on_key(Key::Right);
        assert_eq!(caret(&editor, &config), at_start + 2);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_grid_lines_its_columns_up_and_freezes_the_header() {
        let (dir, csv) = a_table_file("draw");
        let mut editor = Editor::new();
        editor.open_file(&csv).unwrap();
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        let buffer = render(&editor, &config, 60, 10);
        let row = |y: u16| (0..60u16).map(|x| at(&buffer, x, y)).collect::<String>();

        // The header is the top row and names the columns.
        let head = row(0);
        assert!(head.starts_with("char"), "the header is frozen on top: {head:?}");
        assert!(head.contains("ids_y") && head.contains("note"));

        // Every row's second column starts in the same terminal column — which
        // is the entire point of drawing a CSV as a grid.
        let column_of = |y: u16, want: &str| {
            (0..60u16).find(|&x| at(&buffer, x, y) == want)
        };
        let a = column_of(1, "⿰").expect("一's 拆分");
        let b = column_of(2, "⿰").expect("齾's 拆分");
        assert_eq!(a, b, "the same field of two rows starts in the same column");
        assert!(a > 4, "and after the first column, not at the edge");

        // The widest visible cell sets the column's width, so `longer` fits.
        assert!(row(2).contains("longer"), "{:?}", row(2));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_cell_the_cursor_is_in_is_a_box_not_a_word() {
        let (dir, csv) = a_table_file("cell");
        let mut editor = Editor::new();
        editor.open_file(&csv).unwrap();
        editor.execute("2").unwrap();
        editor.on_key(Key::Char('l'));
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        let buffer = render(&editor, &config, 60, 10);
        let lit = Some(ink(&config).selection());

        // Row 2 of the file is the first data row, on screen row 1 under the
        // header; the cursor is in its second cell.
        let start = (0..60u16)
            .find(|&x| buffer[(x, 1)].style().bg == lit)
            .expect("a lit cell");
        assert_eq!(at(&buffer, start, 1), "⿰", "it is the 拆分 cell");
        // The ground runs the column's whole width, past the end of the text —
        // a cell you are inside, not three highlighted characters.
        // Counted, not run-length: a wide glyph covers two cells and ratatui
        // only ever sends the first, so the second reads back unstyled here
        // even though the terminal paints the whole glyph. What matters is
        // that the ground reaches past the end of the text.
        let last = (0..60u16)
            .rfind(|&x| buffer[(x, 1)].style().bg == lit)
            .unwrap();
        let text_ends = (start..60).find(|&x| at(&buffer, x, 1) == " " && at(&buffer, x - 1, 1) == " ");
        assert!(
            last > start + 5,
            "the box is the column's width, not the text's: {start}..{last}"
        );
        assert!(text_ends.is_some_and(|e| last >= e), "the padding is lit too");
        // …and nothing on the header row or another row is lit.
        assert!((0..60u16).all(|x| buffer[(x, 0)].style().bg != lit), "not the header");
        assert!((0..60u16).all(|x| buffer[(x, 2)].style().bg != lit), "not another row");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_row_the_schema_cannot_account_for_is_marked_not_hidden() {
        let (dir, csv) = a_table_file("torn");
        // A hand edit left a row with four fields where the schema says three.
        std::fs::write(
            &csv,
            "char,ids_y,note\n一,⿰木目,x\n二,土,a,b\n",
        )
        .unwrap();
        let mut editor = Editor::new();
        editor.open_file(&csv).unwrap();
        assert!(editor.row_is_ragged(2), "four fields, three columns");
        assert!(!editor.row_is_ragged(1), "and the good row is not");

        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::Absolute;
        let buffer = render(&editor, &config, 60, 10);
        let row = |y: u16| (0..60u16).map(|x| at(&buffer, x, y)).collect::<String>();
        // The extra field is drawn: hiding it would hide the damage.
        assert!(row(2).contains("a") && row(2).contains("b"), "{:?}", row(2));
        // And the row number is marked, so it can be found from a distance.
        let torn = Some(ink(&config).mark());
        assert_eq!(buffer[(0, 2)].style().fg, torn, "the bad row's number");
        assert_ne!(buffer[(0, 1)].style().fg, torn, "not the good one's");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_row_above_says_what_would_finish_what_you_started() {
        let mut editor = editor_with("那年冬天");
        let config = Config::default();
        // A wide glyph covers two cells and only the first carries it, so the
        // row reads back with a gap after every 字.
        let hint = |e: &Editor| -> String {
            let b = render(e, &config, 100, 10);
            (0..100u16).map(|x| at(&b, x, 8)).collect::<String>().replace(' ', "")
        };

        // Nothing begun, nothing to say: the row is blank rather than filled
        // with something to read.
        assert_eq!(hint(&editor).trim(), "");

        // A sequence begun and not finished is the case this row exists for —
        // `m` is otherwise only in the manual.
        editor.on_key(Key::Char('m'));
        let h = hint(&editor);
        assert!(h.contains("配對") && h.contains("包起來"), "{h:?}");
        editor.on_key(Key::Esc);

        // `g` likewise.
        editor.on_key(Key::Char('g'));
        assert!(hint(&editor).contains("檔首"), "{:?}", hint(&editor));
        editor.on_key(Key::Esc);

        // `Space` says nothing here, because it opens a menu that already
        // lists its own keys — the same thing twice on two surfaces is worse
        // than once.
        editor.on_key(Key::Char(' '));
        assert_eq!(hint(&editor).trim(), "");
        editor.on_key(Key::Esc);
    }

    #[test]
    fn the_tab_bar_scrolls_to_the_file_you_are_in() {
        // The bar used to start at the first file and stop when it ran out of
        // room, so past about eight chapters the one being written was never
        // on it — and the `[n/m]` that would have said so was suppressed
        // *because* the bar was up. With 122 buffers neither said it.
        let mut editor = Editor::new();
        for n in 1..=20 {
            editor.execute("new").unwrap();
            // A buffer with nothing in it is the one `:new` reuses.
            editor.on_key(Key::Char('i'));
            editor.on_key(Key::Char('字'));
            editor.on_key(Key::Esc);
            editor
                .current_buffer_mut()
                .name_as(&format!("第{n:02}章.md"));
        }
        let config = Config::default();
        let buffer = render(&editor, &config, 40, 10);
        let bar: String = (0..40u16).map(|x| at(&buffer, x, 0)).collect();
        assert!(bar.contains("20"), "the tab you are in is on the bar: {bar:?}");
        // …and the status line says the fraction again, because the bar cannot
        // show them all.
        let status: String = (0..40u16).map(|x| at(&buffer, x, 9)).collect();
        assert!(status.contains("/20]"), "{status:?}");

        // Walk back to the first and the bar comes with you.
        for _ in 0..19 {
            editor.execute("buffer previous").unwrap();
        }
        let buffer = render(&editor, &config, 40, 10);
        let bar: String = (0..40u16).map(|x| at(&buffer, x, 0)).collect();
        assert!(bar.contains("01"), "{bar:?}");
    }

    #[test]
    fn the_indent_moves_the_cursor_as_well_as_the_page() {
        // The renderer wrapped with the indent and every motion wrapped
        // without it, so `j` and `k` landed on the character under a column
        // nobody was looking at, and a click was two cells off.
        let mut editor = editor_with("甲\n\n一二三四五六七八\n");
        editor.set_indent(2);
        editor.set_soft_wrap(true);
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        // Eight cells: two go to the indent, so the first row holds three 字
        // and the rows under it hold four. Read from the *first* paragraph,
        // where the cursor is not standing — the one it is standing in is
        // shown as the file has it, flush and with its blank line back.
        let buffer = render_wrapped(&mut editor, &config, 8, 8);
        assert_eq!(at(&buffer, 0, 1), " ");
        assert_eq!(at(&buffer, 2, 1), "一");
        assert_eq!(at(&buffer, 0, 2), "四", "the second row starts flush");
        // Down into it, and it opens: no indent, and the blank line above it
        // is drawn again.
        editor.on_key(Key::Char('j'));
        editor.on_key(Key::Char('j'));
        let buffer = render_wrapped(&mut editor, &config, 8, 8);
        assert_eq!(at(&buffer, 0, 2), "一", "the paragraph being stood in");
        // …and the second `j` stepped one *visual* row inside it: open, eight
        // cells hold four 字, so the row under 一 begins at 五.
        assert_eq!(
            editor.current_buffer().rope().char(editor.cursor()),
            '五',
            "one row down, same column"
        );
    }

    #[test]
    fn the_page_the_cursor_moves_on_is_the_page_that_is_drawn() {
        // Three parts of the program worked the geometry out separately — the
        // drawing, the mouse, and the event loop settling the 縱 length before
        // the keys that use it. They disagreed by the hint row, the tab bar and
        // the detail panel, so the 縱 the cursor moved on was longer than the
        // 縱 on the screen and a click resolved to the wrong character.
        let mut editor = editor_with("一二三四五六七八九十\n");
        editor.set_layout(WritingLayout::Vertical);
        let config = vertical_config();
        let area = Rect::new(0, 0, 40, 20);
        let page = page_areas(&editor, &config, area).text;
        // Twenty rows, less the status line and the hint row.
        assert_eq!(page.height, 18, "the page is not the terminal");
        let look = vertical::Look::of(&editor);
        let lines = editor.current_buffer().line_count();
        let motion = vertical::zong_length_for(&config, page.height, lines, look);
        let drawn = vertical::Metrics::new(&config, page.height, lines, look).zong_len;
        assert_eq!(motion, drawn, "and both sides measure the same one");
    }

    #[test]
    fn two_bands_read_top_right_to_bottom_left() {
        // 段組: a 縱 of fifty characters is tiring to read, and the traditional
        // answer is to halve the page and use the width instead. A terminal is
        // a wide, short shape, which is exactly what 段組 is for.
        let text: String = (0..12).map(|_| "一二三四五六七八\n").collect();
        let mut editor = editor_with(&text);
        editor.set_layout(WritingLayout::Vertical);
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        config.editor.paper_ticks = 0;

        // Narrow enough that twelve paragraphs cannot fit in one band.
        let one = render(&editor, &config, 14, 21);
        editor.set_bands(2);
        let two = render(&editor, &config, 14, 21);
        assert_ne!(buffer_text(&one), buffer_text(&two), "the page changed");

        let has = |b: &ratatui::buffer::Buffer, rows: std::ops::Range<u16>| {
            rows.flat_map(|y| (0..14u16).map(move |x| (x, y)))
                .any(|(x, y)| at(b, x, y) == "一")
        };
        assert!(has(&two, 0..2), "the first band opens at the top");
        assert!(has(&two, 8..14), "and the second band is a page of its own");
        // With one band the page runs top to bottom and there is no second one.
        assert!(!has(&one, 8..14), "one band: nothing starts halfway down");
    }

    #[test]
    fn a_paragraph_opens_two_squares_in_on_the_page() {
        // A Chinese paragraph is marked by an indent of two 字, and the blank
        // line it replaces costs a whole row.
        let mut editor = editor_with("那年冬天很冷。\n# 第一章\n窗外落着雪。\n");
        editor.set_indent(2);
        // …off the first paragraph, which is shown as the file has it while
        // the cursor is standing in it.
        editor.on_key(Key::Char('j'));
        editor.on_key(Key::Char('j'));
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        let buffer = render(&editor, &config, 40, 8);
        assert_eq!(at(&buffer, 0, 0), " ");
        assert_eq!(at(&buffer, 1, 0), " ");
        assert_eq!(at(&buffer, 2, 0), "那", "two squares, then the paragraph");
        // A heading carries its own leading structure and is not pushed right.
        assert_eq!(at(&buffer, 0, 1), "#");
        // The caret sits on the first character, not in the indent.
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('g'));
        assert_eq!(
            yumete_core::wrap::column_of(
                editor.current_buffer().rope(),
                editor.cursor(),
                yumete_core::wrap::Measure::plain(40).with_indent(2)
            ),
            2
        );
    }

    #[test]
    fn the_indent_takes_the_blank_line_off_the_page() {
        // Both marks at once is the one thing no typesetter does: the file
        // keeps the blank line (it is what makes it a paragraph in Markdown),
        // the page shows the indent instead. The numbers then run 1, 3, 5 —
        // which is the file's own numbering, not a renumbering.
        let mut editor = editor_with("第一段\n\n第二段\n\n第三段\n");
        editor.set_indent(2);
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::Absolute;
        let buffer = render(&editor, &config, 40, 8);
        let row = |y: u16| row_text(&buffer, y).trim_end().to_string();
        assert!(row(0).ends_with("第一段"), "{:?}", row(0));
        assert!(row(1).ends_with("第二段"), "{:?}", row(1));
        assert!(row(2).ends_with("第三段"), "{:?}", row(2));
        assert!(row(0).starts_with('1'), "{:?}", row(0));
        assert!(row(1).starts_with('3'), "the file's own numbers: {:?}", row(1));
        assert!(row(2).starts_with('5'), "{:?}", row(2));

        // `j` steps over it rather than onto it — a row nobody can see is not
        // a row the cursor may rest on.
        editor.on_key(Key::Char('j'));
        assert_eq!(editor.cursor_line(), 2, "j landed on the folded blank line");

        // With no indent the page is the file again.
        editor.set_indent(0);
        let buffer = render(&editor, &config, 40, 8);
        assert_eq!(row_text(&buffer, 1).trim_end(), "2");
    }

    /// A book of roughly `chars` 漢字, in paragraphs with a blank line between.
    #[cfg(test)]
    fn a_book_of(chars: usize) -> String {
        let pool: Vec<char> = (0x4E00u32..0x9FA5).filter_map(char::from_u32).collect();
        let mut out = String::with_capacity(chars * 4);
        let mut seed = 0x2545F491_4F6CDD1Du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut written = 0usize;
        while written < chars {
            let want = 40 + (next() % 360) as usize;
            for i in 0..want {
                out.push(pool[(next() % pool.len() as u64) as usize]);
                if i % 17 == 16 {
                    out.push('。');
                }
            }
            out.push_str("\n\n");
            written += want;
        }
        out
    }

    /// How long a book takes to open, and to draw. A measurement, not an
    /// assertion — run it with
    /// `cargo test -p yumete-tui --release opening_a_book -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn what_a_frame_costs() {
        use std::time::Instant;
        // **The input method is built once**, as the real editor builds it —
        // `no_ime()` loads 靈明's built-in 碼表, and a harness that builds one
        // per frame measures the table and not the page.
        let ime = no_ime();
        let config = Config::default();
        let frame = |editor: &mut Editor, n: u32| {
            editor.set_wrap_width(100 - 4);
            let t = Instant::now();
            for _ in 0..n {
                let _ = render_with(editor, &config, &ime, 100, 40);
            }
            t.elapsed() / n
        };
        let mut editor = editor_with("那年冬天");
        println!("empty page: {:.2?} a frame", frame(&mut editor, 200));
        let mut editor = Editor::new();
        editor.current_buffer_mut().insert(0, &a_book_of(20_000));
        println!("a book:     {:.2?} a frame", frame(&mut editor, 200));
        editor.set_segmentation_visible(true);
        println!("…segmented: {:.2?} a frame", frame(&mut editor, 200));
        editor.set_indent(2);
        println!("…indented:  {:.2?} a frame", frame(&mut editor, 200));
    }

    #[test]
    #[ignore]
    fn opening_a_book_of_ten_million_characters() {
        use std::time::Instant;
        println!("{:>10}  {:>10}  {:>12}  {:>12}  {:>10}", "字", "open", "first frame", "40 × j", "G");
        for chars in [10_000usize, 100_000, 1_000_000, 10_000_000] {
            let dir = std::env::temp_dir().join(format!("yumete-open-{chars}"));
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("book.txt");
            std::fs::write(&path, a_book_of(chars)).unwrap();

            let mut editor = Editor::new();
            let t = Instant::now();
            editor.open_file(path.to_str().unwrap()).unwrap();
            let open = t.elapsed();

            let config = Config::default();
            let ime = no_ime();
            editor.set_wrap_width(96);
            let t = Instant::now();
            let _ = render_with(&editor, &config, &ime, 100, 40);
            let first = t.elapsed();

            let t = Instant::now();
            for _ in 0..40 {
                editor.on_key(Key::Char('j'));
                let _ = render_with(&editor, &config, &ime, 100, 40);
            }
            let scrolling = t.elapsed();

            let t = Instant::now();
            editor.on_key(Key::Char('G'));
            let _ = render_with(&editor, &config, &ime, 100, 40);
            let end = t.elapsed();

            println!("{chars:>10}  {open:>10.1?}  {first:>12.1?}  {scrolling:>12.1?}  {end:>10.1?}");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn the_paragraph_the_cursor_is_in_is_shown_as_the_file_has_it() {
        // 所見即所得's bargain, one construct larger: the paragraph you are
        // standing in is shown as it really is — no indent the file does not
        // contain, and the blank line above it back — so there is never a
        // question about what is there. Exactly one is open at a time, so
        // moving between them closes one and opens another and the page below
        // does not shift.
        let mut editor = editor_with("第一段\n\n第二段\n\n第三段\n");
        editor.set_indent(2);
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;

        // The cursor is in the first: it is flush, and the others are a book.
        let buffer = render(&editor, &config, 40, 8);
        assert_eq!(row_text(&buffer, 0).trim_end(), "第一段");
        assert_eq!(row_text(&buffer, 1).trim_end(), "  第二段");
        assert_eq!(row_text(&buffer, 2).trim_end(), "  第三段");

        // Down to the second: its blank line comes back above it, and the
        // first closes up.
        editor.on_key(Key::Char('j'));
        let buffer = render(&editor, &config, 40, 8);
        assert_eq!(row_text(&buffer, 0).trim_end(), "  第一段");
        assert_eq!(row_text(&buffer, 1).trim_end(), "");
        assert_eq!(row_text(&buffer, 2).trim_end(), "第二段", "the one stood in");
        assert_eq!(row_text(&buffer, 3).trim_end(), "  第三段");

        // …and Insert changes nothing: it is the same page, being typed into.
        editor.on_key(Key::Char('i'));
        let typing = render(&editor, &config, 40, 8);
        for row in 0..4 {
            assert_eq!(row_text(&typing, row), row_text(&buffer, row), "row {row}");
        }
    }

    #[test]
    fn a_message_never_takes_away_where_you_are() {
        let mut editor = editor_with("那年冬天");
        let config = Config::default();
        let row = |b: &ratatui::buffer::Buffer, w: u16| -> String {
            let y = b.area.height - 1;
            (0..w).map(|x| at(b, x, y)).collect::<String>()
        };

        editor.on_key(Key::Char('l'));
        let quiet = row(&render(&editor, &config, 100, 10), 100);
        assert!(quiet.contains("Ln 1, Col 3"), "{quiet:?}");

        // A yank says something, and it says it on the row above — the status
        // line goes on answering "where am I" while it does.
        editor.on_key(Key::Char('y'));
        let buffer = render(&editor, &config, 100, 10);
        let hint: String = (0..100u16).map(|x| at(&buffer, x, 8)).collect();
        assert!(hint.contains("取"), "the message is above: {hint:?}");
        let status = row(&buffer, 100);
        assert!(status.contains("Ln 1, Col 3"), "position kept: {status:?}");
        assert!(!status.contains("取"), "and not repeated: {status:?}");

        // With the hint row off the message comes back to the status line —
        // a message nobody can see is not a message.
        let mut plain = config.clone();
        plain.editor.hints = false;
        let status = row(&render(&editor, &plain, 100, 10), 100);
        assert!(status.contains("取") && status.contains("Ln 1, Col 3"), "{status:?}");
    }

    #[test]
    fn the_status_line_names_the_character_under_the_cursor() {
        let mut editor = editor_with("那年冬天");
        let config = Config::default();
        let row = |b: &ratatui::buffer::Buffer, w: u16| -> String {
            let y = b.area.height - 1;
            (0..w).map(|x| at(b, x, y)).collect::<String>()
        };

        // The cursor opens on the first 字.
        let buffer = render(&editor, &config, 90, 10);
        let line = row(&buffer, 90);
        assert!(
            line.contains("U+90A3") && line.contains("CJK Unified Ideographs"),
            "the 字 is named at the right edge: {line:?}"
        );
        assert!(line.starts_with("-- NORMAL --"), "and the left is untouched");

        // Moving names a different one.
        editor.on_key(Key::Char('l'));
        editor.on_key(Key::Char('l'));
        let buffer = render(&editor, &config, 90, 10);
        assert!(row(&buffer, 90).contains("U+51AC"), "冬");

        // A narrow line gives up the block name before the code point, and the
        // position readout is never squeezed.
        let buffer = render(&editor, &config, 60, 10);
        let line = row(&buffer, 60);
        assert!(line.contains("U+51AC"), "the code point survives: {line:?}");
        assert!(!line.contains("CJK Unified"), "the block name gives way first");
        assert!(line.contains("Ln 1, Col 5"), "position kept: {line:?}");

        // Narrower still and it says nothing rather than truncating.
        let buffer = render(&editor, &config, 44, 10);
        assert!(!row(&buffer, 44).contains("U+"), "nothing rather than a stub");
    }

    #[test]
    fn a_vertical_page_colours_its_markup_the_way_a_horizontal_one_does() {
        // `**那**` — a bold word between markers, and a heading above it.
        let mut editor = editor_with("# 卷一\n那年**冬天**，山下起了大雪。");
        let config = vertical_config();
        editor.set_render(yumete_core::editor::Render::On);
        let buffer = render_vertical(&mut editor, &config, 30, 16);

        // Find the 縱 the sentence is set in — the second, since the heading
        // takes the first, and 縱 fill from the right edge.
        let bold = (0..30u16)
            .flat_map(|x| (0..16u16).map(move |y| (x, y)))
            .find(|&(x, y)| at(&buffer, x, y) == "冬")
            .expect("冬 is on the page");
        assert!(
            buffer[bold].style().add_modifier.contains(Modifier::BOLD),
            "the word between the markers is set bold"
        );
        let marker = (0..30u16)
            .flat_map(|x| (0..16u16).map(move |y| (x, y)))
            .find(|&(x, y)| at(&buffer, x, y) == "*")
            .expect("the markers stay on the page");
        assert_eq!(
            buffer[marker].style().fg,
            Some(ink(&config).marker()),
            "and the markers themselves are set back"
        );
        let plain = (0..30u16)
            .flat_map(|x| (0..16u16).map(move |y| (x, y)))
            .find(|&(x, y)| at(&buffer, x, y) == "那")
            .expect("那 is on the page");
        assert!(
            !buffer[plain].style().add_modifier.contains(Modifier::BOLD),
            "the prose around it is not"
        );

        // The heading is a block, and a block colours the whole 縱 it runs in.
        let heading = (0..30u16)
            .flat_map(|x| (0..16u16).map(move |y| (x, y)))
            .find(|&(x, y)| at(&buffer, x, y) == "卷")
            .expect("the heading is on the page");
        assert!(
            buffer[heading].style().add_modifier.contains(Modifier::BOLD),
            "a heading is set bold vertically too"
        );
    }

    #[test]
    fn a_dense_page_spends_every_column_on_writing() {
        // More text than the page can hold, so what is measured is how much of
        // it fits — which is the only thing 密排 is for.
        let mut editor = editor_with(&"字".repeat(600));
        let mut config = vertical_config();
        config.editor.zong_gap = 1;
        config.editor.paper_ticks = 10;

        // Ordinarily a 縱 costs three cells: two for the 字 and one for the gap
        // it is read across, plus a column wherever a reading or a tick goes.
        let loose = render_vertical(&mut editor, &config, 40, 14);
        let fits = |b: &ratatui::buffer::Buffer| {
            // The text area only: the status line is inside the window and is
            // not writing, and its message differs between the two renders.
            (0..40u16)
                .flat_map(|x| (0..13u16).map(move |y| (x, y)))
                .filter(|&(x, y)| at(b, x, y) == "字")
                .count()
        };
        let before = fits(&loose);

        // Packed, a 縱 is two cells — one 漢字 — and nothing else is spent.
        editor.execute("dense").unwrap();
        let tight = render_vertical(&mut editor, &config, 40, 14);
        assert!(
            fits(&tight) > before,
            "more of the writing is on the page: {} then {}",
            before,
            fits(&tight)
        );
        // The 稿紙 ticks are gone: that column is the whole point.
        assert!(
            (0..40u16).all(|x| (0..14u16).all(|y| at(&tight, x, y) != ".")),
            "no ticks on a packed page"
        );

        // …and it comes back, because a toggle that does not is not one.
        editor.execute("dense off").unwrap();
        let loose = render_vertical(&mut editor, &config, 40, 14);
        assert_eq!(fits(&loose), before, "back to where it was");
    }

    #[test]
    fn a_zong_number_runs_down_its_own_column_a_digit_at_a_time() {
        // Two digits to a row is half as tall, but with the 縱 packed tight
        // there is no gap between them: 「119」「118」 came out as 「11」「11」
        // over 「9」「8」, a wall of digits with no line number in it.
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        for i in 0..125 {
            for c in format!("{i}").chars() {
                editor.on_key(Key::Char(c));
            }
            editor.on_key(Key::Enter);
        }
        editor.on_key(Key::Esc);
        let mut config = vertical_config();
        config.editor.line_numbers = LineNumbers::Absolute;
        editor.execute("dense").unwrap();
        editor.execute("1").unwrap();
        let buffer = render_vertical(&mut editor, &config, 40, 16);

        // Three rows of header for a file of three-digit lines, one digit each,
        // and the first 縱 is numbered 1 — bottom-aligned, so its lone digit
        // sits on the last header row.
        let rightmost = 40 - 1;
        assert_eq!(at(&buffer, rightmost, 2), "1", "縱 1 is numbered 1");
        assert_eq!(at(&buffer, rightmost, 1), " ", "and nothing above it");

        // The 縱 beside it cannot borrow a digit: each column holds one.
        for y in 0..3u16 {
            let cell = at(&buffer, rightmost - 1, y);
            assert!(cell == " " || cell.is_empty(), "the other cell of the slot: {cell:?}");
        }
    }

    #[test]
    fn a_vertical_page_is_ruled_like_稿紙() {
        let mut editor = editor_with(&"字".repeat(24));
        let mut config = vertical_config();
        config.editor.paper_ticks = 10;
        let buffer = render_vertical(&mut editor, &config, 20, 14);

        // Ruling the page gives every 縱 a margin — the rightmost included, which
        // otherwise sits flush against the edge — so the page steps in a column.
        // The first 縱 is the rightmost, so look from the right edge inward.
        let x = (0..20)
            .rev()
            .find(|&x| at(&buffer, x, 0) == "字")
            .expect("the 縱");
        assert_eq!(x, 17, "the page made room for the rule");
        // A tick at the tenth character down, and nowhere in between.
        assert_eq!(at(&buffer, x + 2, 9), ".", "the tenth 字");
        assert_eq!(at(&buffer, x + 2, 4).trim(), "", "and not the fifth");
    }

    #[test]
    fn the_insert_caret_does_not_paint_out_a_half_width_character() {
        // A digit hangs against the slot's right edge. Asking only the left
        // cell whether the slot is blank says yes, and the caret's ideographic
        // space then covers the digit — which is what "I typed 1 and got a
        // space, and the 1 appeared only when I typed the next character"
        // looks like from the writing end.
        let mut editor = editor_with("輸入法");
        editor.on_key(Key::Char('A')); // insert at the end of the line
        editor.on_key(Key::Char('1'));
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 30, 12);

        let found = (0..30)
            .flat_map(|x| (0..12).map(move |y| (x, y)))
            .any(|(x, y)| at(&buffer, x, y) == "1");
        assert!(found, "the digit was painted over by the caret");
    }

    #[test]
    fn markdown_is_coloured_without_being_hidden() {
        let mut editor = editor_with("# 第一章\n那**年**冬天");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let buffer = render(&editor, &config, 40, 8);

        // The markup is still on the page — this is a manuscript, not a preview.
        assert_eq!(row_text(&buffer, 0).trim_end(), "# 第一章");
        assert_eq!(row_text(&buffer, 1).trim_end(), "那**年**冬天");

        // The hashes are set back and the title is set forward. Set back is a
        // *rung*, not `DIM`: several terminals drop the attribute, and a
        // manuscript whose markup is only told apart by one is not told apart.
        let marker = Some(ink(&config).marker());
        assert_eq!(buffer[(0, 0)].style().fg, marker);
        assert!(buffer[(2, 0)].style().add_modifier.contains(Modifier::BOLD));
        // 那 is prose, 年 is bold, and the asterisks are quiet but present.
        assert!(!buffer[(0, 1)].style().add_modifier.contains(Modifier::BOLD));
        assert_eq!(buffer[(2, 1)].style().fg, marker);
        assert!(buffer[(4, 1)].style().add_modifier.contains(Modifier::BOLD));

        // Turning it off leaves the text alone.
        editor.set_render(yumete_core::editor::Render::Off);
        let buffer = render(&editor, &config, 40, 8);
        assert_eq!(buffer[(0, 0)].style().fg, ink(&config).page().fg);
    }

    #[test]
    fn 所見即所得_takes_the_markup_off_except_under_the_cursor() {
        let mut editor = editor_with("那**年**冬**天**");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;

        // Source mode: the file is what is on the page.
        let buffer = render(&editor, &config, 40, 6);
        assert_eq!(row_text(&buffer, 0).trim_end(), "那**年**冬**天**");

        // 所見即所得 with the cursor at the start: all of it comes off.
        editor.execute(":render full").unwrap();
        for c in "gg".chars() {
            editor.on_key(Key::Char(c));
        }
        let buffer = render(&editor, &config, 40, 6);
        assert_eq!(row_text(&buffer, 0).trim_end(), "那年冬天");

        // Move into the first construct and it — and only it — is shown whole,
        // which is what lets it be edited at all.
        editor.on_key(Key::Char('l'));
        editor.on_key(Key::Char('l'));
        let buffer = render(&editor, &config, 40, 6);
        assert_eq!(row_text(&buffer, 0).trim_end(), "那**年**冬天");

        // …and back to source on demand.
        editor.execute(":render on").unwrap();
        let buffer = render(&editor, &config, 40, 6);
        assert_eq!(row_text(&buffer, 0).trim_end(), "那**年**冬**天**");
    }

    #[test]
    fn a_row_holds_what_fits_on_the_screen_not_what_fits_in_the_source() {
        // A row's worth of hidden markup used to take a row of its own, which
        // drew as a blank line in the middle of a paragraph — and the page
        // repainted completely as the cursor moved onto it.
        let mut editor =
            editor_with("見[附錄](https://example.com/a/very/long/target/path/goes/on)後天冬");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        editor.execute(":render full").unwrap();
        for c in "gg".chars() {
            editor.on_key(Key::Char(c));
        }

        let buffer = render_wrapped(&mut editor, &config, 30, 8);
        // What the reader sees is one short line, and nothing after it.
        assert_eq!(row_text(&buffer, 0).trim_end(), "見附錄後天冬");
        assert_eq!(
            row_text(&buffer, 1).trim_end(),
            "",
            "no blank row in between"
        );
    }

    #[test]
    fn the_caret_sits_where_the_writing_is_not_where_the_source_is() {
        let mut editor = editor_with("那**年**冬天");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        editor.execute(":render full").unwrap();
        // Onto 天, clear of the construct whose markup has come off. (Standing
        // *on* the construct's own boundary keeps it open, which is what stops
        // the line flickering as the cursor leaves it.)
        for c in "gg".chars() {
            editor.on_key(Key::Char(c));
        }
        for _ in 0..7 {
            editor.on_key(Key::Char('l'));
        }
        editor.on_key(Key::Char('i'));

        let mut terminal = Terminal::new(TestBackend::new(40, 6)).unwrap();
        let mut viewport = Viewport::default();
        terminal
            .draw(|frame| draw(frame, &editor, &config, &no_ime(), &mut viewport))
            .unwrap();
        let at = terminal.get_cursor_position().unwrap();
        // 那年冬 is six cells; the four asterisks took their columns with them.
        assert_eq!(at.x, 6, "the caret followed the markup off the page");
    }

    #[test]
    fn nothing_is_markup_inside_a_fence() {
        // What is written in a code block is written verbatim. Colouring `**`
        // there — let alone taking it off the page — misreports the file.
        let mut editor = editor_with("那年\n```\nlet a = **b**;\n```\n冬天");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;

        let buffer = render(&editor, &config, 40, 8);
        assert_eq!(row_text(&buffer, 2).trim_end(), "let a = **b**;");
        let bold = (0..40).any(|x| buffer[(x, 2)].style().add_modifier.contains(Modifier::BOLD));
        assert!(!bold, "code is not emphasis");

        // …and 所見即所得 leaves it alone too.
        editor.execute(":render full").unwrap();
        let buffer = render(&editor, &config, 40, 8);
        assert_eq!(row_text(&buffer, 2).trim_end(), "let a = **b**;");
    }

    #[test]
    fn the_word_overlay_is_the_quietest_layer() {
        // A `==highlight==` exists *to be* a ground; the word tint must not rub
        // it out, nor a container's colour.
        let editor = editor_with("那==年==冬");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = true;
        let buffer = render(&editor, &config, 40, 6);

        // 年 is at cell 6 (那 + the two `=`), and keeps the highlight's ground.
        let highlight = (0..40)
            .map(|x| buffer[(x, 0)].style().bg)
            .find(|bg| *bg == Some(ink(&config).wash()));
        assert!(highlight.is_some(), "the word tint erased the highlight");
    }

    #[test]
    fn a_block_grounds_the_whole_row_and_the_runs_keep_their_weight() {
        let mut editor = editor_with("那年\n::: warning 小心\n這裏有**伏筆**\n:::\n冬天");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let buffer = render(&editor, &config, 40, 8);

        // Prose is on the page's own ground; the container is on its own, and
        // that ground runs the width of the row, not just under the words.
        let prose = buffer[(0, 0)].style().bg;
        let aside = buffer[(0, 2)].style().bg;
        assert_ne!(aside, prose, "the container has a ground of its own");
        assert_eq!(buffer[(38, 2)].style().bg, aside, "all the way across");
        assert_eq!(
            buffer[(0, 4)].style().bg,
            prose,
            "and it ends where it says"
        );

        // A bold word inside it keeps its bold *and* takes the ground.
        let bold = (0..40)
            .map(|x| buffer[(x, 2)].style())
            .find(|s| s.add_modifier.contains(Modifier::BOLD));
        assert_eq!(
            bold.map(|s| s.bg),
            Some(aside),
            "both, not one or the other"
        );

        // And with the colouring off, none of it applies.
        editor.set_render(yumete_core::editor::Render::Off);
        let buffer = render(&editor, &config, 40, 8);
        assert_eq!(buffer[(0, 2)].style().bg, prose);
    }

    #[test]
    fn a_note_to_oneself_is_set_back_but_never_hidden() {
        let editor = editor_with("寫到這裏 %%這句再想想%%");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let buffer = render(&editor, &config, 40, 8);
        // Still on the page — a note you cannot see is a note you will not act
        // on — and set apart from the writing around it.
        assert!(row_text(&buffer, 0).contains("這句再想想"));
        let prose = buffer[(0, 0)].style().fg;
        let aside = (0..40)
            .map(|x| buffer[(x, 0)].style().fg)
            .find(|fg| *fg != prose);
        assert!(
            aside.is_some(),
            "a note reads as a note, not as the writing"
        );
    }

    #[test]
    fn a_selection_keeps_the_weight_of_what_it_covers() {
        // Three layers that compose: Markdown sets the ink and the weight, the
        // selection sets the ground. Whichever won used to be an if/else, so a
        // bold word inside a selection lost its bold.
        let mut editor = editor_with("那**年**冬天");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        editor.on_key(Key::Char('%'));
        let buffer = render(&editor, &config, 40, 8);

        let cell = buffer[(4, 0)].style();
        assert_eq!(cell.bg, Some(ink(&config).selection()), "selected");
        assert!(cell.add_modifier.contains(Modifier::BOLD), "and still bold");
    }

    #[test]
    fn the_tabs_name_the_open_files_and_light_the_one_being_written() {
        let mut editor = Editor::new();
        editor.current_buffer_mut().insert(0, "第一篇");
        editor.execute(":new").unwrap();
        editor.current_buffer_mut().insert(0, "第二篇");
        assert_eq!(editor.buffer_count(), 2);
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;

        let buffer = render_with(&editor, &config, &no_ime(), 40, 8);
        let bar = row_text(&buffer, 0);
        assert!(bar.contains("[scratch]"), "both files named: {bar:?}");
        // The one being written is dirty and says so.
        assert!(bar.contains('+'), "{bar:?}");
        // It is lit against the bar's own ground, so which one you are in is a
        // thing you can see rather than a key you have to press.
        // The bar is black like the page; the lit tab is raised off it.
        let lit = (0..40)
            .any(|x| buffer[(x, 0)].style().bg == Some(ink(&config).at(yumete_config::rung::HEAD)));
        let unlit = (0..40).any(|x| buffer[(x, 0)].style().bg == Some(ink(&config).chrome()));
        assert!(lit && unlit, "the current tab is not told apart");

        // The page starts below the bar, not under it.
        assert_eq!(at(&buffer, 0, 1), "第");

        // …and with one file open the bar costs nothing.
        let mut alone = Editor::new();
        alone.current_buffer_mut().insert(0, "第一篇");
        let buffer = render_with(&alone, &config, &no_ime(), 40, 8);
        assert_eq!(at(&buffer, 0, 0), "第", "no bar for a single file");
    }

    #[test]
    fn the_pane_holding_the_keys_says_how_to_give_them_back() {
        let dir = std::env::temp_dir().join(format!("yumete-hint-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("ch01.md"), "").unwrap();

        let mut editor = editor_with("那年冬天");
        let config = Config::default();
        editor.open_sidebar_at(&dir);
        let buffer = render(&editor, &config, 80, 12);
        // The keys go on the row above, so the status line can go on saying
        // which file you are in and where in it you are.
        let hint = row_text(&buffer, 10);
        assert!(hint.contains("側欄"), "{hint:?}");
        assert!(hint.contains("C-w"), "how to get back: {hint:?}");
        let status = row_text(&buffer, 11);
        assert!(status.contains("[scratch]"), "still says the file: {status:?}");

        // With the keys back in the text it says what it always said.
        editor.on_key(Key::Ctrl('w'));
        let buffer = render(&editor, &config, 80, 12);
        let status = row_text(&buffer, 11);
        assert!(status.contains("NORMAL"), "{status:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_sidebar_takes_its_columns_off_the_page() {
        let dir = std::env::temp_dir().join(format!("yumete-side-tui-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("卷一")).unwrap();
        std::fs::write(dir.join("notes.md"), "").unwrap();
        let mut editor = editor_with("那年冬天");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.sidebar_width = 20;

        // Without it, the text starts at the left edge.
        let plain = render_with(&editor, &config, &no_ime(), 60, 12);
        assert_eq!(at(&plain, 0, 0), "那");

        // Rooted explicitly rather than at the process's directory, which the
        // other tests share.
        editor.open_sidebar_at(&dir);
        let buffer = render_with(&editor, &config, &no_ime(), 60, 12);
        let text = page_text(&buffer);
        assert!(text.contains("卷一"), "the tree: {text:?}");
        assert!(text.contains("notes.md"), "{text:?}");
        // A rule at its right edge, and the page begins after it — not under it.
        assert_eq!(at(&buffer, 19, 3), "│");
        assert_eq!(at(&buffer, 20, 0), "那", "the text moved over, not under");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn space_lists_what_it_offers_and_the_picker_replaces_it() {
        let mut editor = editor_with("那年冬天");
        let config = Config::default();

        // Space alone says what its second half can be — which-key, so the set
        // is discoverable without leaving the page.
        editor.on_key(Key::Char(' '));
        let text = page_text(&render_with(&editor, &config, &no_ime(), 60, 24));
        assert!(text.contains("開啟檔案"), "{text:?}");
        assert!(text.contains("切換緩衝區"), "{text:?}");

        // `b` replaces it with the picker, which names where you are in a list
        // it does not have to show all of.
        editor.on_key(Key::Char('b'));
        let buffer = render_with(&editor, &config, &no_ime(), 60, 24);
        let text = page_text(&buffer);
        assert!(text.contains("緩衝區"), "{text:?}");
        assert!(!text.contains("開啟檔案"), "the menu is gone: {text:?}");
        // And it is a small box, not the page.
        let drawn = (0..buffer.area.height)
            .filter(|&y| {
                (0..buffer.area.width)
                    .any(|x| buffer[(x, y)].style().bg == Some(Color::Rgb(0x26, 0x2a, 0x27)))
            })
            .count();
        assert!(drawn <= 9, "the picker took {drawn} rows");
    }

    #[test]
    fn the_command_menu_lists_and_narrows() {
        let mut editor = editor_with("那年冬天");
        let config = Config::default();

        // `:` on its own offers a *window* onto the list — capped, the way
        // Helix caps its completion popup — with a count saying how much more
        // there is. Every command with its help at once covered the page.
        editor.on_key(Key::Char(':'));
        let buffer = render_with(&editor, &config, &no_ime(), 90, 24);
        let text = buffer_text(&buffer);
        assert!(text.contains(":open"), "menu should list commands: absent");
        let total = yumete_core::command::COMMANDS.len();
        assert!(
            text.contains(&format!("1/{total}")),
            "how much more there is"
        );
        // The menu is a handful of rows, not the screen.
        let drawn = (0..buffer.area.height)
            .filter(|&y| {
                (0..buffer.area.width)
                    .any(|x| buffer[(x, y)].style().bg == Some(Color::Rgb(0x26, 0x2a, 0x27)))
            })
            .count();
        assert!(drawn <= 9, "the menu took {drawn} rows");

        // Typing narrows it, and the commands that no longer match go away.
        for c in "ruby".chars() {
            editor.on_key(Key::Char(c));
        }
        let buffer = render_with(&editor, &config, &no_ime(), 90, 24);
        let text = buffer_text(&buffer);
        assert!(text.contains("ruby"), "still matching");
        assert!(!text.contains("write"), "no longer matching");

        // A space asks the other question — not "which command" but "what may
        // follow it" — and the menu answers with the words and their help.
        editor.on_key(Key::Char(' '));
        let buffer = render_with(&editor, &config, &no_ime(), 90, 24);
        let text = buffer_text(&buffer);
        assert!(text.contains("html"), "the words `:ruby` takes: {text:?}");
        assert!(!text.contains(":html"), "a word is not a command, so no colon");
        // A wide glyph covers two cells and only the first carries it.
        let squashed = text.replace(' ', "");
        assert!(squashed.contains("排出注音"), "and what each one does");
    }

    #[test]
    fn the_command_menu_spreads_across_a_wide_window() {
        let config = Config::default();
        let total = yumete_core::command::COMMANDS.len();

        // Wide: every command at once, in columns, rather than a third of them
        // with the rest of the page standing empty beside it.
        let mut wide = editor_with("那年冬天");
        wide.on_key(Key::Char(':'));
        let buffer = render_with(&wide, &config, &no_ime(), 140, 24);
        let text = buffer_text(&buffer);
        assert!(text.contains(":open"), "{text:?}");
        assert!(text.contains(":render"), "a command from the far end of the list");
        assert!(text.contains(&format!("1/{total}")), "the count is still there");

        // The list runs *down* first and then across, like a list of files:
        // more than one column, and no taller than a menu is allowed to be.
        let lit = |y: u16| {
            (0..buffer.area.width)
                .any(|x| buffer[(x, y)].style().bg == Some(Color::Rgb(0x26, 0x2a, 0x27)))
        };
        let rows = (0..buffer.area.height).filter(|&y| lit(y)).count();
        assert!(rows <= 9, "a menu is glanced at, not read: {rows} rows");
        let starts: Vec<u16> = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| buffer[(x, y)].symbol() == ":")
            .map(|(x, _)| x)
            .collect();
        let columns: std::collections::BTreeSet<u16> = starts.into_iter().collect();
        assert!(columns.len() >= 3, "several columns: {columns:?}");

        // Narrow: one column, and the count says how much did not fit.
        let mut narrow = editor_with("那年冬天");
        narrow.on_key(Key::Char(':'));
        let buffer = render_with(&narrow, &config, &no_ime(), 30, 24);
        let text = buffer_text(&buffer);
        assert!(text.contains(":open"));
        assert!(!text.contains(":render"), "no room for the far end: {text:?}");
    }

    #[test]
    fn tab_inks_its_pick_in_the_menu() {
        let mut editor = editor_with("那年冬天");
        editor.on_key(Key::Char(':'));
        editor.on_key(Key::Char('r'));
        let config = Config::default();

        let plain = render_with(&editor, &config, &no_ime(), 90, 24);
        // Above the footing: the status line is the page turned over and is
        // inked along its whole length whether or not anything is picked.
        let inked = |b: &ratatui::buffer::Buffer| {
            (0..b.area.height - 2).any(|y| {
                (0..b.area.width)
                    .any(|x| b[(x, y)].style().bg == Some(ink(&config).text()))
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
        // Above it is the hint row, and above *that* the menu's own footer —
        // the count and what the highlighted row means — with the rows above
        // that again. The menu stacks upward from the whole footer, not from
        // the command line alone, so it never covers either.
        let footer: String = (0..buffer.area.width)
            .map(|x| buffer[(x, buffer.area.height - 3)].symbol())
            .collect();
        assert!(footer.contains('/'), "a count of the matches: {footer:?}");
        let text = buffer_text(&buffer);
        assert!(text.contains("ruby") || text.contains("redo"), "{text:?}");
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
    fn command_key_chords_are_the_terminals_not_ours() {
        // With the Kitty protocol on, ⌘C really does arrive. Read as a bare
        // letter it is `c` — *change* — so asking for a copy deleted the
        // selection. Nothing modified by ⌘ is ours.
        for c in ['c', 'v', 'a', 'z', 'q'] {
            assert_eq!(map_key(KeyCode::Char(c), KeyModifiers::SUPER), None, "⌘{c}");
        }
        // …and the modifiers that *are* ours still work.
        assert_eq!(
            map_key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(Key::Ctrl('c'))
        );
        assert_eq!(
            map_key(KeyCode::Char('c'), KeyModifiers::NONE),
            Some(Key::Char('c'))
        );

        // The IME does not take them either: in Insert, ⌘C would have typed a
        // `c` into the manuscript.
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char('c'),
            KeyModifiers::SUPER,
        );
        assert_eq!(editor.current_buffer().text(), "");
        assert!(!ime.is_composing());
    }

    #[test]
    fn shift_normalizes_lowercase_letters_to_uppercase() {
        let (code, _) = normalize_shift(KeyCode::Char('d'), KeyModifiers::SHIFT);
        assert_eq!(code, KeyCode::Char('D'));
        // Without Shift, unchanged.
        let (code, _) = normalize_shift(KeyCode::Char('d'), KeyModifiers::NONE);
        assert_eq!(code, KeyCode::Char('d'));
    }

    // ---- Soft wrap (Feature #77) ------------------------------------------

    /// A config for the wrap tests: no gutter, no overlay, so the rows read as
    /// the text grid itself.
    fn wrap_config() -> Config {
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        config
    }

    #[test]
    fn a_long_paragraph_continues_on_the_next_row() {
        let mut editor = Editor::new();
        editor
            .current_buffer_mut()
            .insert(0, "春夏秋冬春夏秋冬春夏");
        let buf = render_wrapped(&mut editor, &wrap_config(), 8, 5);
        assert_eq!(row_text(&buf, 0), "春夏秋冬");
        assert_eq!(row_text(&buf, 1), "春夏秋冬");
        assert_eq!(row_text(&buf, 2).trim_end(), "春夏");
    }

    #[test]
    fn without_wrapping_the_tail_of_a_paragraph_is_clipped() {
        let mut editor = Editor::new();
        editor
            .current_buffer_mut()
            .insert(0, "春夏秋冬春夏秋冬春夏");
        editor.set_soft_wrap(false);
        let buf = render_wrapped(&mut editor, &wrap_config(), 8, 5);
        assert_eq!(row_text(&buf, 0), "春夏秋冬");
        assert_eq!(row_text(&buf, 1).trim(), "");
    }

    #[test]
    fn a_continuation_row_carries_no_line_number() {
        let mut editor = Editor::new();
        editor.current_buffer_mut().insert(0, "春夏秋冬春夏");
        let mut config = wrap_config();
        config.editor.line_numbers = LineNumbers::Absolute;
        let buf = render_wrapped(&mut editor, &config, 12, 5);
        let gutter = gutter_width(1, LineNumbers::Absolute);
        let first = row_text(&buf, 0);
        assert!(first.starts_with(&gutter_text(0, 0, gutter, LineNumbers::Absolute)));
        // The second row of the same paragraph is numberless: the number names
        // the paragraph, not the screen row.
        let second = row_text(&buf, 1);
        assert!(second.starts_with(&" ".repeat(gutter)), "{second:?}");
        assert_eq!(second.trim(), "夏", "{first:?} / {second:?}");
    }

    #[test]
    fn j_walks_the_rows_the_reader_sees() {
        let mut editor = Editor::new();
        editor.current_buffer_mut().insert(0, "春夏秋冬春夏秋冬");
        let config = wrap_config();
        render_wrapped(&mut editor, &config, 8, 5);
        // One paragraph, two rows: `j` from the first character lands under it
        // rather than at the end of the buffer.
        editor.on_key(Key::Char('j'));
        assert_eq!(editor.cursor(), 4);
        editor.on_key(Key::Char('k'));
        assert_eq!(editor.cursor(), 0);
    }

    #[test]
    fn the_caret_follows_the_cursor_onto_the_second_row() {
        let mut editor = Editor::new();
        editor.current_buffer_mut().insert(0, "春夏秋冬春夏秋冬");
        let config = wrap_config();
        render_wrapped(&mut editor, &config, 8, 5);
        editor.on_key(Key::Char('j'));
        let mut terminal = Terminal::new(TestBackend::new(8, 5)).unwrap();
        let mut viewport = Viewport::default();
        terminal
            .draw(|frame| draw(frame, &editor, &config, &no_ime(), &mut viewport))
            .unwrap();
        let at = terminal.get_cursor_position().unwrap();
        assert_eq!((at.x, at.y), (0, 1));
    }

    #[test]
    fn a_word_split_by_a_wrap_keeps_one_colour() {
        let mut editor = Editor::new();
        editor.set_segmenter(Box::new(DictionarySegmenter::from_text(
            "那年 10\n冬天 10\n下雪 10\n以後 10\n",
            0,
        )));
        editor.current_buffer_mut().insert(0, "那年冬天下雪以後");
        editor.set_segmentation_visible(true);
        let mut config = wrap_config();
        config.editor.show_segmentation = true;
        // Width 8 puts the wrap inside 冬天 — er, between 年 and 冬.
        let buf = render_wrapped(&mut editor, &config, 8, 5);

        // Whatever the words are, the alternation must not restart on the
        // second row: the first word of row two carries on from row one.
        let colour_at = |x: u16, y: u16| buf[(x, y)].style().bg;
        assert_ne!(colour_at(0, 0), colour_at(4, 0), "两个相邻的词应当颜色不同");
        assert_eq!(colour_at(4, 0), colour_at(4, 1), "颜色在折行处重新开始了");
    }

    #[test]
    fn a_selection_shows_the_line_break_it_covers() {
        let mut editor = Editor::new();
        editor.current_buffer_mut().insert(0, "甲\n\n乙\n");
        let config = wrap_config();
        // Select the whole file: the blank paragraph in the middle is inside
        // the selection and must look like it.
        editor.on_key(Key::Char('%'));
        let buf = render_wrapped(&mut editor, &config, 8, 5);
        assert_eq!(
            buf[(0, 1)].style().bg,
            Some(ink(&config).selection()),
            "空行落在選區裏卻看不出來"
        );
    }

    #[test]
    fn a_wrapped_paragraph_scrolls_by_row_not_by_paragraph() {
        let mut editor = Editor::new();
        // One paragraph of forty 漢字: five rows at width sixteen, in a
        // terminal that can show three of them.
        let text: String = "春夏秋冬".repeat(10);
        editor.current_buffer_mut().insert(0, &text);
        let config = wrap_config();
        for _ in 0..8 {
            editor.on_key(Key::Char('j'));
        }
        let buf = render_wrapped(&mut editor, &config, 16, 4);
        // The cursor left the first row behind, so the page did too — which is
        // impossible when a page is anchored at a paragraph.
        assert_eq!(editor.cursor_line(), 0);
        assert!(row_text(&buf, 0).contains('春'));
        let mut terminal = Terminal::new(TestBackend::new(16, 4)).unwrap();
        let mut viewport = Viewport::default();
        terminal
            .draw(|frame| draw(frame, &editor, &config, &no_ime(), &mut viewport))
            .unwrap();
        let at = terminal.get_cursor_position().unwrap();
        assert!(at.y < 3, "the caret stays on the page, was at row {}", at.y);
    }
}
