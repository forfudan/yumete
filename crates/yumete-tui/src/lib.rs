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
pub mod width;

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
use yumete_cjk::Segmenter;
use yumete_core::sidebar::View;
use yumete_core::wrap::{self, Anchor as WrapAnchor};
use yumete_core::zong::{Anchor, Layout as WritingLayout};
use yumete_core::{say, Editor, Key, KeyOutcome, Mode, TextStore};
use yumete_ime::{CommitStrategy, DataFault, DataProblem, ImeSession, PanelDisplay, Scheme};

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

/// The reading a launch puts off until the page is on the screen.
///
/// The language data is 14 MB and, read cold, is most of what「開個檔案要幾
/// 秒」 was: none of it is needed to *draw* the first page — it is needed by
/// `w`, by the word overlay, and by typing 漢字, all of which are at least one
/// keystroke away. It cannot be read on another thread (yume's engine holds
/// `Rc`s and is not `Send`), so it is read here, after the first frame, where
/// the wait is something the reader watches finish rather than something they
/// wait through in front of a blank terminal.
pub type Deferred = Box<dyn FnOnce(&mut ImeSession) -> String>;

/// The dictionary that decides where one word ends and the next begins.
///
/// **Best first**, and one copy of the answer: the start-up path and
/// `:word list reload` used to be two, and only one of them knew about the
/// language model.
///
/// 1. Yume's own language model — over a million weighted entries in both
///    scripts, already loaded with the IME and shared by reference.
/// 2. A `segmentation.txt` the reader wrote, in the data directory.
/// 3. The compact list bundled with the binary, which covers common prose.
///
/// `level` is `:word level`, applied to whichever of the three it settled on.
pub fn choose_words(ime: &ImeSession, level: yumete_cjk::WordLevel) -> Box<dyn Segmenter> {
    let yume = ime.segmenter();
    let mut words: Box<dyn Segmenter> = if yume.is_available() {
        Box::new(yume)
    } else if let Some(written) = read_word_list() {
        Box::new(written)
    } else {
        Box::new(yumete_core::DictionarySegmenter::builtin(0))
    };
    words.set_level(level);
    words
}

/// A `word<TAB>weight` list the reader wrote, from the first data directory
/// that has one.
fn read_word_list() -> Option<yumete_core::DictionarySegmenter> {
    for dir in yumete_config::data_search_dirs() {
        if let Ok(text) = std::fs::read_to_string(dir.join("segmentation.txt")) {
            return Some(yumete_core::DictionarySegmenter::from_text(&text, 0));
        }
    }
    None
}

/// **One frame, drawn into a string** — the page as it would appear, with no
/// terminal involved.
///
/// `:shot` hands the screen to the platform's own screenshot program, which
/// needs a window, a GUI session and a person to look at the result. This is
/// the same picture for everyone else: a reviewer on a headless machine, a
/// regression that has to be *seen* rather than asserted about, a bug report
/// that can carry the page it is about.
///
/// Text only. Colour is what the theme tests already check cell by cell; what
/// a picture is wanted for is where things are — the wrap, the caret, the
/// gutter, the panel, the 縱 that the page is laid out in.
pub fn frame_to_text(
    editor: &mut Editor,
    config: &Config,
    ime: &ImeSession,
    width: u16,
    height: u16,
) -> String {
    frame_to(editor, config, ime, width, height, false)
}

/// The same frame **with its colours**, as one self-contained HTML `<pre>`.
///
/// Text answers 「where is everything」; this answers 「what does it look
/// like」, which is the other half of a screenshot and the half a theme is
/// judged on. Every cell becomes a span carrying its own ink and ground, so
/// what a browser shows is what the terminal would show — no palette, no
/// approximation, the actual bytes the renderer produced.
pub fn frame_to_html(
    editor: &mut Editor,
    config: &Config,
    ime: &ImeSession,
    width: u16,
    height: u16,
) -> String {
    frame_to(editor, config, ime, width, height, true)
}

fn frame_to(
    editor: &mut Editor,
    config: &Config,
    ime: &ImeSession,
    width: u16,
    height: u16,
    html: bool,
) -> String {
    // **The mood, before the first cell is painted.** `run` settles this and
    // the shot did not, so every picture came out in the theme's default mood
    // however loudly the config said `mode = "dark"` — 48 frames of a
    // light/dark comparison, all of them light. There is no terminal here to
    // ask, so `auto` takes the same answer an unanswering terminal gets: dark.
    crate::theme::settle(config, None);
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("a terminal over a buffer");
    let mut viewport = Seats::default();
    // **The page is settled the way the loop settles it**, or the picture is
    // not of the page: without the wrap width a paragraph runs off the right
    // edge and the shot shows a book with no second line in it. The 縱 length
    // is the same question asked the other way round.
    let areas = page_areas(editor, config, Rect::new(0, 0, width, height));
    let page = areas.panes[editor.live_pane().min(1)];
    let lines = editor.current_buffer().line_count();
    if editor.layout() == WritingLayout::Vertical {
        let look = vertical::Look::of(editor);
        editor.set_zong_length(vertical::zong_length_for(config, page.height, lines, look));
        let metrics = vertical::Metrics::new(config, page.height, lines, look);
        editor.set_page(page.height as usize, metrics.capacity(page.width));
    } else {
        let gutter = gutter_width(lines, config.editor.line_numbers);
        editor.set_wrap_width((page.width as usize).saturating_sub(gutter));
        editor.set_page(page.height as usize, page.width.max(1) as usize);
    }
    // …and the candidate goes into the text, the same last step the loop takes
    // before it draws. Without it `--shot` cannot photograph `bare` at all —
    // and a picture is how this repo reviews anything that touches the front
    // end.
    settle_inline_candidate(editor, ime);
    terminal
        .draw(|frame| draw(frame, editor, config, ime, &mut viewport))
        .expect("draw one frame");
    let buffer = terminal.backend().buffer();
    if html {
        return buffer_to_html(buffer);
    }
    let mut out = String::new();
    for y in 0..buffer.area.height {
        let mut row = String::new();
        let mut x = 0;
        while x < buffer.area.width {
            let symbol = buffer[(x, y)].symbol();
            row.push_str(symbol);
            // A wide glyph owns the cell beside it, which the backend keeps as
            // a blank. Copying that blank out puts a space between every two
            // 漢字 — the one thing a picture of a Chinese page must not do.
            //
            // **`drawn_width`, not `str_width`**: the cells were laid out by
            // ratatui, which counts Ambiguous characters narrow whatever the
            // editor was told. With `ambiguous_width = "wide"` the editor's
            // own width says `—` owns two cells; the renderer put the next
            // character in the second one, and stepping by the editor's answer
            // silently dropped it — one character per em dash, on a page of
            // Chinese prose.
            x += yumete_cjk::drawn_width(symbol).max(1) as u16;
        }
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

pub fn run(
    editor: &mut Editor,
    config: &Config,
    ime: &mut ImeSession,
    mut deferred: Option<Deferred>,
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

    let mut viewport = Seats::default();
    let mut shift = ShiftTap::default();
    // A typesetter started with `:preview`, if one is running — and one left
    // behind by a session that ended badly, which is stopped before this one
    // can start another.
    let mut job: Option<Job> = None;
    if let Some(said) = adopt_an_orphan() {
        editor.set_status(said);
    }
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
            // …and the half of it the **keys** are in: with a split open the
            // whole text area is twice the page, so `C-f` turned two pages and
            // `C-d` moved a whole pane instead of half of one.
            let areas = page_areas(editor, config, Rect::new(0, 0, size.width, size.height));
            let page = areas.panes[editor.live_pane().min(1)];
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
        // …and what the input method is offering, in the text, before anybody
        // measures the page. `draw` only holds the editor by reference, and
        // the caret, `j`, the mouse and 折行 all have to agree that the
        // candidate is on the page — which is the whole reason #210 made ghost
        // text an input to the layout rather than something painted over it.
        settle_inline_candidate(editor, ime);
        if let Err(err) = terminal.draw(|frame| draw(frame, editor, config, ime, &mut viewport)) {
            break Err(err);
        }
        // …and the picture is of *this* frame, which is the one with no
        // command line across it.
        if editor.take_screenshot_request() {
            editor.set_status(take_a_picture(config));
            continue;
        }
        // The page is up; now the 14 MB. Between these two lines is the whole
        // of what a cold start used to spend before showing anything.
        if let Some(load) = deferred.take() {
            let said = load(ime);
            if !said.is_empty() {
                editor.set_status(said);
            }
            editor.set_ime_available(ime.available());
            let words = ime.segmenter();
            if words.is_available() {
                editor.set_segmenter(Box::new(words));
                // The book's own words sit on top of whichever dictionary it
                // turned out to be.
                editor.reload_project_words();
            }
            continue;
        }
        // **`:reload auto` needs a clock, not a keystroke.** `event::read`
        // blocks, so the case the setting is for — alt-tab away, run a script,
        // come back and look — produced nothing at all until a key was pressed
        // (Feature #214). Only while it is on: an editor that wakes up twice a
        // second for nobody is an editor that flattens a battery.
        if editor.reload_auto() {
            match event::poll(DISK_POLL) {
                Ok(false) => {
                    editor.disk_tick();
                    continue;
                }
                Ok(true) => {}
                Err(err) => break Err(err),
            }
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
                // **`C-Space` 開／關輸入法.** The lesson opens with it, `:help`
                // lists it, and nothing implemented it: the key fell through to
                // the editor as an unbound `Ctrl(' ')` and was ignored, so the
                // first thing this editor asks a new reader to press did
                // nothing at all. A terminal that cannot tell Ctrl+Space from
                // NUL sends `Char('\0')`; both spellings arrive here.
                let control_space = mods.contains(KeyModifiers::CONTROL)
                    && matches!(code, KeyCode::Char(' ') | KeyCode::Char('\0') | KeyCode::Null);
                if control_space && composes(editor.mode()) {
                    let want = if ime.is_chinese() { "-" } else { "+" };
                    let said = switch_scheme(ime, want, config);
                    editor.set_status(said);
                    continue;
                }
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
                // `:word list reload` — the dictionary is the front end's to
                // build (it holds the IME and knows the data directory), and
                // the level the reader chose survives the rebuild.
                if editor.take_words_request() {
                    let level = editor.word_level();
                    editor.set_segmenter(choose_words(ime, level));
                    editor.set_status(say!("詞表重讀了：{0}", editor.words_in_force()));
                }
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
                // **What a language was told to run** (Feature #197): the
                // editor says which verb and which file, the config says what
                // to run, and the front end is the only one that can run it.
                if let Some(want) = editor.take_language_run() {
                    let said = run_for_language(editor, config, &want);
                    editor.set_status(said);
                }
                if let Some(want) = editor.take_preview_request() {
                    // Already running: hand back the address and open the page
                    // again. Killing it and starting another is a fresh compile
                    // of the whole book to answer 「where was that page?」.
                    if let yumete_core::editor::Preview::Show = want {
                        if let Some(url) = editor.preview_at().map(str::to_string) {
                            show(&url);
                            editor.set_status(say!("預覽：{0}（`:preview off` 停）", url));
                        }
                        continue;
                    }
                    if let Some(mut running) = job.take() {
                        let _ = running.child.kill();
                        let _ = running.child.wait();
                        forget_the_server();
                        editor.set_preview_at(None);
                        editor.set_status(say!("預覽：{0} 已停", running.what));
                    }
                    if let yumete_core::editor::Preview::Start { path, syntax } = want {
                        // **What the config says, if it says anything.** The
                        // typesetter for a language is a fact about the
                        // reader's machine, not about the editor — see
                        // `yumete_config::Runner`. The built-in tinymist below
                        // is what happens when nobody has said otherwise.
                        let declared = config
                            .language
                            .get(syntax.name())
                            .and_then(|verbs| verbs.get("preview"))
                            .filter(|r| r.kind == yumete_config::RunKind::Server);
                        if let Some(runner) = declared {
                            let file = path.display().to_string();
                            match runner.argv(&file).and_then(|argv| {
                                argv.split_first().map(|(p, a)| (p.clone(), a.to_vec()))
                            }) {
                                Some((program, args)) => match Job::server(&program, &args) {
                                    Ok(started) => {
                                        editor.set_status(say!("預覽：{0} 起來中……", program));
                                        job = Some(started);
                                    }
                                    Err(why) => editor.set_status(why),
                                },
                                None => editor.set_status(say!("命令寫壞了：{0}", runner.run)),
                            }
                            continue;
                        }
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
                                // `export!`, not `export`: the plain form
                                // refuses to overwrite a file that is already
                                // there, which is right for a manuscript and
                                // wrong for the scratch page this rewrites
                                // every time. Without the `!` the *second*
                                // `:preview` of a Markdown file failed, and
                                // said so in the language of a command the
                                // reader had not typed.
                                match editor.execute(&format!("export! html {}", out.display())) {
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
                        // Remembered, not just said: a line on the status bar
                        // is gone by the next keystroke, and the address is
                        // what a writer comes back to ask for.
                        editor.set_preview_at(Some(url.clone()));
                        editor.set_status(say!("預覽：{0}（`:preview off` 停）", url));
                    }
                }
                if let Some(tag) = editor.take_scheme_request() {
                    editor.set_status(switch_scheme(ime, &tag, config));
                    editor.set_ime_available(ime.available());
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
                // …and ask the disk whether the file moved under us
                // (Feature #214). Throttled inside too, and off by default.
                // Also asked on the idle path above, which is where it matters.
                editor.disk_tick();
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
        let _ = running.child.wait();
    }
    forget_the_server();

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

/// How long an idle `:reload auto` session waits before looking at the disk.
///
/// The same two seconds the core throttles at, so the wait and the throttle do
/// not beat against each other: one look per wake, and no wake at all while
/// the setting is off.
const DISK_POLL: std::time::Duration = std::time::Duration::from_secs(2);

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
    /// How many columns of writing are off the **left** edge, in horizontal
    /// layout. Zero whenever the rows fit, which with soft wrap on is always;
    /// with `:wrap off` a paragraph is one row of any length, and without this
    /// the page could only ever show its first screenful (Feature #221).
    left: usize,
    /// The paragraph and piece the rightmost visible 縱 sits at, in vertical
    /// layout. An anchor rather than a 縱 number: see `vertical::draw`.
    zong: Anchor,
    /// Where the grid is scrolled to, when the file is read as one.
    table: table::Viewport,
}

/// Where each work area is scrolled to (Feature #176).
///
/// Two, in **screen order**, so that switching panes moves the keys and not
/// the pages: the half you were reading stays where it is on the screen.
#[derive(Default)]
struct Seats([Viewport; 2]);

impl std::ops::Index<usize> for Seats {
    type Output = Viewport;
    fn index(&self, which: usize) -> &Viewport {
        &self.0[which.min(1)]
    }
}

impl std::ops::IndexMut<usize> for Seats {
    fn index_mut(&mut self, which: usize) -> &mut Viewport {
        &mut self.0[which.min(1)]
    }
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
    let mut child = shell_command(line)
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
    let status = shell_command(line).status();
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
    if let Some(asked) = &name {
        match yumete_config::ThemeConfig::named(asked) {
            Some(theme) => crate::theme::choose(theme),
            // …unless it is the name the reader gave their *own* theme in the
            // config, which is a theme too.
            None if asked == &config.theme.name => crate::theme::choose(config.theme.clone()),
            None => return say!("沒有這個主題：{0}", asked),
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
    say!("主題：{0}（{1}）", crate::theme::name(config), mood)
}

/// Put a picture of the screen on the clipboard (`:shot`).
///
/// The window system's job, so it is a shell line in the config rather than
/// something built in here — and it is run **without** giving up the terminal,
/// because handing the screen over is exactly what would spoil the picture.
fn take_a_picture(config: &Config) -> String {
    let line = config.editor.screenshot.trim();
    if line.is_empty() {
        return say!("這個平台上沒有截圖命令——`[editor] screenshot` 寫一條");
    }
    match shell_command(line).status() {
        Ok(status) if status.success() => say!("畫面已放進剪貼簿"),
        Ok(status) => say!("截圖失敗（{0}）", status),
        Err(err) => say!("截圖失敗（{0}）", err),
    }
}

/// A command that runs `line` through the shell.
///
/// The program and the flag come from **one** place because they are one
/// decision: `cmd.exe` wants `/C` and every Unix shell wants `-c`, and a call
/// site that got the program right and the flag wrong would run a shell that
/// sits waiting for input on a terminal the editor has taken over.
///
/// `$SHELL` on Unix, `%ComSpec%` on Windows — what the machine says it uses,
/// falling back to what it is certain to have.
fn shell_command(line: &str) -> std::process::Command {
    #[cfg(windows)]
    {
        let shell = std::env::var("ComSpec").unwrap_or_else(|_| "cmd.exe".to_string());
        let mut command = std::process::Command::new(shell);
        // `/C` and not `/c`: identical to `cmd.exe`, and it reads as a flag
        // rather than as a stray letter in the line being run.
        command.arg("/C").arg(line);
        command
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let mut command = std::process::Command::new(shell);
        command.arg("-c").arg(line);
        command
    }
}

/// `~` at the front of a path, the way a shell would read it.
///
/// The config crate's, so that a `~` in a command line and a `~` in
/// `config.toml` mean the same directory — including on Windows, where the
/// home is `%USERPROFILE%` and nobody sets `HOME`.
fn shellexpand(path: &str) -> String {
    yumete_config::expand_tilde(path)
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

/// Where the pid of a running typesetter is written.
///
/// A preview server holds a port and a few hundred megabytes, and it is killed
/// when the session ends — but only when the session ends *properly*. A panic,
/// a `SIGKILL`, a closed terminal window, and it is still there tomorrow, still
/// holding both, and nothing in the editor knows it exists. So the pid goes to
/// a file, and the next yumete to start looks.
fn server_note() -> std::path::PathBuf {
    std::env::temp_dir().join(format!("yumete-preview-{}.pid", whose()))
}

/// Which user's file this is: two people on one machine do not share a pid.
fn whose() -> String {
    std::env::var("USER").unwrap_or_else(|_| "anon".to_string())
}

/// Remember that this process is running a typesetter, and which one.
///
/// **The name goes in beside the pid.** Adoption has to check that the pid is
/// still the program we started — a pid is reused, and killing whatever
/// inherited it would be far worse than leaving a server running — and the
/// check used to compare against the literal string `tinymist`. A preview
/// server declared in a project's own config (`[language.markdown] preview`)
/// was therefore never adopted: it is exactly the orphan nobody would think to
/// look for.
///
/// **Written without following a symlink.** The path is predictable and lives
/// in a directory anyone can write to, so the plain `fs::write` would happily
/// follow a link somebody left there and truncate the file at the other end.
fn note_the_server(pid: u32, program: &str) {
    let line = format!("{pid}\t{program}");
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let opened = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(server_note());
        if let Ok(mut file) = opened {
            let _ = file.write_all(line.as_bytes());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = std::fs::write(server_note(), line);
    }
}

/// Forget it: the server has been stopped.
fn forget_the_server() {
    let _ = std::fs::remove_file(server_note());
}

/// Kill a typesetter left behind by a session that ended badly.
///
/// **Checked before it is killed**, because a pid is reused: the process must
/// still be one of ours by name. A pid file naming something else — or nothing —
/// is simply removed. Returns what it did, for the status line.
#[cfg(unix)]
fn adopt_an_orphan() -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let note = server_note();
    // Read without following a link, for the same reason it is written that
    // way: the path is predictable and the directory is public.
    let mut wrote = String::new();
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&note)
        .ok()?
        .read_to_string(&mut wrote)
        .ok()?;
    let (pid, started) = match wrote.trim().split_once('\t') {
        Some((pid, program)) => (pid, program.to_string()),
        // A note from a version that wrote the pid alone.
        None => (wrote.trim(), "tinymist".to_string()),
    };
    let pid: u32 = pid.trim().parse().ok()?;
    let named = std::process::Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "comm="])
        .output()
        .ok()?;
    let name = String::from_utf8_lossy(&named.stdout).trim().to_string();
    let _ = std::fs::remove_file(&note);
    // Still the program we started? `ps` gives a path for some programs and a
    // bare name for others, and the config may have named either.
    let started = std::path::Path::new(&started)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or(started);
    if started.is_empty() || !name.ends_with(&started) {
        return None;
    }
    // SAFETY: a pid this process wrote down, checked to still be the program
    // we started, and SIGTERM, which is the polite one.
    let killed = unsafe { libc::kill(pid as libc::pid_t, libc::SIGTERM) } == 0;
    killed.then(|| say!("上次留下的預覽伺服器（{0}）停掉了", pid))
}

#[cfg(not(unix))]
fn adopt_an_orphan() -> Option<String> {
    let _ = std::fs::remove_file(server_note());
    None
}

impl Job {
    /// Start `tinymist preview`, watching its log for the address it opens on.
    fn typst(path: &std::path::Path) -> Result<Job, String> {
        Job::server(
            "tinymist",
            &[
                "preview".to_string(),
                "--no-open".to_string(),
                path.display().to_string(),
            ],
        )
    }

    /// Start a long-running program and watch its output for an address.
    ///
    /// **No shell**, and the arguments arrive already split — see
    /// `yumete_config::Runner::argv`. A server declared in a project's config
    /// is therefore a program with arguments, never a line somebody can hide a
    /// second command in.
    fn server(program: &str, args: &[String]) -> Result<Job, String> {
        let mut child = std::process::Command::new(program)
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| say!("{0}：{1}", program, e))?;
        note_the_server(child.id(), program);
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
            what: Box::leak(program.to_string().into_boxed_str()),
            child,
            said,
        })
    }
}

/// Run what a language declares for this verb (Feature #197).
///
/// **No shell.** The line is split by [`yumete_config::Runner::argv`] and the
/// placeholders become whole arguments, so nothing in a file name can turn
/// into a second command — which is what makes it safe for a *project's* config
/// to declare these, and a project's config is where they belong: whether a
/// `.typ` is formatted by `typstfmt` is a fact about this book, not about the
/// editor.
fn run_for_language(
    editor: &mut Editor,
    config: &Config,
    want: &yumete_core::editor::LanguageRun,
) -> String {
    let Some(runner) = config
        .language
        .get(&want.language)
        .and_then(|verbs| verbs.get(&want.verb))
    else {
        return say!(
            "{0} 沒有說 {1} 要跑什麼——在設定裏寫 [language.{0}] {1} = …",
            want.language,
            want.verb
        );
    };
    let file = want.path.display().to_string();
    let Some(argv) = runner.argv(&file) else {
        return say!("命令寫壞了：{0}", runner.run);
    };
    let Some((program, args)) = argv.split_first() else {
        return say!("命令寫壞了：{0}", runner.run);
    };
    match runner.kind {
        // The buffer goes in and comes back: the file on disk is not touched,
        // so a formatter that fails cannot destroy anything, and `u` takes the
        // formatting back like any other edit.
        yumete_config::RunKind::Filter => {
            let text = editor.current_buffer().text();
            match run_program(program, args, Some(&text)) {
                Ok(ran) if ran.ok => {
                    editor.provide_pipe_output(&ran.said);
                    say!("{0}：換好了", want.verb)
                }
                Ok(ran) => say!("沒有動你的字：{0}", ran.why()),
                Err(err) => say!("跑不動：{0}", err),
            }
        }
        // It reads and rewrites the file itself, so the buffer is re-read
        // afterwards — and only when it is clean, because re-reading over
        // unsaved changes is losing them.
        yumete_config::RunKind::Once => {
            if editor.current_buffer().is_modified() {
                return say!("先存檔——外面的程序讀的是檔案");
            }
            match run_program(program, args, None) {
                Ok(ran) if ran.ok => {
                    let _ = editor.current_buffer_mut().reread();
                    say!("{0}：跑完了", want.verb)
                }
                Ok(ran) => say!("{0}：{1}", want.verb, ran.why()),
                Err(err) => say!("跑不動：{0}", err),
            }
        }
        // A server is a life of its own; `:preview` owns that path.
        yumete_config::RunKind::Server => {
            say!("{0} 是個伺服器——用 :preview 開它", want.verb)
        }
    }
}

/// Run a program **directly**, with no shell between.
fn run_program(program: &str, args: &[String], input: Option<&str>) -> io::Result<Ran> {
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(match input.is_some() {
            true => std::process::Stdio::piped(),
            false => std::process::Stdio::null(),
        })
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    if let (Some(text), Some(mut pipe)) = (input, child.stdin.take()) {
        io::Write::write_all(&mut pipe, text.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    Ok(Ran {
        ok: out.status.success(),
        said: String::from_utf8_lossy(&out.stdout).into_owned(),
        complained: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
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

/// 上屏方式, in the words the input method's own panel uses.
fn commit_name(cs: CommitStrategy) -> String {
    match cs {
        CommitStrategy::Delayed => say!("延遲（頂字）"),
        CommitStrategy::Unique => say!("唯一"),
        CommitStrategy::Fluency => say!("整句"),
    }
}

/// `:yume commit [delayed|unique|fluency]` — **when** a finished code goes to
/// the page (Feature #209).
///
/// The three are yume's own, merged in yume's own core, so what is chosen here
/// is what the same word means in the input method everywhere else. A scheme
/// that can only be typed as whole sentences — 拼音 has no 碼表 to look a
/// segment up in — says so rather than pretending the choice took.
fn commit_method(ime: &mut ImeSession, mode: &str) -> String {
    if mode.is_empty() {
        return say!("上屏方式：{0}", commit_name(ime.commit_strategy()));
    }
    let Some(cs) = CommitStrategy::from_str_tag(mode) else {
        return say!("沒有「{0}」這種上屏方式——delayed、unique、fluency", mode);
    };
    ime.set_commit_strategy(Some(cs));
    let now = ime.commit_strategy();
    if now != cs {
        return say!(
            "{0} 只能整句上屏（沒有碼表可以逐段查），上屏方式仍然是{1}",
            ime.scheme_name(),
            commit_name(now)
        );
    }
    say!("上屏方式：{0}", commit_name(now))
}

/// 候選面板: the bordered list, or the sentence itself (Feature #211).
///
/// The other axis from [`commit_method`], and independent of it: **when** a
/// word lands on the page and **where** you read the candidate are two
/// questions. `bare` is 空空如也 — yume's own front end already has a full
/// panel, and the surface a novel is written on does not need a second one
/// hanging off the caret.
fn panel_method(ime: &mut ImeSession, mode: &str) -> String {
    if mode.is_empty() {
        return say!("候選面板：{0}", panel_name(ime.panel_display()));
    }
    let Some(display) = PanelDisplay::parse(mode) else {
        return say!("沒有「{0}」這種候選面板——full、bare", mode);
    };
    ime.set_panel_display(display);
    say!("候選面板：{0}", panel_name(display))
}

/// The name a 候選面板 is called by, in the language the writer reads.
fn panel_name(display: PanelDisplay) -> String {
    match display {
        PanelDisplay::Full => say!("候選框"),
        PanelDisplay::Bare => say!("行內預覽"),
    }
}

/// The data files that are installed and doing nothing, in one clause (#220).
///
/// A file that is simply **not there** is left out: half the manifest is
/// optional and an ordinary install is missing several, so listing those would
/// bury the line that matters. What is left is 「somebody put this here and the
/// core will not have it」, which is what a binary format changing its magic
/// under an old data directory looks like from the writer's chair — and which
/// was, until this, completely silent: the 拆分 comments simply stopped
/// appearing and nothing anywhere said why.
///
/// Only the first is spelled out. One reason is enough to send somebody to
/// `scripts/build.sh`, and they are nearly always the same reason.
fn data_faults(ime: &ImeSession) -> String {
    let loud: Vec<&DataProblem> = ime.problems().iter().filter(|p| p.is_loud()).collect();
    let Some(first) = loud.first() else {
        return String::new();
    };
    let one = match &first.fault {
        DataFault::Rejected {
            reason,
            magic: Some(magic),
        } => say!(
            "{0} 核心不認：{1}（期望 {2}，檔頭是 {3}）",
            first.file,
            reason,
            magic.expected,
            magic.found
        ),
        DataFault::Rejected { reason, magic: None } => {
            say!("{0} 核心不認：{1}", first.file, reason)
        }
        DataFault::Unreadable(why) => say!("{0} 讀不了：{1}", first.file, why),
        // `is_loud` admits no others.
        _ => return String::new(),
    };
    match loud.len() {
        1 => one,
        n => say!("{0}（另有 {1} 個檔同樣沒進去）", one, n - 1),
    }
}

/// Put the candidate `bare` is offering into the text, or take it away again.
///
/// Wholesale, every frame, because [`Editor::set_ghost`] is wholesale: what is
/// on the page now is exactly what this says, so a committed candidate leaves
/// nothing behind and a cancelled one disappears without anybody remembering
/// to clear it.
///
/// **Not in a prompt.** A `/` search composes on the status line, which has no
/// page to draw into; the panel comes up there whatever this setting says.
fn settle_inline_candidate(editor: &mut Editor, ime: &ImeSession) {
    let want = inline_candidate(editor, ime);
    // A page with no candidate on it pays nothing — and must not be marked
    // dirty by a `set_ghost` that changes nothing, since both layout memos are
    // keyed on the runs.
    if want.is_empty() {
        if editor.has_ghost() {
            editor.set_ghost(Vec::new());
        }
        return;
    }
    editor.set_ghost(vec![(editor.cursor_line(), editor.cursor_column(), want)]);
}

/// The text `bare` draws into the sentence, or empty when it draws nothing.
fn inline_candidate(editor: &Editor, ime: &ImeSession) -> String {
    if ime.panel_is_full()
        || !composes(editor.mode())
        || !page_can_hold_a_candidate(editor)
        || !ime.available()
        || !ime.is_composing()
    {
        return String::new();
    }
    ime.inline_candidate()
}

/// Whether what is on screen is a page that ghost text can be drawn into.
///
/// The two gates — panel or inline — are complementary, and this is the term
/// they share, so there can be no state that draws both and none that draws
/// neither. Two things on screen are not that page:
///
/// * a **prompt**, which composes on the status line;
/// * a **grid**, which `table::draw` renders cell by cell out of the cells
///   themselves and knows nothing about ghost runs (that is #212's job).
///
/// In both, `bare` gives the panel back rather than showing nothing at all.
fn page_can_hold_a_candidate(editor: &Editor) -> bool {
    editor.prompt().is_none() && !editor.table().is_some_and(|t| t.is_grid())
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
        let head = if ime.available() {
            format!(
                "{} · 碼表 {} · 拆分 {} · 上屏 {}",
                ime.scheme_name(),
                ime.table_source(),
                if ime.annotations_enabled() { "開" } else { "關" },
                commit_name(ime.commit_strategy()),
            )
        } else if yumete_ime::has_builtin_table() {
            "還沒開始打字——`:yume scheme` 載入碼表".to_string()
        } else {
            "還沒開始打字，而且這個二進制不帶碼表——先裝資料".to_string()
        };
        // 面板 only when there is not one: a writer who sees no candidate list
        // and wonders where it went is the only one who needs telling, and the
        // line is already four clauses long (Feature #211).
        let head = match ime.panel_display() {
            PanelDisplay::Full => head,
            display => format!("{head} · {}", say!("面板 {0}", panel_name(display))),
        };
        // The one thing `:yume` could not say before #220: a file that is
        // installed and doing nothing. It goes last because it is rare, and it
        // goes here because this is the question it answers.
        let faults = data_faults(ime);
        return match faults.is_empty() {
            true => head,
            false => format!("{head} · {faults}"),
        };
    }
    if let Some(mode) = tag.strip_prefix("commit:") {
        return commit_method(ime, mode);
    }
    if let Some(mode) = tag.strip_prefix("panel:") {
        return panel_method(ime, mode);
    }
    if let Some(path) = tag.strip_prefix('=') {
        let path = std::path::PathBuf::from(shellexpand(path));
        return match ImeSession::from_table_file(&path) {
            Ok(mut table) => {
                table.set_page_size(config.panel.page_size);
                table.set_commit_strategy(ime.commit_override());
                table.set_panel_display(ime.panel_display());
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
        full.set_commit_strategy(ime.commit_override());
        full.set_panel_display(ime.panel_display());
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
        full.set_commit_strategy(ime.commit_override());
        full.set_panel_display(ime.panel_display());
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
            full.set_commit_strategy(ime.commit_override());
            full.set_annotations(ime.annotations_enabled());
            full.set_panel_display(ime.panel_display());
            let name = full.scheme_name().to_string();
            *ime = full;
            return format!("方案：{name}");
        }
        return say!(
            "{0} 的碼表沒有裝——放進 {1}，或者把自己的碼表放進 .yumete/",
            tag,
            yumete_config::data_dir().display()
        );
    }
    let was = ime.scheme();
    if ime.set_scheme(scheme) {
        return format!("方案：{}", ime.scheme_name());
    }
    // Put back what was working rather than leaving the writer unable to type.
    ime.set_scheme(was);
    say!(
        "{0} 的碼表沒有裝——放進 {1}，或者把自己的碼表放進 .yumete/",
        tag,
        yumete_config::data_dir().display()
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
        | KeyCode::PageUp
        | KeyCode::PageDown
        | KeyCode::Delete
        | KeyCode::BackTab
            if composing => {}
        // 空空如也 with a way out: `Tab` brings the whole list up for the one
        // word that needs it, and it goes away with that word (Feature #211).
        // Swallowed either way — a Tab typed mid-composition has never been an
        // indent, and under `full` there is nothing to summon.
        KeyCode::Tab if composing => {
            ime.summon_panel();
        }
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
        KeyCode::PageUp => Some(Key::PageUp),
        KeyCode::PageDown => Some(Key::PageDown),
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
    /// The two work areas, **in screen order** — `panes[0]` is the one drawn
    /// first (top, or right in 縱書). Which of them holds the keys is
    /// [`Editor::live_pane`], and it is a different question on purpose:
    /// switching panes must not make the top one jump to the bottom.
    panes: [Rect; 2],
    /// The rule between them, when there are two.
    divider: Option<Rect>,
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
    let (text, detail) = table::split_detail(editor, config, page);
    // 工作區 (Feature #176). **The cut runs across the direction the text
    // advances in**: 橫排 advances downward, so the panes are 上下; 縱書
    // advances leftward, so they are 左右 and the second takes the left. That
    // is what keeps the measure untouched — both panes keep the full width in
    // 橫排 and the full height in 縱書, so not one row rewraps and not one 縱
    // is shortened. A divider carries the boundary and the caption.
    let (panes, divider) = match editor.other_pane().is_some() {
        false => ([text, Rect::new(text.x, text.y, 0, 0)], None),
        true => match editor.layout() {
            WritingLayout::Vertical if !editor.table().is_some_and(|t| t.is_grid()) => {
                let half = text.width.saturating_sub(1) / 2;
                let rule = Rect::new(text.x + half, text.y, 1.min(text.width), text.height);
                let right = Rect::new(
                    text.x + half + 1,
                    text.y,
                    text.width.saturating_sub(half + 1),
                    text.height,
                );
                let left = Rect::new(text.x, text.y, half, text.height);
                // 縱 fill from the right edge, so the page you were reading
                // keeps the right and the new one opens to the left of it.
                ([right, left], Some(rule))
            }
            _ => {
                let half = text.height.saturating_sub(1) / 2;
                let rule = Rect::new(text.x, text.y + half, text.width, 1.min(text.height));
                let top = Rect::new(text.x, text.y, text.width, half);
                let bottom = Rect::new(
                    text.x,
                    text.y + half + 1,
                    text.width,
                    text.height.saturating_sub(half + 1),
                );
                ([top, bottom], Some(rule))
            }
        },
    };
    Areas {
        sidebar,
        tabs,
        text,
        panes,
        divider,
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
    viewport: &mut Seats,
) {
    let area = frame.area();
    let areas = page_areas(editor, config, area);
    let Areas {
        sidebar,
        panes,
        divider,
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
    // One work area, or two. The live one is drawn from the editor's own
    // cursor; the other from the place it was left at, with its hit marked —
    // it has no cursor at all, which is what keeps this feature from being
    // multi-cursor by the back door.
    let live = editor.live_pane().min(1);
    let mut cursor = (text_area.x, text_area.y);
    for (which, rect) in panes.iter().enumerate() {
        if rect.width == 0 || rect.height == 0 {
            continue;
        }
        let seat = &mut viewport[which];
        let peek = (which != live).then(|| editor.other_pane()).flatten();
        let at = match editor.layout() {
            // A grid is not prose and is not drawn as prose: no wrapping, no
            // markup, one row per line, columns that line up.
            // A `|` table lives inside a page of prose and is drawn by whatever
            // draws that page — the paragraph above it must not vanish because
            // the cursor landed in a cell.
            _ if editor.table().is_some_and(|t| t.is_grid()) => {
                table::draw(frame, editor, config, *rect, &mut seat.table, peek)
            }
            WritingLayout::Horizontal => {
                draw_horizontal(frame, editor, config, *rect, &mut seat.top, &mut seat.left, peek)
            }
            WritingLayout::Vertical => {
                vertical::draw(frame, editor, config, *rect, &mut seat.zong, peek)
            }
        };
        if which == live {
            cursor = at;
        }
    }
    if let Some(rule) = divider {
        draw_divider(frame, editor, config, rule);
    }
    let (cursor_x, cursor_y) = cursor;

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
    // One panel for every half-pressed sequence, `空格` included — it used to
    // draw its own menu and every other prefix got a row.
    // **The page's rectangle, not the frame's**: a menu drawn from the frame
    // covers the sidebar, which is a list the reader may be in the middle of
    // using.
    draw_which_key(frame, editor, config, text_area, footer.y, cursor_x);
    // …and the same string beside the caret, where the eyes are.
    draw_hud(frame, editor, config, ime, text_area, (cursor_x, cursor_y));

    // In vertical layout the cursor is a block drawn into the page: a hardware
    // cursor is one cell wide and would sit lopsided inside a two-cell 縱.
    if editor.picker().is_some() {
        // `draw_picker` put the caret in its query, which is the prompt while a
        // picker is open.
    } else if let Some((_, _)) = editor.prompt() {
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

    // `bare` draws no panel — the candidate is already in the sentence and the
    // code is under the caret. Unless there is no sentence to draw it into:
    // see `page_can_hold_a_candidate`.
    let panel = ime.panel_is_full() || !page_can_hold_a_candidate(editor);
    if panel && composes(editor.mode()) && ime.available() && ime.is_composing() {
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

/// What the mark under the caret says: a half-typed command, or a half-typed
/// code (Feature #211).
///
/// Two callers with one answer — the HUD row and the status line's right edge —
/// so the two surfaces can never disagree about what is being typed.
///
/// They cannot both be true at once: `typed_so_far` is Normal mode's, and a
/// composition is Insert's. In `full` the code is the panel's first row and
/// this stays empty, which is why the mark is not simply always drawn — in
/// Normal, while prose is being typed, nothing may flicker beside the
/// characters.
fn hud_line(editor: &Editor, ime: &ImeSession) -> String {
    if editor.prompt().is_some() {
        return String::new();
    }
    if composes(editor.mode()) && ime.available() && ime.is_composing() && !ime.panel_is_full() {
        // 空空如也 leaves the code nowhere else to be: the candidate is in the
        // sentence, and what was typed to get it is not.
        return ime.display_buffer();
    }
    match editor.mode() {
        Mode::Normal => editor.typed_so_far(),
        _ => String::new(),
    }
}

/// The **HUD**: what you have typed, beside the caret.
///
/// The status line's right edge is where vi has always put this, and it is
/// twenty rows from where the eyes are. So it is drawn twice: there, and here,
/// one row under the caret — small, 金 on the band, and gone the moment the
/// command completes. What it says is [`hud_line`]'s to decide.
fn draw_hud(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    ime: &ImeSession,
    page: Rect,
    caret: (u16, u16),
) {
    let typed = hud_line(editor, ime);
    if typed.is_empty() || page.height < 2 {
        return;
    }
    let ink = crate::theme::Palette::of(config);
    let text = format!("╰ {typed}");
    let width = yumete_cjk::str_width(&text) as u16;
    let (caret_x, caret_y) = caret;
    if width >= page.width {
        return;
    }
    let right = page.x + page.width;
    // **Never over the writing.** It used to start at the caret's own column
    // and paint over whatever was on the row below — 整整 covered by `╰ 30`,
    // and in 縱書 over a live 縱, with the leading `╰` swallowed by a wide
    // glyph's second cell. So it goes *after* what is drawn on that row: the
    // margin is the only part of a page that is not somebody's writing.
    let after_the_writing = |frame: &mut Frame, y: u16| -> u16 {
        let buf = frame.buffer_mut();
        let mut last = page.x;
        for x in page.x..page.x + page.width {
            let Some(cell) = buf.cell((x, y)) else { continue };
            let symbol = cell.symbol();
            if symbol.trim().is_empty() {
                continue;
            }
            // **Past the whole glyph.** A wide character's second cell reads
            // back empty, and writing into it is writing into the middle of a
            // 漢字: the terminal never receives it, so the mark simply vanishes.
            last = x + yumete_cjk::str_width(symbol).max(1) as u16;
        }
        last
    };
    // **Under the caret, then over it** — and never *on* it, so the character
    // being worked on stays visible. Both rows are tried rather than only the
    // one: a full row below used to make the HUD vanish, when the row above
    // was empty margin. (The status line's right edge carries the same string
    // whatever happens here, so a HUD with nowhere to go loses nothing.)
    let below = (caret_y + 1 < page.y + page.height).then_some(caret_y + 1);
    let above = (caret_y > page.y).then(|| caret_y - 1);
    let mut placed = None;
    for y in below.into_iter().chain(above) {
        let after = after_the_writing(frame, y);
        let x = caret_x.max(after).min(right.saturating_sub(width));
        if x >= after && x + width <= right {
            placed = Some((x, y));
            break;
        }
    }
    let Some((x, y)) = placed else {
        return;
    };
    let style = Style::default()
        .bg(ink.at(yumete_config::rung::BAND))
        .fg(ink.gold());
    put_text(frame.buffer_mut(), x, y, right, &text, style);
}

/// The **which-key panel**: what the half-pressed key can be finished with.
///
/// A bordered list, titled in its own top border, in the corner the writing
/// ends at — bottom right in 橫排, bottom left in 縱書, because that is where
/// the eye already is when a line runs out. It replaces the hint row for the
/// sequence it is about: two surfaces saying the same thing is the thing this
/// editor keeps taking apart.
///
/// One column while it fits, two when it does not. Not scrolling: a menu you
/// have to scroll is one you cannot answer at a glance, which is the whole of
/// what it is for.
fn draw_which_key(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    area: Rect,
    bottom: u16,
    caret_x: u16,
) {
    let Some((title, keys)) = editor.pending_menu() else {
        return;
    };
    if keys.is_empty() {
        return;
    }
    let ink = crate::theme::Palette::of(config);
    // The keys line up, so the meanings do: a ragged left edge on a list of
    // two-character keys reads as noise.
    let key_width = keys
        .iter()
        .map(|(k, _)| yumete_cjk::str_width(k))
        .max()
        .unwrap_or(1);
    let rows: Vec<(String, String)> = keys
        .iter()
        .map(|(k, what)| {
            let pad = " ".repeat(key_width.saturating_sub(yumete_cjk::str_width(k)));
            (format!("{k}{pad}"), what.clone())
        })
        .collect();
    let one = rows
        .iter()
        .map(|(k, what)| yumete_cjk::str_width(k) + 2 + yumete_cjk::str_width(what))
        .max()
        .unwrap_or(0);

    // **Half the page, and half the width.** A menu is a thing you glance at
    // beside your writing: one that fills the window has stopped being a menu,
    // and one wider than half the page cannot dodge the caret — it covers the
    // corner it was trying to avoid either way.
    let room = (area.height.saturating_sub(2) / 2).max(1) as usize;
    let across = if rows.len() > room { 2 } else { 1 };
    let deep = rows.len().div_ceil(across);
    let widest = (area.width as usize).saturating_sub(2);
    // Each column gets its share, and what does not fit is cut *inside* the
    // column rather than beyond the border — where it used to be dropped
    // silently, leaving keys with no meanings beside them.
    let one = one.min(widest.saturating_sub((across - 1) * 2) / across.max(1));
    let inner = one * across + (across - 1) * 2;
    let width = (inner + 2)
        .max(yumete_cjk::str_width(&title) + 4)
        .min(area.width as usize) as u16;
    let height = (deep + 2) as u16;
    // It may take half the page's height and no more, and it must leave the
    // page something: at a very small window there is nowhere to put a menu,
    // and covering the manuscript with one is worse than not drawing it.
    if height > area.height / 2 + 1 || bottom < height || width < 8 {
        return;
    }
    // **The corner the cursor is not in.** A fixed corner is right half the
    // time and covers what you are working on the other half; the panel goes to
    // whichever side of the page the caret is not on. One rule for both
    // layouts, because in both of them the caret has a column.
    let far = area.x + area.width.saturating_sub(width);
    let x = match caret_x >= area.x + area.width / 2 {
        true => area.x,
        false => far,
    };
    let panel = Rect::new(x, bottom - height, width, height);
    frame.render_widget(Clear, panel);
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .border_type(if config.panel.rounded {
                BorderType::Rounded
            } else {
                BorderType::Plain
            })
            // `rule()`, the rung every other ring on the screen is drawn at.
            .border_style(Style::default().fg(ink.rule()).bg(ink.paper()))
            .title(Span::styled(
                title,
                Style::default().fg(ink.gold()).bg(ink.paper()),
            ))
            .style(Style::default().bg(ink.paper())),
        panel,
    );
    let ground = Style::default().bg(ink.paper());
    let buf = frame.buffer_mut();
    for (i, (key, what)) in rows.iter().enumerate() {
        let (column, row) = (i / deep, i % deep);
        let x = panel.x + 1 + (column * (one + 2)) as u16;
        let y = panel.y + 1 + row as u16;
        let limit = panel.x + width - 1;
        put_text(buf, x, y, limit, key, ground.fg(ink.gold()));
        put_text(
            buf,
            x + key_width as u16 + 2,
            y,
            limit,
            what,
            ground.fg(ink.text()),
        );
    }
}

/// Write `text` from `x`, stopping at `limit`, one cell per column./// Write `text` from `x`, stopping at `limit`, one cell per column.
/// A drawn buffer as one HTML `<pre>`: a span per run of same-styled cells.
fn buffer_to_html(buffer: &ratatui::buffer::Buffer) -> String {
    use ratatui::style::Color;
    // The terminal's own default, for a cell that names no colour of its own.
    // A picture has no terminal, so it says what the theme's page says.
    let hex = |c: Option<Color>, fallback: &str| -> String {
        match c {
            Some(Color::Rgb(r, g, b)) => format!("#{r:02X}{g:02X}{b:02X}"),
            Some(Color::Indexed(i)) => format!("var(--ansi-{i}, {fallback})"),
            _ => fallback.to_string(),
        }
    };
    let escape = |s: &str| -> String {
        s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
    };
    let ground = hex(buffer[(0, buffer.area.height.saturating_sub(1))].style().bg, "#111111");
    let mut out = format!("<pre class=\"yumete-shot\" style=\"background:{ground}\">");
    for y in 0..buffer.area.height {
        let mut x = 0;
        while x < buffer.area.width {
            let cell = &buffer[(x, y)];
            let style = cell.style();
            let fg = hex(style.fg, "#DDDDDD");
            let bg = hex(style.bg, &ground);
            let bold = style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD);
            let reversed = style
                .add_modifier
                .contains(ratatui::style::Modifier::REVERSED);
            let (fg, bg) = match reversed {
                true => (bg, fg),
                false => (fg, bg),
            };
            // One span per *run* of cells that look the same, or a page of
            // Chinese prose is three thousand spans.
            let mut run = String::new();
            while x < buffer.area.width {
                let here = &buffer[(x, y)];
                let same = here.style() == style;
                if !same {
                    break;
                }
                run.push_str(here.symbol());
                x += yumete_cjk::drawn_width(here.symbol()).max(1) as u16;
            }
            let weight = if bold { ";font-weight:600" } else { "" };
            out.push_str(&format!(
                "<span style=\"color:{fg};background:{bg}{weight}\">{}</span>",
                escape(&run)
            ));
        }
        out.push('\n');
    }
    out.push_str("</pre>");
    out
}

/// `text` with the characters a terminal would *obey* taken out.
///
/// **A manuscript is data, not instructions.** A file can contain `\x1b]0;…\x07`
/// as easily as it can contain 「他抬頭」, and every one of those bytes used to
/// be handed to the terminal verbatim: a title bar rewritten by opening a file,
/// text hidden behind an SGR run, and on a terminal with a permissive OSC 52
/// handler, a clipboard written by a page that was only ever *displayed*.
///
/// Removing them costs nothing on the page, which is why this is the whole fix:
/// a control character measures zero cells ([`yumete_cjk::char_width`]), so
/// every column, caret, wrap and click map already places the row as though it
/// were not there. Newlines never reach a row, and tabs are expanded upstream.
///
/// Borrowed when the text is clean, which is every row of every real
/// manuscript.
fn drawable(text: &str) -> std::borrow::Cow<'_, str> {
    match text.contains(|c: char| c.is_control()) {
        false => std::borrow::Cow::Borrowed(text),
        true => std::borrow::Cow::Owned(text.chars().filter(|c| !c.is_control()).collect()),
    }
}

fn put_text(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    limit: u16,
    text: &str,
    style: Style,
) {
    let mut at = x;
    let text = drawable(text);
    for g in yumete_cjk::graphemes(&text) {
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
    // …and what it is waiting for, when it is waiting for something. A
    // prerequisite belongs *here*, before the command is run: the writer who
    // typed `:hanging` on a horizontal page found out by pressing Enter and
    // watching nothing happen.
    let unmet: Vec<&str> = editor
        .unmet_needs(matches[focus].needs)
        .iter()
        .map(|need| need.says())
        .collect();
    let footer = match unmet.is_empty() {
        true => format!(
            "{}/{}  {}",
            focus + 1,
            matches.len(),
            yumete_core::messages::say(matches[focus].help, &[])
        ),
        false => format!(
            "{}/{}  {}  ⟨{}⟩",
            focus + 1,
            matches.len(),
            yumete_core::messages::say(matches[focus].help, &[]),
            say!("需要 {0}，句末加 force", unmet.join(&say!("、")))
        ),
    };
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
    seats: &Seats,
    mouse: ratatui::crossterm::event::MouseEvent,
) -> Option<usize> {
    let size = size?;
    // The very rectangle the page was drawn into — hint row, tab bar, sidebar
    // and detail panel all already taken off. Working it out again by hand is
    // how a click came to land a row or two from where it was pointed.
    // …and the work area it landed in, which with two of them is the one
    // question a click has to answer before any of the others.
    let areas = page_areas(editor, config, Rect::new(0, 0, size.width, size.height));
    let live = editor.live_pane().min(1);
    let area = areas.panes[live];
    if mouse.column < area.x
        || mouse.column >= area.x + area.width
        || mouse.row < area.y
        || mouse.row >= area.y + area.height
    {
        return None;
    }
    let viewport = &seats[live];
    // **A grid is drawn by the grid.** Its columns are padded to line up while
    // the file behind them is ragged, and it scrolls sideways by whole columns
    // — so resolving a click as if the page were prose landed it somewhere
    // else on every table, off by the padding of every column to the left.
    if editor.table().is_some_and(|t| t.is_grid()) {
        return table::char_at(editor, config, area, &viewport.table, mouse);
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
            let ghost = |line: usize| editor.ghost_on_line(line);
            let measure = wrap::Measure::new(width, &hide)
                .with_indent(editor.paragraph_indent())
                .with_folds(&fold)
                .with_ghost(&ghost)
                .with_open_line(editor.open_line());
            // The same walk the page was drawn with: a row with a reading
            // over it takes two screen rows, so counting rows from the top
            // would land a click one row low for every reading above it.
            let want_row = (mouse.row - area.y) as usize;
            let (_, row) = rows_on_screen(
                editor,
                buffer.rope(),
                measure,
                viewport.top,
                // One row further, or the row *below* a reading — the row the
                // reading belongs to — is never in the list, and the click was
                // dropped.
                want_row + 2,
            )
            .into_iter()
            // A click on a reading belongs to the row it reads: that is the
            // 字 the reader was pointing at.
            .find(|(y, _)| *y == want_row || *y == want_row + 1)?;
            // Which character of that row the column landed on, counting only
            // what is drawn — hidden markup takes no columns.
            let want = (mouse.column - area.x) as usize;
            // …plus what is off the left edge, so a click on a long line that
            // the page has scrolled sideways to follow lands on the 字 under
            // the pointer and not on one a screenful back (Feature #221). A
            // click in the gutter means the first character on the page.
            let goal = want.saturating_sub(gutter) + viewport.left;
            let hidden = editor.hidden_on_line(row.line);
            let ghosts = editor.ghost_on_line(row.line);
            let line_start = buffer.rope().line_to_char(row.line);
            // The row starts where it was drawn: a click anywhere in a
            // paragraph's opening indent means its first character.
            let mut column =
                measure.indent_of(row.line, &buffer.rope().line(row.line).to_string(), row.index_in_line);
            // **A click on ghost text means the character it stands before.**
            // The cells are on the page but not in the file, so they are the
            // one thing a click cannot land *in*.
            let ghost_width = |at: usize| -> usize {
                ghosts
                    .iter()
                    .filter(|&&(g, _)| g == at)
                    .map(|(_, text)| yumete_cjk::str_width(text))
                    .sum()
            };
            for at in row.start..row.end {
                let c = buffer.rope().char(at);
                let g = ghost_width(at - line_start);
                if goal < column + g {
                    return Some(at);
                }
                column += g;
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
    // A sidebar narrower than three cells cannot be drawn — and the drawing
    // used to *return* at that width, leaving the rectangle it had been given
    // unpainted: a black stripe down a light page, for the third time. Below
    // three cells there is no sidebar, so no rectangle is handed out.
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
    match (want as u16).min(total.saturating_sub(8)) {
        got if got < 3 => 0,
        got => got,
    }
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
    // The caret sits in the query, which is typed text like any other prompt.
    // The footer is `title  n/total  query`, so the query begins as far in as
    // everything before it is wide.
    let before = footer.len() - picker.query().len();
    let col = yumete_cjk::str_width(&footer[..before])
        + yumete_cjk::str_width(&picker.before_caret());
    frame.set_cursor_position(Position::new(
        status.x + 1 + col as u16,
        status.y,
    ));
}

/// The composition in progress, when a `/` or `:` prompt is open.
fn prompt_preedit(editor: &Editor, ime: &ImeSession) -> String {
    if editor.prompt().is_some() && ime.available() && ime.is_composing() {
        ime.display_buffer()
    } else {
        String::new()
    }
}

/// A drawn row with `left` columns of **writing** taken off its left edge.
///
/// The gutter is furniture and does not scroll: line numbers that slid away
/// with the text would leave the reader with no way to say where they are. So
/// the cut is made between the two — the first `gutter` columns are kept, the
/// next `left` are dropped, and the rest moves over.
///
/// A 漢字 the cut lands in the middle of becomes air: half a 字 is not a
/// character the terminal can draw, and drawing the whole one would put a
/// column of writing where the reader is owed none.
fn scrolled(line: Line<'static>, gutter: usize, left: usize) -> Line<'static> {
    if left == 0 {
        return line;
    }
    let cut = gutter + left;
    let mut out: Vec<Span<'static>> = Vec::new();
    let mut column = 0usize;
    let style = line.style;
    for span in line.spans {
        let span_style = span.style;
        let mut text = String::new();
        for ch in span.content.chars() {
            let end = column + yumete_cjk::char_width(ch);
            if end <= gutter || column >= cut {
                text.push(ch);
            } else if column < gutter {
                // A character straddling the gutter's edge: only the part of it
                // that is furniture survives.
                text.push_str(&" ".repeat(gutter - column));
            } else if end > cut {
                text.push_str(&" ".repeat(end - cut));
            }
            column = end;
        }
        if !text.is_empty() {
            out.push(Span::styled(text, span_style));
        }
    }
    Line::from(out).style(style)
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
    left: &mut usize,
    peek: Option<&yumete_core::editor::Pane>,
) -> (u16, u16) {
    let buffer = editor.current_buffer();
    let total_lines = buffer.line_count();
    let height = text_area.height as usize;
    let mode = config.editor.line_numbers;
    let gutter = gutter_width(total_lines, mode);
    let rope = buffer.rope();
    // The half that is only being read is drawn a rung back, all of it — that
    // is how you can see which half the keys are in without looking for the
    // cursor.
    let ink = match peek {
        None => crate::theme::Palette::of(config),
        Some(_) => crate::theme::Palette::of(config).faded(),
    };

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
    let ghost = |line: usize| editor.ghost_on_line(line);
    let measure = wrap::Measure::new(width, &hide)
        .with_indent(editor.paragraph_indent())
        .with_folds(&fold)
        .with_ghost(&ghost)
        .with_open_line(editor.open_line());

    // A pane that is only being read has no cursor: it is drawn from the
    // place it was left at, and *that* is what the page is scrolled around.
    let at = peek.map_or_else(|| editor.cursor(), |pane| pane.cursor());
    let cursor_line = rope.char_to_line(at.min(rope.len_chars()));
    let cursor_pos = wrap::position(rope, at, measure);
    let cursor_anchor = WrapAnchor::from(cursor_pos);

    // …and sideways, by the same promise: **the caret is on the page**. With
    // soft wrap on this never moves — a row is folded before it can reach the
    // right edge, so the column is always inside the window and `left` settles
    // back to zero. With `:wrap off` a paragraph is one row of whatever length
    // it happens to be, and until this was here the window showed its first
    // screenful and nothing else: `gl` walked the cursor off the right edge and
    // the writing it landed in was never drawn (Feature #221).
    //
    // Scrolled by the single column that is needed and no more — no
    // half-screen jump — because a caret walking one 字 at a time along a long
    // line should not make the whole page slide out from under the reader.
    let page = (text_area.width as usize).saturating_sub(gutter).max(1);
    if cursor_pos.column < *left {
        *left = cursor_pos.column;
    } else if cursor_pos.column >= *left + page {
        *left = cursor_pos.column + 1 - page;
    }
    let left = *left;

    // Scroll so the cursor's row stays on the page with `scrolloff` rows of
    // context above and below. Counted from the page's own anchor rather than
    // from a global row number, which cannot be found without walking the
    // document from the top on every keystroke.
    let scrolloff = config.editor.scrolloff.min(height.saturating_sub(1) / 2);
    let last_row = height.saturating_sub(1);
    // **A jump lands in the middle, wherever it came from.** Whether the page
    // has to scroll at all is the wrong question to key this on: a hit two rows
    // below the bottom edge and a hit two rows above it are the same act, and
    // one of them used to land on the fourth row from the top while the other
    // landed in the middle. The editor knows which moves are jumps — it is the
    // same answer `C-o` is built on.
    // **Where the cursor sits on a page is the editor's answer** — one rule,
    // asked by this page, by 縱書 and by the grid. A jump lands in the middle
    // (a search hit, `gg`, a mark), a step off an edge nudges by `scrolloff`,
    // and typewriter mode keeps the row being written in the middle whatever
    // it was.
    let found = wrap::distance(rope, *viewport, cursor_anchor, measure, last_row);
    let cursor_row = match editor.page_inset(found, last_row, scrolloff) {
        None => found.unwrap_or(0),
        Some(inset) => {
            // `k` at the top edge is one row away, not a jump: it nudges.
            let inset = match found.is_none()
                && wrap::distance(rope, cursor_anchor, *viewport, measure, scrolloff + 1).is_some()
                && !editor.typewriter()
                && !editor.jumped()
            {
                true => scrolloff,
                false => inset,
            };
            *viewport = wrap::retreat(rope, cursor_anchor, measure, inset);
            wrap::distance(rope, *viewport, cursor_anchor, measure, height).unwrap_or(0)
        }
    };

    // …and the readings push it further down: a row with one over it takes
    // two screen rows, so the cursor can be on the page by row count and off
    // it by *screen* row. Walked rather than guessed, and one row at a time,
    // because how many readings there are is a property of what is on screen.
    let mut cursor_row = cursor_row;
    while cursor_row > 0 {
        let above = wrap::rows_from(rope, *viewport, measure, cursor_row + 1)
            .iter()
            .filter(|row| row_has_reading(editor, rope, row))
            .count();
        if cursor_row + above < height {
            break;
        }
        *viewport = wrap::advance(rope, *viewport, measure, 1);
        cursor_row -= 1;
    }

    // What is marked: the live pane's selection, or the hit the other one was
    // opened to show.
    let (sel_start, sel_end) = match peek {
        None => editor.selection(),
        Some(pane) => pane.highlight.unwrap_or((at, at)),
    };
    // **The one you are standing on**, told apart from every other mark on the
    // page. A hit in the ordinary selection ground is easy to lose in a long
    // line of 漢字, so it gets 朱 washed to a highlighter's ground — the one
    // colour on the page that is not a quantity of ink — and the row it sits
    // on carries a band, the way a table bands the row the cursor is in.
    let hit = editor.current_hit().filter(|_| {
        peek.is_some_and(|pane| pane.highlight.is_some()) || peek.is_none()
    });
    let hit_line = hit.map(|(from, _)| rope.char_to_line(from.min(rope.len_chars())));
    let hit_style = Style::default().bg(ink.wash());
    // Asked of the editor, not of the range: the selection always covers the
    // cursor's own grapheme, so a bare cursor would otherwise be drawn as a
    // one-character highlight and the word-tint overlay would never appear.
    let has_selection = match peek {
        None => editor.has_selection(),
        Some(pane) => pane.highlight.is_some(),
    };
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

    let readings_above = wrap::rows_from(rope, *viewport, measure, cursor_row + 1)
        .iter()
        .filter(|row| row_has_reading(editor, rope, row))
        .count();
    let mut lines: Vec<Line> = Vec::new();
    for (_, row) in rows_on_screen(editor, rope, measure, *viewport, height) {
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
            let band = match editor.number_fill() {
                true => ink.ground(yumete_config::rung::HEAD),
                false => ink.page(),
            };
            // 朱 on the number of the row the current hit is on. **The number
            // is kept** — an arrow in its place would take away the one thing
            // the gutter is for, which is saying *which* line; a coloured
            // number says both at once and costs no column.
            let band = match hit_line == Some(row.line) && row.starts_line() {
                true => band.fg(ink.mark()).add_modifier(Modifier::BOLD),
                false => band.fg(ink.furniture()),
            };
            spans.push(Span::styled(label, band));
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
        // The row the current hit is on, banded — 「在哪一行」 answered before
        // you have found the word itself.
        let ground = match hit_line == Some(row.line) {
            true => ground.patch(ink.ground(yumete_config::rung::BAND)),
            false => ground,
        };
        // 所見即所得: the markup comes off the page. It is dropped from what is
        // *drawn*, not from the buffer — and never on the construct the cursor
        // is in, so the cursor is never inside text that is not on the screen.
        let hide = editor.hidden_on_line(row.line);
        let ghost_runs = editor.ghost_on_line(row.line);
        let start_in_line = row.start - rope.line_to_char(row.line);
        let shown: Vec<bool> = (0..chars.len())
            .map(|i| {
                let at = start_in_line + i;
                !hide.iter().any(|&(a, b)| at >= a && at < b)
            })
            .collect();
        // **Text on the page the file has no bytes for** (Feature #210): the
        // candidate being typed, a table's padding. Anchored at a column and
        // drawn before the character there, which is exactly where the measure
        // charged it — so the caret, `j` and the mouse land on the same cells.
        let ghosts: Vec<(usize, String)> = ghost_runs
            .iter()
            .filter(|&&(at, _)| {
                at >= start_in_line
                    && (at < start_in_line + chars.len()
                        || (row.ends_line && at == start_in_line + chars.len()))
            })
            .map(|(at, text)| (at - start_in_line, text.clone()))
            .collect();
        // A rung back from the writing, the way a reading is set: it is *about*
        // the text and is not in it, and ghost text in the text's own ink reads
        // as something that has already been written.
        let ghost_style = ground.fg(ink.quiet());

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

        // The reading goes above the row it reads — pushed after the row's
        // own spans are styled, because it is placed by the columns the row
        // is actually drawn in.
        let drawn = Drawn {
            chars: &chars,
            shown: &shown,
            ghosts: &ghosts,
        };
        if let Some(reading) = reading_line(editor, ink, rope, &row, drawn, gutter + indent) {
            lines.push(scrolled(reading, gutter, left));
        }

        // …and the hit itself, over everything else on the row.
        if let Some((from, to)) = hit {
            if to > row.start && from < row.end {
                let a = from.saturating_sub(row.start).min(chars.len());
                let b = (to.saturating_sub(row.start)).min(chars.len());
                for style in styles.iter_mut().take(b).skip(a) {
                    *style = style.patch(hit_style);
                }
            }
        }

        // Coalesce the per-character styles into as few spans as the row needs,
        // leaving out what is not on the page.
        let mut at = 0;
        let mut gi = 0;
        loop {
            while gi < ghosts.len() && ghosts[gi].0 <= at {
                spans.push(Span::styled(ghosts[gi].1.clone(), ghost_style));
                gi += 1;
            }
            if at >= chars.len() {
                break;
            }
            let style = styles[at];
            let mut to = at + 1;
            while to < chars.len()
                && styles[to] == style
                && shown[to] == shown[at]
                && ghosts.get(gi).is_none_or(|&(g, _)| g != to)
            {
                to += 1;
            }
            if shown[at] {
                let text: String = chars[at..to].iter().filter(|c| !c.is_control()).collect();
                spans.push(Span::styled(text, style));
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
            // A whole width of spaces, and the page truncates them. Working out
            // where the row ends and padding *exactly* the rest was one width
            // question too many: it asked `char_width`, while the columns are
            // dealt out by ratatui, and the two disagree about every East-Asian
            // ambiguous character (`—` `…` `▓`). One `——` in a fenced line and
            // the ground stopped two cells short of the edge. Over-filling
            // cannot be wrong: nothing is drawn past the area.
            // …plus what the sideways scroll will cut off the front of it, so a
            // scrolled row is still painted to the right edge.
            spans.push(Span::styled(
                " ".repeat(text_area.width as usize + left),
                ground,
            ));
        }
        lines.push(scrolled(Line::from(spans), gutter, left));
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
        let edge = text_area.x + (gutter + ruler.saturating_sub(left)) as u16;
        let buf = frame.buffer_mut();
        for y in text_area.y..text_area.y + text_area.height {
            for x in edge..text_area.x + text_area.width {
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
        let x = text_area.x + (gutter + ruler) as u16 - left.min(ruler) as u16;
        if ruler >= left && x < text_area.x + text_area.width {
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

    // The caret sits where the writing is, not where the source is — and
    // `wrap::position` is where that is decided, for the caret and for `j`
    // alike. It used to be worked out **again** here, subtracting the hidden
    // width from a column that had been measured over the source: two
    // derivations of one rule, agreeing about the caret and disagreeing about
    // every motion, which is exactly the shape this editor keeps getting wrong.
    // Clamped to the page: a caret resting past a row that exactly fills the
    // width would otherwise be drawn in the column after the last one.
    let x = (gutter + cursor_pos.column - left)
        .min(text_area.width.saturating_sub(1) as usize);
    (
        text_area.x + x as u16,
        // …in **screen** rows: a reading takes a row of its own above the row
        // it reads, so a caret placed by wrap-row index sat one row high for
        // every reading above it — and in Insert that is the terminal's own
        // cursor, so the characters appeared on a different row from the bar.
        text_area.y + (cursor_row + readings_above).min(last_row) as u16,
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
        // Locked, and said so standing (Feature #213). A writer who cannot type
        // needs to know that from the screen and not from the status line's
        // memory of a refusal three keystrokes ago.
        let locked = if buffer.is_readonly() { " [只讀]" } else { "" };
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
        // A running typesetter is a **process**, holding a port and a few
        // hundred megabytes for as long as it runs. It said its address once,
        // hours ago, and nothing since — so it gets a standing mark, the way a
        // recovered draft does.
        let preview = match editor.preview_at().is_some() {
            true => " [preview]",
            false => "",
        };
        let left = format!(
            "-- {} --  {}{}{}{}{}{}{}",
            editor.mode_label(),
            ime_tag,
            buffer.display_name(),
            dirty,
            locked,
            draft,
            preview,
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
    // **What has been typed so far**, where vi has put it since 1976 and where
    // Helix puts it: the far right of the status line. Pressing `3` used to
    // change nothing on the screen at all, so `30d` and `3d` were told apart by
    // memory. It takes the place of the character readout while a command is
    // half-typed — that readout is about the character you are standing on, and
    // right now you are in the middle of saying something.
    // The same string the HUD carries — and the reason the HUD may give up on
    // finding a row: whatever happens beside the caret, it is also here.
    let typed = hud_line(editor, ime);
    let right = match typed.is_empty() {
        true => char_info(editor, config),
        false => (typed, String::new()),
    };
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

/// Whether `row` has any reading over it — which costs it a screen row.
fn row_has_reading(editor: &Editor, rope: &yumete_core::Rope, row: &wrap::Row) -> bool {
    // **疏排 (`:dense off`) on the horizontal page** is line spacing: a row of
    // air above every row. 密排 is a 縱書 word for the same thing — there it is
    // the gap between columns — and this is the other axis of it.
    //
    // It is the row a reading lives in, which is why it is *this* function:
    // the page already knows how to give a row two screen rows, and a page set
    // loose has that row whether or not anything is written in it.
    if editor.loose_rows() && editor.layout() == WritingLayout::Horizontal {
        return true;
    }
    let groups = editor.readings_on_line(row.line);
    if groups.is_empty() {
        return false;
    }
    let start = row.start - rope.line_to_char(row.line);
    let end = start + (row.end - row.start);
    groups.iter().any(|g| g.base.1 > start && g.base.0 < end)
}

/// The rows a page of `height` screen rows holds, and where each is drawn.
///
/// **One walk, asked by three** — the drawing, the mouse, and the scroll —
/// because a row with a reading over it takes two screen rows and the three
/// would otherwise disagree about which row a given line of the terminal is.
fn rows_on_screen(
    editor: &Editor,
    rope: &yumete_core::Rope,
    measure: wrap::Measure,
    top: wrap::Anchor,
    height: usize,
) -> Vec<(usize, wrap::Row)> {
    let mut out = Vec::with_capacity(height);
    let mut y = 0usize;
    for row in wrap::rows_from(rope, top, measure, height) {
        // The reading sits *above* its base, so the row it belongs to moves
        // down one — and a row whose reading would be the last thing on the
        // page is not drawn at all, rather than drawn without it.
        y += usize::from(row_has_reading(editor, rope, &row));
        if y >= height {
            break;
        }
        out.push((y, row));
        y += 1;
    }
    out
}

/// One row as it is **drawn**: the characters on it, which of them reach the
/// page at all, and the text standing between them that the file has no bytes
/// for.
///
/// Three answers to one question — which cell does each thing go in — so they
/// travel together. Anything placed by column rather than by character (a
/// reading, a rule, a highlight) needs all three or it lands somewhere else.
#[derive(Clone, Copy)]
struct Drawn<'a> {
    chars: &'a [char],
    shown: &'a [bool],
    ghosts: &'a [(usize, String)],
}

/// The readings over one row, as the line that is drawn above it.
///
/// Placed by *column*, not by character: a reading belongs over the base it
/// reads, and the base may be anywhere along the row once the markup that
/// wrote it has come off the page. Two readings that would collide are not
/// squeezed — the second is left out, because a reading over the wrong 字 is
/// worse than no reading at all.
fn reading_line(
    editor: &Editor,
    ink: crate::theme::Palette,
    rope: &yumete_core::Rope,
    row: &wrap::Row,
    drawn: Drawn,
    lead: usize,
) -> Option<Line<'static>> {
    let Drawn {
        chars,
        shown,
        ghosts,
    } = drawn;
    let groups = editor.readings_on_line(row.line);
    if groups.is_empty() {
        // 疏排: the row of air itself. Painted rather than skipped, so the page
        // keeps its ground.
        return editor
            .loose_rows()
            .then(|| Line::from(Span::styled("", ink.page())));
    }
    let line_start = rope.line_to_char(row.line);
    let start_in_line = row.start - line_start;
    let text: Vec<char> = yumete_core::zong::line_chars(rope, row.line);
    // Where each of the row's characters is drawn, in cells from the left edge
    // of the page — the gutter and the paragraph's indent included, so the
    // reading lands over its own 字 and not two cells to the left of it.
    let mut column = Vec::with_capacity(chars.len() + 1);
    let mut at = lead;
    // Ghost text takes cells on the row like anything else, so a reading over a
    // base after it belongs that much further right.
    let ghost_before = |i: usize| -> usize {
        ghosts
            .iter()
            .filter(|&&(g, _)| g == i)
            .map(|(_, text)| yumete_cjk::str_width(text))
            .sum()
    };
    for (i, &c) in chars.iter().enumerate() {
        at += ghost_before(i);
        column.push(at);
        if shown[i] {
            at += yumete_cjk::char_width(c);
        }
    }
    at += ghost_before(chars.len());
    column.push(at);
    let mut out = String::new();
    let mut col = 0usize;
    for group in &groups {
        // **The row the base *begins* on owns the reading.** A group whose base
        // starts before this row is a group the wrap cut in half, and drawing
        // it again here put a second, complete copy of the reading over the
        // tail — 「上海」 broken across two rows, `zaonhe` written above both.
        // One reading, over the row the word starts on.
        if group.base.0 < start_in_line || group.base.0 >= start_in_line + chars.len() {
            continue;
        }
        let i = group.base.0.saturating_sub(start_in_line);
        let Some(&want) = column.get(i) else { continue };
        // **A reading wider than its base runs on past it**, and the next one
        // then wants to start before the pen has got there. It used to be
        // dropped: 「<ruby>永<rt>ㄩㄥˇ</rt></ruby><ruby>和<rt>ㄏㄜˊ</rt></ruby>」
        // — two 字, two readings, one of them silently gone. Push it right
        // instead. A reading a cell off its base is a misalignment the reader
        // can see and correct for; a missing one is a reading they will never
        // know was there.
        let mut want = want.max(col + usize::from(col > 0));
        let reading: String = text[group.reading.0.min(text.len())..group.reading.1.min(text.len())]
            .iter()
            .collect();
        if reading.is_empty() {
            continue;
        }
        // **Group ruby is centred over its base** (JLREQ §3.3.6): 「大阪」 read
        // 「おおさか」 is one reading of one word, and setting it flush left
        // over a four-square base leaves it pointing at the first character.
        // Only when there is room — a reading wider than its base already
        // starts where its base does.
        let base_end = group.base.1.saturating_sub(start_in_line);
        if let Some(&end) = column.get(base_end) {
            let base_width = end.saturating_sub(want);
            let reading_width = yumete_cjk::str_width(&reading);
            if base_width > reading_width {
                want += (base_width - reading_width) / 2;
            }
        }
        out.push_str(&" ".repeat(want - col));
        out.push_str(&reading);
        col = want + yumete_cjk::str_width(&reading);
    }
    (!out.trim().is_empty()).then(|| {
        // A rung back from the writing, the way the 縱書 margin sets one: a
        // reading is *about* the text, and a reading in the text's own colour
        // reads as a second line of it.
        Line::from(Span::styled(out, ink.page().fg(ink.quiet())))
    })
}

/// The rule between two work areas, and the caption of the one being read.
///
/// The boundary and the label for one row: 橫排 gets a `─` across the page
/// with the caption at its left, 縱書 a `│` down it and no caption — a line of
/// text cannot be written down one cell, and the status line says it instead.
fn draw_divider(frame: &mut Frame, editor: &Editor, config: &Config, area: Rect) {
    let ink = crate::theme::Palette::of(config);
    let rule = ink.page().fg(ink.rule());
    let buf = frame.buffer_mut();
    let across = area.height == 1;
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(match across {
                    true => "─",
                    false => "│",
                })
                .set_style(rule);
            }
        }
    }
    // 金墨, because a caption is not prose. It names the pane being *read* —
    // the one you are standing in is named by the status line, which has said
    // where you are all along.
    if across {
        if let Some(pane) = editor.other_pane() {
            if !pane.caption.is_empty() {
                let text = format!(" {} ", pane.caption);
                put_text(
                    buf,
                    area.x + 2,
                    area.y,
                    area.x + area.width,
                    &text,
                    ink.page().fg(ink.gold()),
                );
            }
        }
    }
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
    // On the page's own ground, and *painted* — this row set colours and no
    // background at all, so on a light page over a dark terminal it came out
    // as a black band with the page's dark ink on it, which is to say
    // unreadable. A blank hint row is part of the margin, not a hole in it.
    let page = ink.page();
    // News is the loud kind; keys are the quiet kind and read as furniture.
    let news = page.fg(ink.text());
    // The key is what the eye is hunting for, so it is the lit half; what it
    // does is the half you only read once.
    let key = page.fg(ink.text());
    let what = page.fg(ink.furniture());
    // 金墨: the label names what mode you are in, which is not prose either.
    let label = page.fg(ink.gold()).add_modifier(Modifier::BOLD);
    let right = area.x + area.width;
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..right {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(page);
            }
        }
    }
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
        // **The panel has this now.** A row holds four keys and `空格` has
        // fourteen, so a half-pressed sequence is drawn as a list you can read
        // down; the row keeps what it was always for — what just happened.
        Hint::Keys(..) if editor.pending_menu().is_some() => {}
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
    /// #220: an installed data file the core refuses names the *version*, not
    /// the symptom. Before this, `:yume` said nothing at all and the writer saw
    /// only 拆分 comments that had stopped appearing.
    #[test]
    fn yume_names_a_data_file_the_core_will_not_have() {
        let entry = yumete_ime::data_set(Scheme::Lingming)
            .into_iter()
            .find(|f| f.kind == yumete_ime::DataKind::Annotations)
            .expect("拆分 is in the manifest");
        let dir = std::env::temp_dir().join(format!("yumete-yume-fault-{}", std::process::id()));
        let path = dir.join(entry.file.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("fixture dir");
        let mut stale = b"YDV20260828".to_vec();
        stale.extend_from_slice(&[0u8; 64]);
        std::fs::write(&path, &stale).expect("fixture file");

        let ime = ImeSession::new(Scheme::Lingming, vec![dir]);
        let said = data_faults(&ime);
        assert!(said.contains("chaifen.ydiv"), "names the file: {said}");
        assert!(said.contains("YDV20260828"), "names what the file says: {said}");
        let want = yumete_ime::expected_magic(yumete_ime::DataKind::Annotations).expect("拆分 has a magic");
        assert!(
            said.contains(&String::from_utf8_lossy(want).into_owned()),
            "names what this build wants: {said}"
        );
    }

    /// The other half of the same rule: a directory with nothing in it is
    /// missing everything, and says nothing. Otherwise the clause would be on
    /// screen for every writer who has not installed the optional data.
    #[test]
    fn nothing_installed_is_not_a_fault_worth_saying() {
        let ime = ImeSession::new(Scheme::Lingming, vec![std::path::PathBuf::from("/no/such/dir")]);
        assert_eq!(data_faults(&ime), "");
    }

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
        let mut viewport = Seats::default();
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
        let mut viewport = Seats::default();
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
        let mut viewport = Seats::default();
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

    /// Feature #210: text on the page the file has no bytes for.
    ///
    /// Drawn where the *measure* charged for it — this is the whole point of
    /// the runs going through [`wrap::Measure`] rather than living in the
    /// renderer: the cells the candidate takes are cells the caret, `j` and
    /// the mouse all already know about.
    #[test]
    fn a_candidate_is_drawn_in_the_cells_the_measure_charged_for() {
        let mut editor = editor_with("春夏秋冬");
        editor.set_ghost(vec![(0, 2, "候補".to_string())]);
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let buffer = render_wrapped(&mut editor, &config, 20, 4);
        assert_eq!(row_text(&buffer, 0).trim_end(), "春夏候補秋冬");

        // A rung back from the writing: it is *about* the text and is not in
        // it, and a candidate in the text's own ink reads as already written.
        let quiet = ink(&config).quiet();
        assert_eq!(buffer[(4, 0)].fg, quiet, "the candidate");
        assert_ne!(buffer[(0, 0)].fg, quiet, "…but not 春");
    }

    #[test]
    fn the_caret_lands_past_the_candidate_it_typed() {
        let mut editor = editor_with("春夏秋冬");
        editor.set_ghost(vec![(0, 2, "候補".to_string())]);
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        editor.set_wrap_width(20);
        // On 秋 — which is drawn four cells further right than the file alone
        // would put it, and the caret has to be there and not on 候.
        editor.on_key(Key::Char('l'));
        editor.on_key(Key::Char('l'));
        let (_, at) = render_caret(&editor, &config, 20, 4);
        assert_eq!(at.map(|p| p.x), Some(8));
    }

    /// A configuration for reading one bare row: no gutter, no word tint, no
    /// measure — only the writing, so a column in the assertions is a column
    /// on the page.
    fn bare() -> Config {
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        config.editor.ruler = 0;
        config
    }

    /// With `:wrap off` the page follows the caret off the right edge
    /// (Feature #221).
    ///
    /// A paragraph is then one row of whatever length it happens to be, and
    /// the window used to show its first screenful and nothing else: `gl`
    /// walked the cursor to the end of the line and the writing it landed in
    /// was never drawn.
    #[test]
    fn the_page_scrolls_sideways_to_keep_the_caret_on_it() {
        let mut editor = editor_with("abcdefghijklmnopqrstuvwxyz");
        editor.set_soft_wrap(false);
        let config = bare();

        // At the head of the line the page is not scrolled at all.
        let (buffer, at) = render_caret(&editor, &config, 10, 4);
        assert!(row_text(&buffer, 0).starts_with("abcdefghij"));
        assert_eq!(at.map(|p| p.x), Some(0));

        // `gl` — the end of the line, twenty-six columns along a ten-column
        // window.
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('l'));
        assert_eq!(editor.cursor(), 26, "past the last character");
        let (buffer, at) = render_caret(&editor, &config, 10, 4);
        assert_eq!(
            row_text(&buffer, 0).trim_end(),
            "rstuvwxyz",
            "the tail of the line, not its head"
        );
        assert_eq!(at.map(|p| p.x), Some(9), "the caret is on the page");

        // …and back: `gh` is the head of the line, and the page comes with it.
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('h'));
        let (buffer, at) = render_caret(&editor, &config, 10, 4);
        assert!(row_text(&buffer, 0).starts_with("abcdefghij"));
        assert_eq!(at.map(|p| p.x), Some(0));
    }

    /// One column at a time, not a screenful.
    ///
    /// A caret walking along a long line should not make the whole page slide
    /// out from under the reader every tenth character.
    #[test]
    fn the_page_slides_by_the_one_column_it_needs() {
        let mut editor = editor_with("abcdefghijklmnopqrstuvwxyz");
        editor.set_soft_wrap(false);
        let config = bare();
        for _ in 0..10 {
            editor.on_key(Key::Char('l'));
        }
        let (buffer, at) = render_caret(&editor, &config, 10, 4);
        assert_eq!(row_text(&buffer, 0).trim_end(), "bcdefghijk");
        assert_eq!(at.map(|p| p.x), Some(9));
    }

    /// The gutter is furniture: it does not scroll with the writing.
    ///
    /// Line numbers that slid away with the text would leave the reader with
    /// no way to say where they are.
    #[test]
    fn the_gutter_stays_where_it_is_while_the_writing_slides() {
        let mut editor = editor_with("abcdefghijklmnopqrstuvwxyz");
        editor.set_soft_wrap(false);
        let mut config = bare();
        config.editor.line_numbers = LineNumbers::Absolute;
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('l'));
        let buffer = render(&editor, &config, 14, 4);
        let row = row_text(&buffer, 0);
        assert!(row.trim_start().starts_with('1'), "the number is still there: {row:?}");
        assert!(
            row.trim_end().ends_with("xyz"),
            "and the tail of the line beside it: {row:?}"
        );
    }

    /// A click lands on the 字 under the pointer, not on one a screenful back.
    #[test]
    fn a_click_counts_from_what_is_on_the_page() {
        let mut editor = editor_with("abcdefghijklmnopqrstuvwxyz");
        editor.set_soft_wrap(false);
        let config = bare();
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('l'));
        let (w, h) = (10u16, 4u16);
        let mut seats = Seats::default();
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| draw(frame, &editor, &config, &no_ime(), &mut seats))
            .unwrap();
        let click = |x: u16| {
            let mouse = ratatui::crossterm::event::MouseEvent {
                kind: ratatui::crossterm::event::MouseEventKind::Down(
                    ratatui::crossterm::event::MouseButton::Left,
                ),
                column: x,
                row: 0,
                modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            };
            text_at(&editor, &config, Some((w, h).into()), &seats, mouse)
        };
        // The leftmost cell is `r`, the seventeenth character.
        assert_eq!(click(0), Some(17));
        assert_eq!(click(8), Some(25));
    }

    /// A 漢字 the scroll cuts in half is drawn as air, not as half a 字.
    #[test]
    fn a_character_split_by_the_scroll_becomes_a_blank() {
        let mut editor = editor_with(&"字".repeat(20));
        editor.set_soft_wrap(false);
        let config = bare();
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('l'));
        // The caret rests past the twentieth 字, at column 40, in a window ten
        // columns wide — so the cut falls at column 31, inside a 字.
        let (buffer, caret) = render_caret(&editor, &config, 10, 4);
        assert_eq!(caret.map(|p| p.x), Some(9), "the caret is on the page");
        assert_eq!(at(&buffer, 0, 0), " ", "the half 字 is air");
        assert_eq!(at(&buffer, 1, 0), "字", "and the whole ones follow it");
    }

    /// A click on ghost text means the character it stands before.
    ///
    /// The cells are on the page but not in the file, so they are the one
    /// thing a click cannot land *in* — and the candidate's own cells belong
    /// to the character being typed, which is the character after them.
    #[test]
    fn a_click_on_a_candidate_means_the_character_it_stands_before() {
        let mut editor = editor_with("春夏秋冬");
        editor.set_ghost(vec![(0, 2, "候補".to_string())]);
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        editor.set_wrap_width(20);
        let (w, h) = (20u16, 4u16);
        let mut seats = Seats::default();
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| draw(frame, &editor, &config, &no_ime(), &mut seats))
            .unwrap();

        let click = |x: u16| {
            let mouse = ratatui::crossterm::event::MouseEvent {
                kind: ratatui::crossterm::event::MouseEventKind::Down(
                    ratatui::crossterm::event::MouseButton::Left,
                ),
                column: x,
                row: 0,
                modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
            };
            text_at(&editor, &config, Some((w, h).into()), &seats, mouse)
        };
        // 春夏候補秋冬 — the candidate is the four cells from 4.
        assert_eq!(click(0), Some(0), "春");
        assert_eq!(click(2), Some(1), "夏");
        for x in 4..8 {
            assert_eq!(click(x), Some(2), "the candidate at {x} means 秋");
        }
        assert_eq!(click(8), Some(2), "秋 itself");
        assert_eq!(click(10), Some(3), "冬");
    }

    #[test]
    fn a_candidate_takes_rows_of_its_own_down_the_column() {
        let mut editor = editor_with("春夏秋冬");
        editor.set_ghost(vec![(0, 2, "候補".to_string())]);
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 30, 12);
        let column: Vec<String> = (0..6).map(|y| at(&buffer, 28, y)).collect();
        assert_eq!(column, ["春", "夏", "候", "補", "秋", "冬"]);
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
        // …when it is asked for. Off by default now, and off in both layouts:
        // 縱書 painted this band and 橫排 painted nothing.
        // Set vertically the numbers sit above the 縱, in the text's own
        // columns. Position separates nothing, so the band has to be told apart
        // by colour or it reads as digits somebody typed.
        let mut editor = editor_with("春江\n潮水");
        let mut config = vertical_config();
        config.editor.line_numbers = LineNumbers::Absolute;
        editor.set_number_fill(true);
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
    fn the_commit_method_says_which_one_is_answering() {
        // `:yume commit` with nothing after it is the question (Feature #209).
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "a 啊\n");
        let said = commit_method(&mut ime, "");
        assert!(said.contains("延遲"), "{said}");
        let said = commit_method(&mut ime, "unique");
        assert!(said.contains("唯一"), "{said}");
        assert_eq!(ime.commit_strategy(), CommitStrategy::Unique);
        // A word it cannot read names the three rather than picking one.
        let said = commit_method(&mut ime, "slow");
        assert!(said.contains("slow") && said.contains("fluency"), "{said}");
        assert_eq!(ime.commit_strategy(), CommitStrategy::Unique, "unchanged");
        // …and `:yume` itself says which one is answering, beside the 方案 and
        // the 碼表 — the question a writer asks is 「what is it doing」.
        let said = switch_scheme(&mut ime, "?", &Config::default());
        assert!(said.contains("上屏 唯一"), "{said}");
    }

    /// #211: 空空如也 — the candidate goes into the sentence and the panel
    /// does not come up at all.
    #[test]
    fn bare_puts_the_candidate_in_the_text_and_draws_no_panel() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八 巴
");
        ime.set_panel_display(PanelDisplay::Bare);
        ime.input('b');
        settle_inline_candidate(&mut editor, &ime);

        // The first candidate is on the page, at the caret, though the file
        // holds not one byte of it.
        assert_eq!(editor.current_buffer().text(), "");
        assert_eq!(editor.ghost_on_line(0), vec![(0, "吧".to_string())]);

        let config = Config::default();
        let buffer = render_with(&editor, &config, &ime, 40, 8);
        let rows: Vec<String> = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect();
        // The gutter, then the candidate, and nothing else on the row.
        assert_eq!(rows[0].trim_end(), "1 吧", "{:?}", rows[0]);
        // No panel: the second and third candidates are nowhere on the screen.
        assert!(
            !rows.iter().any(|r| r.contains('八') || r.contains('巴')),
            "{rows:#?}"
        );
        // …and the code has somewhere to be — beside the caret, and on the
        // status line's right edge.
        assert_eq!(hud_line(&editor, &ime), "b");
        assert!(rows.iter().any(|r| r.contains("╰ b")), "{rows:#?}");
    }

    /// #213: a locked buffer says so standing, not once.
    #[test]
    fn a_locked_buffer_wears_it_on_the_status_line() {
        let mut editor = Editor::new();
        editor.current_buffer_mut().insert(0, "讀一讀\n");
        let config = Config::default();

        let rows = |editor: &Editor| -> String {
            let buffer = render(editor, &config, 60, 6);
            (0..buffer.area.height)
                .map(|y| {
                    (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
                // A wide 字 fills two cells, so the second is a blank the
                // renderer owns; squeezing them out is how you ask what the
                // line *says*.
                .replace(' ', "")
        };
        assert!(!rows(&editor).contains("只讀"));
        editor.current_buffer_mut().set_readonly(true);
        assert!(rows(&editor).contains("[只讀]"), "{}", rows(&editor));
    }

    /// #211: `Tab` is the way back to the whole list, for one word.
    #[test]
    fn tab_summons_the_panel_and_it_leaves_with_the_word() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八 巴
");
        ime.set_panel_display(PanelDisplay::Bare);
        ime.input('b');
        assert!(!ime.panel_is_full());

        assert!(ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Tab,
            KeyModifiers::NONE
        ));
        assert!(ime.panel_is_full(), "the whole list, for this one word");
        // …so nothing is drawn into the text while it is up: two surfaces
        // saying the same thing is the thing this editor keeps taking apart.
        settle_inline_candidate(&mut editor, &ime);
        assert!(!editor.has_ghost());

        // The word lands, and the panel goes with it.
        ime.space();
        editor.insert_committed(&ime.take_committed());
        assert_eq!(editor.current_buffer().text(), "吧");
        assert!(!ime.panel_is_full());
        ime.input('b');
        assert!(!ime.panel_is_full(), "a new word starts空空如也 again");
    }

    /// #211: the setting, the command, and the question, in the writer's words.
    #[test]
    fn the_panel_says_which_way_it_is_drawing() {
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "a 啊
");
        let said = panel_method(&mut ime, "");
        assert!(said.contains("候選框"), "{said}");
        let said = panel_method(&mut ime, "bare");
        assert!(said.contains("行內預覽"), "{said}");
        assert_eq!(ime.panel_display(), PanelDisplay::Bare);
        // …and `:yume` says so, because a page with no candidate list on it is
        // the thing a writer asks about.
        let said = switch_scheme(&mut ime, "?", &Config::default());
        assert!(said.contains("面板 行內預覽"), "{said}");
        // A word it cannot read names the two rather than picking one.
        let said = panel_method(&mut ime, "invisible");
        assert!(said.contains("invisible") && said.contains("bare"), "{said}");
        assert_eq!(ime.panel_display(), PanelDisplay::Bare, "unchanged");
    }

    /// #211: a `/` search composes on the status line, which has no page to
    /// draw a candidate into — so the panel comes up there whatever the
    /// setting says.
    #[test]
    fn a_prompt_keeps_its_panel_even_under_bare() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('/'));
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八 巴
");
        ime.set_panel_display(PanelDisplay::Bare);
        ime.input('b');
        settle_inline_candidate(&mut editor, &ime);
        assert!(!editor.has_ghost(), "nothing goes into the manuscript");
        assert_eq!(hud_line(&editor, &ime), "", "nor beside the caret");

        let config = Config::default();
        let buffer = render_with(&editor, &config, &ime, 40, 8);
        let screen: String = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
            .map(|at| buffer[at].symbol().to_string())
            .collect();
        assert!(screen.contains('八'), "the list is where it can be read");
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
        // rung::RULE, where every ring on the screen is drawn.
        let ring = Color::Rgb(0x72, 0x70, 0x62);
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
        for c in "reco".chars() {
            editor.on_key(Key::Char(c));
        }
        let config = Config::default();
        let buffer = render_with(&editor, &config, &no_ime(), 90, 24);

        let row = buffer.area.height - 1;
        let line: String = (0..buffer.area.width)
            .map(|x| buffer[(x, row)].symbol())
            .collect();
        assert!(line.starts_with(":recover"), "guess shown: {line:?}");

        // A rung back, not `DIM`: the attribute is dropped by enough terminals
        // that a guess would read as typed on them.
        let ink = ink(&config);
        let fg = |x: u16| buffer[(x, row)].style().fg;
        assert_eq!(fg(3), Some(ink.text()), "`rec` was typed");
        assert_eq!(
            fg(5),
            Some(ink.at(yumete_config::rung::RULE)),
            "`ver` is only a guess"
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

        // **The column numbers** are the very top row: every numeric key in a
        // grid counts columns, and a 28-column 拆分表 gives no other way to
        // count to seventeen.
        let numbers = row(0);
        assert!(numbers.contains('1') && numbers.contains('2') && numbers.contains('3'), "{numbers:?}");

        // The header is under them and names the columns; both are frozen.
        let head = row(1);
        assert!(head.starts_with("char"), "the header is frozen on top: {head:?}");
        assert!(head.contains("ids_y") && head.contains("note"));

        // Every row's second column starts in the same terminal column — which
        // is the entire point of drawing a CSV as a grid.
        let column_of = |y: u16, want: &str| {
            (0..60u16).find(|&x| at(&buffer, x, y) == want)
        };
        let a = column_of(2, "⿰").expect("一's 拆分");
        let b = column_of(3, "⿰").expect("齾's 拆分");
        assert_eq!(a, b, "the same field of two rows starts in the same column");
        assert!(a > 4, "and after the first column, not at the edge");

        // The widest visible cell sets the column's width, so `longer` fits.
        assert!(row(3).contains("longer"), "{:?}", row(3));

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

        // Row 2 of the file is the first data row, on screen row 2 under the
        // column numbers and the header; the cursor is in its second cell.
        let start = (0..60u16)
            .find(|&x| buffer[(x, 2)].style().bg == lit)
            .expect("a lit cell");
        assert_eq!(at(&buffer, start, 2), "⿰", "it is the 拆分 cell");
        // The ground runs the column's whole width, past the end of the text —
        // a cell you are inside, not three highlighted characters.
        // Counted, not run-length: a wide glyph covers two cells and ratatui
        // only ever sends the first, so the second reads back unstyled here
        // even though the terminal paints the whole glyph. What matters is
        // that the ground reaches past the end of the text.
        let last = (0..60u16)
            .rfind(|&x| buffer[(x, 2)].style().bg == lit)
            .unwrap();
        let text_ends = (start..60).find(|&x| at(&buffer, x, 2) == " " && at(&buffer, x - 1, 2) == " ");
        assert!(
            last > start + 5,
            "the box is the column's width, not the text's: {start}..{last}"
        );
        assert!(text_ends.is_some_and(|e| last >= e), "the padding is lit too");
        // …and nothing on the frozen rows or on another data row is lit.
        assert!((0..60u16).all(|x| buffer[(x, 0)].style().bg != lit), "not the numbers");
        assert!((0..60u16).all(|x| buffer[(x, 1)].style().bg != lit), "not the header");
        assert!((0..60u16).all(|x| buffer[(x, 3)].style().bg != lit), "not another row");

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
        // The extra field is drawn: hiding it would hide the damage. Two rows
        // are frozen above the data now — the column numbers and the header.
        assert!(row(3).contains("a") && row(3).contains("b"), "{:?}", row(3));
        // And the row number is marked, so it can be found from a distance.
        let torn = Some(ink(&config).mark());
        assert_eq!(buffer[(0, 3)].style().fg, torn, "the bad row's number");
        assert_ne!(buffer[(0, 2)].style().fg, torn, "not the good one's");

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

        // **A half-pressed sequence belongs to the panel now**, not to this
        // row: a row holds four keys and `空格` has fourteen. The row keeps
        // what it was always for — what just happened — and the two surfaces
        // stop saying the same thing.
        for key in ['m', 'g', ' '] {
            editor.on_key(Key::Char(key));
            assert_eq!(hint(&editor).trim(), "", "{key} belongs to the panel");
            editor.on_key(Key::Esc);
        }
    }

    /// A click in a grid lands on the cell it was pointed at.
    ///
    /// The columns are padded to line up on screen while the file behind them
    /// is ragged, so a click resolved as prose lands off by the padding of
    /// every column to its left — which looks random, and was.
    #[test]
    fn a_click_in_a_grid_lands_where_it_was_pointed() {
        let dir = std::env::temp_dir().join(format!("yumete-click-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".yumete").join("tables")).unwrap();
        std::fs::write(
            dir.join(".yumete").join("tables").join("t.toml"),
            "[table]\nfile = ['d.csv']\nkey = 'char'\n\
             [[table.column]]\nname = 'char'\n[[table.column]]\nname = 'ids_y'\n\
             [[table.column]]\nname = 'note'\n",
        )
        .unwrap();
        let csv = dir.join("d.csv");
        // Ragged on purpose: one-character cells beside long ones is exactly
        // what padding hides and what the old mapping tripped over.
        std::fs::write(
            &csv,
            "char,ids_y,note\n木,木,樹\n相,⿰木目,看\n林,⿰木木,樹林很密\n杏,⿱木口,果\n",
        )
        .unwrap();

        let mut editor = Editor::new();
        editor.open_file(&csv).unwrap();
        let config = Config::default();
        let mut seats = Seats::default();
        let (w, h) = (40u16, 10u16);
        // Draw once so the viewport is settled the way a click will read it.
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| draw(frame, &editor, &config, &no_ime(), &mut seats))
            .unwrap();
        let buffer = terminal.backend().buffer().clone();

        // Point at every cell of every row, by finding what is drawn there —
        // and take the row's own number, drawn in the gutter, as the truth
        // about which line that is.
        let rope = editor.current_buffer().rope();
        let numbered = |y: u16| -> Option<usize> {
            let n: String = (0..6).map(|x| at(&buffer, x, y)).collect();
            n.trim().parse::<usize>().ok().map(|n| n - 1)
        };
        // The two frozen rows — the column numbers and the header — are not
        // rows of the table, and the numbers row is all digits.
        for y in 2..h {
            let Some(line) = numbered(y) else { continue };
            // The phantom last line a trailing newline opens has no cells to
            // point at.
            if rope.line(line).to_string().trim().is_empty() {
                continue;
            }
            for x in 0..w {
                let symbol = at(&buffer, x, y);
                if symbol.trim().is_empty() {
                    continue;
                }
                let mouse = ratatui::crossterm::event::MouseEvent {
                    kind: ratatui::crossterm::event::MouseEventKind::Down(
                        ratatui::crossterm::event::MouseButton::Left,
                    ),
                    column: x,
                    row: y,
                    modifiers: ratatui::crossterm::event::KeyModifiers::NONE,
                };
                let Some(at_char) = text_at(&editor, &config, Some((w, h).into()), &seats, mouse)
                else {
                    continue;
                };
                // The line is the row that was clicked…
                assert_eq!(
                    rope.char_to_line(at_char),
                    line,
                    "click at ({x},{y}) on {symbol:?}"
                );
                // …and the character is the one drawn there, or its first half.
                let got = rope.char(at_char).to_string();
                assert!(
                    got == symbol || yumete_cjk::str_width(&got) == 2,
                    "click at ({x},{y}): drawn {symbol:?}, landed on {got:?}"
                );
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// **What you have typed is on the screen** — twice: at the status line's
    /// right edge, where vi has put it since 1976, and beside the caret, where
    /// the eyes are. Pressing `3` used to change nothing at all.
    #[test]
    fn what_has_been_typed_shows_beside_the_caret_and_in_the_corner() {
        let mut editor = editor_with("那年冬天，山下起了大雪。\n雪一直下到開春。\n");
        let config = Config::default();
        let page = |editor: &Editor| -> String {
            let b = render(editor, &config, 60, 10);
            (0..b.area.height)
                .map(|y| (0..b.area.width).map(|x| at(&b, x, y)).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
                .replace(' ', "")
        };
        assert!(!page(&editor).contains("╰"), "nothing pending, nothing drawn");

        editor.on_key(Key::Char('3'));
        let drawn = page(&editor);
        assert!(drawn.contains("╰3"), "beside the caret: {drawn}");
        assert!(
            drawn.lines().last().unwrap().ends_with('3'),
            "and in the corner: {drawn}"
        );

        // `30` is not `3`, which is the whole point.
        editor.on_key(Key::Char('0'));
        assert!(page(&editor).contains("╰30"), "{}", page(&editor));

        // The command completes and it is gone.
        editor.on_key(Key::Char('l'));
        assert!(!page(&editor).contains("╰"), "{}", page(&editor));

        // Insert mode never draws it: nothing may flicker beside the writing.
        editor.on_key(Key::Char('i'));
        editor.on_key(Key::Char('甲'));
        assert!(!page(&editor).contains("╰"));
    }

    /// **It takes the row that has room.**
    ///
    /// Under the caret first, over it when that row is full — a HUD that
    /// vanishes because the line below happens to reach the edge is a HUD you
    /// cannot rely on, and the eye stops looking for it.
    #[test]
    fn the_hud_takes_whichever_row_has_room_for_it() {
        // Above the caret: a short line with margin to spare. Below it: a line
        // that reaches the right edge.
        let full = "那年冬天山下起了大雪一直下到開春天氣才回暖起來了。";
        let mut editor = editor_with(&format!("短。\n短二。\n{full}\n{full}\n"));
        editor.on_key(Key::Char('j'));
        let config = Config::default();
        let rows = |editor: &Editor| -> Vec<String> {
            let b = render(editor, &config, 30, 8);
            (0..b.area.height)
                .map(|y| (0..b.area.width).map(|x| at(&b, x, y)).collect::<String>())
                .collect()
        };
        // Caret on the short first line: the row below is full to the edge, so
        // the HUD goes *above* — the top row of the page is empty margin.
        editor.on_key(Key::Char('3'));
        let drawn = rows(&editor);
        let where_is_it = drawn
            .iter()
            .position(|r| r.contains('╰'))
            .unwrap_or_else(|| panic!("the HUD had a row and did not take it: {drawn:#?}"));
        assert_eq!(where_is_it, 0, "the row above, since the one below is full: {drawn:#?}");
    }

    /// The panel says what can finish the key you pressed, and stands on the
    /// side of the page the cursor is not on.
    #[test]
    fn the_panel_lists_what_would_finish_the_sequence() {
        let mut editor = editor_with("那年冬天，山下起了大雪。\n");
        let config = Config::default();
        // A wide glyph leaves its second cell empty, so the spaces come out.
        let drawn = |editor: &Editor| -> Vec<String> {
            let b = render(editor, &config, 60, 14);
            (0..b.area.height)
                .map(|y| {
                    (0..b.area.width)
                        .map(|x| at(&b, x, y))
                        .collect::<String>()
                        .replace(' ', "")
                })
                .collect()
        };

        // Nothing pending: no panel.
        assert!(!drawn(&editor).iter().any(|row| row.contains("配對")));

        editor.on_key(Key::Char('m'));
        let page = drawn(&editor);
        let panel: Vec<&String> = page.iter().filter(|r| r.contains('│')).collect();
        assert!(!panel.is_empty(), "a bordered panel: {page:?}");
        assert!(
            page.iter().any(|r| r.contains("配對")) && page.iter().any(|r| r.contains("包起來")),
            "{page:?}"
        );
        // Its title is in its own border.
        assert!(page.iter().any(|r| r.contains('m')), "{page:?}");

        // The cursor is at the line's start, so the panel keeps to the right:
        // the panel's rows are the ones with a ring on them, and they start
        // well past the middle of a 60-column page.
        let b = render(&editor, &config, 60, 14);
        let ring = (0..b.area.height)
            .find_map(|y| {
                (0..b.area.width)
                    .find(|&x| at(&b, x, y) == "│")
                    .map(|x| (x, y))
            })
            .expect("a ring");
        assert!(ring.0 > 30, "the panel is on the right: {ring:?}");
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

    /// Render horizontally with the readings laid out.
    fn render_with_ruby(
        editor: &mut Editor,
        config: &Config,
        w: u16,
        h: u16,
    ) -> ratatui::buffer::Buffer {
        editor.set_ruby(yumete_core::ruby::Dialects::only(
            yumete_core::ruby::Dialect::Html,
        ));
        render_wrapped(editor, config, w, h)
    }

    #[test]
    fn the_hit_you_are_standing_on_is_told_apart_from_the_rest() {
        // On a long line of 漢字 a hit in the ordinary selection ground is easy
        // to lose. Three cues, none of which costs a column: the row is banded,
        // the hit itself takes 朱 washed to a highlighter's ground, and the
        // row's *number* turns 朱 — the number is kept rather than replaced by
        // an arrow, because which line it is is what a gutter is for.
        let mut editor = editor_with("那年冬天很冷。\n第二行。\n那年夏天很熱。\n");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::Absolute;
        // `g?`: 「這個詞還在哪裏」, shown in the other work area.
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('?'));
        assert_eq!(editor.peeked_line(), Some(2), "{}", editor.status());

        let buffer = render(&editor, &config, 40, 9);
        let ink = ink(&config);
        // The hit is in the *other* half, which is drawn a rung back — so the
        // marks there are the faded ones.
        let quiet = ink.faded();
        let rows = buffer.area.height;
        let marked = (0..rows).any(|y| {
            (0..buffer.area.width)
                .any(|x| buffer[(x, y)].style().bg == Some(quiet.wash()))
        });
        assert!(marked, "the hit is washed in 朱");
        let numbered = (0..rows).any(|y| buffer[(0, y)].style().fg == Some(quiet.mark()));
        assert!(numbered, "and its line number is 朱");
    }

    /// 疏排 on the horizontal page: a row of air above every row.
    #[test]
    fn a_loose_horizontal_page_keeps_a_row_of_air() {
        let mut editor = editor_with("那年冬天。\n雪一直下。\n山路斷了。\n");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        config.editor.hints = false;
        let rows = |editor: &Editor| -> Vec<String> {
            let b = render(editor, &config, 30, 10);
            (0..b.area.height)
                .map(|y| {
                    (0..b.area.width)
                        .map(|x| at(&b, x, y))
                        .collect::<String>()
                        .trim_end()
                        .to_string()
                })
                .collect()
        };
        // Packed: the rows are against each other.
        assert!(rows(&editor)[1].contains("雪"), "{:?}", rows(&editor));

        editor.execute(":dense off").unwrap();
        let loose = rows(&editor);
        assert!(loose[0].trim().is_empty(), "a row of air first: {loose:?}");
        assert!(loose[1].contains("那"), "{loose:?}");
        assert!(loose[2].trim().is_empty(), "and between them: {loose:?}");
        assert!(loose[3].contains("雪"), "{loose:?}");

        // …and the caret follows the page it is drawn on.
        editor.on_key(Key::Char('j'));
        let (_, caret) = render_caret(&editor, &config, 30, 10);
        assert_eq!(caret.map(|p| p.y), Some(3), "the second row is drawn at 3");

        editor.execute(":dense on").unwrap();
        assert!(rows(&editor)[1].contains("雪"));
    }

    /// Typewriter mode: the row being written stays in the middle, and the
    /// paper moves under it.
    #[test]
    fn typewriter_keeps_the_line_you_are_writing_in_the_middle() {
        let mut editor = editor_with(&(1..=60).map(|n| format!("第{n}行。\n")).collect::<String>());
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        config.editor.hints = false;
        let rows = 13u16;
        let middle = 5;
        let mut seats = Seats::default();

        editor.execute(":typewriter").unwrap();
        editor.execute(":20").unwrap();
        assert_eq!(
            caret_over_time(&editor, &config, &mut seats, 30, rows).map(|p| p.y),
            Some(middle)
        );
        // …and it *stays* there: a step does not nudge, the page moves.
        for _ in 0..4 {
            editor.on_key(Key::Char('j'));
            assert_eq!(
                caret_over_time(&editor, &config, &mut seats, 30, rows).map(|p| p.y),
                Some(middle),
                "the paper moves, not the line"
            );
        }
        // Off again, and a step is a step.
        editor.execute(":typewriter off").unwrap();
        editor.on_key(Key::Char('j'));
        assert_eq!(
            caret_over_time(&editor, &config, &mut seats, 30, rows).map(|p| p.y),
            Some(middle + 1)
        );
    }

    /// **A jump lands in the middle; a step nudges.**
    ///
    /// Which it was is the editor's answer, not the page's: a hit two rows
    /// below the bottom edge and one two rows above it are the same act, and
    /// keying the decision on「did the page have to scroll」put one of them on
    /// the fourth row from the top and the other in the middle.
    /// Render repeatedly **keeping the page's own scroll**, the way the event
    /// loop does: a fresh `Seats` starts at the top of the document and
    /// re-derives the scroll from nothing, which is not what a second keystroke
    /// sees.
    fn caret_over_time(
        editor: &Editor,
        config: &Config,
        viewport: &mut Seats,
        w: u16,
        h: u16,
    ) -> Option<Position> {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| draw(frame, editor, config, &no_ime(), viewport))
            .unwrap();
        terminal.get_cursor_position().ok()
    }

    #[test]
    fn a_jump_lands_in_the_middle_and_a_step_does_not() {
        let mut editor = editor_with(&(1..=60).map(|n| format!("第{n}行。\n")).collect::<String>());
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        config.editor.hints = false;
        // 13 terminal rows: twelve of page and the status line, so the last
        // row of the page is 11 and the middle of it is 5.
        let rows = 13u16;
        let middle = 5;

        let mut seats = Seats::default();

        // `:30` is a jump: line 30 lands in the middle of the page.
        editor.execute(":30").unwrap();
        let caret = caret_over_time(&editor, &config, &mut seats, 30, rows);
        assert_eq!(caret.map(|p| p.y), Some(middle), "a jump centres");

        // `j` from there is a step: it moves one row, it does not re-centre.
        editor.on_key(Key::Char('j'));
        let caret = caret_over_time(&editor, &config, &mut seats, 30, rows);
        assert_eq!(caret.map(|p| p.y), Some(middle + 1), "a step nudges");

        // …and a search hit is a jump, whichever direction it was found in.
        editor.execute(":search 第55行").unwrap();
        let caret = caret_over_time(&editor, &config, &mut seats, 30, rows);
        assert_eq!(caret.map(|p| p.y), Some(middle), "forwards");
        editor.execute(":search 第9行").unwrap();
        let caret = caret_over_time(&editor, &config, &mut seats, 30, rows);
        assert_eq!(caret.map(|p| p.y), Some(middle), "and backwards");
    }

    /// A block's ground is painted to the edge of the page even on a row whose
    /// width the editor and ratatui disagree about.
    ///
    /// `▓` and `—` are East-Asian *ambiguous*: two cells to an editor set up
    /// for 漢字 prose, one to ratatui, which lays every span out with the Latin
    /// widths and has no CJK setting. The fill used to be sized by subtracting
    /// the editor's width from the page's, so a fenced line with eleven `▓` in
    /// it stopped its ground eleven cells short of the right edge — visible in
    /// the manual's own `:wrap` example.
    #[test]
    fn a_blocks_ground_reaches_the_edge_whatever_the_widths_say() {
        let editor = editor_with("前一段。\n\n```\n那年冬天。   ▓▓▓▓▓▓\n雪——一直下。\n```\n");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        let buffer = render(&editor, &config, 40, 8);
        let band = ink(&config).at(yumete_config::rung::BAND);
        // Rows 2–5 are the fence and what is inside it.
        for y in 2..=5 {
            let last = buffer.area.width - 1;
            assert_eq!(
                buffer[(last, y)].style().bg,
                Some(band),
                "row {y} of the fence is grounded to the last column",
            );
        }
    }

    #[test]
    fn every_cell_of_the_frame_is_painted() {
        // The black-hole-in-light-mode bug, three times over: a rectangle
        // handed out and not painted shows the terminal's own ground. This is
        // the assertion that keeps it gone.
        let mut editor = editor_with("那年冬天很冷。\n第二行\n第三行");
        let config = Config::default();
        let ink = ink(&config);
        for (w, h) in [(9u16, 12u16), (10, 12), (12, 5), (40, 10), (80, 24)] {
            for sidebar in [false, true] {
                if sidebar {
                    editor.execute("open .").ok();
                    editor.on_key(Key::Char(' '));
                    editor.on_key(Key::Char('e'));
                }
                let buffer = render(&editor, &config, w, h);
                for y in 0..h {
                    // Walked by display width: the second half of a wide glyph
                    // is ratatui's own — it resets that cell and then never
                    // sends it, because the glyph covers both columns.
                    let mut x = 0;
                    while x < w {
                        let cell = &buffer[(x, y)];
                        let bg = cell.style().bg;
                        assert!(
                            bg.is_some() && bg != Some(ratatui::style::Color::Reset),
                            "{w}x{h} sidebar={sidebar}: ({x},{y}) shows the terminal's own                              ground — a light page over a dark terminal has a hole there"
                        );
                        x += grapheme_width(cell.symbol()).max(1) as u16;
                    }
                }
            }
        }
        let _ = ink;
    }

    #[test]
    fn the_other_work_area_shows_a_place_without_going_there() {
        // 「誰用了卵」 is a question about two places at once, and the answer
        // used to be a jump: you were taken to one of them and could no longer
        // see the other. The second work area answers it as it was asked.
        let mut editor = editor_with("第一行\n第二行\n第三行\n第四行\n");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;

        // One area: the whole page.
        let buffer = render(&editor, &config, 40, 9);
        assert_eq!(row_text(&buffer, 0).trim_end(), "第一行");

        // 空格 w opens the other one, showing the same place; the divider
        // carries its caption.
        editor.on_key(Key::Char(' '));
        editor.on_key(Key::Char('w'));
        assert!(editor.other_pane().is_some());
        let buffer = render(&editor, &config, 40, 9);
        let rows: Vec<String> = (0..9).map(|y| row_text(&buffer, y).trim_end().to_string()).collect();
        let divider = rows.iter().position(|r| r.starts_with('─')).expect("a rule between them");
        assert!(divider > 0 && divider + 1 < 9, "an equal cut: {rows:?}");
        assert!(rows[divider].contains("[scratch]"), "the caption: {:?}", rows[divider]);
        // Both halves show the file.
        assert_eq!(rows[0], "第一行");
        assert_eq!(rows[divider + 1], "第一行");

        // 空格 w again hands it the keys — and the halves stay where they are.
        editor.on_key(Key::Char(' '));
        editor.on_key(Key::Char('w'));
        assert_eq!(editor.live_pane(), 1, "the keys are in the second half");

        // 空格 W keeps the half you are standing in.
        editor.on_key(Key::Char(' '));
        editor.on_key(Key::Char('W'));
        assert!(editor.other_pane().is_none());
        assert_eq!(editor.live_pane(), 0);
        let buffer = render(&editor, &config, 40, 9);
        assert!(
            !(0..9).any(|y| row_text(&buffer, y).starts_with('─')),
            "and the rule is gone"
        );
    }

    #[test]
    fn a_reading_is_set_over_the_字_it_reads() {
        // 橫排 laid out no readings at all: the markup sat on the page as the
        // characters it is, and `<ruby>韋<rt>wéi</rt></ruby>` is not a word
        // anybody wrote. The 縱書 page has always put it in the margin; here
        // it goes in the row above, over its own base.
        let mut editor = editor_with("那<ruby>韋<rt>wéi</rt></ruby>字。");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        let buffer = render_with_ruby(&mut editor, &config, 40, 8);

        // The tags are off the page and the base is not.
        assert_eq!(row_text(&buffer, 1).trim_end(), "那韋字。");
        // …and the reading is above the 字 it reads: 那 is two cells, so 韋
        // begins at cell 2 and so does its reading.
        let reading = row_text(&buffer, 0);
        assert_eq!(reading.trim_end(), "  wéi", "{reading:?}");

        // With no dialect laid out the file is the page again.
        editor.set_ruby(yumete_core::ruby::Dialects::NONE);
        let buffer = render_wrapped(&mut editor, &config, 40, 8);
        assert_eq!(
            row_text(&buffer, 0).trim_end(),
            "那<ruby>韋<rt>wéi</rt></ruby>字。"
        );
    }

    #[test]
    fn two_readings_in_a_row_both_get_drawn() {
        // 注音 is wider than the 字 it reads, so the second reading wanted to
        // start before the first had finished — and was dropped. On 注音-annotated
        // prose, which is what this feature is *for*, that is most of them.
        let mut editor = editor_with("<ruby>永<rt>ㄩㄥˇ</rt></ruby><ruby>和<rt>ㄏㄜˊ</rt></ruby>九年");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        let buffer = render_with_ruby(&mut editor, &config, 40, 8);
        let reading = row_text(&buffer, 0);
        assert!(reading.contains("ㄩㄥˇ"), "{reading:?}");
        assert!(reading.contains("ㄏㄜˊ"), "the second reading too: {reading:?}");
        assert_eq!(row_text(&buffer, 1).trim_end(), "永和九年");
    }

    #[test]
    fn a_reading_is_not_written_twice_when_the_wrap_cuts_its_word() {
        // 橫排 has no ruby-aware wrap yet, so a group's base can be split
        // across two rows. What must not happen is the *reading* being drawn
        // in full over both halves — two readings of one word, and the second
        // one over characters it does not read.
        let mut editor = editor_with("一二三<ruby>上海<rt>zaonhe</rt></ruby>四五");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        let buffer = render_with_ruby(&mut editor, &config, 10, 8);
        let rows: Vec<String> = (0..buffer.area.height)
            .map(|y| row_text(&buffer, y).trim_end().to_string())
            .collect();
        let times = rows.iter().filter(|r| r.contains("za")).count();
        assert_eq!(times, 1, "one reading, not one per row: {rows:?}");
    }

    #[test]
    fn a_group_reading_is_centred_over_its_word() {
        // JLREQ §3.3.6: a reading of a *word* is centred over the word. Set
        // flush left over a four-square base it points at the first character
        // and reads as a reading of that one 字.
        let mut editor = editor_with("<ruby>上海話<rt>zaonhe</rt></ruby>很好聽");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        let buffer = render_with_ruby(&mut editor, &config, 40, 8);
        // The base is six cells wide, the reading six — nothing to centre.
        assert_eq!(row_text(&buffer, 1).trim_end(), "上海話很好聽");
        assert_eq!(row_text(&buffer, 0).trim_end(), "zaonhe", "{:?}", row_text(&buffer, 0));

        // A short reading over a wide base is centred: 「zon」 is three cells
        // over six, so it starts one cell in — the half cell an exact centre
        // would want is not a thing a terminal has.
        let mut editor = editor_with("<ruby>上海話<rt>zon</rt></ruby>很好聽");
        let buffer = render_with_ruby(&mut editor, &config, 40, 8);
        assert_eq!(row_text(&buffer, 0).trim_end(), " zon", "{:?}", row_text(&buffer, 0));
    }

    #[test]
    fn a_row_with_no_reading_costs_no_row() {
        // The reading row is *per row*, not per page: a paragraph with one
        // annotated 字 in it does not double-space the whole book.
        let mut editor = editor_with("第一行\n第二行<ruby>甲<rt>jiǎ</rt></ruby>\n第三行");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        let buffer = render_with_ruby(&mut editor, &config, 40, 8);
        assert_eq!(row_text(&buffer, 0).trim_end(), "第一行");
        assert_eq!(row_text(&buffer, 1).trim_end(), "      jiǎ", "over 甲, six cells in");
        assert_eq!(row_text(&buffer, 2).trim_end(), "第二行甲");
        assert_eq!(row_text(&buffer, 3).trim_end(), "第三行");
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
        let mut viewport = Seats::default();
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

    /// `C-Space` is the first key the lesson asks a reader to press.
    /// The headless picture: `--shot`, and what a reviewer sees.
    #[test]
    fn a_frame_can_be_drawn_without_a_terminal() {
        let mut editor = editor_with("那年冬天，雪下得早。\n山路斷了。\n");
        let config = Config::default();
        let ime = ImeSession::empty(Scheme::Lingming);
        let shot = frame_to_text(&mut editor, &config, &ime, 40, 8);
        // The writing is in it, one 漢字 to two cells and no space between two
        // of them — the blank a wide glyph owns is not part of the picture.
        assert!(shot.contains("那年冬天，雪下得早。"), "{shot}");
        assert!(shot.contains("山路斷了。"), "{shot}");
        // …and so is the status line, which is half of what a picture is for.
        assert!(shot.contains("NORMAL"), "{shot}");
        assert_eq!(shot.lines().count(), 8, "one line per row: {shot}");
    }

    #[test]
    fn control_space_turns_the_ime_on_and_off() {
        // The switch itself, spelled the way the main loop spells it. It went
        // unimplemented for as long as the lesson has taught it: `Ctrl(' ')`
        // reached the editor, which has no binding for it, and nothing
        // happened or was said.
        let config = Config::default();
        let mut ime = ImeSession::from_table_text(Scheme::Lingming, "b 吧 八\n");
        assert!(ime.is_chinese(), "a loaded table starts in Chinese");
        let said = switch_scheme(&mut ime, "-", &config);
        assert!(!ime.is_chinese(), "{said}");
        let said = switch_scheme(&mut ime, "+", &config);
        assert!(ime.is_chinese(), "{said}");
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
        let mut viewport = Seats::default();
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
        let mut viewport = Seats::default();
        terminal
            .draw(|frame| draw(frame, &editor, &config, &no_ime(), &mut viewport))
            .unwrap();
        let at = terminal.get_cursor_position().unwrap();
        assert!(at.y < 3, "the caret stays on the page, was at row {}", at.y);
    }
}
