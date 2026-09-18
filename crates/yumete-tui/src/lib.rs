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

pub mod chrome;
pub mod panel;
pub mod table;
pub mod theme;
pub mod vertical;
pub mod ambiguous;
pub mod backend;
pub mod system_ime;
pub mod typed_ahead;

use std::io::{self, stdout, Write as _};

use ratatui::crossterm::cursor::SetCursorStyle;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture,
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    ModifierKeyCode, MouseButton, MouseEventKind, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::supports_keyboard_enhancement;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use yumete_config::{Config, LineNumbers};
use yumete_cjk::{Segmenter, WordMark};
use yumete_core::command::Engagement;
use yumete_core::editor::Hud;
use yumete_core::sidebar::{Layer, Panel, Side, Transient, View};
use yumete_core::wrap::{self, Anchor as WrapAnchor};
use yumete_core::zong::{Anchor, Layout as WritingLayout};
use yumete_core::{diag, say, Editor, Key, KeyOutcome, Mode, ShotJob, TextStore};
use yumete_ime::{
    CommitStrategy, DataFault, DataProblem, FuncKey, ImeSession, ModifierTap, PanelDisplay,
    Scheme,
};

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
/// `:word-list reload` used to be two, and only one of them knew about the
/// language model.
///
/// 1. Yume's own language model — over a million weighted entries in both
///    scripts, already loaded with the IME and shared by reference.
/// 2. A `segmentation.txt` the reader wrote, in the data directory.
/// 3. The compact list bundled with the binary, which covers common prose.
///
/// `level` is `:word-level`, applied to whichever of the three it settled on.
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

/// Install what a 自動認詞 worker found, or leave what is installed alone.
///
/// Its own function because the invariant is one a reader of the loop cannot
/// see: **a worker sends twice** — the answer, then the empty message
/// `Unlatch` posts as the thread ends — so this is called a second time, with
/// nothing, on the turn after every successful scan.
fn install_detected(
    editor: &mut yumete_core::Editor,
    found: &[yumete_core::discover::Found],
) {
    // ⚠️ **Nothing found, nothing touched** (#491). Every worker
    // sends **twice**: its answer, and then the empty one `Unlatch`
    // posts as the thread ends. The receiver takes one message per
    // turn, so the empty one landed on the turn *after* the answer —
    // and the clear below wiped the two hundred words that had just
    // gone in, one keystroke after they went in. 自動認詞 installed
    // nothing that outlived a single key, and 「宇夢」 in `yume.md`
    // never tinted however often it was scanned for.
    //
    // So the clear belongs **inside** the branch that has something to
    // put back, which is also the shape `discover_words` already uses.
    if !found.is_empty() {
        // ⚠️ **Forget the last scan before judging this one** (#466).
        // The filter below asks the segmenter in force 「do you already
        // join this?」 — and the segmenter in force **contains the
        // previous scan's own answer**. The worker has its own
        // dictionary and finds 阿寧 every time; the editor then threw it
        // away because run 1 had taught it, and `set_detected_words`
        // replaces rather than merges, so run 2 installed almost
        // nothing. The tint came and went on alternate saves.
        let had = editor.take_detected_words();
        let mut list = yumete_cjk::WordList::default();
        // The worker has already capped the list to a share of what it
        // read; everything it sends is meant to be used.
        for word in found.iter() {
            if !editor.joins_as_one(&word.word) {
                list.add(&word.word);
            }
        }
        // ⚠️ **A scan whose every word was already known is not an
        // answer either** (#466). `:word-discover` ends by *opening*
        // the listing it wrote, and opening a file asks for a scan —
        // of the listing, in `.yumete/`, where every word occurs once
        // and `MIN_COUNT` is five. So the command that had just
        // installed two hundred words watched them vanish on the next
        // keystroke. A scan that learns nothing must not unteach.
        editor.set_detected_words(match list.is_empty() {
            true => had,
            false => list,
        });
    }
}

/// Sends an empty answer when a 自動認詞 worker ends, however it ends.
///
/// The receiver clears `detecting` on anything it takes and installs nothing
/// for an empty list, so this costs one channel message and buys the guarantee
/// that a dead worker cannot switch the feature off for the session.
struct Unlatch(std::sync::mpsc::Sender<(Vec<yumete_core::discover::Found>, usize)>);

impl Drop for Unlatch {
    fn drop(&mut self) {
        let _ = self.0.send((Vec::new(), 0));
    }
}

/// How long before 自動認詞 scans a project again (#448).
///
/// 「其实我觉得 discover 可以过一段时间触发一次（不用太密集）」 — and a scan
/// answers a question that changes at the speed of writing, not of typing. Five
/// minutes of prose is a few hundred characters; the names in it were already
/// in the chapters the last scan read.
const DETECT_AGAIN: std::time::Duration = std::time::Duration::from_secs(5 * 60);


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
    let areas = page_areas(editor, config, Rect::new(0, 0, width, height), 0);
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
    match html {
        true => buffer_to_html(buffer),
        false => buffer_to_text(buffer),
    }
}

/// Which build drew the pictures — set once, by the binary, at start-up.
///
/// A shot pasted into a bug report is useless if nobody can say what it is a
/// picture *of*: 「面板畫錯了」 about a binary three days old is a different
/// conversation from the same sentence about HEAD. The binary knows its own
/// stamp ([`env!("YUMETE_VERSION")`]) and hands it over here; a caller that
/// never does — every test in this crate — gets frames with no footer, which
/// is what a frame compared against an expected picture needs.
static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Tell the shot machinery which build it is drawing for.
pub fn set_build(version: &str) {
    let _ = BUILD.set(version.to_string());
}

/// The footer line, or nothing when no build was declared.
fn build_footer() -> Option<String> {
    BUILD.get().map(|version| format!("-- yumete {version}"))
}

/// One drawn frame as plain text — the cells, with the blanks a wide glyph
/// owns left out.
///
/// The build stamp goes **under** the frame, not in it: a picture of the page
/// has to stay a picture of the page, and a reviewer counting columns must not
/// find a column that the editor never drew.
fn buffer_to_text(buffer: &ratatui::buffer::Buffer) -> String {
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
    if let Some(footer) = build_footer() {
        out.push_str(&footer);
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
    // `ratatui::init` for the raw mode, the alternate screen and the panic hook
    // that undoes both — and then a terminal of our own over it, so a wiki
    // name can be underlined with dots (#287, `backend.rs`).
    drop(ratatui::init());
    let mut terminal = ratatui::Terminal::new(crate::backend::Dotted::new(stdout()))?;

    // Enable the Kitty keyboard protocol — the base level, which every session
    // holds: unambiguous escape codes, and press told from release.
    //
    // **`REPORT_ALL_KEYS_AS_ESCAPE_CODES` is not in the base level** (#271).
    // With it on, a text key is no longer sent as text: it arrives as
    // `CSI <key> ; <mods> ; <text> u`, and the text is in the third parameter —
    // which crossterm parses and throws away (`REPORT_ASSOCIATED_TEXT` is a
    // commented-out line in its `event.rs`, in 0.28 and 0.29 alike, and its
    // `KeyCode::Char` could not carry a two-character commit anyway). For an
    // ASCII key that costs nothing, because the key *is* the text. For the
    // **system** input method it costs everything: 中文 committed with the
    // space bar arrives as `KeyCode::Char(' ')` — the commit key — and the
    // sentence is a row of spaces.
    //
    // **And it is the only flag that reports a bare Shift** ("Additionally,
    // with this mode, events for pressing modifier keys are reported" — the
    // protocol says it of that flag and of no other), so leaving it out cost
    // the lone-Shift tap, silently, from #271 until #290. It is now pushed and
    // popped in the loop, held exactly while yume has the keyboard, which is
    // exactly when the system's input method is not composing anyway.
    let enhanced = matches!(supports_keyboard_enhancement(), Ok(true));
    if enhanced {
        let _ = execute!(
            stdout(),
            PushKeyboardEnhancementFlags(
                KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                    | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                    | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
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
    // Whether the extra enhancement level — the one with
    // `REPORT_ALL_KEYS_AS_ESCAPE_CODES` in it — is on the terminal's stack.
    // Pushed and popped as the input method takes and gives back the keyboard,
    // so it has to be counted: popping one that was never pushed takes the
    // base level away with it.
    let mut all_keys = false;
    // What Insert's 中/英 was when a prompt borrowed it (#225). See [`Borrow`].
    let mut borrowed = Borrow::default();
    // A typesetter started with `:view-preview`, if one is running — and one left
    // behind by a session that ended badly, which is stopped before this one
    // can start another.
    let mut job: Option<Job> = None;
    if let Some(said) = adopt_an_orphan() {
        editor.set_status(said);
    }
    // **Say once that the lone-Shift tap is not coming** (#339). Without the
    // Kitty protocol a bare Shift is never reported, so the gesture a writer
    // uses dozens of times an hour does nothing at all — and says nothing
    // either, which reads as yumete being broken rather than as the terminal
    // having no way to tell us. Said at start-up and not repeated: it is a
    // fact about the terminal, and it will not change while this session runs.
    // It gives way to an adopted orphan and to whatever opening the files had
    // to say — those are about this run, and this is about the whole session.
    if !enhanced && editor.status().is_empty() {
        editor.set_status(match language_key(config) {
            Some(_) => say!("ime.no-lone-shift", config.editor.language_key.trim()),
            None => say!("ime.no-lone-shift-off"),
        });
    }
    let mut last_mode = None;
    // **讓開** — the system's own input method, held aside while yumete wants
    // the keys (see [`system_ime`]). Its `Drop` puts the input source back, so
    // leaving this function restores it however it is left, a panic included.
    let mut system_ime = crate::system_ime::SystemIme::new(config.ime.system);
    // The colour the terminal draws its own cursor in — ours to set, and the
    // one thing on the screen the palette could not reach (#493).
    let mut last_caret: Option<((u8, u8, u8), (u8, u8, u8))> = None;
    // An event read ahead of its turn and handed back — see [`drain_the_flick`].
    // The queue is the terminal's, not ours, and this is the one place anything
    // is ever taken out of order: the rest of a wheel gesture, read early so it
    // can be drawn once instead of once a notch.
    // **Read on a thread of its own** (#360). See `spawn_reader`.
    let events = spawn_reader();
    let mut inbox: std::collections::VecDeque<Event> = std::collections::VecDeque::new();
    let mut painted = std::time::Instant::now();
    // What the last frame cost — the terminal's answer to how fast it can be fed.
    let mut last_frame = std::time::Duration::ZERO;
    // The frame `:shot` will photograph, kept only when one was asked for.
    let mut drawn: Option<ratatui::buffer::Buffer> = None;
    // **自動認詞** (#448): the book's own names, found without being asked.
    // The counting runs on a thread and the answer arrives here; nothing waits
    // for it, and until it lands the page is segmented by the language model
    // alone. **This file only** (#452) — the wider reads are the three
    // `:word-discover-*` spellings, asked for by hand.
    // The words, and how many 漢字 were read to find them — the cap is a share
    // of the second, not a constant (`discover::cap`).
    let (found_tx, found_rx) =
        std::sync::mpsc::channel::<(Vec<yumete_core::discover::Found>, usize)>();
    let mut detecting = false;
    let mut detected: Option<(String, std::time::Instant)> = None;

    let result = loop {
        // **Extend is a mode as far as the cursor is concerned** (2026-09-12).
        // helix's `[editor.cursor-shape]` has three slots — normal, insert and
        // **select** — and its default theme paints the select cursor its own
        // colour (`ui.cursor.primary.select`). yumete's ladder cannot spare a
        // rung for it: HEAD 815 and SELECTION 700 are already only 5% apart, so
        // a third ground between them would be a difference nobody can see. The
        // shape is free, and the terminal draws it.
        // ⚠️ **The terminal's cursor is not painted by the palette** (#493).
        // Horizontally the caret *is* the terminal's own — we only choose its
        // shape — and its colour comes from the reader's terminal profile,
        // which does not know the page turned light: 「浅色模式肉眼很难找到光标
        // 的位置」. A white block on cream is invisible, and it is invisible
        // exactly where the reader is typing. So the page says what colour its
        // caret is (OSC 12), the way it already says what is on the clipboard
        // (OSC 52) — and puts the profile's own colour back on the way out
        // (OSC 112), because the cursor outlives the process.
        //
        // 縱書 does not need this: there the block is drawn *into* the page
        // with `REVERSED`, which takes its two colours from the cell it covers
        // and is therefore right in both moods by construction.
        //
        // ⚠️ **And the page has to say its own two colours as well** (#502).
        // A terminal draws the character *under* a block cursor in its own
        // configured background, not in the cell's — so a dark profile under a
        // light page gave a black block with a black character in it, and the
        // 字 the caret was standing on could not be read at all: 「Light 的光标
        // 是墨色的，但至少字应该是纸色吧。。。不然怎么看？？」 There is no
        // per-cell control over that, and no need for one: OSC 10 and 11 tell
        // the terminal what this page's ink and paper *are*, and then every
        // answer it works out for itself — the character under the cursor, the
        // padding round the grid — agrees with the page. Put back on the way
        // out (110/111/112), because all three outlive the process.
        let ink = crate::theme::Palette::of(config);
        let colours = (ink.caret(), ink.paper_bytes());
        if last_caret != Some(colours) {
            last_caret = Some(colours);
            let ((ir, ig, ib), (pr, pg, pb)) = colours;
            let _ = write!(
                stdout(),
                "\x1b]10;#{ir:02X}{ig:02X}{ib:02X}\x07\
                 \x1b]11;#{pr:02X}{pg:02X}{pb:02X}\x07\
                 \x1b]12;#{ir:02X}{ig:02X}{ib:02X}\x07"
            );
        }
        // **Is yumete going to read this key itself?** Wherever 中文 does not
        // belong the key is a command and the answer is always yes; where it
        // does, only if yume can actually type it — which is the same pair the
        // status line stands on (`standing_language_tag`) and the same pair
        // the keyboard flags are pushed for, two lines apart on purpose.
        //
        // ⚠️ **`composes_here`, not `mode != Normal`.** The narrow test cost
        // an evening twice over. `[ime] start` is `false` out of the box, so a
        // fresh session is engaged with **no 碼表**, and a first version that
        // asked only `engaged` took the system's input method away from a
        // writer whose yumete could not type 漢字 either: 「你把输入法切走了，
        // 我 i 进入 insert 模式用什么？」 A second version then handed the
        // keyboard back in *every* mode that is not Normal — including the
        // command line, where the command **name** is ASCII, so `:w` had its
        // `w` composed by the system's input method and the file was never
        // written. `composes_here` is the one gate that already knows all of
        // this: the preedit, the candidate panel and the lone-Shift tap light
        // up together with it, and now so does this.
        let yume_has_the_keys = ime.available() && ime.engaged();
        system_ime.want(!composes_here(editor) || yume_has_the_keys);
        let shown = (editor.mode(), editor.is_extending());
        if last_mode != Some(shown) {
            let (mode, extending) = shown;
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
                match (mode, vertical, extending) {
                    (Mode::Insert, false, _) => SetCursorStyle::SteadyBar,
                    (Mode::Insert, true, _) => SetCursorStyle::SteadyUnderScore,
                    // `v` is on: every motion from here widens the selection,
                    // and the caret says so before the status line's tail does.
                    // **Steady, not blinking.** A blink is a second signal for
                    // nothing — the writer is reading their own prose while
                    // this is on, and a flashing caret is the wrong thing to
                    // have on the page.
                    (_, _, true) => SetCursorStyle::SteadyUnderScore,
                    _ => SetCursorStyle::SteadyBlock,
                }
            );

            // Leaving the command line gives Insert its 中/英 back (#225) —
            // *its* state, not 中文 unconditionally. Forcing 中文 back on put
            // the borrow only one way round: a command line opened from 英,
            // during which something turned 中 on, handed Insert a language it
            // never had.
            // `:` and `::` are one prompt for this purpose (#224): stepping
            // between them is not leaving the command line, and handing Insert
            // its language back on the way *in* to `::` would put 中文 on a
            // line that is about to be typed in 英.
            // **Asked, not listed** (#351). `/`, the ruby reading and the
            // picker are prompt lines too, and the list that used to be
            // written out here left all three of them out (#340): `/` typed
            // straight after a 中文 name kept the composition running, and
            // gave nothing back on the way out either.
            if ime.available() {
                let door = borrowed.crossing(last_mode.map(|(m, _)| m), mode, ime.is_chinese());
                if door.escape && ime.is_composing() {
                    ime.escape();
                }
                if door.toggle {
                    ime.toggle_language();
                }
            }
            last_mode = Some(shown);
        }
        // Where the loop is, for the watchdog (#359). Two relaxed stores.
        diag::beat(diag::Stage::Measuring, 0);
        // The 縱 wrap length depends on the terminal height, and the motions
        // that cross 縱 run before the next draw, so settle it up front.
        if let Ok(size) = terminal.size() {
            // The page's own rectangle, not the terminal's: the command row, the
            // tab bar and the detail panel are not writing, and a 縱 measured
            // against them is one longer than the 縱 on the screen.
            // …and the half of it the **keys** are in: with a split open the
            // whole text area is twice the page, so `C-f` turned two pages and
            // `C-d` moved a whole pane instead of half of one.
            let areas = page_areas(editor, config, Rect::new(0, 0, size.width, size.height), 0);
            let page = areas.panes[editor.live_pane().min(1)];
            let lines = editor.current_buffer().line_count();
            // **Where the page starts**, from the scroll this loop already
            // keeps (#378): a table's columns are measured against the rows on
            // the screen, and this is the only place that knows which those
            // are. Last frame's, necessarily — it is settled while drawing.
            editor.set_page_top(viewport[editor.live_pane().min(1)].top.line);
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
        // candidate is on the page — which is the whole reason #210 made drawn
        // text an input to the layout rather than something painted over it.
        settle_inline_candidate(editor, ime);
        // …and 字典 asked about a character (#215). The 拆分表 is yume's, not
        // the editor's, so the question is parked in the editor and answered
        // here — before the draw, so the panel is filled on the very frame the
        // key opened it.
        if let Some(ch) = editor.take_dictionary_query() {
            let found = ime.glosses(ch);
            editor.set_dictionary(ch, found);
        }
        // The frame is drawn **and kept**: `draw` hands back the buffer it just
        // filled, and that is the only honest way to reach it — see the picture
        // below. The copy costs one walk of the screen per drawn frame, which
        // is what ratatui already spends diffing the two buffers.
        //
        // **A frame per queued keystroke is a frame nobody sees** (#314). The
        // wheel has coalesced its notches since #282 (`drain_the_flick`, below);
        // keys never did, so once a keystroke costs more than the interval
        // between repeats the redraws queue up behind the finger and the cursor
        // goes on moving after it lifts. Nothing here is dropped — the events
        // are still read and still handled — it is only the *picture* between
        // two of them that nobody was going to see. `FRAME_FLOOR` keeps a held
        // key from holding the page still.
        take_what_arrived(&events, &mut inbox);
        let waiting = !inbox.is_empty();
        // The picture is wanted only when somebody asked for one, and the copy
        // is a walk of the whole screen: 24 µs at 120×40, 94 µs at 400×100,
        // every frame, for a `:shot` almost nobody presses. Asked before the
        // draw and used after it — the request was made a keystroke ago, so
        // this frame is the one it means either way.
        let wants_picture = editor.take_screenshot_request();
        // A frame somebody is about to photograph is never the frame to skip:
        // skipping it hands `:shot` a blank page.
        // What the last frame cost, and how long that buys the backlog.
        let floor = (last_frame * FRAME_SLACK).clamp(FRAME_FLOOR, FRAME_CEILING);
        if wants_picture.is_some() || !waiting || painted.elapsed() >= floor {
            // The detail is the last frame's cost in milliseconds: if the loop
            // stalls here, the stall line says whether drawing was already
            // expensive before it stopped (#359).
            diag::beat(diag::Stage::Drawing, last_frame.as_millis() as u64);
            let began = std::time::Instant::now();
            let completed = match terminal.draw(|frame| draw(frame, editor, config, ime, &mut viewport)) {
                Ok(completed) => completed,
                Err(err) => break Err(err),
            };
            last_frame = began.elapsed();
            painted = std::time::Instant::now();
            drawn = wants_picture.is_some().then(|| completed.buffer.clone());
        }
        // …and the picture is of *this* frame, which is the one with no
        // command line across it.
        if let Some(job) = wants_picture {
            let drawn = drawn.take().unwrap_or_default();
            let said = match job {
                ShotJob::Screen => photograph_the_screen(config, None),
                // Not the frame: the same program `:shot` uses, told where to
                // put the picture instead of filling the clipboard.
                ShotJob::Png { target } => photograph_the_screen(config, Some(&target)),
                // **The buffer `draw` handed back**, not `current_buffer_mut`.
                // ratatui swaps its two buffers at the end of every `draw` and
                // resets the one it swaps in, so the *current* buffer here is
                // blank paper — which is exactly what `:shot txt` wrote from
                // #189 until this line: a page of empty rows and a footer.
                // Drawing a second time is no answer either: that picture is of
                // the state *after* this frame, and waiting a frame was the
                // whole point.
                ShotJob::Page { target, text } => {
                    let written = match text {
                        true => buffer_to_text(&drawn),
                        false => buffer_to_html(&drawn),
                    };
                    match yumete_core::buffer::write_file_atomically(&target, &written) {
                        Ok(()) => say!("ui.shot-drawn", target.display()),
                        Err(err) => say!("ui.shot-failed", err),
                    }
                }
            };
            editor.set_status(said);
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
            // The 拆分表 arrives with that same 14 MB, so `:ruby-auto` has
            // something to say only from here on.
            editor.set_reader(Box::new(ime.reader()));
            continue;
        }
        // **`:reload-auto` needs a clock, not a keystroke.** `event::read`
        // blocks, so the case the setting is for — alt-tab away, run a script,
        // come back and look — produced nothing at all until a key was pressed
        // (Feature #214). Only while it is on: an editor that wakes up twice a
        // second for nobody is an editor that flattens a battery.
        // **The flag that reports a bare Shift is held exactly while yume has
        // the keyboard** (#290). It is the same flag that stops the *system's*
        // input method from composing (#271), so it cannot simply stay on; and
        // a lone Shift is invisible without it, so it cannot simply stay off.
        // Engagement is the line between those two, which is the whole reason
        // 「ABC」 and 「關」 are two states and not one.
        //
        // **Crossing that line is a command, not a gesture** — settled
        // 2026-09-08: 「空格快捷键太宝贵了……我建议还是做成 command。」 A chord
        // was tried and taken back out: `C-Space` is spent twice over by
        // macOS, and `Shift+Space` is 全／半角 in most system input methods,
        // which are the ones holding the keyboard while yume is 關 —
        // so the one direction that matters most is the one it could not
        // be relied on for. `:yume on|abc|off` says it, and the lone-Shift
        // tap — the switch a writer actually reaches for — needs no key of
        // its own at all.
        if enhanced {
            let want = ime.available() && ime.engaged();
            if want != all_keys {
                let _ = match want {
                    true => execute!(
                        stdout(),
                        PushKeyboardEnhancementFlags(
                            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                                | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
                                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
                        )
                    ),
                    false => execute!(stdout(), PopKeyboardEnhancementFlags),
                };
                all_keys = want;
            }
        }
        // **自動認詞, off the front of the loop** (#448). Opening a file
        // asks for a scan; whether one actually runs is decided here,
        // because only this side knows one is already running and how
        // long ago the last one finished.
        //
        // ⚠️ **Nothing is said and nothing is opened.** The whole point
        // is that the writer's own words work without being asked for,
        // and a scan nobody asked for must not take the screen, the
        // status line or a tab. `:word-discover` is the one that talks.
        if let Some(ask) = editor.take_detect_request() {
            // ⚠️ **The path, not the name** (#466). `display_name()` is the
            // file name alone, so a novel laid out as `卷一/ch01.md` and
            // `卷二/ch01.md` gave both volumes one key: opening the second
            // within five minutes of the first counted as 「already scanned」
            // and kept the first volume's words. Every unnamed buffer shared
            // `[scratch]` too.
            let name = editor
                .current_buffer()
                .path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| editor.current_buffer().display_name());
            let due = match &detected {
                // Another file is always due: its words are not the ones in
                // hand, and 自動認詞 reads the file being written.
                Some((was, _)) if *was != name => true,
                Some((_, when)) => when.elapsed() >= DETECT_AGAIN,
                None => true,
            };
            if !detecting && due {
                detecting = true;
                detected = Some((name, std::time::Instant::now()));
                let tx = found_tx.clone();
                std::thread::spawn(move || {
                    // The bundled dictionary, built here: 8 ms, and it keeps
                    // the thread from touching anything the editor owns. It
                    // knows less than 宇浩's own table, which only leaves a few
                    // words in the answer that are already joined — filtered
                    // below, where the segmenter in force can be asked.
                    // ⚠️ **The flag is cleared by this guard, not by the
                    // send** (#476). `found_tx` is held by the main loop, so a
                    // worker that panicked would never disconnect the channel:
                    // `try_recv` would answer `Empty` for ever and 自動認詞
                    // would be off for the rest of the session with nothing
                    // said. Dropping this sends an empty answer, which the
                    // receiver ignores and which unlatches the flag.
                    let _guard = Unlatch(tx.clone());
                    let seg = yumete_core::DictionarySegmenter::builtin(0);
                    let joins =
                        |w: &str| yumete_cjk::Segmenter::segment(&seg, w).len() == 1;
                    // **This file first, and it keeps its half whatever the
                    // folder says** (#453). Two passes rather than one corpus:
                    // read together, a folder's commonest strings take the cap
                    // and its longer ones absorb this file's — measured, 「宇夢」
                    // goes from 8th of 60 to gone. The words of the chapter
                    // being written are the ones the writer is looking at.
                    let mine = yumete_core::discover::han_count(&ask.text);
                    let mut out = yumete_core::discover::words(&ask.text, &joins);
                    // **Half the list is this file's, and the list is a share
                    // of what was read** — 200 for a chapter, a thousand for a
                    // novel (`discover::cap`).
                    let mut han = mine;
                    out.truncate(yumete_core::discover::cap(mine));
                    if let Some(folder) = &ask.folder {
                        let (near, _, near_han) =
                            yumete_core::editor::detect_words_in(folder, &joins);
                        han += near_han;
                        let room = yumete_core::discover::cap(han);
                        for found in near {
                            if out.len() >= room {
                                break;
                            }
                            if !out.iter().any(|f| f.word == found.word) {
                                out.push(found);
                            }
                        }
                    }
                    let _ = tx.send((out, han));
                });
            }
        }
        // …and the answer, whenever it turns up. **Not waited for**: the loop
        // blocks on the terminal, so a scan that finishes while nobody is
        // typing lands on the next turn — the next key, click or resize — and
        // that is the first moment it could have mattered anyway.
        if let Ok((found, _han)) = found_rx.try_recv() {
            detecting = false;
            install_detected(editor, &found);
        }
        if editor.reload_auto() && inbox.is_empty() {
            match events.recv_timeout(DISK_POLL) {
                Ok(Ok(event)) => inbox.push_back(event),
                Ok(Err(err)) => break Err(err),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    editor.disk_tick();
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
            }
        }
        diag::beat(diag::Stage::Reading, 0);
        let arrived = match inbox.pop_front() {
            Some(event) => Ok(event),
            // **Wait with a deadline when, and only when, something is owed.**
            // `autosave_due_in` is `None` unless a crash right now would take
            // writing the recovery copy does not have — so an idle editor with
            // nothing unsaved still blocks forever and costs nothing, and one
            // that has just been typed in wakes once, writes, and goes back to
            // blocking. That one wake is the trailing edge the throttle never
            // had (#383): the last few seconds of typing used to be written
            // only by the *next* keystroke, and pausing to think meant there
            // was no next keystroke.
            None => match editor.autosave_due_in() {
                None => match events.recv() {
                    Ok(outcome) => outcome,
                    // The reader is gone, which is the terminal saying it is done.
                    Err(_) => break Ok(()),
                },
                Some(wait) => match events.recv_timeout(wait) {
                    Ok(outcome) => outcome,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        editor.autosave_tick();
                        // One frame is redrawn on the way round — the loop
                        // draws at its head — and exactly one, because writing
                        // the copy makes the draft no longer stale and the next
                        // pass goes back to blocking with no deadline at all.
                        continue;
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break Ok(()),
                },
            },
        };
        // **One place, on the way in.** Anything that is not a key and is not a
        // wheel notch is about to move the caret — a click, a drag, a paste —
        // and a half-typed word must be ended *before* it does (#336). Put at
        // the branches instead, this was three call sites and two of them were
        // missing; put here, an event kind added tomorrow is covered the day it
        // is added. (#350 is the general form of that.)
        if let Ok(event) = &arrived {
            let moves_the_caret = match event {
                Event::Paste(_) => true,
                Event::Mouse(mouse) => !matches!(
                    mouse.kind,
                    MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                ),
                _ => false,
            };
            if moves_the_caret {
                end_the_composition(ime, editor);
            }
        }
        match arrived {
            Ok(Event::Key(key)) => {
                // A lone-Shift tap toggles 中/英 in Insert mode; other Shift
                // activity is swallowed so it never reaches the editor.
                match shift.update(&key) {
                    ShiftResult::Tap(tapped) => {
                        if composes_here(editor) && ime.available() && ime.engaged() {
                            // The same answer as `:yume on`, given by the hand
                            // rather than by the command line, so it ends the
                            // borrow the same way (#225) — but only when the
                            // hand is answering about Insert (#338).
                            borrowed.answered_in(editor.mode());
                            // **What Shift means here is yume's to say.** On an
                            // empty buffer the factory value is the 中/英
                            // toggle; mid-code it commits the raw code first,
                            // so a half-typed 拆分 lands in the manuscript
                            // instead of vanishing. Rebind it in yume and this
                            // follows, because nothing here decides it.
                            if ime.press_modifier(tapped) {
                                editor.insert_committed(&ime.take_committed());
                            }
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
                // The stand-in for a Shift this terminal cannot report (#339).
                // It asks yume the same question the tap does, so a rebound
                // Shift is rebound here too; outside a place that composes
                // there is nothing to commit, so it is only the switch.
                if ime.available()
                    && ime.engaged()
                    && language_key(config).is_some_and(|k| is_language_key(k, code, mods))
                {
                    borrowed.answered_in(editor.mode());
                    match composes_here(editor) {
                        true => {
                            if ime.press_modifier(FuncKey::ShiftL) {
                                editor.insert_committed(&ime.take_committed());
                            }
                        }
                        false => ime.toggle_language(),
                    }
                    continue;
                }
                let consumed = composes_here(editor)
                    && ime.available()
                    && ime.engaged()
                    && ime_handle(ime, editor, code, mods);
                if !consumed {
                    if let Some(k) = map_key(code, mods) {
                        // Kept for the crash report, and for nothing else: a
                        // push of a `Copy` enum into a ring of 64 (#300).
                        diag::note_key(k);
                        diag::beat(diag::Stage::Key, 0);
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
                // `:word-list reload` — the dictionary is the front end's to
                // build (it holds the IME and knows the data directory), and
                // the level the reader chose survives the rebuild.
                if editor.take_words_request() {
                    let level = editor.word_level();
                    editor.set_segmenter(choose_words(ime, level));
                    editor.set_status(say!("word.lists-reread", editor.words_in_force()));
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
                        // `:view-preview` — the real typesetter, in the background. Its
                // address arrives on a later turn of the loop.
                // `:sh` brings the answer back; `:!` hands over the screen.
                if let Some(want) = editor.take_shell_request() {
                    use yumete_core::editor::How;
                    match want.how {
                        How::Terminal => {
                            // The screen is about to belong to somebody
                            // else's program, which may well want a person to
                            // type into it. Give the system's input method
                            // back first; the next turn of the loop, which is
                            // after that program has finished, takes it again.
                            system_ime.release();
                            match hand_over(&mut terminal, &want.line, &events) {
                                Ok(()) => editor.set_status(say!("shell.finished", want.line)),
                                Err(err) => editor.set_status(say!("shell.cannot-run", err)),
                            }
                        }
                        // Showing you the run: the complaints belong with the
                        // answer, since between them they are what happened.
                        How::Capture => match run_capturing(&want.line, None) {
                            Ok(ran) => {
                                let mut text = ran.said;
                                text.push_str(&ran.complained);
                                editor.provide_shell_output(&want.line, &text);
                            }
                            Err(err) => editor.set_status(say!("shell.cannot-run", err)),
                        },
                        // Editing your text: a command that failed does not get
                        // to touch it. `tr -D ' '` is a typo, and its answer is
                        // an error message — replacing a paragraph with that is
                        // an edit nobody asked for, undoable or not.
                        How::Pipe(input) => match run_capturing(&want.line, Some(&input)) {
                            Ok(ran) if ran.ok => {
                                editor.provide_pipe_output(&ran.said);
                                if !ran.complained.trim().is_empty() {
                                    editor.set_status(say!("shell.replaced-with-complaints", ran.why()));
                                }
                            }
                            Ok(ran) => editor.set_status(say!("shell.left-alone", ran.why())),
                            Err(err) => editor.set_status(say!("shell.cannot-run", err)),
                        },
                        // Same rule as `Pipe`, for the same reason and with
                        // more riding on it: opencc exiting non-zero means the
                        // conversion did not happen, and a manuscript is not
                        // overwritten with an error message. Judged by the
                        // **exit code alone** — a converter that complains on
                        // stderr while succeeding is still a converter that
                        // succeeded.
                        How::Convert(input) => match run_capturing(&want.line, Some(&input)) {
                            Ok(ran) if ran.ok => editor.provide_conversion(&ran.said),
                            Ok(ran) => editor.set_status(say!("shell.left-alone", ran.why())),
                            Err(err) => editor.set_status(say!("shell.cannot-run", err)),
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
                // **A link the reader followed** (Feature #285). The editor
                // has already decided this is `http` or `https` and refused
                // everything else; all that is left is handing it over, as one
                // argument, with no shell anywhere in the path.
                if let Some(url) = editor.take_open_request() {
                    show(&url);
                }
                if let Some(want) = editor.take_preview_request() {
                    // Already running: hand back the address and open the page
                    // again. Killing it and starting another is a fresh compile
                    // of the whole book to answer 「where was that page?」.
                    if let yumete_core::editor::Preview::Show = want {
                        if let Some(url) = editor.preview_at().map(str::to_string) {
                            show(&url);
                            editor.set_status(say!("preview.running", url));
                        }
                        continue;
                    }
                    if let Some(mut running) = job.take() {
                        let _ = running.child.kill();
                        let _ = running.child.wait();
                        forget_the_server();
                        editor.set_preview_at(None);
                        editor.set_status(say!("preview.stopped", running.what));
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
                                        editor.set_status(say!("preview.starting", program));
                                        job = Some(started);
                                    }
                                    Err(why) => editor.set_status(why),
                                },
                                None => editor.set_status(say!("command.broken-line", runner.run)),
                            }
                            continue;
                        }
                        match syntax {
                            yumete_core::syntax::Syntax::Typst => match Job::typst(&path) {
                                Ok(started) => {
                                    editor.set_status(say!("preview.typst-starting"));
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
                                // `:view-preview` of a Markdown file failed, and
                                // said so in the language of a command the
                                // reader had not typed.
                                match editor.execute(&format!("export! html {}", out.display())) {
                                    Ok(_) => {
                                        show(&out.to_string_lossy());
                                        editor.set_status(say!("preview.made", out.display()));
                                    }
                                    Err(err) => editor.set_status(say!("preview.failed", err)),
                                }
                            }
                            // Nothing to typeset: a file with no markup is
                            // already what it is going to look like, and a
                            // `:diff` listing is a report about two of them.
                            yumete_core::syntax::Syntax::Text
                            | yumete_core::syntax::Syntax::Diff
                            | yumete_core::syntax::Syntax::Code(_) => {
                                editor.set_status(say!("preview.nothing-to-preview"))
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
                        editor.set_status(say!("preview.running", url));
                    }
                }
                if let Some(tag) = editor.take_scheme_request() {
                    // `:yume on` / `:yume abc` / `:yume off` **is** an answer
                    // about the language, typed on the command line that
                    // borrowed it. Putting the borrow back afterwards undid
                    // the command the writer had just run, silently, one
                    // keystroke later.
                    //
                    // **Spelled the way the request is spelled now** (#412).
                    // This asked for `+` and `-`, which is what the request
                    // was before there were three answers to give (#290) —
                    // and nothing has sent either since, so the undoing this
                    // line exists to prevent had quietly come back.
                    if answers_the_language(&tag) {
                        borrowed.settled();
                    }
                    // The page says what it is doing before it stops answering
                    // for a tenth of a second (#341).
                    if loading_the_table(&tag, ime) {
                        editor.set_status(say!("ime.loading"));
                        if let Err(err) =
                            terminal.draw(|frame| draw(frame, editor, config, ime, &mut viewport))
                        {
                            break Err(err);
                        }
                    }
                    // `:yume-where` answers with a page, not a line — see
                    // `where_report`. Everything else rides `switch_scheme`.
                    if tag == "where" {
                        let report = where_report(ime);
                        editor.open_report(&say!("yume.where.name"), &report);
                        editor.set_status(say!("yume.where.opened"));
                    } else {
                        editor.set_status(switch_scheme(ime, &tag, config));
                    }
                    editor.set_ime_available(ime.available());
                    // The scheme's own language data may be better than what
                    // was loaded before it.
                    let words = ime.segmenter();
                    if words.is_available() {
                        editor.set_segmenter(Box::new(words));
                    }
                    // A 自定義方案 may bring its own 拆分表, so the readings
                    // are re-read with the scheme.
                    editor.set_reader(Box::new(ime.reader()));
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
                if let Some(on) = editor.take_fill_request() {
                    let on = crate::theme::set_fill(on, config);
                    editor.set_status(match on {
                        true => say!("theme.fill-on"),
                        false => say!("theme.fill-off"),
                    });
                }
                if let Some(on) = editor.take_chaifen_request() {
                    let settled = ime.set_annotations(on);
                    editor.set_chaifen(settled);
                    editor.set_status(if settled {
                        say!("chaifen.on")
                    } else if ime.annotations_available() {
                        say!("chaifen.off")
                    } else {
                        say!("chaifen.unavailable")
                    });
                }
            }
            // Text arriving whole, from the system clipboard by way of the
            // terminal. It is inserted as writing, never run as keys.
            Ok(Event::Paste(text)) => {
                editor.paste_text(&text);
            }
            Ok(Event::Mouse(mouse)) => match mouse.kind {
                // A notch moves `[editor] wheel_step` 縱 — three by default,
                // the three lines a terminal scrolls by, counted in the unit
                // the page is set in and settable with `:wheel` (#222).
                //
                // **The whole flick is taken at once** (Feature #268). A wheel
                // sends one event per notch and a trackpad sends hundreds per
                // gesture; a frame drawn for each is a frame the terminal has
                // to paint, and a fast scroll down 資治通鑑 measured 3000
                // frames and 60 MB of escape sequences — long enough that the
                // reader's only way out was to kill the tab. Reading the rest
                // of the burst first turns a flick into one scroll and one
                // frame.
                MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
                    let back = mouse.kind == MouseEventKind::ScrollUp;
                    diag::beat(diag::Stage::Wheel, 0);
                    let notches = 1 + drain_the_flick(mouse.kind, &events, &mut inbox);
                    diag::beat(diag::Stage::Scrolling, notches as u64);
                    editor.scroll(editor.wheel_step() * notches, back);
                }
                // A tab is a thing you point at; the mouse is already captured
                // for the wheel, so this costs nothing but the arithmetic.
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(i) = tab_at(editor, config, terminal.size().ok(), mouse) {
                        editor.show_buffer_at(i);
                    } else if let Some(at) =
                        text_at(editor, config, terminal.size().ok(), &viewport, mouse)
                    {
                        editor.point_at(at);
                        // **Ctrl-click follows a link** (Feature #285) — the
                        // gesture every other editor already means by it, and
                        // the one modifier that survives a terminal. A bare
                        // click stays a bare click: a manuscript is clicked in
                        // all day, and a link that opened on one would open by
                        // accident all day.
                        if mouse.modifiers.contains(KeyModifiers::CONTROL) {
                            editor.follow_link_here();
                        }
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
            // The release is not coming: whatever was held when focus left is
            // not a tap any more. Upstream keeps a `reset` for exactly this,
            // and without calling it the *next* genuine tap is eaten.
            Ok(Event::FocusLost) => {
                shift.reset();
                // **And give the keyboard back** — the window in front is
                // somebody else's, and arriving there in a layout yumete
                // chose is worse than anything this feature prevents.
                system_ime.set_focus(false);
            }
            Ok(Event::FocusGained) => system_ime.set_focus(true),
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
    // **Give the terminal its own three colours back** (#493, #502). They
    // outlive the process, so a page that set them and did not put them back
    // leaves the reader's shell wearing yumete's ink and paper.
    let _ = write!(stdout(), "\x1b]110\x07\x1b]111\x07\x1b]112\x07");
    if enhanced {
        if all_keys {
            let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
        }
        let _ = execute!(stdout(), PopKeyboardEnhancementFlags);
    }
    ratatui::restore();
    result
}

/// Put the terminal back after a panic tore the loop down (#300).
///
/// `run`'s own teardown is at the end of `run`, which a panic goes straight
/// past — so the reader would be left in raw mode, inside the alternate
/// screen, with the mouse captured. Every step is best effort and none of them
/// mind being done twice.
pub fn restore_terminal() {
    let _ = execute!(
        stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        DisableFocusChange,
        PopKeyboardEnhancementFlags
    );
    ratatui::restore();
}

/// How long one gesture is swallowed before the page is drawn anyway.
///
/// A trackpad's momentum can go on sending for seconds after the finger has
/// left it. Without a ceiling the page would stay on the frame the flick
/// started at until the terminal's buffer ran dry.
///
/// **A count was the wrong unit** (2026-09-09). The ceiling used to be 64
/// notches, and a notch costs about 20 µs — so it bounded the work at 1.3 ms
/// while a hard flick queues *thousands* of events, which came back as three
/// hundred turns of the loop, each drawing a frame, long after the finger had
/// stopped. Measured on this project's own roadmap (`the_cost_of_a_flick`):
/// 1,024 notches is 20 ms of scrolling and 2 ms of drawing, so the work was
/// never the problem — the number of frames was. A time box bounds what
/// matters instead: spend at most this long swallowing what has arrived, then
/// act on all of it at once. Events are unbounded, latency is not.
const WHEEL_DRAIN: std::time::Duration = std::time::Duration::from_millis(12);

/// Read the terminal on a thread of its own, and hand the events over.
///
/// **The deadlock this breaks** (#360). `terminal.draw` *writes* — while
/// scrolling, every row changes, so the diff is the whole screen and that is a
/// hundred thousand characters of escape sequence. A terminal that has fallen
/// behind stops reading it, and the write blocks. Meanwhile the same terminal
/// is trying to write *to us*: a hard trackpad flick sends thousands of mouse
/// reports, and with nobody reading them that write blocks too — so it stops
/// reading ours, and both sides wait for the other. It takes a few gestures to
/// fill both buffers, which is exactly what 「it freezes on the fourth flick」
/// looked like, with the heartbeat stopped in `drawing` after a frame that had
/// cost 8 ms.
///
/// A reader that never draws cannot be blocked by drawing. The channel is
/// unbounded, so the terminal's write to us always completes and it goes on
/// consuming what we send.
fn spawn_reader() -> std::sync::mpsc::Receiver<io::Result<Event>> {
    let (tx, rx) = std::sync::mpsc::channel();
    // **What the reader typed before the editor was on screen goes first.**
    // The two start-up probes read the descriptor directly and used to throw
    // away everything that was not the terminal's reply — which is the first
    // four tenths of a second of the session. See [`crate::typed_ahead`] for
    // what is kept and what is deliberately not.
    for c in crate::typed_ahead::take() {
        let code = match c {
            '\r' | '\n' => KeyCode::Enter,
            '\t' => KeyCode::Tab,
            c => KeyCode::Char(c),
        };
        let event = KeyEvent::new(code, KeyModifiers::NONE);
        if tx.send(Ok(Event::Key(event))).is_err() {
            break;
        }
    }
    std::thread::spawn(move || loop {
        let outcome = event::read();
        let failed = outcome.is_err();
        // A send that fails means the loop has gone; so does an error, and it
        // is delivered first so the loop can say why.
        if tx.send(outcome).is_err() || failed {
            break;
        }
    });
    rx
}

/// Move everything the reader has ready into the inbox, without waiting.
fn take_what_arrived(
    events: &std::sync::mpsc::Receiver<io::Result<Event>>,
    inbox: &mut std::collections::VecDeque<Event>,
) {
    while let Ok(Ok(event)) = events.try_recv() {
        inbox.push_back(event);
    }
}

/// Take the rest of a wheel gesture off the queue, and say how many more
/// notches of the same direction it held.
///
/// Anything that is *not* that same scroll is left at the front of the inbox
/// for the next turn of the loop: a burst ends at the first event of any other
/// kind, so a click or a keystroke landing mid-flick is neither swallowed nor
/// reordered.
fn drain_the_flick(
    kind: MouseEventKind,
    events: &std::sync::mpsc::Receiver<io::Result<Event>>,
    inbox: &mut std::collections::VecDeque<Event>,
) -> usize {
    let began = std::time::Instant::now();
    let mut more = 0;
    loop {
        take_what_arrived(events, inbox);
        // However much has arrived, only this long is spent taking it. The
        // scroll that follows stops itself at the end of the buffer, so a
        // gesture nobody could have meant costs one document, not one queue.
        if began.elapsed() >= WHEEL_DRAIN {
            break;
        }
        match inbox.front() {
            Some(Event::Mouse(next)) if next.kind == kind => {
                inbox.pop_front();
                more += 1;
            }
            // Nothing waiting, or something that is not this gesture: a
            // gesture that has paused is over as far as the page is concerned.
            _ => break,
        }
    }
    more
}

/// How long an idle `:reload-auto` session waits before looking at the disk.
///
/// The same two seconds the core throttles at, so the wait and the throttle do
/// not beat against each other: one look per wake, and no wake at all while
/// the setting is off.
const DISK_POLL: std::time::Duration = std::time::Duration::from_secs(2);

/// The longest the page may go unrefreshed while input is still arriving (#314).
///
/// Skipping a frame when input is already waiting is what stops a held key from
/// queueing one full redraw per repeat — but skipping *every* frame would hold
/// the screen still for as long as the finger is down. This is the floor: at
/// worst the reader sees the page ten times a second while a key repeats.
///
/// **It is a floor, not the rule** (#360). A frame is not free and it is not
/// even mostly ours: `terminal.draw` computes the page in a millisecond or two
/// and then *writes it to a terminal*, and while scrolling every row changes,
/// so the diff is the whole screen — a hundred thousand characters of escape
/// sequence. If the emulator falls behind, that write **blocks**, and a fixed
/// floor then does exactly the wrong thing: forcing a frame every 100 ms when
/// one costs 200 ms spends the whole loop drawing and never drains the
/// gesture. So the real floor is whichever is longer, this or a multiple of
/// what the last frame actually cost — the page gives way to the backlog when
/// the terminal says it cannot keep up.
const FRAME_FLOOR: std::time::Duration = std::time::Duration::from_millis(100);

/// How many times the last frame's cost must pass before another is forced.
///
/// Four, so at most a quarter of a congested loop goes on drawing and the rest
/// goes on catching up with what the reader is still doing.
const FRAME_SLACK: u32 = 4;

/// …and however slow a frame was, no longer than this between two of them.
///
/// The other half of the same mistake: a frame that blocked for two seconds
/// would buy eight seconds of not drawing, and eight seconds of a page that
/// does not move is 「frozen」 whatever the loop is doing underneath. Half a
/// second is long enough to drain a gesture and short enough to still read as
/// a screen that is alive.
const FRAME_CEILING: std::time::Duration = std::time::Duration::from_millis(500);

/// Whether the IME may run for what is being typed **right now** (#225).
///
/// ⚠️ **The command line is ASCII from `:` to the end** (2026-09-16),
/// arguments included. It used to permit 中文 once the caret had walked past
/// the command's name — `:e 第三章.md` — and that was two things at once: a
/// rule the writer had to hold in their head, and a boundary the caret crossed
/// back and forth over, which with 模態掛起 means the system's input method is
/// told to stand down and stand up again with every step of `←`. One line, one
/// answer: 「命令模式完全不允許中文最好，這樣我們的 yume mode 也更加乾淨」.
///
/// Nothing is lost that has nowhere else to go: `::` (命令搜索) takes 中文 and
/// finds the command **by what it does**, the picker (`空格 f`) takes 中文 and
/// finds the file, and `/` takes 中文 and finds the words. Those are the three
/// places a Chinese name is actually typed.
fn composes_here(editor: &Editor) -> bool {
    // ⚠️ **The picker has two layers, and only one of them is typing**
    // (2026-09-17). `Mode::Picker` composes because the query takes 中文 — but
    // with the keys in the *list*, `j` and `k` walk it, and handing them to the
    // engine made them a code: 「我按了 space f 進入 picker，按 jk 他開始輸入」.
    if let Some(picker) = editor.picker() {
        return picker.typing();
    }
    if editor.mode().composes() {
        return true;
    }
    // `f`、`r`、`ms`、`mi`、`mr` 打中文 (§5.2.3 ②, #414): Normal mode, but
    // the next character is *text*. One line, because every gate in this file
    // asks this one question — the preedit, the panel and the lone-Shift tap
    // all light up together.
    editor.takes_a_character()
}

/// Is this scheme request an answer about the **language**, as against the
/// 碼表 (#412)?
///
/// The two sides of this are written in different files — the core spells the
/// request, the loop reads it — and they came apart once already, so
/// [`the_language_requests_are_spelled_the_way_the_loop_reads_them`] drives
/// the commands and reads what actually comes out.
fn answers_the_language(tag: &str) -> bool {
    tag.starts_with("lang:")
}

/// What Insert's 中/英 was when a prompt borrowed it (#225).
///
/// A prompt is one line of somebody else's text on top of the page, and the
/// language it wants is rarely the language the page was in. So the engine's
/// state is **borrowed**: taken on the way in, handed back on the way out.
/// `None` means nothing is owed — no prompt is open, or something said what
/// the language should be and that answer is not a state to put back.
#[derive(Default)]
struct Borrow {
    owed: Option<bool>,
}

/// What the engine is owed on a change of mode.
#[derive(Debug, Default, PartialEq, Eq)]
struct AtTheDoor {
    /// End a composition that the new line has no way to finish.
    escape: bool,
    /// Flip 中/英.
    toggle: bool,
}

impl Borrow {
    /// Crossing out of `was` into `now`, with the engine in `chinese`.
    ///
    /// `:` and `::` are **one** prompt for this purpose (Feature #224):
    /// stepping between them is not leaving the command line, and handing
    /// Insert its language back on the way *in* to `::` would put 中文 on a
    /// line that is about to be typed in 英.
    fn crossing(&mut self, was: Option<Mode>, now: Mode, chinese: bool) -> AtTheDoor {
        let was_prompt = was.is_some_and(Mode::is_prompt);
        if was_prompt && !now.is_prompt() {
            // Only one way round: a prompt opened from 英, during which
            // something turned 中 on, must not hand Insert a language it
            // never had — which is why the answer is compared, not applied.
            return match self.owed.take() {
                Some(owed) => AtTheDoor {
                    escape: false,
                    toggle: chinese != owed,
                },
                None => AtTheDoor::default(),
            };
        }
        if now.is_prompt() && !was_prompt {
            self.owed = Some(chinese);
            return AtTheDoor {
                escape: true,
                toggle: now.prompt_opens_in_english() && chinese,
            };
        }
        AtTheDoor::default()
    }

    /// The language was answered outright — a lone-Shift tap, or `:yume on`.
    ///
    /// **Only when the answer is about Insert** (#338). A tap on an open
    /// prompt changes that one line and nothing else; forgetting the borrow
    /// there threw away the way back halfway through the trip, and 英 Insert
    /// came out of `:e 第三章.md` speaking 中文 for the rest of the session.
    fn answered_in(&mut self, mode: Mode) {
        if !mode.is_prompt() {
            self.owed = None;
        }
    }

    /// The language was answered by a command typed on the prompt itself.
    ///
    /// `:yume on` / `:yume off` **is** an answer about the language, and it
    /// was typed on the very line that borrowed it. Putting the borrow back
    /// afterwards undid the command the writer had just run, one keystroke
    /// later and without a word.
    fn settled(&mut self) {
        self.owed = None;
    }
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
    /// with `:view-wrap off` a paragraph is one row of any length, and without this
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

/// The key that stands in for a lone-Shift tap on a terminal that cannot report
/// one (#339), as `[editor] language_key` names it.
///
/// `C-<char>`, `A-<char>`, `F1`…`F12`, `off`. Anything else is `None` — a
/// misspelt key is no key, and the start-up line says which key there is.
fn language_key(config: &Config) -> Option<(KeyCode, KeyModifiers)> {
    let name = config.editor.language_key.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("off") {
        return None;
    }
    if let Some(n) = name
        .strip_prefix(['F', 'f'])
        .and_then(|d| d.parse::<u8>().ok())
    {
        return (1..=12).contains(&n).then_some((KeyCode::F(n), KeyModifiers::NONE));
    }
    let (mods, rest) = match name.split_once('-')? {
        ("C" | "c", rest) => (KeyModifiers::CONTROL, rest),
        ("A" | "a", rest) => (KeyModifiers::ALT, rest),
        _ => return None,
    };
    let mut chars = rest.chars();
    let c = chars.next()?;
    chars
        .next()
        .is_none()
        .then(|| (KeyCode::Char(control_alias(c, mods)), mods))
}

/// The one name a control chord has, whichever protocol delivered it.
///
/// A legacy terminal sends `Ctrl+^` as the single byte `0x1E`, and crossterm
/// reads `0x1C`–`0x1F` back as `4`–`7` — so on Apple Terminal `C-^` *is* `C-6`,
/// and the terminal this whole feature exists for is exactly that one. Under
/// the Kitty protocol the same chord arrives as `^`. Both are folded here, so
/// the configured name matches either way.
fn control_alias(c: char, mods: KeyModifiers) -> char {
    if !mods.contains(KeyModifiers::CONTROL) {
        return c;
    }
    match c {
        '\\' => '4',
        ']' => '5',
        '^' => '6',
        '_' => '7',
        _ => c.to_ascii_lowercase(),
    }
}

/// Whether this keystroke is the [`language_key`].
fn is_language_key(want: (KeyCode, KeyModifiers), code: KeyCode, mods: KeyModifiers) -> bool {
    // Shift is not compared: it is how `^` is typed in the first place.
    let real = |m: KeyModifiers| m & (KeyModifiers::CONTROL | KeyModifiers::ALT);
    if real(want.1) != real(mods) {
        return false;
    }
    match (want.0, code) {
        (KeyCode::Char(a), KeyCode::Char(b)) => a == control_alias(b, mods),
        (a, b) => a == b,
    }
}

/// The result of feeding a key event to the lone-Shift-tap tracker.
enum ShiftResult {
    /// A lone tap of this modifier completed — ask yume what it means.
    Tap(FuncKey),
    /// A Shift key event that isn't a completed tap; swallow it.
    Consumed,
    /// Not a Shift key event; handle it normally.
    Pass,
}

/// Detects a *lone* Shift tap (press then release with no other key in between).
///
/// **The state machine is yume's** ([`ModifierTap`]), not a fourth copy of it.
/// This is only the part that is genuinely terminal: turning crossterm's
/// `KeyEvent` into the three questions upstream asks — which modifier, down or
/// up, and were any *other* real modifiers held at that moment. Two things fell
/// out of adopting it: left and right Shift are tracked apart (one shared
/// `down` flag let `LeftShift↓ RightShift↓ RightShift↑` fire a toggle with the
/// left one still held), and losing a release — ⌘-Tab away with Shift down —
/// is recoverable, because upstream gives a [`ModifierTap::reset`] for exactly
/// that and the loop calls it on `FocusLost`.
///
/// Requires the Kitty keyboard protocol so bare modifier presses and releases
/// are reported at all.
#[derive(Default)]
struct ShiftTap(ModifierTap);

impl ShiftTap {
    fn update(&mut self, key: &KeyEvent) -> ShiftResult {
        let shift = match key.code {
            KeyCode::Modifier(ModifierKeyCode::LeftShift) => Some(FuncKey::ShiftL),
            KeyCode::Modifier(ModifierKeyCode::RightShift) => Some(FuncKey::ShiftR),
            _ => None,
        };
        // 「Any *other* real modifier held right now」 — Shift itself does not
        // count, and crossterm reports it on the Shift event's own modifiers.
        let others = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        match (shift, key.kind) {
            (Some(key), KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.0.modifier(key, true, others);
                ShiftResult::Consumed
            }
            (Some(key), KeyEventKind::Release) => match self.0.modifier(key, false, others) {
                Some(tapped) => ShiftResult::Tap(tapped),
                None => ShiftResult::Consumed,
            },
            // Any other key press while Shift is held taints the tap.
            (None, KeyEventKind::Press | KeyEventKind::Repeat) => {
                self.0.other_key();
                ShiftResult::Pass
            }
            (None, KeyEventKind::Release) => ShiftResult::Pass,
        }
    }

    /// The release is not coming — the terminal lost focus while a key was down.
    fn reset(&mut self) {
        self.0.reset();
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
    // ⚠️ **The manuscript goes down the pipe on its own thread.** Writing it
    // here and only then reading the answer deadlocks on anything longer than
    // a pipe buffer: a filter that reads and writes as it goes (`sed`, `cat`,
    // `opencc` — nearly all of them) fills its stdout, blocks on the write,
    // and stops draining its stdin, while this side is still blocked filling
    // it. Both wait forever, **on the main thread** — the screen freezes, no
    // key answers, `:w` cannot be typed, and only `kill -9` ends it, which
    // skips the panic handler and so writes no recovery copy. Measured before
    // the fix: `:!cat` hung at ~150 KB, `:!sed` at ~200,000 characters,
    // `:convert` with real opencc at 3.1 MB. `sort` never hung, which is why
    // the manual's own example was safe and this stayed hidden.
    //
    // Writing on a thread while the parent drains stdout means neither side
    // can be the one that stops reading.
    let feeder = match (input, child.stdin.take()) {
        (Some(text), Some(mut pipe)) => {
            let text = text.to_string();
            Some(std::thread::spawn(move || {
                // Written and *closed* — a filter still waiting for more input
                // never gets round to answering. `pipe` is dropped at the end
                // of the closure, which is the close.
                io::Write::write_all(&mut pipe, text.as_bytes())
            }))
        }
        // Nothing to feed, or the child took no stdin: nothing to close either.
        _ => None,
    };
    let out = child.wait_with_output()?;
    if let Some(feeder) = feeder {
        // A filter is allowed to stop reading early (`head -1` does), and the
        // broken pipe that follows is not the writer's failure — the exit
        // status is what says whether the run worked. A panic in the thread is
        // not reachable from `write_all`, and treating it as a failed run is
        // the safe reading if it ever were.
        match feeder.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) if err.kind() == io::ErrorKind::BrokenPipe => {}
            Ok(Err(err)) => return Err(err),
            Err(_) => return Err(io::Error::other("could not feed the command")),
        }
    }
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
            .map(str::to_string)
            .unwrap_or_else(|| say!("shell.failed"))
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
    events: &std::sync::mpsc::Receiver<io::Result<Event>>,
) -> io::Result<()> {
    let _ = execute!(
        stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        DisableFocusChange
    );
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
    println!("{}", say!("shell.press-any-key"));
    let _ = io::Write::flush(&mut stdout());
    ratatui::crossterm::terminal::enable_raw_mode()?;
    // Anything at all: this is "I have read it", not a command.
    //
    // **From the reader, not from the terminal.** Since #360 a thread of its
    // own is the only thing reading stdin, so a second reader here would race
    // it for the key — and lose, because the thread is already blocked in
    // `read`. This waited forever the first time it was tried.
    loop {
        match events.recv() {
            Ok(Ok(Event::Key(key))) if is_actionable(key.kind) => break,
            Ok(Ok(_)) => {}
            // The terminal has nothing more to say; do not wait for a key that
            // is not coming.
            Ok(Err(_)) | Err(_) => break,
        }
    }
    execute!(
        terminal.backend_mut(),
        ratatui::crossterm::terminal::EnterAlternateScreen
    )?;
    let _ = execute!(
        stdout(),
        EnableMouseCapture,
        EnableBracketedPaste,
        // Asked for so a lost Shift release can be noticed: ⌘-Tab away with a
        // modifier down and the release never arrives (see `ShiftTap`).
        EnableFocusChange
    );
    terminal.clear()?;
    status.map(|_| ())
}

/// Answer a `:theme`, and say where things stand afterwards.
///
/// A theme is two questions — which one, and dark or light — and either may be
/// left out, so a bare `:theme` changes nothing and reports. The config's own
/// `[theme]` is not rewritten: this is for the afternoon the room gets bright,
/// and the file is for what you want every day.
pub fn set_theme(
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
            None => return say!("theme.no-such-theme", asked),
        }
    }
    if let Some(mood) = mood {
        crate::theme::choose_mood(match mood {
            Mood::Dark => true,
            Mood::Light => false,
            // Back to whatever the terminal said at start-up; a terminal that
            // never answered keeps what the config settled on.
            Mood::System => crate::theme::terminal_answer().unwrap_or(crate::theme::dark()),
        });
    }
    let mood = match crate::theme::dark() {
        true => say!("cmd.moods.dark"),
        false => say!("cmd.moods.light"),
    };
    say!("theme.set", crate::theme::name(config), mood)
}

/// Photograph the screen — onto the clipboard (`:shot`), or into `dest`
/// (`:shot png`).
///
/// The window system's job, so it is a shell line in the config rather than
/// something built in here — and it is run **without** giving up the terminal,
/// because handing the screen over is exactly what would spoil the picture.
///
/// This photographs the **window**: its title bar, its tab strip and whatever
/// is in front of it. `:shot html` draws the page instead (#189), which is the
/// one a review or a bug report about the *text* wants.
///
/// **One config line does both**, because they are one decision — where the
/// window is and how to crop to it — and a second line would be the same
/// `osascript` with a different tail, kept in step by hand. Where the picture
/// goes is passed in the environment as `$YUMETE_SHOT`, which the shipped line
/// spends as `"${YUMETE_SHOT:--c}"`: a path when there is one, and
/// `screencapture`'s own 「onto the clipboard」 flag when there is not. A line
/// somebody wrote themselves before `:shot png` existed does not know that
/// name, so the file is looked for afterwards and its absence is said out
/// loud rather than reported as a picture that was never written.
fn photograph_the_screen(config: &Config, dest: Option<&std::path::Path>) -> String {
    let line = config.editor.screenshot.trim();
    if line.is_empty() {
        return say!("ui.no-screenshot-command");
    }
    let mut command = shell_command(line);
    match dest {
        Some(path) => command.env("YUMETE_SHOT", path),
        None => command.env_remove("YUMETE_SHOT"),
    };
    match command.status() {
        Ok(status) if status.success() => match dest {
            None => say!("ui.screenshot-taken"),
            Some(path) if path.exists() => say!("ui.shot-photographed", path.display()),
            Some(_) => say!("ui.shot-kept-the-clipboard"),
        },
        Ok(status) => say!("ui.screenshot-failed", status),
        Err(err) => say!("ui.screenshot-failed", err),
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
    killed.then(|| say!("preview.orphan-stopped", pid))
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
            .map_err(|e| say!("find.file-and-message", program, e))?;
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
            "language.no-such-action",
            want.language,
            want.verb
        );
    };
    let file = want.path.display().to_string();
    let Some(argv) = runner.argv(&file) else {
        return say!("command.broken-line", runner.run);
    };
    let Some((program, args)) = argv.split_first() else {
        return say!("command.broken-line", runner.run);
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
                    say!("language.replaced", want.verb)
                }
                Ok(ran) => say!("language.text-untouched", ran.why()),
                Err(err) => say!("language.cannot-run", err),
            }
        }
        // It reads and rewrites the file itself, so the buffer is re-read
        // afterwards — and only when it is clean, because re-reading over
        // unsaved changes is losing them.
        yumete_config::RunKind::Once => {
            if editor.current_buffer().is_modified() {
                return say!("language.save-first");
            }
            match run_program(program, args, None) {
                Ok(ran) if ran.ok => {
                    let _ = editor.current_buffer_mut().reread();
                    say!("language.done", want.verb)
                }
                Ok(ran) => say!("find.file-and-message", want.verb, ran.why()),
                Err(err) => say!("language.cannot-run", err),
            }
        }
        // A server is a life of its own; `:view-preview` owns that path.
        yumete_config::RunKind::Server => {
            say!("language.is-a-server", want.verb)
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
        CommitStrategy::Delayed => say!("commit.name.delayed"),
        CommitStrategy::Unique => say!("commit.name.unique"),
        CommitStrategy::Fluency => say!("commit.name.fluency"),
    }
}

/// `:yume-commit [delayed|unique|fluency]` — **when** a finished code goes to
/// the page (Feature #209).
///
/// The three are yume's own, merged in yume's own core, so what is chosen here
/// is what the same word means in the input method everywhere else. A scheme
/// that can only be typed as whole sentences — 拼音 has no 碼表 to look a
/// segment up in — says so rather than pretending the choice took.
fn commit_method(ime: &mut ImeSession, mode: &str) -> String {
    if mode.is_empty() {
        return say!("commit.set", commit_name(ime.commit_strategy()));
    }
    let Some(cs) = CommitStrategy::from_str_tag(mode) else {
        return say!("commit.no-such-method", mode);
    };
    ime.set_commit_strategy(Some(cs));
    let now = ime.commit_strategy();
    if now != cs {
        return say!(
            "commit.scheme-is-sentence-only",
            ime.scheme_name(),
            commit_name(now)
        );
    }
    say!("commit.set", commit_name(now))
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
        return say!("panel.set", panel_name(ime.panel_display()));
    }
    let Some(display) = PanelDisplay::parse(mode) else {
        return say!("panel.no-such-panel", mode);
    };
    ime.set_panel_display(display);
    say!("panel.set", panel_name(display))
}

/// `:yume-menu-size` — how many candidates a page holds.
///
/// **A session property, not a config one.** It is read out of the session on
/// every scheme switch (see [`switch_scheme`]), so a size set here survives
/// changing schemes; the config seeds the first session at startup and has no
/// further say.
fn menu_size(ime: &mut ImeSession, n: &str) -> String {
    if let Some(n) = n.parse::<usize>().ok().filter(|n| (1..=9).contains(n)) {
        ime.set_page_size(n);
    }
    say!("ime.menu-size-is", ime.page_size())
}

/// `:yume-autocompletion` — 輸入預測 on, off, or (bare) the other one.
fn autocompletion(ime: &mut ImeSession, want: &str) -> String {
    let on = match want {
        "on" => true,
        "off" => false,
        // Bare toggles, the way `:yume-chaifen` does.
        _ => !ime.prediction_enabled(),
    };
    ime.set_prediction_enabled(on);
    say!(
        "ime.autocompletion-is",
        match on {
            true => say!("label.on"),
            false => say!("label.off"),
        }
    )
}

/// The name a 候選面板 is called by, in the language the writer reads.
fn panel_name(display: PanelDisplay) -> String {
    match display {
        PanelDisplay::Full => say!("panel.name.full"),
        PanelDisplay::Bare => say!("panel.name.bare"),
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
            "data.refused-magic",
            first.file,
            reason,
            magic.expected,
            magic.found
        ),
        DataFault::Rejected { reason, magic: None } => {
            say!("data.refused", first.file, reason)
        }
        DataFault::Unreadable(why) => say!("data.unreadable", first.file, why),
        // `is_loud` admits no others.
        _ => return String::new(),
    };
    match loud.len() {
        1 => one,
        n => say!("data.and-more-failed", one, n - 1),
    }
}

/// Put the candidate `bare` is offering into the text, or take it away again.
///
/// Wholesale, every frame, because [`Editor::set_candidate`] is wholesale: what is
/// on the page now is exactly what this says, so a committed candidate leaves
/// nothing behind and a cancelled one disappears without anybody remembering
/// to clear it.
///
/// **Not in a prompt.** A `/` search composes on the status line, which has no
/// page to draw into; the panel comes up there whatever this setting says.
fn settle_inline_candidate(editor: &mut Editor, ime: &ImeSession) {
    let want = inline_candidate(editor, ime);
    // A page with no candidate on it pays nothing — and must not be marked
    // dirty by a `set_candidate` that changes nothing, since both layout memos are
    // keyed on the runs.
    if want.is_empty() {
        // Cleared outright, not only when there is something to clear.
        // Setting an empty list that is already empty changes no key: both
        // memos are keyed on the runs themselves.
        editor.set_candidate(Vec::new());
        return;
    }
    editor.set_candidate(vec![(editor.cursor_line(), editor.cursor_column(), want)]);
}

/// The text `bare` draws into the sentence, or empty when it draws nothing.
fn inline_candidate(editor: &Editor, ime: &ImeSession) -> String {
    if ime.panel_is_full()
        || !composes_here(editor)
        || !page_can_hold_a_candidate(editor)
        || !ime.available()
        || !ime.is_composing()
    {
        return String::new();
    }
    ime.inline_candidate()
}

/// Whether what is on screen is a page that drawn text can be drawn into.
///
/// The two gates — panel or inline — are complementary, and this is the term
/// they share, so there can be no state that draws both and none that draws
/// neither. Two things on screen are not that page:
///
/// * a **prompt**, which composes on the status line;
/// * a **grid**, which `table::draw` renders cell by cell out of the cells
///   themselves and knows nothing about drawn runs. #212 did not change that:
///   what it squares up is a `|` table **on the text page**, where the runs
///   are what everything measures; a whole-file grid draws its own columns and
///   has nothing for a run to stand before.
///
/// In both, `bare` gives the panel back rather than showing nothing at all.
fn page_can_hold_a_candidate(editor: &Editor) -> bool {
    editor.prompt().is_none() && !editor.grid_has_the_pane()
}

/// Switch the IME to the named scheme, and say what happened.
///
/// Only 靈明 ships with yumete. The others are yume's own data, installed the
/// way yume installs it — `scripts/build.sh`, or a download from
/// yuhao-assess-data into the data directory — and a 碼表 of one's own goes in
/// `.yumete/` beside the manuscript. So the failure worth naming is not "no
/// such scheme" but "that scheme's tables are not on this machine".
/// Put the input method into one of its three states, and say which (#290).
///
/// **Loading is part of `Chinese`**: turning it on with no 碼表 loaded is
/// 「開始打中文」, and nobody who asked for that wanted to be told they are not
/// ready. The other two need nothing loaded — handing the keyboard back is
/// something a session with no table can do just as well.
///
/// Handing it back leaves 中/ABC alone, so coming back comes back to what you
/// were typing in.
fn engage(ime: &mut ImeSession, want: Engagement, config: &Config) -> String {
    if want == Engagement::Chinese && !ime.available() {
        let loaded = switch_scheme(ime, "", config);
        if !ime.available() {
            return loaded;
        }
    }
    ime.set_engaged(want != Engagement::Off);
    let chinese = want == Engagement::Chinese;
    if want != Engagement::Off && ime.is_chinese() != chinese {
        ime.toggle_language();
    }
    match want {
        Engagement::Chinese => say!("ime.chinese", ime.scheme_name()),
        Engagement::Ascii => say!("ime.abc"),
        Engagement::Off => say!("ime.off"),
    }
}

/// Will this request go to the disk for a 碼表 (#341)?
///
/// Loading is a tenth of a second and more — 134.7 ms for the binary form of
/// 一百二十五萬條, 546.1 ms for the same table as text — and it happens inside
/// the keystroke that asked for it. The loop paints 「正在載入碼表…」 before
/// making the call rather than loading on a thread of its own: what the writer
/// wants to type next is 漢字 out of the table that is not there yet, so there
/// is nothing useful for them to do while it loads, and a session swapped in
/// under a half-typed code is a way to be wrong that a sentence on the status
/// line is not.
///
/// Erring towards saying it: a request that turns out to load nothing has
/// painted one frame too many, and that frame is the truth one keystroke early.
/// It is the same shape as the cold start, which puts the page up first and
/// reads the 14 MB after (`Deferred`) — this is that line reached from a
/// command instead of from launch.
fn loading_the_table(tag: &str, ime: &ImeSession) -> bool {
    match tag {
        // The question, and the two settings: all three answer out of the
        // session that is already here.
        "?" => false,
        _ if tag.starts_with("commit:")
            || tag.starts_with("panel:")
            || tag.starts_with("menu:")
            || tag.starts_with("predict:") =>
        {
            false
        }
        // Handing the keyboard back needs no 碼表, and asking for 中文 needs
        // one only when there is none — that is the once-a-session cost that
        // startup did not pay (#290).
        _ if tag.starts_with("lang:") => tag == "lang:chinese" && !ime.available(),
        // A named scheme, a file of one's own, the system's copy, the builtin:
        // every one of them reads a table.
        _ => true,
    }
}

/// Where the editor looks for 碼表 and 字料, layer by layer, and what it found.
///
/// **Why this is a command and not a comment.** Six directories are searched in
/// an order nobody can see, and the failure it exists for is silent: a reader
/// installs 宇浩, types 漢字, gets the cut table anyway, and has no way to ask
/// *which* of the six the editor read. `:yume-where` is that question, and the
/// answer opens as a buffer so it can be searched and its paths followed.
///
/// The order is `yumete_config::data_search_layers()` — first directory holding
/// a file wins — with the built-in tables named last, because they are what
/// answers when none of the six do.
fn where_report(ime: &ImeSession) -> String {
    use std::fmt::Write as _;
    use yumete_config::DataSource;
    let manifest = ime.data_file_names();
    let mut out = String::new();
    let _ = writeln!(out, "{}\n", say!("yume.where.head"));
    let mut last: Option<DataSource> = None;
    let mut nth = 0;
    for found in yumete_config::data_search_layers() {
        if last != Some(found.source) {
            nth += 1;
            let label = match found.source {
                DataSource::Config => say!("yume.where.from.config"),
                DataSource::Env => say!("yume.where.from.env"),
                DataSource::Own => say!("yume.where.from.own"),
                DataSource::Prefix => say!("yume.where.from.prefix"),
                DataSource::BesideExe => say!("yume.where.from.beside-exe"),
                DataSource::Yume => say!("yume.where.from.yume"),
            };
            let _ = writeln!(out, "{}", say!("yume.where.layer", nth, label));
            last = Some(found.source);
        }
        // **Name the files, do not count them.** 「7 個檔」 does not say whether
        // the 語言模型 is among them, and that is the one a reader is usually
        // missing.
        let here: Vec<&str> = manifest
            .iter()
            .filter(|rel| found.dir.join(rel).is_file())
            .filter_map(|rel| std::path::Path::new(rel).file_name().and_then(|n| n.to_str()))
            .collect();
        let said = match here.len() {
            0 => say!("yume.where.nothing"),
            n if n <= 4 => here.join(" "),
            n => format!("{} … {}", here[..4].join(" "), say!("yume.where.more", n)),
        };
        let _ = writeln!(out, "    {}\n        {said}", found.dir.display());
    }
    let _ = writeln!(out, "{}", say!("yume.where.layer", nth + 1, say!("yume.where.builtin")));
    let _ = writeln!(
        out,
        "    {}",
        match yumete_ime::builtin_version() {
            Some(when) => say!("yume.where.builtin-is", when),
            None => say!("yume.where.nothing"),
        }
    );
    let _ = writeln!(out, "\n{}", say!("yume.where.now", ime.table_source()));
    out
}

fn switch_scheme(ime: &mut ImeSession, tag: &str, config: &Config) -> String {
    // Two questions ride the same request, because both are about the session
    // the front end holds and neither is worth a second channel.
    if tag == "?" {
        let head = if ime.available() {
            say!(
                "ime.state",
                ime.scheme_name(),
                ime.table_source(),
                match ime.annotations_enabled() {
                    true => say!("label.on"),
                    false => say!("label.off"),
                },
                commit_name(ime.commit_strategy()),
            )
        } else if yumete_ime::has_builtin_table() {
            say!("ime.not-started")
        } else {
            say!("ime.not-started-no-builtin")
        };
        // 面板 only when there is not one: a writer who sees no candidate list
        // and wonders where it went is the only one who needs telling, and the
        // line is already four clauses long (Feature #211).
        let head = match ime.panel_display() {
            PanelDisplay::Full => head,
            display => format!("{head} · {}", say!("scheme.panel-is", panel_name(display))),
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
    if let Some(n) = tag.strip_prefix("menu:") {
        return menu_size(ime, n);
    }
    if let Some(want) = tag.strip_prefix("predict:") {
        return autocompletion(ime, want);
    }
    if let Some(path) = tag.strip_prefix('=') {
        let path = std::path::PathBuf::from(shellexpand(path));
        return match ImeSession::from_table_file(&path) {
            Ok(mut table) => {
                table.set_page_size(ime.page_size());
                table.set_commit_strategy(ime.commit_override());
                table.set_panel_display(ime.panel_display());
                let skipped = table.table_skipped();
                *ime = table;
                let loaded = say!("ime.table-loaded", path.display());
                // Said only when there is something to say. A table of your
                // own is a path you typed, and the quiet failure is pointing
                // it at the wrong file: rows that do not fit are dropped, so
                // without this the panel would answer from a table with
                // holes in it and nothing would have mentioned them (#344).
                match skipped {
                    0 => loaded,
                    n => format!("{loaded} · {}", say!("ime.table-skipped", n)),
                }
            }
            Err(why) => why,
        };
    }
    // The three states, by name (#290). The lone-Shift tap crosses between
    // the first two; the third is reached from here and nowhere else.
    if let Some(want) = tag.strip_prefix("lang:") {
        let want = match want {
            "chinese" => Engagement::Chinese,
            "abc" => Engagement::Ascii,
            _ => Engagement::Off,
        };
        return engage(ime, want, config);
    }
    // The 碼表 the system has — `builtin`'s other half.
    if tag == "~" {
        let mut full = ImeSession::from_default_dirs(ime.scheme());
        if !full.available() {
            return say!("ime.no-table-installed", ime.scheme_name());
        }
        full.set_page_size(ime.page_size());
        full.set_annotations(ime.annotations_enabled());
        full.set_commit_strategy(ime.commit_override());
        full.set_panel_display(ime.panel_display());
        *ime = full;
        return say!("ime.scheme-from-system", ime.scheme_name());
    }
    if tag == "!" {
        if !yumete_ime::has_builtin_table() {
            return say!("ime.no-builtin-table");
        }
        let mut full = ImeSession::builtin_lingming();
        full.set_page_size(ime.page_size());
        full.set_annotations(ime.annotations_enabled());
        full.set_commit_strategy(ime.commit_override());
        full.set_panel_display(ime.panel_display());
        *ime = full;
        return say!("ime.builtin-lingming");
    }
    // No name means "the one this project writes in" — `:yume s` is the whole
    // of starting to type, and the config already said which.
    let tag = if tag.is_empty() {
        config.ime.scheme.as_str()
    } else {
        tag
    };
    let Some(scheme) = Scheme::from_tag(tag) else {
        let names = Scheme::all()
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
            full.set_page_size(ime.page_size());
            full.set_commit_strategy(ime.commit_override());
            full.set_annotations(ime.annotations_enabled());
            full.set_panel_display(ime.panel_display());
            let name = full.scheme_name().to_string();
            *ime = full;
            return say!("ime.scheme-now", name);
        }
        return say!(
            "scheme.table-not-installed",
            tag,
            yumete_config::data_dir().display()
        );
    }
    let was = ime.scheme();
    if ime.set_scheme(scheme) {
        return say!("ime.scheme-now", ime.scheme_name());
    }
    // Put back what was working rather than leaving the writer unable to type.
    ime.set_scheme(was);
    say!(
        "scheme.table-not-installed",
        tag,
        yumete_config::data_dir().display()
    )
}

/// Route one Insert-mode key press to the IME. Returns `true` when the IME
/// consumed it (so the editor must not also see it). Committed text is inserted
/// into the editor at the cursor.
/// End a half-typed composition before something that is not a key moves the
/// caret (#336).
///
/// **`Event::Mouse` and `Event::Paste` are siblings of `Event::Key`, and they
/// never asked whether the IME was in the middle of a word.** Type half a 拆分,
/// click another tab, and `show_buffer_at` changes the buffer while the preedit
/// is still alive — it redraws at the caret of the **new file**, and the next
/// space commits it *there*. A few characters of somebody else's chapter, in a
/// document they were not even looking at, and nothing on the screen said so.
///
/// Discarded rather than committed. A preedit is not writing yet: half a code
/// has not chosen a character, and inserting whatever the engine happens to
/// have on top would be putting a word in the writer's mouth. What is lost is
/// three keystrokes they were about to abandon anyway — they clicked away.
///
/// ⚠️ **Not on the wheel.** Scrolling moves the page, not the caret, so the
/// composition is still where the writer left it and cancelling it because
/// they looked somewhere would be its own small betrayal.
fn end_the_composition(ime: &mut ImeSession, editor: &mut Editor) {
    if !ime.is_composing() {
        return;
    }
    ime.escape();
    // Anything the engine had already handed over is writing, and belongs in
    // the buffer the writer typed it into — which is still the current one,
    // because this runs before the click is acted on.
    let committed = ime.take_committed();
    if !committed.is_empty() {
        editor.insert_committed(&committed);
    }
}

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
    // **快捷符號 owns the keyboard while it is up** — every printable key goes
    // straight to the engine, which has `shortcut_input` for exactly this: a
    // letter commits its symbol, the lead key again commits 「；」, anything
    // else leaves the mode and is re-read as a fresh keypress.
    //
    // ⚠️ **Ahead of the 選重 arms, not after them.** The buffer holds the lead
    // key and the table counts as candidates, so `key_action` reads the state
    // as 「組字中，有候選」 — and in 靈明 that makes `;` mean 選二. `;;` then
    // committed the **second entry** (`b` ＝ 「～」) instead of 「；」, and `'`
    // would have committed the third. The mode has no 選重 in it at all, which
    // is why yume's own front ends branch on it before anything else
    // (`frontends/windows/src/KeyHandler.cpp:675`).
    if ime.is_shortcut() {
        match code {
            // The lead key again, Space: both 「；」. Enter commits the raw
            // buffer, Backspace and Esc put the table away.
            KeyCode::Char(' ') => ime.space(),
            KeyCode::Enter => ime.enter(),
            KeyCode::Backspace => {
                ime.backspace();
            }
            KeyCode::Esc => ime.escape(),
            // Nothing here to navigate: the choosing is done with letters.
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Home
            | KeyCode::End
            | KeyCode::PageUp
            | KeyCode::PageDown
            | KeyCode::Delete
            | KeyCode::Tab
            | KeyCode::BackTab => {}
            KeyCode::Char(_)
                if mods.intersects(
                    KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META,
                ) => {}
            KeyCode::Char(c) if !mods.contains(KeyModifiers::CONTROL) => ime.input(c),
            _ => return false,
        }
        let committed = ime.take_committed();
        if !committed.is_empty() {
            editor.insert_committed(&committed);
        }
        return true;
    }
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
        // indent.
        //
        // With the list already up, `Tab` is the 字典 on the candidate the
        // highlight is on (#215) — the panel has one line per candidate and
        // spends it on 拆分 and a code, and this is where the rest of what the
        // 拆分表 knows about that character is. A 詞 is asked about by its
        // first character: a 拆分 is a thing a single 字 has.
        //
        // So under `full` `Tab` is the 字典 at once, and under `bare` it takes
        // two — you cannot ask about a candidate you cannot see. The cost is
        // that a panel summoned by mistake can no longer be put away with a
        // second `Tab`; it goes away with the word, which is as long as the
        // undo ever bought.
        KeyCode::Tab if composing => match ime.panel_is_full() {
            true => {
                if let Some(ch) = ime.inline_candidate().chars().next() {
                    editor.look_up(ch, false);
                }
            }
            false => {
                ime.summon_panel();
            }
        },
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
        // ⚠️ **`C-[` is Esc, and it is ours to give back** (#514). On a
        // terminal with the Kitty protocol, `DISAMBIGUATE_ESCAPE_CODES` — which
        // this editor asks for, a few hundred lines up — makes the terminal
        // report `C-[` as a chord instead of as the escape byte it has always
        // been. That flag buys `Esc` telling itself apart from the head of an
        // arrow-key sequence, and it costs this: a reader for whom `C-[` has
        // meant Esc since vi found that it meant it everywhere *except* here.
        // The flag stays and the key is handed back.
        KeyCode::Char('[') if modifiers.contains(KeyModifiers::CONTROL) => Some(Key::Esc),
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

/// How much air stands between a line number and the writing.
///
/// **Two cells, and the second one is spoken for** (#484). It is where the
/// 改動條 goes — the column that says which lines differ from the disk, or from
/// git (#298, #55) — so the space is being left now rather than taken from the
/// writing later: a page whose measure shifts by a cell the day a feature
/// lands is a page that reflows under the reader.
///
/// Until then it is air, and air is no loss: one cell beside a 漢字 is half a
/// character's margin, and the numbers crowded the text.
const GUTTER_AIR: usize = 2;

/// The width of the line-number gutter for a given mode (digits + the air).
fn gutter_width(total_lines: usize, mode: LineNumbers) -> usize {
    match mode {
        LineNumbers::None => 0,
        _ => total_lines.max(1).to_string().len() + GUTTER_AIR,
    }
}

/// The gutter text for line `i` (0-based) given the cursor line and mode.
fn gutter_text(i: usize, cursor_line: usize, width: usize, mode: LineNumbers) -> String {
    match mode {
        LineNumbers::None => String::new(),
        LineNumbers::Absolute => {
            format!("{:>w$}{}", i + 1, " ".repeat(GUTTER_AIR), w = width - GUTTER_AIR)
        }
        LineNumbers::Relative => {
            let air = " ".repeat(GUTTER_AIR);
            let w = width - GUTTER_AIR;
            if i == cursor_line {
                // Show the absolute number on the cursor line, left-aligned.
                format!("{:<w$}{air}", i + 1)
            } else {
                format!("{:>w$}{air}", i.abs_diff(cursor_line))
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
    /// **The two panel slots**, indexed by [`Side`] (#293). A slot with
    /// nothing in it is zero columns wide, which is how the page stays exactly
    /// as wide as it was before there were two.
    panels: [Rect; 2],
    /// The two work areas, **in screen order** — `panes[0]` is the one drawn
    /// first (top, or right in 縱書). Which of them holds the keys is
    /// [`Editor::live_pane`], and it is a different question on purpose:
    /// switching panes must not make the top one jump to the bottom.
    panes: [Rect; 2],
    /// The rule between them, when there are two.
    divider: Option<Rect>,
    tabs: Rect,
    /// **Two rows at the top of the page, or none** (#379): the column numbers
    /// and names of a table whose own head has scrolled away.
    ///
    /// A region rather than an overlay, which is the whole of why it exists at
    /// all — the page is two rows shorter and nothing else changes, the same
    /// arithmetic the tab bar and the command row already do.
    head: Rect,
    /// What the page itself is drawn into.
    text: Rect,
    status: Rect,
    /// **The bottom row of all, or none** (#302): where `:` and `/` are typed,
    /// where a message about what just happened lands, and where the keys you
    /// can press are listed.
    ///
    /// Below the status line and not above it, which is the whole point of the
    /// order: the status line then sits against the writing and marks its
    /// bottom edge with its own ground, and this row — on the page's ground —
    /// reads as the margin it is until something is typed into it.
    command: Rect,
}

/// Divide `area` up. Pure: it draws nothing and depends only on what the
/// editor and the config say.
fn page_areas(editor: &Editor, config: &Config, area: Rect, head_rows: u16) -> Areas {
    // Two rows at the foot, answering two questions. The upper one is *where
    // am I* and never changes shape; the one below it is *what am I typing,
    // what just happened, and what can I press*, and is blank when it is none
    // of the three. Splitting them is what lets the status line stay still: a
    // message used to push the position along the line, or take it away
    // outright, and a `:` used to replace the whole row.
    let command_rows = u16::from(config.editor.command_line && area.height > 4);
    let body_h = area.height.saturating_sub(command_rows + 1);
    let status = Rect::new(area.x, area.y + body_h, area.width, 1);
    let command = Rect::new(area.x, area.y + body_h + 1, area.width, command_rows);
    // A panel takes its columns off its own side; set vertically the left is
    // still the left, because the 縱 fill from the right edge and the page
    // simply ends sooner.
    let left = sidebar_columns(editor, config, Side::Left, area.width);
    let right = sidebar_columns(editor, config, Side::Right, area.width.saturating_sub(left));
    let panels = [
        Rect::new(area.x, area.y, left, body_h),
        Rect::new(area.x + area.width.saturating_sub(right), area.y, right, body_h),
    ];
    let body = Rect::new(
        area.x + left,
        area.y,
        area.width.saturating_sub(left + right),
        body_h,
    );
    // The tab bar takes the row off the top of what is left.
    let (tabs, page) = match config.editor.tabs.showing(editor.buffer_count()) && body.height > 1 {
        true => (
            Rect::new(body.x, body.y, body.width, 1),
            Rect::new(body.x, body.y + 1, body.width, body.height - 1),
        ),
        false => (Rect::new(body.x, body.y, body.width, 0), body),
    };
    // …and the column bar takes its rows off what the tab bar left, for the
    // same reason and by the same arithmetic.
    let head_rows = head_rows.min(page.height.saturating_sub(1));
    let head = Rect::new(page.x, page.y, page.width, head_rows);
    let page = Rect::new(page.x, page.y + head_rows, page.width, page.height - head_rows);
    // The detail panel used to be carved out here, off the right of the page
    // (`table::split_detail`, gone). It is the bottom layer of the right slot
    // now, taken off before the tab bar like every other panel — so the tab
    // bar no longer runs over the top of it (#293).
    let text = page;
    // 工作區 (Feature #176). **The cut runs across the direction the text
    // advances in**: 橫排 advances downward, so the panes are 上下; 縱書
    // advances leftward, so they are 左右 and the second takes the left. That
    // is what keeps the measure untouched — both panes keep the full width in
    // 橫排 and the full height in 縱書, so not one row rewraps and not one 縱
    // is shortened. A divider carries the boundary and the caption.
    let (panes, divider) = match editor.other_pane().is_some() {
        false => ([text, Rect::new(text.x, text.y, 0, 0)], None),
        true => match editor.layout() {
            WritingLayout::Vertical if !editor.grid_has_the_pane() => {
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
        panels,
        tabs,
        head,
        text,
        panes,
        divider,
        status,
        command,
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
    // **Two rows for a table's columns, and only once its own head has gone**
    // (#379). Asked before the page is divided, because the answer is how tall
    // the page is, and answered from last frame's scroll together with this
    // frame's cursor — see `table_head_is_off_the_page`.
    let top = viewport[editor.live_pane().min(1)].top.line;
    let head_rows = 2 * u16::from(editor.table_head_is_off_the_page(top));
    let areas = page_areas(editor, config, area, head_rows);
    let Areas {
        panels: slots,
        panes,
        divider,
        tabs: tab_area,
        head: head_area,
        text: text_area,
        status: status_area,
        command: command_area,
    } = areas;
    // Where a `:` or `/` is typed. Its own row when there is one; the status
    // line when the window is too short for two, because a prompt with nowhere
    // to be drawn is a prompt nobody can answer.
    let prompt_area = match command_area.height {
        0 => status_area,
        _ => command_area,
    };
    let mut panel_caret: Option<Position> = None;
    for side in Side::BOTH {
        for (layer, rect) in Layer::BOTH
            .into_iter()
            .zip(slot_layers(editor, side, slots[side as usize]))
        {
            if rect.width == 0 || rect.height == 0 {
                continue;
            }
            match layer {
                Layer::Top => {
                    // A panel with a box in it has a caret, and the candidate
                    // panel has to stand under **that** one — see `draw_search`.
                    if let Some(at) = draw_sidebar(frame, editor, config, side, rect) {
                        panel_caret = Some(at);
                    }
                }
                Layer::Bottom => match editor.transient(side) {
                    Some(Transient::Detail) => {
                        table::draw_detail(frame, editor, config, side, rect)
                    }
                    Some(Transient::Dictionary) => {
                        draw_dictionary(frame, editor, config, side, rect)
                    }
                    None => {}
                },
            }
        }
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
        // **Everything below reads the file the caption names** (#281). The
        // whole dispatch is inside the scope, not only the drawing: which
        // layout a half is drawn in, and whether a grid has it, are questions
        // about *that* half's document and place.
        let _viewing = peek.map(|pane| editor.view_pane(pane));
        let at = match editor.layout() {
            // A grid is not prose and is not drawn as prose: no wrapping, no
            // markup, one row per line, columns that line up.
            // A `|` table lives inside a page of prose and is drawn by whatever
            // draws that page — the paragraph above it must not vanish because
            // the cursor landed in a cell.
            _ if editor.grid_has_the_pane() => {
                table::draw(frame, editor, config, *rect, &mut seat.table, peek)
            }
            WritingLayout::Horizontal => {
                draw_horizontal(
                    frame,
                    editor,
                    config,
                    *rect,
                    &mut seat.top,
                    &mut seat.left,
                    peek,
                    // Only the pane the keys are in gets the bar, and only in
                    // the layout that reserved it.
                    (which == editor.live_pane().min(1)).then_some(head_area),
                )
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

    // **Where every panel put itself**, gathered as it is drawn. `:view-hud full`
    // covers writing on purpose and so cannot tell 「有字」 from 「有面板」 by
    // reading the buffer back the way `:view-hud basic` does; a panel it covered
    // would be a panel with a hole in it.
    let mut panels: Vec<Rect> = Vec::new();

    draw_status(frame, editor, config, ime, status_area, tab_area, command_area.height == 1);
    if command_area.height == 1 {
        draw_command(frame, editor, config, ime, command_area);
    }
    // The floating panels stack upward from the status line, which is the top
    // of the footer: the command row is *below* it, so a panel that stopped at
    // the command row would be drawn over the position readout.
    let footer = status_area;
    panels.extend(draw_command_menu(frame, editor, config, area, footer));
    panels.extend(draw_lookfor_menu(frame, editor, config, area, footer));
    panels.extend(draw_reference_menu(frame, editor, config, area, footer));
    // Where the picker put its caret, so the candidate panel can stand under
    // the query instead of over the page the list is already covering.
    let picker = draw_picker(frame, editor, config, ime, area);
    let picker_caret = picker.map(|(caret, _)| caret);
    panels.extend(picker.and_then(|(_, list)| list));
    // One panel for every half-pressed sequence, `空格` included — it used to
    // draw its own menu and every other prefix got a row.
    // **The page's rectangle, not the frame's**: a menu drawn from the frame
    // covers the sidebar, which is a list the reader may be in the middle of
    // using.
    // ⚠️ **One float at a time, and this is the order** (2026-09-18):
    //
    //   a `:` menu  >  the picker  >  a half-pressed sequence  >  a note
    //
    // Each of them wants the same corner, and each of them means 「the reader
    // is doing *this* right now」 — so the one they are furthest into wins and
    // the others are not drawn at all. 作者 2026-09-18, looking at a 百科
    // entry and the `:` menu crowding one screen: 「一次只會出現一個面板，那麽
    // 輸入命令的時候百科窗口自然就會消失」.
    //
    // It used to be four unconditional calls, later ones painting over
    // earlier ones — and the comment here claimed the opposite of what the
    // code did (the note was drawn *after* the which-key, so the note won).
    let taken = !panels.is_empty() || picker_caret.is_some();
    let keys_panel = match taken {
        true => None,
        false => draw_which_key(frame, editor, config, text_area, footer.y, (cursor_x, cursor_y)),
    };
    // The footnote, the comment, or the 百科 entry the cursor is standing on
    // (#294, #287) — the quietest of the four, and the first to give way.
    let note_panel = match taken || keys_panel.is_some() {
        true => None,
        false => draw_note(frame, editor, config, text_area, footer.y, (cursor_x, cursor_y)),
    };
    panels.extend(keys_panel);
    panels.extend(note_panel);
    // `bare` draws no panel — the candidate is already in the sentence and the
    // code is under the caret. Unless there is no sentence to draw it into:
    // see `page_can_hold_a_candidate`.
    // A picker always gets the panel: its list is drawn over the page, so
    // there is no sentence left down there to put a bare candidate into.
    //
    // Asked here rather than at the panel's own call further down, because
    // `:view-hud full` wants the same cell — 「候選面板和釘住的 HUD 都要
    // (cursor_x, cursor_y+1)」 — and the candidate panel is the one that wins.
    // ⚠️ `panel_caret.is_some()` too: the inline preview writes into the
    // *manuscript's* cells, and the characters being typed are in a box in a
    // panel. Previewing them on the page would put them where they are not.
    let panel = ime.panel_is_full()
        || picker_caret.is_some()
        || panel_caret.is_some()
        || !page_can_hold_a_candidate(editor);
    let candidate_panel = panel && composes_here(editor) && ime.available() && ime.is_composing();
    // …and the same string beside the caret, where the eyes are.
    draw_hud(
        frame,
        editor,
        config,
        ime,
        text_area,
        (cursor_x, cursor_y),
        &panels,
        candidate_panel,
    );

    // In vertical layout the cursor is a block drawn into the page: a hardware
    // cursor is one cell wide and would sit lopsided inside a two-cell 縱.
    if editor.picker().is_some() {
        // `draw_picker` put the caret in its query, which is the prompt while a
        // picker is open.
    } else if let Some(prefix) = editor.prompt_label() {
        // Measured in cells, not characters: a Chinese search pattern is twice
        // as wide as it is long — and up to the **caret**, not to the end of
        // the line, now that the prompt can be edited in the middle.
        // The prefix is measured too, because `::` is two cells wide (#224)
        // and a hard-coded 1 put the caret inside the second colon — and since
        // #505 a search says `搜索:`, which is five.
        let col = yumete_cjk::str_width(&prefix)
            + yumete_cjk::str_width(&editor.prompt_before_caret())
            + yumete_cjk::str_width(&prompt_preedit(editor, ime));
        frame.set_cursor_position(Position::new(prompt_area.x + col as u16, prompt_area.y));
    } else if editor.layout() == WritingLayout::Horizontal || editor.mode() == Mode::Insert {
        // Vertically the terminal's cursor is shown only in Insert, where it is
        // the caret; in Normal the block is painted into the page and a second,
        // half-width cursor on top of it would only confuse.
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }

    if candidate_panel {
        // The panel follows the page, not the prompt: a `/` search in a
        // vertically set document still picks its candidates out of a vertical
        // list, and one panel wearing a different skin from the other reads as a
        // different program.
        let (at_x, at_y) = match editor.prompt() {
            _ if picker_caret.is_some() => {
                let at = picker_caret.expect("just checked");
                (at.x, at.y)
            }
            // A box inside a panel — the search panel's query or replace row.
            // It is not `prompt()` (that is the command line) and not a picker,
            // so without this the panel stood over the manuscript instead of
            // under the characters being typed.
            _ if panel_caret.is_some() => {
                let at = panel_caret.expect("just checked");
                (at.x, at.y)
            }
            Some((prefix, text)) => {
                let col = yumete_cjk::str_width(prefix)
                    + yumete_cjk::str_width(text)
                    + yumete_cjk::str_width(&prompt_preedit(editor, ime));
                (prompt_area.x + col as u16, prompt_area.y)
            }
            None => (cursor_x, cursor_y),
        };
        // **The panel may cover the writing; it may not cover the footer.**
        // `area` here is the whole window, so on a tall terminal the panel
        // never reached the bottom and nobody noticed — but at twelve rows a
        // full page of nine candidates ran straight over the last row, and the
        // mode, the file name and the position came out as
        // `--╰──────────╯[中a 靈b明f] ch1.md   Ln 1, Col 1` (#387). Stopping at
        // the status line keeps the command row below it clear too, which it
        // has to be: the caret the panel belongs to is standing in it.
        let room = Rect {
            height: status_area.y.saturating_sub(area.y).max(1),
            ..area
        };
        match editor.layout() {
            WritingLayout::Horizontal => draw_candidate_panel(frame, ime, config, room, at_x, at_y),
            WritingLayout::Vertical => {
                vertical::draw_candidate_panel(frame, ime, config, room, at_x, at_y)
            }
        }
    }
    // Last, and over everything: the editor is stopped behind it (#295).
    draw_query(frame, editor, config, area);
    // …and last of all, the sweep: whatever any of the above put on the page,
    // no cell of it is a character the terminal obeys.
    settle_control_characters(frame.buffer_mut());
}

/// Blank every cell holding a character the terminal would **act on** (#375).
///
/// **The one place nobody can forget.** [`drawable`] is the first line and it
/// works — where the drawing goes through [`put_text`]. About thirty spans do
/// not: block titles carry a file's name, the tab bar carries it again, the
/// HUD carries what has just been typed, and the status line carried the
/// character under the cursor, which on a 碼表 is a TAB. That one cost #375
/// and #377 together, and the reading of it is the rule this
/// enforces, 2026-09-11：「TAB 是个不稳定渲染。除了文本区有 tab 外，我们在其他
/// 位置不应该有 tab 存在。太危险了。」
///
/// Every drawing path ends in a cell, so the cells are where the rule can be
/// kept once instead of thirty times. A control character measures nought to
/// every width this editor asks, so the layout was computed as though it were
/// not there — and a terminal handed one does something instead: TAB jumps to
/// the next stop, CR returns to the margin, BS steps back, ESC begins a
/// sequence. Blanking it is therefore not a loss of anything: the cell was
/// already spoken for, and now it holds what the measure said it held.
///
/// A space rather than nothing, because the cell exists either way and an
/// empty symbol is a hole the row behind shows through.
fn settle_control_characters(buf: &mut ratatui::buffer::Buffer) {
    let area = buf.area;
    for y in area.y..area.y.saturating_add(area.height) {
        for x in area.x..area.x.saturating_add(area.width) {
            let Some(cell) = buf.cell_mut((x, y)) else {
                continue;
            };
            // One byte is the whole test: every control character is ASCII or
            // a C1 that cannot begin a UTF-8 sequence, so anything with a
            // first byte at or above `0x20` is clean and pays nothing.
            if cell.symbol().bytes().any(|b| b < 0x20 || b == 0x7f) {
                cell.set_symbol(" ");
            }
        }
    }
}

/// The **question panel**: the editor has stopped, and this is what it asked.
///
/// Drawn last and in the middle, which is the opposite of every other panel
/// here — the which-key menu, the candidate list and the command menu all dodge
/// the caret, because they are things you read *while* you write. This one is
/// not: the editor is stopped behind it (`Editor::on_key` gives it every key),
/// and a modal question that hides in a corner is one a writer answers without
/// reading. Centre, border, name in gold — #273's panel, at the middle.
fn draw_query(frame: &mut Frame, editor: &Editor, config: &Config, area: Rect) {
    let Some(asked) = editor.query() else { return };
    let ink = crate::theme::Palette::of(config);
    // Wide enough to read a sentence on, narrow enough to read as a panel: the
    // body is the only thing here that wraps, so it decides the width.
    let want = 56usize.min(area.width.saturating_sub(6) as usize);
    let body = wrap_to(&asked.body, want.max(16));
    let keys: Vec<String> = asked
        .choices
        .iter()
        .map(|a| format!("{}  {}", a.key, a.label))
        .collect();
    let inner = body
        .iter()
        .chain(keys.iter())
        .map(|l| yumete_cjk::str_width(l))
        .max()
        .unwrap_or(0)
        .max(yumete_cjk::str_width(&asked.title) + 2);
    let width = (inner + 4).min(area.width as usize) as u16;
    let height = (body.len() + keys.len() + 3) as u16;
    // Nowhere to put it is not a reason to answer for the writer: the question
    // stays open and the status line still carries the command that asked it.
    if width < 12 || height + 2 > area.height {
        return;
    }
    let Some(panel) = crate::chrome::place(area, (width, height), crate::chrome::Anchor::Centre)
    else {
        return;
    };
    crate::chrome::draw(frame, panel, &crate::chrome::Ring {
        rounded: config.panel.rounded,
        border: Style::default().fg(ink.rule()).bg(ink.paper()),
        ground: Style::default().bg(ink.paper()),
        title: Some((
            asked.title.clone(),
            Style::default().fg(ink.gold()).bg(ink.paper()),
        )),
    });
    let ground = Style::default().bg(ink.paper());
    let limit = panel.x + width - 1;
    let buf = frame.buffer_mut();
    let mut y = panel.y + 1;
    for line in &body {
        put_text(buf, panel.x + 2, y, limit, line, ground.fg(ink.text()));
        y += 1;
    }
    y += 1;
    for (a, row) in asked.choices.iter().zip(&keys) {
        // The key is gold and what it does is text: the same two rungs the
        // which-key panel uses, so one glance reads the same way in both.
        put_text(buf, panel.x + 2, y, limit, &a.key.to_string(), ground.fg(ink.gold()));
        put_text(
            buf,
            panel.x + 5,
            y,
            limit,
            row.split_at(3).1,
            ground.fg(ink.text()),
        );
        y += 1;
    }
}

/// Fold `text` into lines no wider than `width` terminal columns.
///
/// Breaks between 漢字 and at ASCII spaces, and never leaves a closing mark at
/// the head of a line — the same 禁則 the page itself keeps, in the one place
/// a panel has running prose in it.
fn wrap_to(text: &str, width: usize) -> Vec<String> {
    const NO_START: &str = "。，、；：！？」』）〕】》〉…·";
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut wide = 0usize;
    for ch in text.chars() {
        let w = yumete_cjk::str_width(&ch.to_string());
        if wide + w > width && !line.is_empty() && !NO_START.contains(ch) {
            lines.push(std::mem::take(&mut line));
            wide = 0;
        }
        line.push(ch);
        wide += w;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The 中/英 indicator, or empty when the IME is not engaged in this mode.
///
/// It has to show in a `/` prompt as much as in Insert: the whole point of
/// composing there is that the pattern is Chinese, and without the tag there is
/// no way to tell why letters are or are not turning into 漢字.
fn language_tag(editor: &Editor, ime: &ImeSession) -> String {
    if !composes_here(editor) {
        return String::new();
    }
    standing_language_tag(ime)
}

/// The same tag, in **every** mode — what the status line stands (#337).
///
/// [`language_tag`] answers「is 漢字 being typed *here*」, so it goes quiet
/// wherever keys are commands. The status line asks the other question:「what
/// will `i` land in」. The language belongs to the process, not to the mode
/// (one `ImeSession` for the whole editor), so that answer exists in Normal
/// too — and it used to be invisible there, which left one way to learn it:
/// press `i`, type a word, and watch it come out wrong.
///
/// It still says nothing when yume has handed the keyboard back
/// (`Engagement::Off`): then there is no 中/ABC to be in.
fn standing_language_tag(ime: &ImeSession) -> String {
    if !ime.available() || !ime.engaged() {
        return String::new();
    }
    if ime.is_chinese() {
        say!("ime.chinese-tag", ime.scheme_name())
    } else {
        "[ABC]".to_string()
    }
}

/// How many rows a menu takes when the window is small, and how many a picker
/// takes always.
///
/// Helix caps its completion popup and scrolls it, and the reason is not screen
/// real estate but reading: a list you have to search is not a list you can
/// glance at. Twenty-odd commands laid out across the whole page hid the very
/// document the command was about to act on.
///
/// How many columns the `:` menu may spread into (2026-09-10).
///
/// The width is what the columns fill (#372), but not without end: past a
/// half-dozen the eye has to find its way back across too far a page, and a
/// menu that has to be *searched* is no longer glanced at.
const MENU_COLUMNS: usize = 6;

/// The picker's height, and only the picker's: the `:` menu is laid out from
/// the width it has (#372).
const MENU_ROWS: usize = 8;

/// The widest a menu gets. Past this the eye stops reading a row as one thing.
const MENU_WIDTH: u16 = 56;

/// The widest the `::` panel gets (Feature #224).
///
/// Wider than [`MENU_WIDTH`], because a row there is not a name — it is a
/// sentence saying what the command does, and that sentence is the whole
/// reason the panel is open. At 56 cells 「段組：把竪排的頁面橫着分成幾條，右上
/// 讀到左上，」 stopped there, on a comma. This is about a printed line's
/// measure: long enough for most of the corpus to land whole, short enough
/// that the eye still comes back to the left edge without hunting.
const LOOKFOR_WIDTH: u16 = 78;

/// Cut `line` to `width` cells, marking that something was cut.
///
/// By display width, not by `char`s — a Chinese sentence half-cut by a `char`
/// count overruns the panel it was measured for. The mark is one cell, so what
/// is kept is one cell less.
fn elide(line: &str, width: usize) -> String {
    if yumete_cjk::str_width(line) <= width {
        return line.to_string();
    }
    let room = width.saturating_sub(1);
    let mut kept = String::new();
    let mut wide = 0;
    for g in yumete_cjk::graphemes(line) {
        let w = yumete_cjk::grapheme_width(g);
        if wide + w > room {
            break;
        }
        kept.push_str(g);
        wide += w;
    }
    kept.push('…');
    kept
}

/// Draw a compact list just above `bottom`, scrolled so `selected` is on it.
///
/// One column, capped, with a footer naming where you are in the list and what
/// the highlighted row means. Both the `:` menu and the pickers use it, so they
/// look like one idea rather than two.
struct List<'a> {
    items: &'a [Row],
    /// Which entry has to stay on screen.
    focus: usize,
    /// Which entry is inked, if any.
    highlight: Option<usize>,
    /// The line under it: a count, and what the inked entry means.
    footer: &'a str,
    /// Whether it may spread across the window.
    columns: bool,
    /// The name in the top-left of the ring, the way a which-key panel is
    /// named. 2026-09-05: 「command 提示面板的設計感不如快捷鍵提示
    /// 面板。」 — a floating rectangle with no edge and no name is a thing that
    /// appeared, not a panel that opened.
    title: &'a str,
    /// The widest one entry may be, padding and all.
    ///
    /// [`MENU_WIDTH`] for a list of names; [`LOOKFOR_WIDTH`] for the one list
    /// whose rows are sentences.
    cap: usize,
    /// **The whole list, and its widest entry** — what the shape is measured
    /// from, whatever is being shown right now (#372).
    ///
    /// Not `items`, which is what is left after what has been typed. A shape
    /// worked out from the *filtered* list moves under the reader's hands:
    /// the panel that was six columns of eight when they pressed `:` would be
    /// two of eight by the time they had typed `view`, and a place on the page
    /// is only worth learning if it stays where it was. `None` for a list with
    /// no unfiltered form — a picker's paths, a search's answers.
    whole: Option<(usize, usize)>,
}

/// One line of a list panel: what to type, and — for the rows that need it —
/// a word beside it in the quiet ink (#291).
///
/// **The note is dropped before the text is.** It explains the row; it is not
/// the row, and a column too narrow for both has to keep the half you would
/// type.
struct Row {
    text: String,
    note: Option<String>,
}

impl Row {
    fn plain(text: String) -> Self {
        Row { text, note: None }
    }

    /// How wide the row wants to be, note and all.
    fn width(&self) -> usize {
        let mut w = yumete_cjk::str_width(&self.text);
        if let Some(note) = &self.note {
            w += NOTE_GAP + yumete_cjk::str_width(note);
        }
        w
    }
}

/// The air between a row's name and its note.
const NOTE_GAP: usize = 2;

/// Returns the rectangle it covered, so a pinned HUD can keep off it (#284).
///
/// Reading the drawn buffer back used to answer 「is anything there」 for every
/// kind of thing at once — writing, this menu, the picker, a panel's ring —
/// and that was the whole of the HUD's collision avoidance. `:view-hud full` covers
/// writing on purpose, so 「有字」 and 「有面板」 stop reading alike and every
/// panel has to say where it put itself.
fn draw_list(
    frame: &mut Frame,
    ink: crate::theme::Palette,
    rounded: bool,
    area: Rect,
    bottom: u16,
    list: List,
) -> Option<Rect> {
    let List {
        items,
        focus,
        highlight,
        footer,
        columns,
        title,
        cap,
        whole,
    } = list;
    if items.is_empty() && footer.is_empty() {
        return None;
    }
    // Twenty-six commands down one column is three screenfuls with the rest of
    // the page standing empty beside it; in three columns it is one glance.
    // Column-major, so reading runs *down* and then across — the way a list of
    // files does, and the way the numbers on it stay in order.
    //
    // **Each column is as wide as its own longest entry**, not as the longest
    // entry anywhere in the list. One long entry used to widen the
    // six columns beside it by two cells each and cost the list a whole column
    // — the seventh, the one it needed to be shown whole.
    let room = (area.width as usize).saturating_sub(4);
    let lay = |deep: usize, from: usize| -> Vec<usize> {
        let mut widths = Vec::new();
        let mut used = 0;
        for chunk in items[from.min(items.len())..].chunks(deep.max(1)) {
            let one = chunk
                .iter()
                .map(|i| i.width())
                .max()
                .unwrap_or(0)
                .saturating_add(2)
                .min(cap);
            if !widths.is_empty() && (used + one > room || widths.len() == MENU_COLUMNS) {
                break;
            }
            used += one;
            widths.push(one);
        }
        widths
    };
    //
    // **The shape is two caps and nothing else** (2026-09-10): eight
    // rows, or half the window when that is shorter; and as many columns as
    // the width holds, up to six. The entries fill it downwards and then
    // across, so nine commands on eight rows are eight and one.
    //
    // It used to be worked out from the number of entries — the fewest columns
    // that showed them all — and the shape moved as the list did: a menu that
    // was three columns of eighteen at one moment and eight of six at the
    // next. Two caps say the same thing about every list, which is what makes
    // a place on the page worth learning.
    let (mut widths, deep) = if columns {
        let half = ((area.height / 2) as usize).saturating_sub(3); // footer, rings
        // **How many columns the width holds**, measured off the widest entry
        // the whole list has rather than off the ones left after typing — a
        // column is only as wide as its own longest row, so this is the
        // careful answer, and the careful answer is the one that does not move.
        let (total, widest) = whole.unwrap_or((items.len(), cap));
        let across = (room / widest.saturating_add(2).max(1))
            .clamp(1, MENU_COLUMNS);
        // …and then as deep as those columns need to be to hold the whole
        // list, up to half the window. Eight rows was a number that happened
        // to suit forty-seven commands; this is the rule that number was
        // standing in for, and it goes on being right as the list grows.
        let deep = total.div_ceil(across).clamp(1, half.max(1));
        // …and never deeper than there is anything to put in it. The whole
        // list is what the shape is measured from, not what is padded out to:
        // six blank rows under `:t`'s four commands would be a panel drawn
        // around nothing.
        let deep = deep.min(items.len().max(1));
        (lay(deep, 0), deep)
    } else {
        // A picker is paths — hundreds of them, and no arrangement shows them
        // all — so it stays the glanceable eight and scrolls.
        let deep = items.len().clamp(1, MENU_ROWS);
        (lay(deep, 0).into_iter().take(1).collect(), deep)
    };
    let across = widths.len().max(1);
    let visible = (deep * across).min(items.len());
    // The entries, the footer, and the ring above and below them.
    let height = (deep + 3) as u16;
    if height > area.height || bottom < height {
        return None;
    }
    // Scrolled just enough: the selection stays on the list, and a list that
    // fits never scrolls at all.
    let first = focus
        .saturating_sub(visible.saturating_sub(1))
        .min(items.len().saturating_sub(visible));
    // A scrolled list is measured off the rows actually on screen, not off the
    // ones above them: the column is as wide as what is *in* it.
    if first > 0 {
        widths = lay(deep, first);
    }
    let across = widths.len().max(1);
    let visible = (deep * across).min(items.len() - first);

    let inner = widths
        .iter()
        .sum::<usize>()
        .max(yumete_cjk::str_width(footer) + 1)
        .max(yumete_cjk::str_width(title) + 2);
    let width = (inner + 2).min(area.width as usize) as u16;
    let menu = Rect::new(area.x, bottom - height, width, height);
    // The same ring, at the same rung, with its name in the same corner as the
    // which-key panel's: two panels that open in the same place and do the
    // same kind of thing should not look like two different programs.
    crate::chrome::draw(frame, menu, &crate::chrome::Ring {
        rounded,
        border: Style::default().fg(ink.rule()).bg(ink.paper()),
        ground: Style::default().bg(ink.paper()),
        title: Some((
            title.to_string(),
            Style::default().fg(ink.gold()).bg(ink.paper()),
        )),
    });

    let ground = Style::default().bg(ink.paper());
    let text = ground.fg(ink.text());
    let quiet = ground.fg(ink.quiet());
    let on = Style::default().bg(ink.text()).fg(ink.paper());

    let buf = frame.buffer_mut();
    for slot in 0..visible {
        let i = first + slot;
        if i >= items.len() {
            break;
        }
        let (column, row) = (slot / deep, slot % deep);
        let x = menu.x + 1 + widths[..column].iter().sum::<usize>() as u16;
        let end = (x + widths[column] as u16).min(menu.x + width - 1);
        let y = menu.y + 1 + row as u16;
        let picked = highlight == Some(i);
        let style = if picked { on } else { text };
        if picked {
            for cx in x..end {
                if let Some(cell) = buf.cell_mut((cx, y)) {
                    cell.set_symbol(" ").set_style(style);
                }
            }
        }
        let after = put_text(buf, x + 1, y, end, &items[i].text, style);
        // The note in the quiet ink — except on the inked row, where the whole
        // line is one ground and a second colour on it reads as a mistake.
        if let Some(note) = &items[i].note {
            let at = after + NOTE_GAP as u16;
            if at < end {
                let note_style = if picked { style } else { quiet };
                put_text(buf, at, y, end, note, note_style);
            }
        }
    }
    put_text(
        buf,
        menu.x + 1,
        menu.y + 1 + deep as u16,
        menu.x + width - 1,
        footer,
        quiet,
    );
    Some(menu)
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
    if composes_here(editor) && ime.available() && ime.is_composing() && !ime.panel_is_full() {
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
#[allow(clippy::too_many_arguments)]
fn draw_hud(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    ime: &ImeSession,
    page: Rect,
    caret: (u16, u16),
    panels: &[Rect],
    candidate_panel: bool,
) {
    let how = editor.hud();
    if how == Hud::Off {
        return;
    }
    let typed = hud_line(editor, ime);
    if typed.is_empty() || page.height < 2 {
        return;
    }
    let ink = crate::theme::Palette::of(config);
    let (caret_x, caret_y) = caret;
    if how == Hud::Full {
        draw_hud_panel(
            frame,
            config,
            ink,
            page,
            caret,
            &typed,
            panels,
            candidate_panel,
        );
        return;
    }
    // A 藥丸, not a thread (#284): the corner still points back at the caret,
    // and a cell of ground closes the other end, so what is drawn reads as one
    // small object rather than as a line trailing off the writing. Every mark
    // is one cell wide, so the width does not depend on which corner the
    // placement ends up choosing.
    let width = yumete_cjk::str_width(&typed) as u16 + 3;
    if width >= page.width {
        return;
    }
    let right = page.x + page.width;
    // **Never over the writing.** It used to start at the caret's own column
    // and paint over whatever was on the row below — 整整 covered by `╰ 30`,
    // and in 縱書 over a live 縱, with the leading `╰` swallowed by a wide
    // glyph's second cell. So it goes where nothing is drawn: the margin is the
    // only part of a page that is not somebody's writing.
    //
    // **And margin is a run of blank cells, not the column after the last
    // glyph.** It used to be the latter, which is the margin only on a page
    // that fills left to right. 縱書 fills right to left, so on every row that
    // held any writing at all that column landed hard against the right edge,
    // no candidate could fit, and **the HUD was never drawn for a vertical
    // reader at all**. Runs are the same answer in both directions, and they
    // also find the gap between two short 縱 that a scan for the last glyph
    // walks straight past.
    let free_runs = |frame: &mut Frame, y: u16| -> Vec<(u16, u16)> {
        let mut free = vec![true; page.width as usize];
        {
            let buf = frame.buffer_mut();
            for i in 0..page.width {
                let Some(cell) = buf.cell((page.x + i, y)) else { continue };
                let symbol = cell.symbol();
                if symbol.trim().is_empty() {
                    continue;
                }
                // **The whole glyph, not the cell it starts in.** A wide
                // character's second cell reads back empty, and writing into it
                // is writing into the middle of a 漢字: the terminal never
                // receives it, so the mark simply vanishes.
                for k in 0..yumete_cjk::str_width(symbol).max(1) as u16 {
                    if let Some(slot) = free.get_mut((i + k) as usize) {
                        *slot = false;
                    }
                }
            }
        }
        let mut runs = Vec::new();
        let mut i = 0usize;
        while i < free.len() {
            if !free[i] {
                i += 1;
                continue;
            }
            let start = i;
            while i < free.len() && free[i] {
                i += 1;
            }
            runs.push((page.x + start as u16, page.x + i as u16));
        }
        runs
    };
    // **Anchor, then take the nearest — not the first that fits.**
    //
    // The old rule was 「the row below, else the row above」, and it went wrong
    // exactly where it mattered: a short line being typed with a long line of
    // prose under it put the HUD at the *far end of that prose*, half a screen
    // from the caret, while the empty margin one row up went unused. Whether
    // the mark landed near the eye was decided by how long somebody else's
    // sentence happened to be.
    //
    // This is the placement problem every floating UI has — an anchor (the
    // caret), a **flip** when the preferred side does not fit, a **shift**
    // along the other axis to stay inside the page — with one addition that
    // the usual libraries leave to the caller: the candidates are *scored*, and
    // the winner is the one whose drawn corner ends up closest to the anchor.
    // A row costs eight columns, four 漢字: a mark one row away and level with
    // the caret beats a mark on the caret's own row thirty columns to the
    // right.
    //
    // The caret's own row is a candidate too, and in 橫排 usually the winner:
    // while a sentence is being typed the caret is at the end of it, so the
    // margin immediately to its right is both empty and as near as anything
    // can be.
    let mut best: Option<(u16, u16, u32, String)> = None;
    for step in [0i32, 1, -1, 2, -2] {
        let y = caret_y as i32 + step;
        if y < page.y as i32 || y >= (page.y + page.height) as i32 {
            continue;
        }
        let y = y as u16;
        for (lo, hi) in free_runs(frame, y) {
            // On the caret's own row, stay off the caret and leave it one cell
            // of air on whichever side the mark lands — a mark butted against
            // the character being typed reads as part of it. The caret may be
            // sitting on a wide glyph, so its own cell is two, and the run is
            // cut into the piece before it and the piece after.
            let mut spans = Vec::new();
            if step == 0 {
                let shut = caret_x.saturating_sub(1);
                if lo < shut {
                    spans.push((lo, hi.min(shut)));
                }
                if hi > caret_x + 2 {
                    spans.push((lo.max(caret_x + 2), hi));
                }
            } else {
                spans.push((lo, hi));
            }
            for (lo, hi) in spans {
                if hi.saturating_sub(lo) < width {
                    continue;
                }
                let x = caret_x.clamp(lo, hi - width);
                // The mark points back at the caret, and which end it hangs
                // from depends on which side of the caret the margin turned out
                // to be: in 橫排 nearly always the right, in 縱書 nearly always
                // the left. A corner pointing away from the caret is worse than
                // no corner at all.
                let leftward = x + width <= caret_x;
                let near = if leftward { x + width - 1 } else { x };
                let score = near.abs_diff(caret_x) as u32 + step.unsigned_abs() * 8;
                let corner = match (step.signum(), leftward) {
                    (1, false) => '╰',
                    (1, true) => '╯',
                    (-1, false) => '╭',
                    (-1, true) => '╮',
                    _ => '─',
                };
                let text = if leftward {
                    format!(" {typed} {corner}")
                } else {
                    format!("{corner} {typed} ")
                };
                if best.as_ref().is_none_or(|(_, _, was, _)| score < *was) {
                    best = Some((x, y, score, text));
                }
            }
        }
    }
    let Some((x, y, _, text)) = best else {
        return;
    };
    let style = Style::default()
        .bg(ink.at(yumete_config::rung::BAND))
        .fg(ink.gold());
    put_text(frame.buffer_mut(), x, y, right, &text, style);
}

/// `:view-hud full`: the same string in a **bordered panel, pinned under the
/// caret** — and over whatever is written there (#284).
///
/// The frame and the covering are one decision. A 藥丸 needs five blank cells
/// on one row and can nearly always find them; a panel needs a 3 × (寬+2)
/// rectangle *near the caret*, which a page of prose does not have — so the
/// frame forces the covering. And the converse: once the mark stands on the
/// same paper as the manuscript, nothing but the ring says which characters
/// are not the writer's.
///
/// What that costs is not 「some prose」. In Normal the HUD carries
/// [`Editor::typed_so_far`], so a panel two rows under the caret hides exactly
/// the characters `3`, `2t` and `d3l` are counting. That is the whole reason
/// `basic` is the factory level and this one is asked for by name.
///
/// It keeps off the other panels, which it can no longer see: reading the
/// buffer back cannot tell a drawn 漢字 from a drawn ring, and this level
/// covers 漢字 on purpose. So every panel says where it went (`panels`), and
/// the candidate panel — which wants this same cell and is drawn after — wins
/// outright.
#[allow(clippy::too_many_arguments)]
fn draw_hud_panel(
    frame: &mut Frame,
    config: &Config,
    ink: crate::theme::Palette,
    page: Rect,
    caret: (u16, u16),
    typed: &str,
    panels: &[Rect],
    candidate_panel: bool,
) {
    if candidate_panel {
        return;
    }
    let (caret_x, caret_y) = caret;
    let width = yumete_cjk::str_width(typed) as u16 + 2;
    if width > page.width || page.height < 3 {
        return;
    }
    // Pinned: the column is the caret's, shifted only as far as the page edge
    // makes it. 位置更固定 is what was asked for — a mark that moves with the
    // shape of somebody else's sentence is one the eye has to look for.
    let x = caret_x.min(page.x + page.width - width);
    // Under the caret, and above it when there is no room below. Never *on*
    // it: the character being typed is the one thing the panel must not hide.
    let below = caret_y + 1;
    let above = caret_y.saturating_sub(3);
    let fits = |y: u16| {
        y >= page.y
            && y + 3 <= page.y + page.height
            && !panels.iter().any(|p| {
                p.x < x + width && x < p.x + p.width && p.y < y + 3 && y < p.y + p.height
            })
    };
    let y = if fits(below) {
        below
    } else if caret_y >= page.y + 3 && fits(above) {
        above
    } else {
        // Every place it could stand is somebody else's panel, and a panel
        // with a hole punched in it is worse than one mark fewer.
        return;
    };
    let panel = Rect::new(x, y, width, 3);
    crate::chrome::draw(frame, panel, &crate::chrome::Ring {
        rounded: config.panel.rounded,
        // The same ring at the same rung as every other panel on the screen:
        // what separates a panel from the page is its rule and its 金墨, not a
        // lighter ground.
        border: Style::default().fg(ink.rule()).bg(ink.paper()),
        ground: Style::default().bg(ink.paper()),
        // No name: it is one line of what you just typed, and a title over it
        // would be taller than the thing it names.
        title: None,
    });
    put_text(
        frame.buffer_mut(),
        panel.x + 1,
        panel.y + 1,
        panel.x + width - 1,
        typed,
        Style::default().bg(ink.paper()).fg(ink.gold()),
    );
}

/// The **which-key panel**: what the half-pressed key can be finished with.
///
/// A bordered list, titled in its own top border, in the corner the writing
/// ends at — bottom right in 橫排, bottom left in 縱書, because that is where
/// the eye already is when a line runs out. It replaces the command row for the
/// sequence it is about: two surfaces saying the same thing is the thing this
/// editor keeps taking apart.
///
/// One column while it fits, two when it does not. Not scrolling: a menu you
/// have to scroll is one you cannot answer at a glance, which is the whole of
/// what it is for.
/// The panel for a half-pressed key sequence — `空格` and every other prefix.
fn draw_which_key(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    area: Rect,
    bottom: u16,
    caret: (u16, u16),
) -> Option<Rect> {
    let (title, keys) = editor.pending_menu()?;
    panel::draw(frame, config, area, bottom, caret, &panel::Panel {
        title,
        body: panel::Body::Keys(keys.into_iter().map(|(k, what)| (k.to_string(), what)).collect()),
        tag: None,
    })
}

/// The panel for a footnote or a `%%註釋%%` — what the cursor is standing on.
///
/// It used to be a **full-width four-row strip** along the bottom of the page
/// (#294), which is the wrong price twice over: a note is one short paragraph,
/// so most of those four rows by the whole width were blank, and the four rows
/// came off the manuscript whether the note needed them or not. The panel is
/// the size of what it holds and stands over the page rather than pushing it.
fn draw_note(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    area: Rect,
    bottom: u16,
    caret: (u16, u16),
) -> Option<Rect> {
    // **A wiki name floats the same way** (#287) — when nothing the writer
    // typed answers first, and when the sidebar's 百科 page is not already
    // showing it: one place at a time.
    // **A `[yumete]` line says whether it was read** (#287): the line is
    // inside a comment, so the 批注 panel would otherwise answer first — and
    // 「this comment says `[yumete] 人物.md`」 is not the question anybody has
    // standing on one. Which file, and what became of it, is.
    if let Some(include) = editor.wiki_include_here() {
        use yumete_core::wiki::Source;
        let said = match &include.state {
            Some(Source::Read { entries, .. }) => say!("wiki.read", &include.named, entries),
            Some(Source::Missing { .. }) | None => say!("wiki.missing", &include.named),
            Some(Source::Refused { from, .. }) => {
                say!("wiki.refused", &include.named, from.display())
            }
            Some(Source::Again { .. }) => say!("wiki.again", &include.named),
            Some(Source::TooMany { .. }) => say!("wiki.too-many", &include.named),
        };
        return panel::draw(frame, config, area, bottom, caret, &panel::Panel {
            title: say!("wiki.title"),
            body: panel::Body::Prose(said),
            tag: Some(say!("wiki.open-it")),
        });
    }
    if editor.detail().is_none() {
        let view = editor.wiki_floating()?;
        let tag = view.parts.first().map(|p| {
            let file = p.source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            format!("{file}:{}", p.line + 1)
        });
        return panel::draw(frame, config, area, bottom, caret, &panel::Panel {
            title: view.name.clone(),
            body: panel::Body::Prose(view.as_prose()),
            tag,
        });
    }
    if !editor.detail_visible() || editor.detail_shows_a_row() {
        return None;
    }
    let detail = editor.detail()?;
    let body = detail.rows.first().and_then(|(_, v)| v.clone()).unwrap_or_default();
    if body.is_empty() {
        return None;
    }
    panel::draw(frame, config, area, bottom, caret, &panel::Panel {
        title: detail.title,
        body: panel::Body::Prose(body),
        // Where the note is written, so `gd` has somewhere named to go.
        tag: match detail.links.first() {
            Some(&(_, Some(at))) => Some(say!("detail.on-line", at + 1)),
            _ => None,
        },
    })
}

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
            // **The rule a 分詞 separator draws** (#501). It is a style and not
            // a character, so a picture that dropped it would show the mode as
            // doing nothing at all.
            let underline = style
                .add_modifier
                .contains(ratatui::style::Modifier::UNDERLINED)
                .then(|| hex(style.underline_color, &fg));
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
            // A wiki name's dots survive into the picture (#287).
            let dotted = match style.add_modifier.contains(crate::backend::DOTTED) {
                true => " dotted",
                false => "",
            };
            let rule = match &underline {
                Some(colour) => format!(
                    ";text-decoration:underline{dotted};text-decoration-color:{colour}\
                     ;text-underline-offset:2px"
                ),
                None => String::new(),
            };
            out.push_str(&format!(
                "<span style=\"color:{fg};background:{bg}{weight}{rule}\">{}</span>",
                escape(&run)
            ));
        }
        out.push('\n');
    }
    if let Some(footer) = build_footer() {
        // Dimmed, and outside the drawn rows: the same line as the text shot,
        // in the one colour that reads as「not the page」 on either ground.
        out.push_str(&format!(
            "<span style=\"color:#888888\">{}</span>\n",
            escape(&footer)
        ));
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

///
/// Returns the first cell it did **not** write, so a caller can put something
/// after it — a note beside a row (#291) — without measuring the text twice.
pub(crate) fn put_text(
    buf: &mut ratatui::buffer::Buffer,
    x: u16,
    y: u16,
    limit: u16,
    text: &str,
    style: Style,
) -> u16 {
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
    at
}

/// The `:` command menu — Helix's completion popup, not a wall.
fn draw_command_menu(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    area: Rect,
    status: Rect,
) -> Option<Rect> {
    let Some((":", _)) = editor.prompt() else {
        return None;
    };
    let ink = crate::theme::Palette::of(config);
    let (matches, selected) = editor.command_menu();
    if matches.is_empty() {
        return None;
    }
    // Tab's pick is inked; without one nothing is, because the drawn text on
    // the command line is already saying what the guess is.
    let highlight = selected.map(|i| i.min(matches.len() - 1));
    let focus = highlight.unwrap_or(0);
    let items: Vec<Row> = matches
        .iter()
        // **The whole row is composed in one place** — `Choice::shown` — so
        // that what a menu draws and what the rules say it draws cannot come
        // apart: the name, the spellings worth printing beside it, and how
        // many the fold is standing in front of.
        .map(|e| Row {
            text: format!("{}{}", e.leading, e.shown()),
            // 「冰雪清韻」 beside `custom.6947b838`, for the rows whose name is
            // an identifier and not a word (#291).
            note: e.note.map(str::to_string),
        })
        .collect();
    // Only the highlighted command's help, on one line. Every command's help at
    // once is what covered the page.
    // Its `help` is written in Chinese and is the key it is translated by, the
    // same as every other thing this editor says.
    // …and what it is waiting for, when it is waiting for something. A
    // prerequisite belongs *here*, before the command is run: the writer who
    // typed `:view-hanging` on a horizontal page found out by pressing Enter and
    // watching nothing happen.
    let unmet: Vec<String> = editor
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
            say!("ui.needs-these-first", unmet.join(&say!("label.comma")))
        ),
    };
    // Spread across the window: the command list is short entries and there
    // are a couple of dozen of them, which is exactly the shape that wants
    // columns.
    let title = say!("ui.commands");
    draw_list(
        frame,
        ink,
        config.panel.rounded,
        area,
        status.y,
        List {
            items: &items,
            focus,
            highlight,
            footer: &footer,
            columns: true,
            title: &title,
            cap: MENU_WIDTH as usize,
            // **The list as it stands with nothing typed** — what the shape is
            // measured from, so that typing narrows the panel without moving
            // its rows.
            whole: Some(whole_menu()),
        },
    )
}

/// The `[^` and `](#` panel — the same shape as the `:` menu, in the text
/// (#418 二).
///
/// It stands at the foot beside every other floating panel rather than beside
/// the caret. Two reasons, and the second is the one that decided it: a
/// reference is typed at the end of a sentence, which is where the caret is
/// least likely to have room under it; and a reader who has learned where the
/// `:` menu opens has learned where *the list of things to press Tab through*
/// opens, which is worth more than proximity.
fn draw_reference_menu(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    area: Rect,
    status: Rect,
) -> Option<Rect> {
    let (title, choices, picked) = editor.reference_menu()?;
    let ink = crate::theme::Palette::of(config);
    let highlight = picked.map(|i| i.min(choices.len() - 1));
    let focus = highlight.unwrap_or(0);
    // Without the bracket it closes: the panel is a list of references, and
    // `舊註]` reads as a typo in a list of tags.
    let names: Vec<&str> = choices.iter().map(|c| c.text.trim_end_matches([']', ')'])).collect();
    // The notes in one column, because they are read down rather than across:
    // 「哪一條是我要的」 is answered by the note, and a ragged left edge on the
    // half that answers it makes the eye walk every row.
    let column = names.iter().map(|n| yumete_cjk::str_width(n)).max().unwrap_or(0);
    let items: Vec<Row> = choices
        .iter()
        .zip(&names)
        .map(|(c, name)| Row {
            text: format!("{name}{}", " ".repeat(column - yumete_cjk::str_width(name))),
            note: c.note.clone(),
        })
        .collect();
    let footer = format!("{}/{}", focus + 1, items.len());
    draw_list(
        frame,
        ink,
        config.panel.rounded,
        area,
        status.y,
        List {
            items: &items,
            focus,
            highlight,
            footer: &footer,
            columns: true,
            title: &title,
            cap: MENU_WIDTH as usize,
            // A reference list has no unfiltered form to be measured from —
            // what is on the page *is* the whole of it.
            whole: None,
        },
    )
}

/// How many entries the `:` menu holds with nothing typed, and how wide the
/// widest of them is (#372).
fn whole_menu() -> (usize, usize) {
    let rows: Vec<Row> = yumete_core::command::complete("")
        .iter()
        .map(|e| Row::plain(format!("{}{}", e.leading, e.shown())))
        .collect();
    (rows.len(), rows.iter().map(Row::width).max().unwrap_or(0))
}

/// The `::` search's answers (Feature #224).
///
/// One command a row, name first and then what it does, because *what it
/// does* is what was asked about — a column of bare names would be the
/// command menu again, and the reader who opened this line is the one who
/// could not remember a name. The score is not shown: a number beside every
/// row invites reading it as a ranking to argue with, and the order already
/// says everything it has to say.
fn draw_lookfor_menu(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    area: Rect,
    status: Rect,
) -> Option<Rect> {
    let Some(("::", query)) = editor.prompt() else {
        return None;
    };
    let ink = crate::theme::Palette::of(config);
    let (found, focus) = editor.lookfor_menu();
    // An empty line has nothing to answer yet, and a panel listing every
    // command in the editor is not an answer — it is the whole table.
    if found.is_empty() {
        let footer = match query.is_empty() {
            true => say!("ui.lookfor-empty"),
            false => say!("ui.lookfor-nothing", query),
        };
        draw_list(
            frame,
            ink,
            config.panel.rounded,
            area,
            status.y,
            List {
                items: &[],
                focus: 0,
                highlight: None,
                footer: &footer,
                columns: false,
                title: &say!("ui.lookfor"),
                cap: LOOKFOR_WIDTH as usize,
            whole: None,
            },
        );
        return None;
    }
    // A row is cut with a mark rather than at the panel edge: a sentence that
    // simply stops at the ring reads as the panel being too narrow, and 「…」
    // says instead that the sentence goes on — which is what the reader has to
    // know to decide whether this is the command they meant.
    let room = (area.width as usize)
        .saturating_sub(4)
        .min(LOOKFOR_WIDTH as usize);
    let items: Vec<Row> = found
        .iter()
        .map(|hit| {
            Row::plain(elide(
                &format!(
                    "{}{}   {}",
                    hit.choice.leading,
                    hit.choice.written(),
                    yumete_core::messages::say(hit.choice.help, &[])
                ),
                room,
            ))
        })
        .collect();
    // What ⇥ will do with the highlighted row, spelled out. The one thing a
    // reader has to know here is that nothing on this line runs anything.
    let footer = say!(
        "ui.lookfor-take",
        &format!("{}/{}", focus + 1, found.len()),
        &format!("{}{}", found[focus].choice.leading, found[focus].choice.written())
    );
    let title = say!("ui.lookfor");
    draw_list(
        frame,
        ink,
        config.panel.rounded,
        area,
        status.y,
        List {
            items: &items,
            focus,
            // Always inked, unlike the command menu: there is no drawn on the
            // `::` line saying what ⇥ would take, so the highlight is the only
            // thing that says it.
            highlight: Some(focus),
            footer: &footer,
            // A row is a name **and** a sentence — wide, and long lists of
            // them read down, not across.
            columns: false,
            title: &title,
            cap: room + 2,
            whole: None,
        },
    )
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
    // The very rectangle the page was drawn into — command row, tab bar, sidebar
    // and detail panel all already taken off. Working it out again by hand is
    // how a click came to land a row or two from where it was pointed.
    // …and the work area it landed in, which with two of them is the one
    // question a click has to answer before any of the others.
    let areas = page_areas(editor, config, Rect::new(0, 0, size.width, size.height), 0);
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
    if editor.grid_has_the_pane() {
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
            let drawn = |line: usize| editor.drawn_on_line(line);
            let typed = |line: usize| editor.typed_on_line(line);
            let flat = |line: usize| editor.table_row_at(line);
            let measure = wrap::Measure::new(width, &hide)
                .with_indent(editor.paragraph_indent())
                .with_folds(&fold)
                .with_drawn(&drawn)
                .with_typed_drawn(&typed)
                .with_unwrapped(&flat)
                .with_version(editor.current_buffer().id(), editor.current_buffer().revision())
        .with_edit(editor.current_buffer().edit())
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
            let runs = editor.drawn_on_line(row.line);
            let line_start = buffer.rope().line_to_char(row.line);
            // The row starts where it was drawn: a click anywhere in a
            // paragraph's opening indent means its first character.
            let mut column =
                measure.indent_of(row.line, &buffer.rope().line(row.line).to_string(), row.index_in_line);
            // **A click on drawn text means the character it stands before.**
            // The cells are on the page but not in the file, so they are the
            // one thing a click cannot land *in*.
            let drawn_width = |at: usize| -> usize {
                runs
                    .iter()
                    .filter(|&&(g, _)| g == at)
                    .map(|(_, text)| yumete_cjk::str_width(text))
                    .sum()
            };
            for at in row.start..row.end {
                let c = buffer.rope().char(at);
                let g = drawn_width(at - line_start);
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
    let area = page_areas(editor, config, Rect::new(0, 0, size.width, size.height), 0).tabs;
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
    // **Weight for prose, 品色 for what is not prose** (#449).
    //
    // This said 「weight, not hue」 outright, and it was half right. Bold,
    // italic and a heading *are* writing, and a hue on them is noise — that
    // half stands. But 行内代碼, a quotation and a link are **not writing**,
    // and marking them by being one rung dimmer says 「less important」 about
    // the most exact thing on the line. It is also the first mark to go as the
    // page darkens: dimming the ink from 13.7:1 to 10:1 took 行内代碼 from
    // 7.3:1 to 5.5:1 against the paper, because every rung is a fraction of
    // the ink.
    //
    // So the three of them take 官服品色 — an ordered system, spent in order:
    // 紫 for a literal, 綠 for someone else's words, 藍 for an address. All of
    // them sit **below the prose** in contrast, so the writing is still the
    // brightest thing on the page, which is what the old rule was protecting.
    //
    // A **ground** is not part of this: the backtick is drawn, so the run's
    // extent is already visible. `:theme-fill on` adds one for whoever wants
    // the box.
    let filled = |style: Style, rung| match ink.fill() {
        true => style.bg(ink.at(rung)),
        false => style,
    };
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
        // 一至三品: a literal, to be read character for character.
        Kind::Code => filled(
            Style::default().fg(ink.purple()),
            yumete_config::rung::BAND,
        ),
        // CROSSED_OUT is not everywhere, so the rung carries it as well.
        Kind::Strike => Style::default()
            .fg(ink.furniture())
            .add_modifier(Modifier::CROSSED_OUT),
        // 八至九品: an address is the lowest rank on the page. The underline
        // stays — it is what says 「link」 on a terminal with no colour.
        Kind::Link | Kind::WikiLink => Style::default()
            .fg(ink.azure())
            .add_modifier(Modifier::UNDERLINED),
        // A highlighter pen leaves a ground, so this is a ground — and the pen
        // is 朱, washed until the ink still reads on it.
        Kind::Highlight => Style::default().bg(ink.wash()).fg(ink.text()),
        // A footnote *is* a 朱批.
        Kind::Footnote => Style::default().fg(ink.mark()),
        // **紅刪綠增** (#499), the one place on this page where a colour means
        // what it means everywhere else in the world rather than what the 品色
        // ladder says. 朱 is already 「這裏不對」 and 綠 is already 六七品, so
        // the two ranks that were free are also the two a reader of any diff
        // already knows. The ink says which and the ground says *where* — a
        // changed 的 is one character wide, and one character of coloured ink
        // in a paragraph is not findable.
        Kind::Gone => Style::default()
            .bg(ink.short_wash(crate::theme::Accent::Mark))
            .fg(ink.text()),
        Kind::Added => Style::default()
            .bg(ink.short_wash(crate::theme::Accent::Green))
            .fg(ink.text()),
        // Not part of the book: set back, but never hidden and never below
        // reading — a note you cannot see is a note you will not act on, and
        // this one measured 2.54:1.
        Kind::Comment => Style::default()
            .fg(ink.furniture())
            .add_modifier(Modifier::ITALIC),
        // Typst's own code: the instructions that make the page, not decoration
        // around writing. Set back, never taken away.
        // Typst's own code: a literal too, and the same rank.
        Kind::Code2 => Style::default().fg(ink.purple()),
        // **Code in its own grammar's colours** (#420), after the themes
        // everyone already reads code in — One Dark, Catppuccin, VS Code's
        // Dark+ agree on nearly all of it: keywords purple, functions blue,
        // types yellow, strings green, numbers orange, comments grey and
        // italic, and **names in the plain foreground**.
        //
        // Every rank and its second tone has a job here, **except 朱**, which
        // only ever says 「這裏不對」:
        //
        // | 主調 | 變調 |
        // | --- | --- |
        // | 金 property, key, attribute | 黃 type |
        // | 朱 — | 橙 number, constant |
        // | 紫 keyword | 粉 self, this |
        // | 藍 function | 青 escape, `{…}` |
        // | 綠 string | 黃綠 HTML tag |
        Kind::Token(token) => {
            use yumete_core::code::Token;
            let fg = |c| Style::default().fg(c);
            match token {
                Token::Plain | Token::Operator => fg(ink.text()),
                Token::Keyword => fg(ink.purple()),
                Token::Builtin => fg(ink.pink()),
                Token::Function => fg(ink.azure()),
                Token::Escape => fg(ink.cyan()),
                Token::Property | Token::Attribute => fg(ink.gold()),
                Token::Type => fg(ink.amber()),
                Token::String => fg(ink.green()),
                Token::Tag => fg(ink.lime()),
                Token::Constant => fg(ink.orange()),
                Token::Comment => fg(ink.furniture()).add_modifier(Modifier::ITALIC),
                Token::Punctuation => fg(ink.marker()),
            }
        }
    }
}

/// **The ground a table's row `nth` sits on** — the one answer, for every way
/// a table is drawn (#458).
///
/// `0` is the header, `1` the `---` rule under it, and the body counts on from
/// `2`. There are two renderers — a `|` table read in a chapter, and the
/// whole-window grid `t t` hands the pane — and 「to tf tb tt 的隔行底色应该是
/// 统一的参数」: a reader who turns the same table over four ways must not get
/// four answers. So neither renderer holds a number; both call this.
///
/// ⚠️ **第 88, not the paper.** The stripe began a whole rung out, at 第 90 —
/// which *is* the page — so every other row said two things at once: 「a
/// different row」 and 「out of the table」. 「对比太强烈」.
///
/// ⚠️ **And a ground is not a mark: the area does the work.** 1.09:1 is
/// invisible on a tick or a rule and plenty across a row of cells — 「底色虽然
/// 靠近，但是因为面积大，还是有很好的区分效果」.
pub(crate) fn table_row_depth(nth: usize) -> Option<i32> {
    // **Two grounds, and one of them is the paper** (#480). It was three — a
    // louder one for the header, two quieter ones alternating under the body —
    // and three grounds is one more than a table has things to say: 「表格既然
    // 已经隔行分色了……否则表格会有三种不同的」.
    //
    // So the rows simply alternate, the header included, and every other row
    // is the page itself. ⚠️ **The header is still told apart** — by its ink,
    // which is 金, the colour this palette has always given a table's header
    // row. Ground says 「which row」, ink says 「which kind of row」, and neither
    // is asked to do the other's job.
    // ⚠️ **`None` is 「whatever is under it」, not 「the paper」.** Naming the
    // paper would be right on a page and wrong inside a `:::`, where what is
    // under the row is the callout's wash — the table would have punched the
    // same hole in it that the fence used to.
    match nth {
        // ⚠️ **The header and the rule under it are one thing** (#486), so they
        // take one ground — and the page's, because the header is already told
        // apart by its ink. Banding the header alone split the two: a band that
        // stopped at the `---` read as a row of its own with a line under it,
        // rather than as a head.
        0 | 1 => None,
        // **How deep, not which rung** (#488). Four rungs under whatever the
        // row sits on: the paper on an ordinary page — which is 第 86 檔 to the
        // character, so nothing there moves — and the callout's own wash inside
        // a `:::`, which is what makes the band read as part of the block
        // instead of a patch stuck on it.
        n if n % 2 == 0 => {
            Some(i32::from(yumete_config::rung::PAPER - yumete_config::rung::WORD_TINT))
        }
        _ => None,
    }
}

/// A 品色 block's style: the colour on the writing, and what is under it.
///
/// Three cases, and the third is the one that was missed: **inside a `:::` the
/// callout's wash is the ground**, whatever `:theme-fill` says, because the
/// callout is a rectangle and a hole in it is not a style, it is a mistake.
fn inked(
    colour: ratatui::style::Color,
    inside: Option<yumete_core::markdown::Callout>,
    ink: crate::theme::Palette,
) -> Style {
    match (inside, ink.fill()) {
        (Some(callout), _) => Style::default().bg(ink.washed(wash_of(callout))).fg(colour),
        (None, true) => Style::default().bg(ink.at(yumete_config::rung::BAND)).fg(colour),
        (None, false) => Style::default().fg(colour),
    }
}

/// Which colour a callout wears — GitHub's five, 「从轻到重」 (#483).
///
/// | | | 為什麼 |
/// | --- | --- | --- |
/// | `[!NOTE]` | 藍 八九品 | 記一筆 |
/// | `[!TIP]` | 綠 六七品 | 幫得上忙 |
/// | `[!IMPORTANT]` | 紫 一三品 | 不知道會辦不成 |
/// | `[!WARNING]` | 黃 皇室 | 有風險，現在就看 |
/// | `[!CAUTION]` | 朱 | 會出事 |
///
/// ⚠️ **朱 is not a rank here.** It is 四五品 on the robe chart and the last of
/// these all the same: red for the worst thing is the one convention nobody has
/// to learn, and in this editor 朱 already means 這裏不對 — a merge conflict's
/// markers, a footnote's number, a row with the wrong number of columns. Two
/// colours both meaning that is neither of them meaning it.
fn wash_of(callout: yumete_core::markdown::Callout) -> crate::theme::Accent {
    use crate::theme::Accent;
    use yumete_core::markdown::Callout;
    match callout {
        Callout::Note => Accent::Azure,
        Callout::Tip => Accent::Green,
        Callout::Important => Accent::Purple,
        Callout::Warning => Accent::Amber,
        Callout::Caution => Accent::Mark,
    }
}

/// How a whole row is set, given the block its line belongs to.
///
/// Blocks colour the *row*, inline runs colour the characters, and the two
/// compose — a bold word inside a `::: warning` keeps its bold and gains the
/// container's ground.
fn block_style(block: yumete_core::markdown::Block, ink: crate::theme::Palette) -> Option<Style> {
    use yumete_core::conflict::Side;
    use yumete_core::markdown::Block;
    // **The four callouts are four 品色 now** (#459), in the order the ranks
    // themselves run — 「从轻到重」:
    //
    //   note 藍（八九品） · tip 綠（六七品） · warning 紫（一三品） · danger 朱
    //
    // ⚠️ **This was tried once and reverted, and the reason it failed is not
    // the reason it works now.** The four used to be four *greys* 1.4–2.4 ΔE
    // apart, which is below the threshold at which two flat grounds can be told
    // apart at all. What tells them apart here is hue, held at a fixed distance
    // off the page by [`Palette::washed`]: the distance says 「這是一塊」 and
    // the hue says 「哪一塊」.
    //
    // ⚠️ **朱 stays at the bottom**, though the ranks would put 紫 there. It is
    // the only colour in this editor with an existing hard meaning — a merge
    // conflict's markers, a footnote's number, a row with the wrong number of
    // columns — and two colours both meaning 「這裏不對」 is neither of them
    // meaning it. Red as the worst thing is also the one convention nobody has
    // to learn.
    let band = || Some(Style::default().bg(ink.at(yumete_config::rung::BAND)));
    match block {
        // A note running over several lines has its ink from its runs, and no
        // ground: a one-line note has none either (#288).
        Block::Prose | Block::Heading(_) | Block::Item { .. } | Block::Comment { .. } => None,
        // An aside is a block on the page because it is a block on paper.
        // **A table is a block too** (#270). It was the one thing in this list
        // that had a shape on the page and no ground under it, so a table in a
        // chapter read as prose that happened to have `|` in it. The same rung
        // as the fence and the quote: 「這裏是一塊」 is the whole message, and
        // the cell tint a grid draws (#212, #229) is patched onto this rather
        // than instead of it.
        // 六至七品: a quotation is another voice. 一至三品 for a fence, the
        // same rank as the 行内 form: it is a literal that happens to be
        // several lines long.
        //
        // The ground stays only when `:theme-fill` asks for it — 「>」 is drawn
        // on every line of a quote and a fence has its own ``` — **except
        // inside a `:::`**, where the callout's own wash has to carry on
        // underneath or the block is cut in half (#468).
        Block::Quote { inside } => Some(inked(ink.green(), inside, ink)),
        Block::Code { inside } => Some(inked(ink.purple(), inside, ink)),
        // A callout keeps its ground whatever `:theme-fill` says: 「這是一塊」
        // is half of what it has to say, and it has no marker of its own on
        // every line the way a quote and a fence do.
        Block::Container(callout) => Some(Style::default().bg(ink.washed(wash_of(callout)))),
        // **A table is read across, so the rows are banded alternately** — one
        // row's cells must be tellable from the next's, and in 縱書 a cell that
        // wraps to three lines is unreadable without it. The header takes the
        // rung the panels' own headers take; the body alternates either side of
        // the page, which is a difference the eye finds and does not read.
        //
        // Grounds, not hues: this has to survive a 16-colour terminal and a
        // light page, and it must not spend one of the four 品色 on a shape.
        // The ground is [`table_row_rung`]'s, for every renderer; the header's
        // ink is 金, which is what this palette has always called a table's
        // header row — the same colour the panels label a column with.
        Block::Table { nth, inside } => {
            // Inside a `:::` the two grounds stack: the callout's wash is what
            // the row's *width* is painted with (it is the callout that runs to
            // the edge), and the table's own band goes over it under the row's
            // own characters — see the fill in `draw_horizontal`.
            let under = match inside {
                Some(callout) => Style::default().bg(ink.washed(wash_of(callout))),
                None => Style::default(),
            };
            let beneath = under.bg.unwrap_or(ink.paper());
            let ground = match table_row_depth(nth) {
                Some(depth) => under.bg(ink.over(beneath, depth)),
                None => under,
            };
            Some(match nth {
                0 => ground.fg(ink.gold()),
                1 => ground.fg(ink.furniture()),
                _ => ground,
            })
        }
        // Metadata and scene breaks are furniture, not writing.
        Block::FrontMatter | Block::Rule | Block::FootnoteDef => {
            Some(Style::default().fg(ink.furniture()))
        }
        // **A merge conflict is two grounds and a warning** (#249). The two
        // sides have to be told apart at a glance and there are exactly two of
        // them, so this is the one place hue is doing real work: ours on the
        // same quiet band every other block sits on, theirs on 金 — 「這不是正
        // 文」 — which is as far from a grey band as this palette goes without
        // reaching for 朱. 朱 is kept for the markers themselves, because an
        // unresolved conflict is the definition of 這裏不對. The ancestor is
        // furniture: it is what nobody wrote, only what both sides left.
        Block::Conflict(None) => Some(Style::default().fg(ink.mark())),
        Block::Conflict(Some(Side::Ours)) => band(),
        Block::Conflict(Some(Side::Theirs)) => Some(Style::default().bg(ink.wash())),
        Block::Conflict(Some(Side::Base)) => Some(Style::default().fg(ink.furniture())),
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
fn sidebar_columns(editor: &Editor, config: &Config, side: Side, total: u16) -> u16 {
    // **The two layers share one width**, the wider one's — a slot is one
    // column down the side of the page, not two of different widths with a
    // ragged edge between them.
    let mut want = 0usize;
    if let Some(sidebar) = editor.panel(side) {
        // **The search panel asks for more**, because it is a form: a box,
        // three switches (大小寫 is three ways, not a tick) and the hits. At
        // the tree's width the excerpt is five characters (#419).
        if sidebar.view() == View::Search {
            want = SEARCH_WIDTH.max(config.editor.sidebar_width);
        }
        want = want.max(if sidebar.wide() {
            // One column of padding on the left, the rule on the right, and
            // the two the outline indents its rows by.
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
        });
    }
    // **A transient panel does not open on a page too narrow to spare it.**
    // `split_detail` used to refuse below 30 columns, and the reason holds:
    // the grid is what the window is for, and a panel that leaves eight
    // columns of it is worse than no panel.
    if let Some(kind) = editor.transient(side).filter(|_| total >= DETAIL_WIDTH) {
        let asked = match kind {
            // A 字典 answer is 名 and 值 on one line and cannot be folded: too
            // narrow and the reading runs off the edge of the one panel that
            // exists to show it whole.
            Transient::Dictionary => editor
                .transient_rows(side)
                .iter()
                .map(|row| yumete_cjk::str_width(&row.name) + 3)
                .max()
                .unwrap_or(0),
            Transient::Detail => editor.detail_width().unwrap_or(config.editor.detail_width),
        };
        want = want.max(asked).min(total as usize / 2).max(12);
    }
    // A slot narrower than three cells cannot be drawn — and the drawing used
    // to *return* at that width, leaving the rectangle it had been given
    // unpainted: a black stripe down a light page, for the third time. Below
    // three cells there is no slot, so no rectangle is handed out.
    match (want as u16).min(total.saturating_sub(8)) {
        got if got < 3 => 0,
        got => got,
    }
}

/// How wide the search panel wants to be — Feature #419.
///
/// Wide enough for 「完整匹配」 beside 「正則」 and for an excerpt with a few
/// words in it. It is the one view that is a form rather than a list.
const SEARCH_WIDTH: usize = 32;

/// The narrowest page a transient panel will open on.
///
/// Wide enough for a heading and a 拆分 sequence side by side, and no
/// narrower: below this the grid is what the window is for.
const DETAIL_WIDTH: u16 = 30;

/// **How one slot divides between its two layers** — Feature #293.
///
/// The bottom takes what it needs and no more than half; the top keeps the
/// rest. With nothing resident above it the bottom takes the slot whole, which
/// is how the detail panel keeps the shape it has always had.
///
/// Asked by the drawing and by the mouse, so it is worked out here rather than
/// twice.
fn slot_layers(editor: &Editor, side: Side, slot: Rect) -> [Rect; 2] {
    let none = Rect::new(slot.x, slot.y, 0, 0);
    let top = editor.panel(side).is_some();
    let bottom = editor.transient(side).is_some();
    match (top, bottom) {
        (false, false) => [none, none],
        (true, false) => [slot, none],
        (false, true) => [none, slot],
        (true, true) => {
            // What the bottom would like: its fields, a title and a blank row.
            // Half the slot at most — the top is what a reader opened.
            let want = (editor.transient_len(side) as u16 + 2).min(slot.height / 2);
            let up = slot.height - want;
            [
                Rect::new(slot.x, slot.y, slot.width, up),
                Rect::new(slot.x, slot.y + up, slot.width, want),
            ]
        }
    }
}

/// The file sidebar, in the columns taken off the left of the page.
///
/// A rule rather than a border: one column of `│` says "this is a different
/// thing" and costs one cell, where a box costs four and a corner.
fn draw_sidebar(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    side: Side,
    area: Rect,
) -> Option<Position> {
    let Some(sidebar) = editor.panel(side) else {
        return None;
    };
    if area.width < 3 {
        return None;
    }
    // A form and a list of hits, not rows of a tree (#419).
    if sidebar.view() == View::Search {
        return draw_search(frame, editor, config, side, area);
    }
    // The entry under the cursor, drawn from the cursor (#287).
    if sidebar.view() == View::Wiki {
        draw_wiki(frame, editor, config, side, area);
        return None;
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
    let on = match editor.panel_focus() == Some((side, Layer::Top)) {
        true => Style::default().bg(ink.text()).fg(ink.paper()),
        false => Style::default().bg(ink.selection()).fg(ink.text()),
    };

    frame.render_widget(Clear, area);
    // **A 漢字 cannot be covered by halves.** It owns two cells, and the
    // renderer skips whatever a wide glyph covers — so a border written into
    // the second of them is stored and then never emitted, and the panel opens
    // with its whole left wall missing. Blank the glyph; the wall gets a cell.
    vertical::clear_wide_left_edge(frame.buffer_mut(), area);
    // **The rule goes on the side facing the writing**, so the panel's own
    // columns always sit against the page and its outer edge is the window's.
    let rule = match side {
        Side::Left => area.x + area.width - 1,
        Side::Right => area.x,
    };
    let (from, to) = match side {
        Side::Left => (area.x, rule),
        Side::Right => (area.x + 1, area.x + area.width),
    };
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in from..to {
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
    put_text(buf, from + 1, area.y, to, &sidebar.title(), quiet);
    let rows = sidebar.rows();
    let visible = (area.height as usize).saturating_sub(1);
    if visible == 0 {
        return None;
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
            for x in from..to {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(" ").set_style(style);
                }
            }
        }
        // In the tree a directory says which way it is facing, and a file is
        // indented past where that mark would be so the names line up. The flat
        // views spend `depth` on an index instead, so they get no indent — and
        // in the buffer list `expanded` marks the one being written, while in
        // the outline the pair means 「holds other headings」 and 「open」.
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
            // The mark goes in front of the indent rather than after it, so
            // that 「does this fold, and is it open」 reads down one column
            // (#37). Spending two cells per level on marks instead would cost
            // a narrow sidebar the titles it exists to show.
            View::Outline => {
                let mark = match (row.is_dir, row.expanded) {
                    (true, true) => "▾ ",
                    (true, false) => "▸ ",
                    (false, _) => "  ",
                };
                format!("{mark}{}", row.name)
            }
            // Handled above: they fill no rows.
            View::Search | View::Wiki => row.name.clone(),
        };
        put_text(buf, from + 1, y, to, &line, style);
    }
    // Only the search panel has a caret to report.
    None
}

/// **The sidebar's 百科 page** (#287): the entry the cursor is on, kept in
/// place. The name in 金, the breadcrumb set back, sub-headings in 金 at the
/// depth they have *within* the entry, and the global entries under 「全局」.
fn draw_wiki(frame: &mut Frame, editor: &Editor, config: &Config, side: Side, area: Rect) {
    use yumete_core::editor::WikiLine;
    let ink = crate::theme::Palette::of(config);
    let ground = ink.ground(yumete_config::rung::CHROME);
    let text = ground.fg(ink.text());
    let head = ground.fg(ink.gold()).add_modifier(Modifier::BOLD);
    let quiet = ground.fg(ink.quiet());
    frame.render_widget(Clear, area);
    vertical::clear_wide_left_edge(frame.buffer_mut(), area);
    let rule = match side {
        Side::Left => area.x + area.width - 1,
        Side::Right => area.x,
    };
    let (from, to) = match side {
        Side::Left => (area.x + 1, rule),
        Side::Right => (area.x + 2, area.x + area.width),
    };
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in area.x..area.x + area.width {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(ground);
            }
        }
        if let Some(cell) = buf.cell_mut((rule, y)) {
            cell.set_symbol("│").set_style(quiet);
        }
    }
    let bottom = area.y + area.height;
    let width = to.saturating_sub(from).max(1) as usize;
    let Some(view) = editor.wiki_here() else {
        put_text(buf, from, area.y, to, &say!("wiki.panel-empty"), quiet);
        return;
    };
    let mut y = area.y;
    let line = |buf: &mut ratatui::buffer::Buffer, y: &mut u16, s: &str, style: Style| {
        let chars: Vec<char> = s.chars().collect();
        for (a, b) in yumete_core::wrap::line_rows(s, width) {
            if *y >= bottom {
                return;
            }
            let row: String = chars[a.min(chars.len())..b.min(chars.len())].iter().collect();
            put_text(buf, from, *y, to, &row, style);
            *y += 1;
        }
    };
    let mixed = view.parts.iter().any(|p| p.global) && view.parts.iter().any(|p| !p.global);
    let mut global_said = false;
    for (i, part) in view.parts.iter().enumerate() {
        if i > 0 {
            y += 1;
            let mark = match mixed && part.global && !global_said {
                true => {
                    global_said = true;
                    format!("── {} ──", say!("wiki.global"))
                }
                false => "──".to_string(),
            };
            line(buf, &mut y, &mark, head);
        }
        line(buf, &mut y, &view.name, head);
        if !part.trail.is_empty() {
            line(buf, &mut y, &part.trail.join(" › "), quiet);
        }
        y += 1;
        for body in &part.lines {
            match body {
                WikiLine::Heading(depth, title) => {
                    line(buf, &mut y, &format!("{} {title}", "#".repeat(*depth)), head)
                }
                WikiLine::Text(t) => line(buf, &mut y, t, text),
            }
        }
    }
}

/// **The search panel** — Feature #419.
///
/// A form above a list: what to look for, three switches, and what it found.
/// One line per hit, because a line of a novel is a paragraph — the context of
/// the highlighted one goes in the command row, which is the width of the
/// window instead of the width of a column.
/// Answers **where it put its caret**, when the keys are in one of its boxes.
///
/// ⚠️ The candidate panel has to stand under the caret it belongs to. It knew
/// about the command line's caret and the picker's, and fell back to the
/// *writing's* cursor for anything else — so typing 中文 into this panel's
/// query box drew the candidates halfway down the manuscript, nowhere near the
/// characters they were for.
fn draw_search(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    side: Side,
    area: Rect,
) -> Option<Position> {
    use yumete_core::search_panel::Field;
    let find = editor.search();
    let ink = crate::theme::Palette::of(config);
    let ground = ink.ground(yumete_config::rung::CHROME);
    let text = ground.fg(ink.text());
    let head = ground.fg(ink.gold()).add_modifier(Modifier::BOLD);
    let quiet = ground.fg(ink.quiet());
    let wrong = ground.fg(ink.mark());
    // **A field is a hole in the panel, not another part of its face.** The
    // panel's own ground is 第 81 檔 and the box used to be painted with it, so
    // an empty 尋找 box was a blank strip of panel with a caret somewhere in it
    // — 「不然还是不知道这里有个可以输入的地方」. The ground of a field is the
    // **page's** (第 90 檔), the one place in the interface where writing is
    // typed, and a rule above and below closes it (#447).
    let field = ink.ground(yumete_config::rung::PAPER).fg(ink.text());
    let keys_here = editor.panel_focus() == Some((side, Layer::Top));
    // Whichever cell the keys are on is inked; the rest are quiet — the same
    // 「這裏」 the tree marks its row with, and it costs no colour.
    let on = Style::default().bg(ink.text()).fg(ink.paper());
    let cell = |field: Field| match keys_here && find.field == field {
        true => on,
        false => text,
    };

    frame.render_widget(Clear, area);
    vertical::clear_wide_left_edge(frame.buffer_mut(), area);
    let rule = match side {
        Side::Left => area.x + area.width - 1,
        Side::Right => area.x,
    };
    let (from, to) = match side {
        Side::Left => (area.x, rule),
        Side::Right => (area.x + 1, area.x + area.width),
    };
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in from..to {
            if let Some(c) = buf.cell_mut((x, y)) {
                c.set_symbol(" ").set_style(ground);
            }
        }
        if let Some(c) = buf.cell_mut((rule, y)) {
            c.set_symbol("│").set_style(quiet);
        }
    }
    let left = from + 1;
    // **The title says where it is looking.** One panel behaves two ways —
    // this file is searched as you type, a folder waits for `Enter` — and the
    // difference has to be on the screen (#419).
    // Spelled out rather than asked of `Where`, so the tags sit where the
    // messages test can see them (`messages.rs::said` reads `say!` calls).
    let place = match &find.scope {
        yumete_core::search_panel::Where::Buffer => say!("search.where.buffer"),
        yumete_core::search_panel::Where::Folder => say!("search.where.folder"),
        yumete_core::search_panel::Where::Workspace => say!("search.where.workspace"),
        yumete_core::search_panel::Where::Project => say!("search.where.project"),
        // Named outright: say the name, which is what the reader typed.
        yumete_core::search_panel::Where::Named(path) => path.display().to_string(),
    };
    let title = format!("{}  {place}", say!("label.panel.search"));
    put_text(buf, left, area.y, to, &title, head);
    // **The count, and nothing when nothing was asked.** `0 處` and 「not
    // asked yet」 are two different findings (#419).
    let (tally, style) = match (find.broken, find.stale, find.asked()) {
        (true, _, _) => (say!("search.bad-pattern"), wrong),
        (_, true, true) => (say!("search.enter-to-look"), head),
        (false, _, false) => (String::new(), quiet),
        (false, _, true) if find.total == 0 => (say!("search.none"), quiet),
        (false, _, true) => (say!("search.hits", find.total), quiet),
    };
    if !tally.is_empty() {
        let w = yumete_cjk::str_width(&tally) as u16;
        put_text(buf, to.saturating_sub(w + 1), area.y, to, &tally, style);
    }

    // The boxes. A caret where the keys are, and the whole of one inked when
    // it arrived selected — `空格 /` leaves it that way so one key does both.
    let typing = editor.mode() == yumete_core::input::Mode::Field;
    let room = to.saturating_sub(left + 2) as usize;
    let mut y = area.y + 2;
    let mut caret: Option<Position> = None;
    let mut draw_box = |buf: &mut ratatui::buffer::Buffer, which: Field, what: &str, y: u16| {
        let shown: String = match what.chars().count() > room {
            true => what.chars().skip(what.chars().count() - room).collect(),
            false => what.to_string(),
        };
        let here = find.field == which;
        // **Inked means 「the whole of this is selected」, not 「the keys are
        // here」.** While it is being typed into, the caret says where you
        // are — and a box drawn the same way whether or not `空格 /` had
        // selected its contents would hide what that selection is for.
        let style = match (find.all_selected && here && !shown.is_empty(), typing && here) {
            (true, _) => on,
            (false, true) => field,
            (false, false) => match keys_here && here {
                true => on,
                false => field,
            },
        };
        // **The whole row is painted, not just the characters.** A box with
        // one word in it and no ground behind it does not read as a box —
        // there is nothing to say where you may type or how much room there
        // is. Padded to the panel's width so the field has edges.
        let wide = to.saturating_sub(left) as usize;
        let used = yumete_cjk::str_width(&shown) + 1;
        let filled = format!(" {shown}{}", " ".repeat(wide.saturating_sub(used)));
        put_text(buf, left, y, to, &filled, style);
        if typing && here && !find.all_selected {
            let at = left + 1 + yumete_cjk::str_width(&shown) as u16;
            if at < to {
                if let Some(c) = buf.cell_mut((at, y)) {
                    // On the **field's** ground, not the panel's: a caret cell
                    // painted with the head style punched a panel-coloured
                    // hole in the box it is standing in.
                    c.set_symbol("▏").set_style(field.fg(ink.gold()));
                }
                caret = Some(Position { x: at, y });
            }
        }
    };
    draw_box(buf, Field::Query, &find.query, y);
    // **The replace row is only there when it is meant to be** — `:search` is
    // for looking, `:replace` for changing, and `r`/`R` are live only here.
    if find.replacing {
        y += 1;
        draw_box(buf, Field::Replace, &find.replace, y);
    }
    // The two rules that close the boxes. Drawn after them, because the row
    // below the last box is only known once it is known whether there are two.
    let wide = to.saturating_sub(left) as usize;
    let edge = "─".repeat(wide);
    put_text(buf, left, area.y + 1, to, &edge, quiet);
    put_text(buf, left, y + 1, to, &edge, quiet);
    y += 1;

    // The switches. 大小寫 is three ways, not a tick, so it says which one.
    let tick = |on: bool| match on {
        true => "[x]",
        false => "[ ]",
    };
    // **One switch to a row.** They used to share the first one — 正則 on the
    // left, 完整匹配 pushed to the right edge — and in a narrow panel the
    // second one simply did not appear, because there was no room for it at
    // the right and nowhere else for it to go. Three rows, always all three.
    let y = y + 1;
    put_text(buf, left, y, to, &format!("{} {}", tick(find.regex), say!("search.regex")), cell(Field::Regex));
    let y = y + 1;
    // Spelled out rather than asked of `Case`, so the tags sit where the
    // messages test can see them: it reads `say!` calls, and a tag returned
    // from a `match` is a tag nobody can find (`messages.rs::said`).
    let which = match find.case {
        yumete_core::search_panel::Case::Smart => say!("search.case.smart"),
        yumete_core::search_panel::Case::Sensitive => say!("search.case.sensitive"),
        yumete_core::search_panel::Case::Insensitive => say!("search.case.insensitive"),
    };
    let case = format!("{}  {}", say!("search.case"), which);
    put_text(buf, left, y, to, &case, cell(Field::Case));
    let y = y + 1;
    put_text(buf, left, y, to, &format!("{} {}", tick(find.whole), say!("search.whole")), cell(Field::Whole));

    // What it found. Quiet when the pattern is broken: these are the answer to
    // what the box held a keystroke ago, not to what it holds now.
    let top = y + 2;
    let room = (area.y + area.height).saturating_sub(top) as usize;
    if room == 0 {
        return caret;
    }
    let rows = find.rows();
    let first = find
        .selected
        .saturating_sub(room.saturating_sub(1))
        .min(rows.len().saturating_sub(room));
    for slot in 0..room.min(rows.len().saturating_sub(first)) {
        let i = first + slot;
        let y = top + slot as u16;
        let picked = i == find.selected && !find.broken;
        let inked = picked && keys_here && find.field == Field::Results;
        if inked {
            for x in from..to {
                if let Some(c) = buf.cell_mut((x, y)) {
                    c.set_symbol(" ").set_style(on);
                }
            }
        }
        let (line, plain) = match &rows[i] {
            // A file, with the mark the tree and the outline already use for
            // 「there is more under this」.
            yumete_core::search_panel::Row::File { path, hits, folded } => (
                format!(
                    "{} {}  {hits}",
                    if *folded { "▸" } else { "▾" },
                    path.display()
                ),
                head,
            ),
            yumete_core::search_panel::Row::Hit(at) => {
                let hit = &find.hits[*at];
                // Indented under its file when there is one to be under.
                let pad = match hit.file.is_some() {
                    true => "  ",
                    false => "",
                };
                (format!("{pad}{:>5}  {}", hit.line + 1, hit.excerpt), text)
            }
        };
        let style = match (find.broken, inked) {
            (true, _) => quiet,
            (false, true) => on,
            (false, false) => plain,
        };
        put_text(buf, left, y, to, &line, style);
    }
    caret
}

/// **The 字典, in the bottom layer of a slot** — Feature #215, #293.
///
/// A list of 名／值 pairs with no indent and no marks: they are lined up into
/// columns already, and a narrow slot has no cells to spend on decorating
/// them. Nothing is highlighted either — nothing here is chosen, only read —
/// so what says the keys are in it is the title, inked.
fn draw_dictionary(frame: &mut Frame, editor: &Editor, config: &Config, side: Side, area: Rect) {
    if area.width < 3 || area.height == 0 {
        return;
    }
    let ink = crate::theme::Palette::of(config);
    let ground = ink.ground(yumete_config::rung::CHROME);
    let text = ground.fg(ink.text());
    let head = ground.fg(ink.gold()).add_modifier(Modifier::BOLD);
    let quiet = ground.fg(ink.quiet());

    frame.render_widget(Clear, area);
    vertical::clear_wide_left_edge(frame.buffer_mut(), area);
    let rule = match side {
        Side::Left => area.x + area.width - 1,
        Side::Right => area.x,
    };
    let (from, to) = match side {
        Side::Left => (area.x, rule),
        Side::Right => (area.x + 1, area.x + area.width),
    };
    let buf = frame.buffer_mut();
    for y in area.y..area.y + area.height {
        for x in from..to {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol(" ").set_style(ground);
            }
        }
        if let Some(cell) = buf.cell_mut((rule, y)) {
            cell.set_symbol("│").set_style(quiet);
        }
    }

    let rows = editor.transient_rows(side);
    let focused = editor.panel_focus() == Some((side, Layer::Bottom));
    let title = match focused {
        true => Style::default().bg(ink.text()).fg(ink.paper()),
        false => quiet,
    };
    let name = yumete_core::messages::say(Panel::Dictionary.tag(), &[]);
    put_text(buf, from + 1, area.y, to, &name, title);
    let visible = (area.height as usize).saturating_sub(1);
    let first = editor
        .transient_scroll()
        .min(rows.len().saturating_sub(visible.max(1)));
    for slot in 0..visible.min(rows.len().saturating_sub(first)) {
        let row = &rows[first + slot];
        let style = if row.is_dir { head } else { text };
        put_text(buf, from + 1, area.y + 1 + slot as u16, to, &row.name, style);
    }
}

/// The `Space f` / `Space b` picker.
fn draw_picker(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    ime: &ImeSession,
    area: Rect,
) -> Option<(Position, Option<Rect>)> {
    let picker = editor.picker()?;
    let ink = crate::theme::Palette::of(config);
    let matches = picker.matches();
    // The name, and **which of its characters the query is standing on** — the
    // one thing that says why a name with scattered letters is on the list at
    // all (2026-09-18).
    let items: Vec<(String, Vec<usize>)> = matches
        .iter()
        .map(|i| (i.label().to_string(), picker.hits(i.label())))
        .collect();
    // The code being composed shows in the query, where a `/` search shows it
    // too: the reader has to see 「di3」 turn into 「第」 before choosing.
    let preedit = prompt_preedit(editor, ime);
    // ⚠️ **Built in three parts, because the caret is measured off the first
    // one** (2026-09-17). It used to be one `format!` and the caret's column
    // was worked out backwards — `footer.len() - query.len() - preedit.len()`
    // — which was a byte index into the footer. Adding anything *after* the
    // query put that index inside 「開」 and the editor panicked while drawing:
    // 「space + f + j + j + Esc 我就退出 yumete 了」. Now nothing is sliced.
    let counted = format!(
        "{}/{}  ",
        if items.is_empty() {
            0
        } else {
            picker.selected() + 1
        },
        picker.total(),
    );
    // Which layer the keys are in, said after the query rather than where it
    // would be typed.
    let hint = match picker.typing() {
        true => format!("  {}", say!("picker.in-the-query")),
        false => format!("  {}", say!("picker.in-the-list")),
    };
    let footer = format!("{counted}{}{preedit}{hint}", picker.query());
    let at = picker.selected();
    // **In the middle of the window, the list on the left and what it is
    // standing on to the right of it** (2026-09-17: 「左側是文件窗口，右側是
    // 預覽」). A picker is the one panel that is the whole of what you are
    // doing while it is open — the writing behind it is not being read — so
    // unlike a note it takes the middle rather than a corner.
    //
    // ⚠️ **The box does not measure itself off the list.** It used to be drawn
    // by [`draw_list`], which shrinks to whatever is in it — so a query that
    // matched nothing collapsed the whole picker into a three-line box with no
    // preview beside it, and the one frame where a reader most needs to see
    // 「nothing matched」 is the frame that looked broken: 「一模一样的问题根本
    // 没有变好」. A picker is a place on the page; it keeps its shape whether
    // it holds one name or a hundred and thirty-seven.
    // **Half the window, and never less than ten rows** (2026-09-18:
    // 「面板可以再大一些，比如高度是 max(10, 一半行數)」) — which is what every
    // picker worth copying does: a list eight rows deep in an eighty-row
    // terminal is a keyhole.
    let rows = ((area.height / 2).max(10) + 3).min(area.height).max(4);
    let wide = (area.width * 4 / 5).clamp(24, 120).min(area.width.saturating_sub(2));
    let box_ = crate::chrome::place(area, (wide, rows), crate::chrome::Anchor::Centre)?;
    // Two fifths for the names, the rest for the preview — and a window too
    // narrow for both gives the whole of itself to the names.
    let names = match box_.width >= 56 {
        true => (box_.width * 2 / 5).max(24),
        false => box_.width,
    };
    let left = Rect::new(box_.x, box_.y, names, rows);
    let ground = Style::default().bg(ink.paper());
    crate::chrome::draw(frame, left, &crate::chrome::Ring {
        rounded: config.panel.rounded,
        border: ground.fg(ink.rule()),
        ground,
        title: Some((picker.title.clone(), ground.fg(ink.gold()))),
    });
    // One column: these are paths, long and of every length, and columns of
    // ragged paths are harder to read down than a single list.
    let deep = rows.saturating_sub(3) as usize;
    let first = at
        .saturating_sub(deep.saturating_sub(1))
        .min(items.len().saturating_sub(deep.min(items.len())));
    let on = Style::default().bg(ink.text()).fg(ink.paper());
    let limit = left.x + names - 1;
    {
        let buf = frame.buffer_mut();
        for slot in 0..deep {
            let Some(item) = items.get(first + slot) else {
                break;
            };
            let y = left.y + 1 + slot as u16;
            let picked = first + slot == at;
            let style = match picked {
                true => on,
                false => ground.fg(ink.text()),
            };
            if picked {
                for x in left.x + 1..limit {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_symbol(" ").set_style(style);
                    }
                }
            }
            // **The name first, the folders after it in the quiet ink**
            // (2026-09-18). A row is forty cells and
            // `crates/yumete-core/src/editor/sidebar.rs` is forty-two, so
            // drawing the path as it is spells out the folders and cuts off
            // the one word that was typed. Every picker worth copying puts
            // the name first for this reason; the cut then falls on the
            // folder, which is the half a reader can do without.
            //
            // Character by character, so the ones the query found can be 金 —
            // and on the inked row they stay the inked row's own two colours,
            // where a third would read as a mistake.
            let chars: Vec<char> = item.0.chars().collect();
            let name_at = chars
                .iter()
                .rposition(|&c| c == '/' || c == '\\')
                .map_or(0, |i| i + 1);
            let mut x = left.x + 1;
            let mut ink_at = |n: Option<usize>, ch: char, x: &mut u16, quiet: bool| {
                let w = yumete_cjk::char_width(ch) as u16;
                if *x + w > limit {
                    return false;
                }
                let hit = n.is_some_and(|n| item.1.contains(&n));
                let this = match (hit, picked, quiet) {
                    (true, false, _) => ground.fg(ink.gold()),
                    (true, true, _) => on.add_modifier(Modifier::BOLD),
                    (false, false, true) => ground.fg(ink.quiet()),
                    (false, _, _) => style,
                };
                put_text(buf, *x, y, limit, &ch.to_string(), this);
                *x += w;
                true
            };
            for n in name_at..chars.len() {
                if !ink_at(Some(n), chars[n], &mut x, false) {
                    break;
                }
            }
            if name_at > 0 {
                for ch in "  ".chars() {
                    ink_at(None, ch, &mut x, true);
                }
                // The folders, without the separator the name was split on.
                for n in 0..name_at - 1 {
                    if !ink_at(Some(n), chars[n], &mut x, true) {
                        break;
                    }
                }
            }
        }
        // Nothing matched is something to say, not an empty box to puzzle over.
        if items.is_empty() {
            put_text(
                buf,
                left.x + 1,
                left.y + 1,
                limit,
                &say!("picker.nothing-matched"),
                ground.fg(ink.quiet()),
            );
        }
        put_text(
            buf,
            left.x + 1,
            left.y + rows - 2,
            limit,
            &footer,
            ground.fg(ink.quiet()),
        );
    }
    // The preview, in the columns the names left: the head of the file, or of
    // the buffer if it is already open and has unsaved writing in it.
    let over = Rect::new(box_.x + names, box_.y, box_.width.saturating_sub(names), rows);
    if over.width >= 20 {
        draw_preview(frame, editor, config, ink, over);
    }
    let panels = Some(Rect::new(box_.x, box_.y, names + over.width.max(0), rows));
    // **The caret sits in the query, and the query is inside the panel** — not
    // on the status line, which is where it used to be put and where it was
    // seen to be: 「光标在状态栏中打了j」. Nothing is being typed while the
    // keys are in the list, so there the caret is on the name it is standing
    // on instead.
    let caret = match picker.typing() {
        true => Position::new(
            left.x
                + 1
                + (yumete_cjk::str_width(&counted)
                    + yumete_cjk::str_width(&picker.before_caret())
                    + yumete_cjk::str_width(&preedit)) as u16,
            left.y + rows - 2,
        ),
        false => Position::new(left.x + 1, left.y + 1 + (at - first) as u16),
    };
    frame.set_cursor_position(caret);
    Some((caret, panels))
}

/// What the picker is standing on, drawn beside the list.
///
/// Plain text at the page's own ink: this is a glance at a file to say 「yes,
/// that one」, not a second window on it — the editor itself is one `Enter`
/// away, and anything more here would be a renderer in a panel.
fn draw_preview(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    ink: crate::theme::Palette,
    area: Rect,
) {
    let ground = Style::default().bg(ink.at(yumete_config::rung::CHROME));
    // **The ring is drawn even when there is nothing to put in it** — a query
    // that matches no file left the writing showing through the shape the
    // preview had been occupying a keystroke ago, which reads as a panel that
    // broke rather than one with nothing to say (2026-09-17).
    let (name, lines) = editor
        .picker_preview(area.height.saturating_sub(2) as usize)
        .unwrap_or_default();
    crate::chrome::draw(frame, area, &crate::chrome::Ring {
        rounded: config.panel.rounded,
        border: ground.fg(ink.rule()),
        ground,
        title: Some((name.clone(), ground.fg(ink.gold()))),
    });
    // **Coloured the way the file itself would be** (2026-09-18): a chapter's
    // headings in 金, a `.py`'s keywords through the same tree-sitter pass the
    // page uses. A preview in one flat ink asks the reader to read it; a
    // coloured one they can glance at, which is the whole job of this pane.
    let language = yumete_core::code::Language::from_extension(
        std::path::Path::new(&name).extension().and_then(|e| e.to_str()).unwrap_or(""),
    );
    let coloured = language.map(|language| yumete_core::code::highlight(language, &lines));
    let buf = frame.buffer_mut();
    let limit = area.x + area.width - 1;
    for (i, line) in lines.iter().enumerate() {
        let y = area.y + 1 + i as u16;
        if y + 1 >= area.y + area.height {
            break;
        }
        match coloured.as_ref().and_then(|all| all.get(i)) {
            Some(spans) if !spans.is_empty() => {
                let mut x = area.x + 1;
                for span in spans {
                    let text: String = line
                        .chars()
                        .skip(span.start)
                        .take(span.end.saturating_sub(span.start))
                        .collect();
                    let style = ground.patch(markup_style(span.kind, ink));
                    let after = put_text(buf, x, y, limit, &text, style);
                    x = after;
                    if x >= limit {
                        break;
                    }
                }
            }
            // Markdown and plain writing: the one distinction worth drawing is
            // a heading, which is how a reader knows *where* in the chapter
            // this glance lands.
            _ => {
                let style = match line.trim_start().starts_with('#') {
                    true => ground.fg(ink.gold()),
                    false => match line.trim_start().starts_with('>') {
                        true => ground.fg(ink.quiet()),
                        false => ground.fg(ink.text()),
                    },
                };
                put_text(buf, area.x + 1, y, limit, line, style);
            }
        }
    }
}

/// The composition in progress, when a `/` or `:` prompt — or a picker's
/// query, which is the same thing wearing a list — is open.
fn prompt_preedit(editor: &Editor, ime: &ImeSession) -> String {
    let typing = editor.prompt().is_some() || editor.picker().is_some();
    if typing && ime.available() && ime.is_composing() {
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
    head: Option<Rect>,
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
    let drawn = |line: usize| editor.drawn_on_line(line);
    let typed = |line: usize| editor.typed_on_line(line);
    // **表格所在的行不再 soft wrap** (#275): a cell folded onto the next screen
    // row is not in its column any more, so a table row is one row however long
    // it is and what runs off the right edge is reached by scrolling sideways.
    let flat = |line: usize| editor.table_row_at(line);
    let measure = wrap::Measure::new(width, &hide)
        .with_indent(editor.paragraph_indent())
        .with_folds(&fold)
        .with_drawn(&drawn)
        .with_typed_drawn(&typed)
        .with_unwrapped(&flat)
        .with_version(editor.current_buffer().id(), editor.current_buffer().revision())
        .with_edit(editor.current_buffer().edit())
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
    // back to zero. With `:view-wrap off` a paragraph is one row of whatever length
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
    // **The cell you are standing on** (#229). Insert has always been confined
    // to it, and until now the only sign of that was the column name in the
    // status line — so a table you were editing looked exactly like a table you
    // were not. The grid renderer has said this since #118 (a cursor row banded
    // at `HEAD`, the cell itself at `SELECTION`); this is the same answer for
    // the table that is drawn as part of a page.
    //
    // A rung quieter than the grid's, because here the ground is competing with
    // prose and with a real selection: the cell takes `HEAD`, so a selection
    // inside it — `SELECTION`, one rung louder and patched on afterwards — is
    // still the ground you see first.
    //
    // The **box**, not the content: a `|` table's padding is the column's own
    // width, and an empty cell has no content to tint at all.
    //
    // Asked of the **region**, not of `:table` being on. `j`, `G`, a search and
    // the mouse all walk the cursor out of a `|` table — into the prose under
    // it, or onto a table quoted inside a fence — and nothing puts `:table`
    // away when they do; `cell_position` answers「cell 0」for any line at all,
    // so the ground was drawn on prose. The rule row goes with them: it is the
    // drawing of the alignments, `clear_cell` refuses it and `move_cell_row`
    // steps over it, so a ground saying「an edit lands here」would be a lie.
    //
    // Either kind of region: a block recognised in a document (#216) stops
    // where its delimiter does, and the tint is the only thing on the page
    // that says where that is.
    let cell = match peek.is_none() && !editor.grid_has_the_pane() {
        true => editor.prose_region().and_then(|region| {
            editor
                .cell_position()
                .filter(|&(line, _)| region.holds(line) && !region.is_rule(line))
                .and_then(|(line, at)| editor.cell_box(line, at))
        }),
        // A whole-file grid draws its own cell, and the peek pane is showing
        // somebody else's buffer — the cursor is not in it.
        false => None,
    };
    let cell_style = Style::default().bg(ink.at(yumete_config::rung::HEAD));
    let show_segmentation = editor.segmentation_visible();
    let mark = editor.word_mark();
    let show_markup = editor.markup_visible();
    // The measure is counted in *text*: `ruler = 80` means eighty columns of
    // writing, which is what a writer means by it. The line-number gutter is
    // furniture, not text, so it does not eat into the measure — and the ruler
    // moves with the gutter rather than the writing moving under it.
    // A measure set with `:view-wrap 50` is a ruler by definition — it is the width
    // the writer asked to write to — so it stands in for the configured one,
    // **but only while it is folding rows**. `:view-wrap off` stops the folding and
    // keeps the number, so that `:view-wrap` on its own can put it back; drawing a
    // rule at fifty and tinting everything past it, while the rows run straight
    // through both, names an edge the page no longer has. A ruler out of the
    // config is a mark the reader asked for and stands either way.
    let ruler = editor
        .measure()
        .filter(|_| editor.wrap_width().is_some())
        .unwrap_or(config.editor.ruler);

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
    // 焦點模式 (#246): every row but the paragraph being written stands back a
    // rung — the same `faded()` a peeked pane recedes by, a row at a time.
    // **The 段, not the row**: a wrap point is not a unit of writing, and a
    // sentence that has just wrapped is still the sentence being written.
    //
    // Not in a pane that is only being read: that half is already a rung back.
    let focus = peek.is_none() && editor.focus();
    let stood_back = ink.faded();
    let mut lines: Vec<Line> = Vec::new();
    // What the bar at the top of the page will hold, taken from the first row
    // of the table this page shows (#379).
    let mut bar_lines: Option<(Option<Line<'static>>, Option<Line<'static>>)> = None;
    for (_, row) in rows_on_screen(editor, rope, measure, *viewport, height) {
        // Which palette this row is drawn off. Everything below asks `ink`, so
        // the whole of 焦點模式 is this one decision. The gutter goes with the
        // row: a line number is the row's own furniture, unlike the 縱書 number
        // band, which is the page's.
        let ink = match focus && row.line != cursor_line {
            true => stood_back,
            false => ink,
        };
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
        // **A tab is drawn as the cell it occupies** (#374). The editor counts
        // a tab as one cell — `grapheme_width` gives every ASCII byte one, and
        // the space it advances over is drawn beside it — but `unicode-width`
        // answers `None` for a control character, so ratatui laid the tab out
        // in *no* cell at all and the row came up one short of the column the
        // caret was told it was in. Replaced, never taken off: one character,
        // one cell, exactly as the grid replaces a wall below.
        // …and **every other control character is drawn as its picture**
        // (#398). A NUL left alone reaches the terminal as a NUL: the cell it
        // was charged for comes out blank, so a file with one in it looks like
        // a file with nothing there. Unicode has a block for exactly this —
        // `␀` `␁` … `␡`, one cell each, which is the cell the editor already
        // measured — so the byte stays in the buffer and gets a face on the
        // page. A lone `\r` is **not** among them: `wrap::line_text` takes it
        // off with the rest of the break, so it never reaches this row at all
        // — which is #395, and a different question (is it a line?).
        let chars: Vec<char> = text.chars().map(|c| match c {
            '\t' => ' ',
            _ => yumete_cjk::control_picture(c).unwrap_or(c),
        }).collect();
        // The block grounds the whole row; the inline runs are patched onto it.
        // **The page is painted.** Until this line, the manuscript itself was
        // drawn with no colour at all — the terminal's own ink on the
        // terminal's own ground — while 墨香 dressed five panels around it. A
        // theme that does not own its ground cannot promise anything about
        // contrast, because every tint in it is measured against a colour the
        // editor has never seen.
        let block = blocks.get(row.line).copied();
        // A table's ground stops at its last `|` — unless it is inside a
        // `:::`, where the callout's own wash carries on to the edge under it.
        use yumete_core::markdown::Block as Blk;
        let table_in = match block {
            Some(Blk::Table { inside, .. }) => Some(inside),
            _ => None,
        };
        let row_is_a_table = matches!(table_in, Some(None));
        // Which callout this row is inside, whatever kind of block it is — the
        // fill below is the callout's, so a fence or a quote in a `:::` reaches
        // the edge the way the prose around it does (#468).
        let in_callout = match block {
            Some(Blk::Table { inside, .. }) | Some(Blk::Quote { inside }) | Some(Blk::Code { inside }) => inside,
            _ => None,
        };
        let ground = ink
            .page()
            .patch(block.and_then(|b| block_style(b, ink)).unwrap_or_default());
        // The fill behind a table inside a callout is the **callout's**, so the
        // block still reaches the edge as a block; the table's band is only
        // under the row itself.
        let fill = match in_callout {
            Some(callout) => ink
                .page()
                .patch(block_style(Blk::Container(callout), ink).unwrap_or_default()),
            None => ground,
        };
        // The row the current hit is on, banded — 「在哪一行」 answered before
        // you have found the word itself.
        //
        // ⚠️ **The fill takes it too** (#469). `fill` was snapshotted from
        // `ground` above and this line shadows `ground` alone, so the band
        // stopped at the last character instead of crossing the row — and with
        // `ground = "terminal"`, where the band is the *only* thing that ever
        // gives `fill` a background, it was not drawn past the text at all.
        let (ground, fill) = match hit_line == Some(row.line) {
            true => {
                let band = ink.ground(yumete_config::rung::BAND);
                (ground.patch(band), fill.patch(band))
            }
            false => (ground, fill),
        };
        // 焦點模式 has to name its ink out loud on a page that has none of its
        // own. With `[theme] ground = "terminal"` the page's style is empty on
        // purpose — the reader's palette — so standing a row back by swapping
        // palettes changed nothing that ever reached a cell, and `:view-focus` was
        // a silent no-op for everybody who gave the ground back while the
        // status line said 「焦點：開」. The 縱書 page has had this rescue since
        // it was written (`vertical.rs`); this one had not.
        let ground = match focus && row.line != cursor_line && ground.fg.is_none() {
            true => ground.fg(ink.text()),
            false => ground,
        };
        // 所見即所得: the markup comes off the page. It is dropped from what is
        // *drawn*, not from the buffer — and never on the construct the cursor
        // is in, so the cursor is never inside text that is not on the screen.
        let hide = editor.hidden_on_line(row.line);
        let drawn_runs = editor.drawn_runs_on_line(row.line);
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
        let runs: Vec<yumete_core::drawn::Run> = drawn_runs
            .iter()
            .filter(|run| {
                run.column >= start_in_line
                    && (run.column < start_in_line + chars.len()
                        || (row.ends_line && run.column == start_in_line + chars.len()))
            })
            .map(|run| yumete_core::drawn::Run {
                column: run.column - start_in_line,
                text: run.text.clone(),
                ink: run.ink,
            })
            .collect();
        // A rung back from the writing, the way a reading is set: it is *about*
        // the text and is not in it, and drawn text in the text's own ink reads
        // as something that has already been written. A note (#248) is further
        // back still and in the marker's ink: it is not a word of the
        // manuscript at all, it is the editor pointing at one.
        // The fold mark is neither: it is 金 and it is bold, the page's own
        // way of saying 這不是正文. Grey said it too quietly — the page is a
        // run of greys on purpose, so a grey `>` at the end of a cell is a
        // `>` the writer typed. Ink and weight rather than a ground, because
        // a ground on this page is the *reader's* mark (the selection, the
        // 朱 wash, the cursor's band) and the fold mark is the editor's.
        let run_style = |kind: yumete_core::drawn::Ink| match kind {
            yumete_core::drawn::Ink::Fold => ground.fg(ink.gold()).add_modifier(Modifier::BOLD),
            yumete_core::drawn::Ink::Note => ground.fg(ink.marker()),
            // A tab is the one drawn thing with nothing written in it: what
            // shows it is the ground (#374).
            yumete_core::drawn::Ink::Tab => ground.bg(ink.at(yumete_config::rung::BAND)),
            _ => ground.fg(ink.quiet()),
        };
        // **真表格顯示** (#275): 「完全画成表格」. The `|` the writer typed *is*
        // the wall between two cells, so in this mode it is drawn as one, and
        // the `| --- |` row as the line under the head. Every glyph is one cell
        // wide and replaces one character, so nothing else on this page — the
        // caret, the wrap, the click map, #212's padding — has to know.
        let grid = editor.grid_on_line(row.line);
        // #212's padding writes the rule row's own dashes, so a rule being
        // *drawn* has to draw those too — otherwise the line stops where the
        // file's dashes stopped and the rest of the row is bare.
        let runs: Vec<yumete_core::drawn::Run> = match editor.grid_rule_row(row.line) {
            false => runs,
            true => runs
                .into_iter()
                .map(|run| match run.ink {
                    yumete_core::drawn::Ink::Padding => yumete_core::drawn::Run {
                        text: run.text.chars().map(|_| '┄').collect(),
                        ..run
                    },
                    _ => run,
                })
                .collect(),
        };
        // What the *measure* was told, which is one answer per anchor: the
        // widths have to be the ones the wrap and the click map counted.
        let flat: Vec<(usize, String)> = yumete_core::drawn::flat(&runs);
        let mut chars = chars;
        for &(at, glyph) in &grid {
            if let Some(ch) = at.checked_sub(start_in_line).and_then(|i| chars.get_mut(i)) {
                *ch = glyph;
            }
        }

        let mut styles = vec![ground; chars.len()];
        // ⚠️ **A table's ground starts at its first `|` as well** (#472). The
        // right edge was trimmed to the last wall and the left was not, so a
        // table indented inside a list item wore a two-cell tab sticking out
        // to the left of it. The indent is the list's, not the table's.
        if row_is_a_table {
            // What is under a table is the page, or the callout it sits in —
            // `fill` is the table's own band here, which is the thing being
            // trimmed away.
            let under = match in_callout {
                Some(_) => fill,
                None => ink.page(),
            };
            let indent = chars.iter().take_while(|c| c.is_whitespace()).count();
            for style in styles.iter_mut().take(indent) {
                *style = under;
            }
        }
        // Where a `==highlight==` covers this row, in this row's own indices,
        // so the cell ground below can step around it.
        //
        // **The run, not the colour.** This used to be written as「the ground
        // here is already `wash`」, and `wash` has three painters: a highlight,
        // a `::: danger` container, and the current search hit. A `|` table
        // inside a callout therefore had every character of every row already
        // washed, so the cell ground was skipped on all of them and the mode
        // drew nothing at all.
        let mut highlighted: Vec<(usize, usize)> = Vec::new();

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
                if run.kind == yumete_core::markdown::Kind::Highlight {
                    highlighted.push((a.min(b), b));
                }
                for style in styles.iter_mut().take(b).skip(a.min(b)) {
                    *style = style.patch(markup_style(run.kind, ink));
                }
            }
        }

        // Furniture, not writing: a rule is the same rung as the gutter it
        // lines up under. Set after the markup so a table inside a `::: note`
        // keeps the container's ground and still draws its own walls.
        for &(at, _) in &grid {
            if let Some(style) = at
                .checked_sub(start_in_line)
                .and_then(|i| styles.get_mut(i))
            {
                *style = style.fg(ink.furniture()).remove_modifier(Modifier::BOLD);
            }
        }

        // **A wiki name is underlined with dots** (#287, §5.8.4): told apart
        // from a link's solid line by shape, which survives any theme. The
        // cells ask for it with a modifier bit only `backend::Dotted` reads.
        let wiki = editor.wiki_marks_on_line(row.line);
        let wiki_color = editor.wiki_mark() == yumete_core::wiki::Mark::Color;
        if !wiki.is_empty() {
            let start_in_line = row.start - rope.line_to_char(row.line);
            for &(a, b) in &wiki {
                let a = a.saturating_sub(start_in_line);
                let b = b.saturating_sub(start_in_line).min(chars.len());
                for style in styles.iter_mut().take(b).skip(a.min(b)) {
                    *style = match wiki_color {
                        true => style.fg(ink.gold()),
                        false => style.add_modifier(Modifier::UNDERLINED | crate::backend::DOTTED),
                    };
                }
            }
        }

        // ⚠️ **Not `&& !has_selection`** (#450). The overlay used to go out on
        // **every paragraph on the screen** the moment anything was selected —
        // and in this editor a motion *is* a selection, so holding `w` down
        // made the whole page flash: 選到單字 (a bare cursor, no selection) it
        // came back, 選到多字詞 it went out again, once a keystroke.
        //
        // There was never anything to avoid. The selection's own ground is
        // painted further down, **after** this, and patched over whatever is
        // here — so the cells it covers were never going to show a word tint,
        // and the cells it does not cover had no reason to lose one.
        if show_segmentation {
            let page_bg = ink.page().bg;
            let page_fg = ink.page().fg;
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
                    match mark {
                        // Only where nothing has already claimed the ground. A
                        // word tint is the quietest of the three layers — it
                        // must not rub out a `==highlight==`, which exists *to
                        // be* a ground, nor a container's own colour. The page
                        // itself is not a claim: every style on the row starts
                        // from it now that the paper is painted, and reading
                        // that as taken would have left the overlay with
                        // nowhere it was allowed to draw.
                        WordMark::Tint => {
                            if style.bg.is_none() || style.bg == page_bg {
                                *style = style.bg(ink.word());
                            }
                        }
                        // The other ink asks the opposite question: the ground
                        // is left alone — a highlight or a container keeps it —
                        // and the *writing* takes the mark.
                        //
                        // ⚠️ **Whatever colour it already has, stepped back**
                        // (#461). This used to draw only where the writing was
                        // plain, on the reasoning that a link's 藍 or a
                        // heading's 金 must not be rubbed out. True, and the
                        // consequence was that a link four lines long had no
                        // word boundaries in it at all — 「现在是看不出分词的」.
                        // Stepping the colour that is there keeps the colour
                        // *and* the boundary.
                        WordMark::Ink => {
                            let from = style.fg.or(page_fg).unwrap_or(ink.text());
                            *style = style.fg(ink.marked(from, yumete_config::rung::WORD_INK));
                        }
                        // The same question again, answered with hue instead
                        // of weight (#501) — see [`Ink::word_hue`].
                        WordMark::Color => {
                            let from = style.fg.or(page_fg).unwrap_or(ink.text());
                            // `None` where the writing already has a colour of
                            // its own — a heading's 金, a link's 藍. See
                            // [`Ink::word_hue`].
                            if let Some(hue) = ink.word_hue(from) {
                                *style = style.fg(hue);
                            }
                        }
                        // **Nothing on the writing at all**: a rule under the
                        // word, and bare line between. 橫排 can underline the
                        // whole word because the break between two runs of it
                        // *is* the boundary; 縱書 cannot (there a continuous
                        // underline is one tick per character), so that page
                        // marks each word's last cell instead.
                        // ⚠️ **Never on a cell that is underlined already.** A
                        // link's underline is its own and it wins: 「链接就是用
                        // 链接颜色 overwrite 掉分词下划线」. Composing the two was
                        // tried (the table-band-over-a-callout trick) and came
                        // out `#002857` — a black line on a dark page.
                        WordMark::Line
                            if !style.add_modifier.contains(Modifier::UNDERLINED) =>
                        {
                            *style = style
                                .add_modifier(Modifier::UNDERLINED)
                                .underline_color(ink.word_rule());
                        }
                        WordMark::Line => {}
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

        // The cell first, so that everything louder — the selection, the hit —
        // still goes over it.
        if let Some((from, to)) = cell {
            if to > row.start && from < row.end {
                let a = from.saturating_sub(row.start).min(chars.len());
                let b = to.saturating_sub(row.start).min(chars.len());
                for (i, style) in styles.iter_mut().enumerate().take(b).skip(a) {
                    // Never over a `==highlight==`. The band and the word tint
                    // are quieter than the cell and give way to it, but a
                    // highlight exists *to be* a ground — the same reason the
                    // word tint above steps around it.
                    if highlighted.iter().any(|&(x, y)| i >= x && i < y) {
                        continue;
                    }
                    *style = style.patch(cell_style);
                }
                // **And the wall on each side, in 金** (#407). The cell's own
                // ground is `HEAD` and the selection's is `SELECTION`, one rung
                // apart at the paper end — far enough to measure and not far
                // enough to read, which is what the writer saw: 「w 選擇詞的
                // 背景色和格子的選擇色一樣，導致我不知道選區是什麼」。The
                // ladder has no third ground to give, so which cell you are in
                // is said by a *mark* instead: the two walls beside it, drawn
                // in the one colour on the page that is not a quantity of ink.
                // Then the only thing still using a ground is the selection.
                let gold = Style::default().fg(ink.gold());
                for edge in [a.checked_sub(1), (b < styles.len()).then_some(b)] {
                    if let Some(style) = edge.and_then(|i| styles.get_mut(i)) {
                        *style = style.patch(gold);
                    }
                }
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
            flat: &flat,
        };
        // **The bar at the top of the page** (#379), filled from the *first*
        // row of the table the page is showing — the numbers and the names have
        // to stand over the columns as that row draws them, so they are placed
        // by the same `Drawn` and the same lead the row itself is placed by.
        // Anything else and the bar names one column while pointing at another.
        if head.is_some_and(|bar| bar.height > 0) && bar_lines.is_none() {
            let cells = editor.table_cells_on_line(row.line);
            if !cells.is_empty() {
                let start_in_line = row.start - rope.line_to_char(row.line);
                let lead = gutter + indent;
                let names = editor.table_headings();
                // **The pinned copy wears what the real header wears** (#477).
                // The bar is the table's own top two rows brought up the page,
                // and it was drawn on the page in furniture grey while the row
                // it stands for is banded at 第 82 檔 in 金 — so a scrolled
                // table's header read as something floating outside the table
                // rather than the top of it.
                let head_ground = match table_row_depth(0) {
                    Some(depth) => Style::default().bg(ink.over(ink.paper(), depth)),
                    None => ink.page(),
                };
                bar_lines = Some((
                    labels_line(
                        &cells,
                        |i| (i + 1).to_string(),
                        ink,
                        head_ground,
                        gutter as usize,
                        drawn,
                        lead,
                        start_in_line,
                    ),
                    labels_line(
                        &cells,
                        |i| names.get(i).cloned().unwrap_or_default(),
                        ink,
                        head_ground.fg(ink.gold()),
                        gutter as usize,
                        drawn,
                        lead,
                        start_in_line,
                    ),
                ));
            }
        }
        // **Whether** the row is bought is `row_has_reading`'s answer and only
        // its own — `rows_on_screen` spends the screen row off that same
        // function, and the mouse, the scroll and the caret are all placed by
        // it. `reading_line` says *what* goes in the row it bought. Asking the
        // second one whether the row exists drifts the whole page up a row
        // whenever the two disagree, and they disagree twice: a ruby group the
        // wrap cut in half is drawn over the row its base **begins** on, so the
        // tail row buys a reading nobody will draw; and 疏排's row of air is
        // only reached when the line has no ruby at all, so `:view-margin always` over a
        // line that does have some loses its air row too.
        if row_has_reading(editor, rope, &row) {
            // ⚠️ **The row above belongs to the block below it** (#464). This
            // took the page's own ground, and a `:::` with a table in it was
            // cut in half: the 列號 row over the table came out page-coloured,
            // a bare stripe straight across the callout. A reading over a line
            // of a quote had the same hole.
            //
            // Not the table's band, though — that one belongs to the row
            // itself, and a ruler wearing it would read as another row of the
            // table.
            // A plain table's ruler sits on the page — the table's own band
            // belongs to its rows, and a ruler wearing it reads as one. Inside
            // a callout it is a line of the callout like any other, and the
            // wash has to run under the numbers as well as past them: half a
            // row of page is a notch cut in the box (#468).
            let over = match row_is_a_table {
                true => ink.page(),
                false => fill,
            };
            let mut reading =
                reading_line(editor, ink, over, gutter, rope, &row, drawn, gutter + indent)
                .unwrap_or_else(|| Line::from(Span::styled("", over)));
            if over.bg.is_some() {
                reading.spans.push(Span::styled(
                    " ".repeat(text_area.width as usize + left),
                    over,
                ));
            }
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
            while gi < runs.len() && runs[gi].column <= at {
                // Padding inside the cell is part of the cell. Without this the
                // tint stops at the last character the file actually holds and
                // the column it is squaring up to stays bare — the one place
                // the drawing is *about* alignment is the one place it would
                // have looked ragged.
                let base = run_style(runs[gi].ink);
                let style = match cell {
                    Some((from, to)) => {
                        let col = row.start + runs[gi].column;
                        match col >= from && col <= to {
                            true => base.patch(cell_style),
                            false => base,
                        }
                    }
                    None => base,
                };
                spans.push(Span::styled(runs[gi].text.clone(), style));
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
                && runs.get(gi).is_none_or(|run| run.column != to)
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
        //
        // ⚠️ **A table is the exception** (#460): its edge is the last `|`, not
        // the window's. A quote or a callout is a block *of the page* and takes
        // the page's width; a table is a shape **on** the page and has a width
        // of its own, so a band carried past its last wall paints page as if it
        // were table. 「表格的底色不需要延伸到整个页宽，而是表格宽度就可以了。」
        if fill.bg.is_some() && (!row_is_a_table || hit_line == Some(row.line)) {
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
                fill,
            ));
        }
        lines.push(scrolled(Line::from(spans), gutter, left));
    }
    // `.style` paints the **whole area**, not only the rows there is writing
    // on: past the last line of a short file the page is still the page, and
    // 墨香's light page on a dark terminal made that half of the window black.
    frame.render_widget(Paragraph::new(lines).style(ink.page()), text_area);

    // **The bar, into its own region** (#379). The design, and the
    // reason this is buildable at all — 2026-09-11：「在顶部预留一个信息栏
    // （两行：列号+列名）……这样它是独立的，也就不会侵扰文本的区域了。」A row
    // drawn *over* the text would be a screen row that does not belong to the
    // line it sits on, and the wrap, the caret and the click map would each
    // have had to learn what that means. A region is just a region: the page
    // is two rows shorter, which is arithmetic this file already does twice.
    if let Some(bar) = head.filter(|bar| bar.height > 0) {
        let head_bar = match table_row_depth(0) {
            Some(depth) => Style::default().bg(ink.over(ink.paper(), depth)),
            None => ink.page(),
        };
        let (numbers, names) = bar_lines.unwrap_or((None, None));
        let blank = || Line::from(Span::styled("", ink.page()));
        // **The order the table's own top has**: the 列號標尺 above the head
        // row, and the head row's names under it. The bar is those two rows
        // brought up the page, so seeing it and seeing the table's real top
        // are the same sight. Drawn the other way round until it was
        // caught, 2026-09-11：「你的序號是不是跑到列名的下面了」— it was,
        // because I had compared the bar to the ruler alone and forgotten what
        // the ruler sits above.
        let rows = vec![
            scrolled(numbers.unwrap_or_else(blank), gutter, left),
            scrolled(names.unwrap_or_else(blank), gutter, left),
        ];
        frame.render_widget(Paragraph::new(rows).style(head_bar), bar);
    }

    // The margin: everything past the measure, whether or not there is writing
    // in it. Tinting only the characters that run past says nothing at all
    // when nothing does — which is exactly the case under `:view-wrap 50`, where
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

/// Draw the status line: where am I, and nothing else.
///
/// `command_row` says whether the row below exists. When it does not — a window
/// too short for two, or `[editor] command_line = false` — this line has to
/// take in the three things that row would have carried: a `:` or `/` being
/// typed, a message about what just happened, and the sidebar's keys.
fn draw_status(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    ime: &ImeSession,
    status_area: Rect,
    tab_area: Rect,
    command_row: bool,
) {
    let buffer = editor.current_buffer();
    let ink = crate::theme::Palette::of(config);
    // **A raised strip, or a sunken one — never a reversal.** Reversing gave a
    // white bar under a dark page: the loudest thing on the screen, saying the
    // least. Sinking goes the other way, below the page instead of above it,
    // and on a light theme there is nothing below white so the strip rises
    // as before (`Palette::sunken`).
    let bar = match config.theme.status_bar {
        yumete_config::StatusBar::Sunken => Style::default().bg(ink.sunken()).fg(ink.text()),
        yumete_config::StatusBar::Raised => {
            ink.ground(yumete_config::rung::CHROME).fg(ink.text())
        }
    };
    // The sidebar used to take the whole status line to list its keys. It has
    // the row below for that now, and taking this one as well would mean losing
    // the file name and the position for as long as the sidebar has focus.
    let status = if editor.sidebar_focused() && !command_row {
        say!("ui.sidebar-mode", Editor::sidebar_keys())
    } else if let (false, Some((_, text))) = (command_row, editor.prompt()) {
        let prefix = editor.prompt_label().unwrap_or_default();
        // The composition in progress belongs at the caret, so a search reads as
        // the pattern being typed rather than jumping into place on commit. The
        // 中/英 tag is pushed to the right edge, where it cannot be mistaken for
        // part of the pattern.
        let line = format!("{prefix}{text}{}", prompt_preedit(editor, ime));
        let drawn = editor.prompt_ghost();
        let tag = language_tag(editor, ime);
        let used = yumete_cjk::str_width(&line)
            + yumete_cjk::str_width(&drawn)
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
                Span::styled(drawn, guess),
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
        let locked = match buffer.is_readonly() {
            true => say!("ui.readonly-tag"),
            false => String::new(),
        };
        // 中/英 + scheme, in every mode — Normal included (#337): it is what
        // the next `i` will land in, and there is no other way to find out.
        let ime_tag = match standing_language_tag(ime).as_str() {
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
        // Where you are, and nothing else. What just happened is the row
        // below's question — and with no command row it comes back here, because
        // a message nobody can see is not a message.
        let where_ = position_of(editor);
        let where_short = position_short(editor);
        // **Whole fields give way, in order — the line is never cut through a
        // word.** It used to be one `format!`, and a narrow window simply
        // sliced the end off it: at twenty columns `long.csv   Ln 1, Col 1`
        // came out `long.c`, so the first thing lost was the position — the one
        // thing the status line exists to answer (#394). The order below is
        // what a writer needs in a tmux sliver, from the last to go to the
        // first: the mode, then where the caret is, then that there is unsaved
        // work, then which file, then everything else.
        //
        // ⚠️ **`[n/total]` outranks the file name**, which looks backwards until
        // you notice when it is drawn at all: only when the tab bar could not
        // show every file — and the bar does carry the name of the one you are
        // in. So the name is the duplicate here and the fraction is the only
        // copy of what it says.
        let pieces: [(String, u8); 7] = [
            (format!("-- {} --  ", editor.mode_label()), 0),
            (ime_tag, 5),
            (buffer.display_name(), 4),
            (dirty.to_string(), 1),
            (format!("{locked}{draft}"), 2),
            (preview.to_string(), 6),
            (which, 3),
        ];
        let message = match command_row || editor.status().is_empty() {
            true => String::new(),
            false => format!("   {}", editor.status()),
        };
        let room = status_area.width as usize;
        let build = |give: u8, gap: usize, where_: &str| -> String {
            let left: String = pieces
                .iter()
                .filter(|(_, rank)| *rank == 0 || *rank < give)
                .map(|(text, _)| text.as_str())
                .collect::<Vec<_>>()
                .concat();
            let gap = " ".repeat(gap);
            format!("{}{message}{gap}{where_}", left.trim_end())
        };
        // **The gap goes before the writing does.** One column over is not a
        // reason to lose `[20/20]` whole: the three spaces between the name and
        // the position are the cheapest thing on the line, so they are spent
        // first, and only then does a field give way.
        let mut give = 7u8;
        let mut gap = 3usize;
        let mut where_ = where_.as_str();
        let line = loop {
            let line = build(give, gap, where_);
            if yumete_cjk::str_width(&line) <= room {
                break line;
            }
            if gap > 1 {
                gap = 1;
                continue;
            }
            // ⚠️ **字 goes before any field does** (#500). It is the third
            // number of three, and the other two are what the status line
            // exists to answer; a name or a `[3/20]` is worth more than it.
            if where_ != where_short {
                where_ = &where_short;
                continue;
            }
            // The mode and the position are the floor; below that the terminal
            // is too narrow for anything and the renderer's own cut answers.
            if give == 0 {
                break line;
            }
            give -= 1;
        };
        line
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
        false => [typed, String::new(), String::new()],
    };
    let fits = |status: &str| -> &str {
        let room = (status_area.width as usize)
            .saturating_sub(yumete_cjk::str_width(status));
        right
            .iter()
            .map(String::as_str)
            .find(|t| !t.is_empty() && yumete_cjk::str_width(t) + 2 <= room)
            .unwrap_or("")
    };
    // Three stages of giving way — block name, then the 字, then the readout
    // altogether — and the left side is never squeezed.
    //
    // ⚠️ **…and a fourth, one field to the left** (#500). 字 made the position
    // six cells wider, and the position is this line's floor — so at sixty
    // cells it pushed the readout off entirely, and the readout is the only
    // thing on the screen that answers 「is the character wrong, or the font?」.
    // 行 and 列 place the caret; 字 is the extra one. So when keeping 字 costs
    // the whole readout, 字 is what goes.
    let status = match fits(&status).is_empty() && right.iter().any(|t| !t.is_empty()) {
        true => status.replace(&position_of(editor), &position_short(editor)),
        false => status,
    };
    let tail = fits(&status);
    let used = yumete_cjk::str_width(&status);
    let room = (status_area.width as usize).saturating_sub(used);
    let gap = room.saturating_sub(yumete_cjk::str_width(tail));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(drawable(&status).into_owned(), bar),
            Span::styled(drawable(&format!("{}{tail}", " ".repeat(gap))).into_owned(), bar),
        ]))
        .style(bar),
        status_area,
    );
}


/// Whether `row` has any reading over it — which costs it a screen row.
fn row_has_reading(editor: &Editor, rope: &yumete_core::Rope, row: &wrap::Row) -> bool {
    use yumete_cjk::Margin;
    // A table drawn among the prose (#275) takes one row above its first line
    // for its 列號標尺 — the same mechanism, and the reason this function is
    // the one that answers: the page already knows how to give a row two.
    // **Not the margin.** The ruler is the table's own top edge, so no
    // `:view-margin` answer takes it away.
    if row.starts_line() && !editor.table_ruler_on_line(row.line).is_empty() {
        return true;
    }
    // Across the page the margin lane is **the row above** — which is what a
    // reading and 平仄 (#247) are drawn in — so `:view-margin` decides which
    // rows buy it, the same four ways it decides which 縱 do down the page.
    match editor.margin() {
        Margin::Never => false,
        // A row of air above every row. What `:view-dense off` used to mean
        // across the page, and the only thing it meant here.
        Margin::Always => editor.layout() == WritingLayout::Horizontal,
        // This row, if **it** carries something. A reading is drawn over the
        // row its base begins on, so that is the row that asks.
        Margin::Dense => {
            !readings_in_row(editor, rope, row).is_empty()
                || !meter_in_row(editor, rope, row).is_empty()
        }
        // Every row of the line, if **any** of it carries something — a
        // wrapped paragraph keeps one line spacing all the way down.
        Margin::Loose => {
            !editor.readings_on_line(row.line).is_empty()
                || (editor.meter_drawn() && !editor.meter_on_line(row.line).is_empty())
        }
    }
}

/// The readings **drawn over** `row` — the groups whose base begins on it.
///
/// The one predicate both halves ask. A ruby group is drawn over the row its
/// base *starts* on, so that is what buys the row as well: asking whether the
/// line has any readings at all made a group on the second half of a wrapped
/// paragraph claim the first half's row, which cost that half its 平仄 — and a
/// group the wrap cut in half claimed a row on the tail that nothing would
/// ever be drawn in.
fn readings_in_row(
    editor: &Editor,
    rope: &yumete_core::Rope,
    row: &wrap::Row,
) -> Vec<yumete_core::ruby::Ruby> {
    let start = row.start - rope.line_to_char(row.line);
    let end = start + (row.end - row.start);
    editor
        .readings_on_line(row.line)
        .into_iter()
        .filter(|g| g.base.0 >= start && g.base.0 < end)
        .collect()
}

/// The 平仄 marks that fall on `row`, as columns within the *line*.
///
/// Asked twice — once to buy the row and once to draw in it — and both have to
/// give the same answer or a page of poetry would gain and lose a row as it
/// scrolls. The editor caches the line's marks, so asking twice is a lookup.
fn meter_in_row(
    editor: &Editor,
    rope: &yumete_core::Rope,
    row: &wrap::Row,
) -> Vec<yumete_core::meter::Mark> {
    if !editor.meter_drawn() {
        return Vec::new();
    }
    let start = row.start - rope.line_to_char(row.line);
    let end = start + (row.end - row.start);
    editor
        .meter_on_line(row.line)
        .into_iter()
        .filter(|m| m.column >= start && m.column < end)
        .collect()
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
    flat: &'a [(usize, String)],
}

/// Where each of a row's characters is drawn, in cells from the left edge of
/// the page — the gutter and the paragraph's indent included — with one more
/// entry past the end for where the row stops.
///
/// **The one answer to「which cell is this character in」**, for everything
/// placed above a row rather than on it: a reading over its own 字, a column
/// number over its own column. The markup that came off the page and the drawn
/// text that was never in the file have both moved every character after them.
fn drawn_columns(drawn: Drawn, lead: usize) -> Vec<usize> {
    let Drawn {
        chars,
        shown,
        flat,
    } = drawn;
    let mut column = Vec::with_capacity(chars.len() + 1);
    let mut at = lead;
    let drawn_before = |i: usize| -> usize {
        flat.iter()
            .filter(|&&(g, _)| g == i)
            .map(|(_, text)| yumete_cjk::str_width(text))
            .sum()
    };
    for (i, &c) in chars.iter().enumerate() {
        at += drawn_before(i);
        column.push(at);
        if shown[i] {
            at += yumete_cjk::char_width(c);
        }
    }
    at += drawn_before(chars.len());
    column.push(at);
    column
}

/// The 列號標尺 along a drawn table's top edge (#275), as the line above its
/// first row.
///
/// 「畫，貼在表格上緣」 — one ruler per table, numbering that table's own
/// columns, because every numeric key in a grid (`t3/`, `t20,20g`, `t1s`) asks
/// the reader to count columns and on a 拆分表 that is counting to seventeen by
/// eye. Right-aligned in each column and a rung quieter than the writing, the
/// same way the pane draws it.
fn ruler_line(
    cells: &[(usize, usize)],
    ink: crate::theme::Palette,
    ground: Style,
    gutter: usize,
    drawn: Drawn,
    lead: usize,
    start_in_line: usize,
) -> Option<Line<'static>> {
    labels_line(cells, |i| (i + 1).to_string(), ink, ground, gutter, drawn, lead, start_in_line)
}

/// The same, with something other than a number over each column (#379).
///
/// The bar at the top of the page wants two of these — the numbers and the
/// names — and they are the same placement problem: a label right up against
/// the wall its column ends at, never on top of the one before it. Written
/// once, because a name a cell off its column and a number a cell off its
/// column are the same mistake.
fn labels_line(
    cells: &[(usize, usize)],
    label: impl Fn(usize) -> String,
    ink: crate::theme::Palette,
    ground: Style,
    gutter: usize,
    drawn: Drawn,
    lead: usize,
    start_in_line: usize,
) -> Option<Line<'static>> {
    let column = drawn_columns(drawn, lead);
    let mut out = String::new();
    let mut col = 0usize;
    for (i, &(_, end)) in cells.iter().enumerate() {
        let Some(&edge) = column.get(end.saturating_sub(start_in_line)) else {
            break;
        };
        let n = label(i);
        if n.is_empty() {
            continue;
        }
        // Right up against the wall it belongs to, and never on top of the
        // number before it — a number a cell off its column is still readable,
        // two numbers run together are not.
        let wide = yumete_cjk::str_width(&n);
        let want = edge.saturating_sub(wide).max(col + usize::from(col > 0));
        out.push_str(&" ".repeat(want - col));
        out.push_str(&n);
        col = want + wide;
    }
    // The ground may already name an ink — the pinned header's 金 — and when it
    // does that is the answer; furniture is the default for a ruler.
    let style = match ground.fg {
        Some(_) => ground,
        None => ground.fg(ink.furniture()),
    };
    if out.trim().is_empty() {
        return None;
    }
    // ⚠️ **The line-number gutter is not part of the table** (#481). This line
    // is built from column zero, so a ground meant for the table ran back
    // across the numbers — 「表格列号的背景色侵入了序号栏」. Every other row
    // draws its number on the page; so does this one.
    let cut = out
        .char_indices()
        .take_while(|&(i, _)| i < gutter)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let (numbers, rest) = out.split_at(cut.min(out.len()));
    Some(Line::from(vec![
        Span::styled(numbers.to_string(), ink.page()),
        Span::styled(rest.to_string(), style),
    ]))
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
    ground: Style,
    gutter: usize,
    rope: &yumete_core::Rope,
    row: &wrap::Row,
    drawn: Drawn,
    lead: usize,
) -> Option<Line<'static>> {
    // **The ruler owns the row above its table** (#275), ahead of both a reading
    // and 疏排's row of air: it is the table's top edge, and a `|` header with
    // ruby over it is not a thing anybody has written.
    let ruler = editor.table_ruler_on_line(row.line);
    if !ruler.is_empty() && row.starts_line() {
        let start_in_line = row.start - rope.line_to_char(row.line);
        return ruler_line(&ruler, ink, ground, gutter, drawn, lead, start_in_line);
    }
    let groups = readings_in_row(editor, rope, row);
    if groups.is_empty() {
        // 平仄 (#247), where no reading wants the row. A reading wins it
        // outright rather than sharing: the two would have to be interleaved
        // per character, and a 詞譜 column with holes in it says the wrong
        // thing — the missing marks would read as 輕聲 rather than as
        // 「something else is written here」.
        let marks = meter_in_row(editor, rope, row);
        if !marks.is_empty() {
            let column = drawn_columns(drawn, lead);
            let start_in_line = row.start - rope.line_to_char(row.line);
            let mut out = String::new();
            let mut col = 0usize;
            for mark in marks {
                let i = mark.column.saturating_sub(start_in_line);
                let Some(&want) = column.get(i) else { continue };
                if want < col {
                    continue;
                }
                let glyph = crate::vertical::meter_glyph(mark);
                out.push_str(&" ".repeat(want - col));
                out.push(glyph);
                col = want + yumete_cjk::char_width(glyph);
            }
            // Furniture, not writing, and the same rung the 縱書 margin sets
            // them on: this is the editor talking about the poem.
            return Some(Line::from(Span::styled(
                out,
                ink.page().fg(ink.furniture()),
            )));
        }
        // A row the margin bought with nothing in it — `always`, or a `loose`
        // row whose paragraph carries something elsewhere. Painted rather than
        // skipped, so the page keeps its ground.
        return row_has_reading(editor, rope, row)
            .then(|| Line::from(Span::styled("", ink.page())));
    }
    let line_start = rope.line_to_char(row.line);
    let start_in_line = row.start - line_start;
    // Control characters get their pictures here too (#398) — a reading is
    // text on the page like any other, and a NUL in one would draw nothing.
    let text: Vec<char> = yumete_core::zong::line_chars(rope, row.line)
        .into_iter()
        .map(|c| yumete_cjk::control_picture(c).unwrap_or(c))
        .collect();
    let column = drawn_columns(drawn, lead);
    let mut out = String::new();
    let mut col = 0usize;
    for group in &groups {
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

/// The bottom row: what you are typing, what just happened, or what you can
/// press — in that order, because only one of them can have the row (#302).
///
/// Quieter than the status line, and deliberately: the status line is the
/// page's own footing and is always there, while this only sometimes has
/// something to say. Set on the page's own ground rather than reversed, so a
/// blank one reads as part of the margin instead of as an empty bar.
fn draw_command(
    frame: &mut Frame,
    editor: &Editor,
    config: &Config,
    ime: &ImeSession,
    area: Rect,
) {
    use yumete_core::editor::Hint;
    let ink = crate::theme::Palette::of(config);
    // On the page's own ground, and *painted* — this row set colours and no
    // background at all, so on a light page over a dark terminal it came out
    // as a black band with the page's dark ink on it, which is to say
    // unreadable. A blank command row is part of the margin, not a hole in it.
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
    // **A `:` or `/` takes the whole row**, and takes it from its left edge
    // rather than one column in: the caret is placed by the same arithmetic in
    // `draw`, and a prompt that began a column further along than the caret
    // was told about is a prompt you cannot type into straight.
    if let Some((_, text)) = editor.prompt() {
        let prefix = editor.prompt_label().unwrap_or_default();
        let line = format!("{prefix}{text}{}", prompt_preedit(editor, ime));
        // The guess is a rung back from what was actually typed — a colour,
        // not `DIM`, so it is still visibly *not yet* part of the line on a
        // terminal that drops the attribute.
        let guess = page.fg(ink.at(yumete_config::rung::RULE));
        let drawn = editor.prompt_ghost();
        // The 中/英 tag is pushed to the right edge, where it cannot be
        // mistaken for part of the pattern.
        let tag = language_tag(editor, ime);
        let used = yumete_cjk::str_width(&line)
            + yumete_cjk::str_width(&drawn)
            + yumete_cjk::str_width(&tag);
        let gap = (area.width as usize).saturating_sub(used);
        let mut x = area.x;
        put_text(buf, x, area.y, right, &drawable(&line), news);
        x += yumete_cjk::str_width(&line) as u16;
        put_text(buf, x, area.y, right, &drawable(&drawn), guess);
        x += yumete_cjk::str_width(&drawn) as u16;
        let tail = format!("{}{tag}", " ".repeat(gap));
        put_text(buf, x, area.y, right, &drawable(&tail), news);
        return;
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
    // **The name of the thing, in the one corner nothing else wants** (#498).
    //
    // Asked for as 「用一个比较淡雅的色号」, and the constraint is
    // where rather than whether: the **left** of this row is the most useful
    // strip on the screen — it is where a half-pressed `t` says what may follow
    // it — so a signature there would be standing in the way exactly when the
    // row has something to say. The right end is idle even when the row is
    // busy, and it is the first thing to go when the row is not.
    //
    // FURNITURE, which is what line numbers and an unlit tab are drawn in: it
    // reads as part of the frame rather than as something said. And **two
    // clear cells or it does not appear** — a name run up against the last
    // hint is worse than no name.
    let signature = say!("ui.signature");
    let wide = yumete_cjk::str_width(&signature) as u16;
    if right > wide && x + 2 <= right - wide {
        put_text(
            buf,
            right - wide,
            area.y,
            right,
            &signature,
            page.fg(ink.furniture()),
        );
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
    position_in(editor, true)
}

/// The same, one field shorter (#500).
///
/// ⚠️ **字 is the field that gives way.** Adding it made the position six cells
/// wider, and the position is the status line's floor — so on a sixty-cell
/// terminal it pushed the character readout off the line altogether, and that
/// readout is the only thing on the screen that can answer 「is this character
/// wrong, or is it the font?」. 行 and 列 place the caret; 字 is the extra one,
/// so 字 is the one that goes. The readout already gives way in three stages;
/// this is the same idea one field to the left.
fn position_short(editor: &Editor) -> String {
    position_in(editor, false)
}

fn position_in(editor: &Editor, full: bool) -> String {
    if let Some(where_) = editor.table_status() {
        // **The number the gutter shows.** In the window that is this
        // table's own row; in prose it is the file's line, which is what the
        // gutter draws there.
        // **Row, column, and which column that is** (#497): 「行 105, 列 2
        // [星陳]」. The same shape as the prose line below it, so the two read
        // as one readout rather than two — and the column *number* is what the
        // `t1/` and `t2-10?` keys are addressed by, so it has to be on the
        // page somewhere.
        let column = editor.cell_position().map(|(_, cell)| cell + 1).unwrap_or(1);
        return say!("ui.position-in-table", editor.table_row_number(), column, where_);
    }
    if editor.layout() == WritingLayout::Vertical {
        let at = editor.zong_position();
        // **縱橫字, and they are 行列字 turned a quarter turn** (#500).
        // The mapping: 「竖排的纵＝横排的行，竖排的横＝横排的列。我用纵横
        // 是因为不想和行列混淆。」 So the three numbers mean exactly what the
        // three on a horizontal page mean, and the names are different only so
        // that a reader never has to ask which page they are looking at:
        //
        // * 縱 is the **logical line** — absolute, not the visual column. A
        //   paragraph long enough to wrap runs over several columns and they
        //   all carry the same 縱 number, exactly as a wrapped line on a
        //   horizontal page carries one 行 number.
        // * 橫 is how far along that line, in slots — the 列.
        // * 字 is how far along in characters. It parts company with 橫 wherever
        //   縦中横 packs two half-width characters into one slot.
        // **Spelt out both ways, not looked up.** `messages.toml` is audited
        // against the literal tags the tree says, so a key held in a variable
        // reads as a tag nobody says.
        return match full {
            true => say!(
                "ui.position-vertical",
                at.line + 1,
                at.slot_in_line + 1,
                editor.cursor_column() + 1
            ),
            false => say!("ui.position-vertical-short", at.line + 1, at.slot_in_line + 1),
        };
    }
    // ⚠️ **Said, not spelt out** (#497). This was a hardcoded `Ln {}, Col {}`,
    // so the one part of the status line a reader looks at most was English on
    // a 繁體 page and stayed English under `--lang=zhs` too.
    // **列 and 字 are two different numbers on a Chinese page** (#500), and
    // Both are wanted: 「横排因为有全角半角，列和字不一定一样，字表示的是字符
    // （半角+全角），而列就是半角。」 Eleven cells into a line is the sixth
    // character when five of them are 漢字, and which one you want depends on
    // what you are doing — a ruler at 40 counts cells, a publisher counts 字.
    match full {
        true => say!(
            "ui.position",
            editor.cursor_line() + 1,
            editor.cursor_visual_column() + 1,
            editor.cursor_column() + 1
        ),
        false => say!(
            "ui.position-short",
            editor.cursor_line() + 1,
            editor.cursor_visual_column() + 1
        ),
    }
}

/// What to say about the character under the cursor, in three lengths.
///
/// Three strings rather than one, so a narrow terminal drops what it has to and
/// keeps the rest instead of dropping all of it: the block name goes first,
/// then the 字 itself — which is on the page under the cursor anyway, so the
/// bare `U+51AC` still answers the question the readout exists for (is the
/// character wrong, or is the font?). The last stage earns its keep now that
/// the language tag (#337) stands on every line and takes ten cells with it.
fn char_info(editor: &Editor, config: &Config) -> [String; 3] {
    let nothing = || [String::new(), String::new(), String::new()];
    if !config.editor.char_info || editor.prompt().is_some() {
        return nothing();
    }
    let Some(c) = editor.char_at_cursor() else {
        return nothing();
    };
    let point = yumete_cjk::blocks::codepoint(c);
    // **A character with no printable form is named, not shown** (#375). The
    // readout put the character itself at the front, and for a TAB that is a
    // literal `\t` handed to the terminal: nought cells to `char_width`, so the
    // line was measured as fitting, and a jump to the next tab stop to the
    // terminal, so it did not — it overflowed the last column, wrapped, and
    // made the frame a row taller than the screen. That is #377 as well: a
    // frame too tall scrolls the terminal, and the frame before it stays on
    // the screen, which is the page smeared over itself.
    //
    // The same shape as #374, one floor up: a tab counted as nothing by a
    // measure and advanced over by whoever draws it. There is nothing to show
    // for `U+0009`, so it shows nothing and says the name.
    let short = match yumete_cjk::char_width(c) {
        0 => point.clone(),
        _ => format!("{c} {point}"),
    };
    match yumete_cjk::blocks::block_of(c) {
        Some(block) => [format!("{short} · {block}"), short, point],
        None => [short.clone(), short, point],
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
    let candidates = ime.page_candidates();
    let highlight = ime.highlight();

    // **快捷符號 is a different panel wearing the same frame.** Nothing in it is
    // a candidate: there is no 選重, no paging, and the digits commit nothing —
    // you press the letter that is written beside the symbol. Drawn as a
    // numbered list (which is what the general candidate path made of it) it
    // told the reader to press keys that do not work.
    let shortcut = ime.shortcut_rows();
    if !shortcut.is_empty() {
        let room = area.width.saturating_sub(4).min(56);
        let mut rows = vec![say!("ime.shortcut")];
        rows.extend(shortcut_lines(&shortcut, room));
        return draw_panel_rows(frame, config, area, cursor_x, cursor_y, rows, None);
    }

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

    draw_panel_rows(frame, config, area, cursor_x, cursor_y, rows, Some(highlight));
}

/// 快捷符號's rows: `鍵 文本` cells packed across `room` columns.
///
/// Thirty-one entries one to a line is a panel as tall as the page, and the
/// table is read by **looking for a symbol**, not by scanning a list — so it is
/// laid out as a grid, in the table's own order (`a`–`z`, then the five fixed
/// keys). The cells are one width, taken from the widest entry, so the keys
/// line up in columns.
fn shortcut_lines(rows: &[(String, String)], room: u16) -> Vec<String> {
    const GAP: usize = 2;
    // **A symbol that draws as nothing is drawn as something.** `o` is 全角空格,
    // and a blank cell is indistinguishable from a letter with nothing on it —
    // the reader would conclude `o` is free and never press it.
    let shown = |text: &str| match text.trim().is_empty() {
        true => "␣".to_string(),
        false => text.to_string(),
    };
    let rows: Vec<(String, String)> = rows
        .iter()
        .map(|(key, text)| (key.clone(), shown(text)))
        .collect();
    let width = |(key, text): &(String, String)| {
        yumete_cjk::str_width(key) + 1 + yumete_cjk::str_width(text)
    };
    let Some(cell) = rows.iter().map(width).max() else {
        return Vec::new();
    };
    // At least one to a line: a panel one column wide is still a panel, and a
    // division by a cell wider than the room would be none at all.
    let per = ((room as usize + GAP) / (cell + GAP)).max(1);
    rows.chunks(per)
        .map(|chunk| {
            let mut line = String::new();
            for (i, entry) in chunk.iter().enumerate() {
                if i > 0 {
                    line.push_str(&" ".repeat(GAP));
                }
                line.push_str(&entry.0);
                line.push(' ');
                line.push_str(&entry.1);
                // The last cell on a line is not padded — a trailing run of
                // spaces would widen the panel by a cell nobody can see.
                if i + 1 < chunk.len() {
                    line.push_str(&" ".repeat(cell - width(entry)));
                }
            }
            line
        })
        .collect()
}

/// Frame `rows` in the floating panel below the caret, lighting row `lit`
/// (counted from the first row **after** the header).
///
/// Shared by the candidate list and 快捷符號 because the frame, the placing and
/// the clamping are the same job — only the rows and whether anything is
/// selected differ. `lit` is `None` for 快捷符號: nothing there is selected,
/// and a lit first row would read as 「press Space for this one」.
fn draw_panel_rows(
    frame: &mut Frame,
    config: &Config,
    area: Rect,
    cursor_x: u16,
    cursor_y: u16,
    rows: Vec<String>,
    lit: Option<usize>,
) {
    let skin = vertical::Skin::from(config);
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
    // **And then clamped into `area` whatever the caret said.** Neither branch
    // above is a bound when the caret is *outside* `area`, which is exactly
    // where it stands while a `:` or `/` is being typed: the prompt lives in
    // the command row, below the status line, and `area` stops above it. The
    // flip then measures back from a row the panel may not touch and lands on
    // the status line. `panel_h` is already capped at `area.height`, so this
    // never inverts. (#387)
    let y = y
        .min((area.y + area.height).saturating_sub(panel_h))
        .max(area.y);
    let panel = Rect::new(x, y, panel_w, panel_h);

    let mut lines: Vec<Line> = Vec::with_capacity(rows.len());
    for (i, row) in rows.into_iter().enumerate() {
        if i == 0 {
            // The code as typed, a shade back from the candidates.
            lines.push(Line::from(Span::styled(
                row,
                Style::default().bg(skin.paper()).fg(skin.helper()),
            )));
        } else if lit == Some(i - 1) {
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

    let ground = Style::default().bg(skin.paper()).fg(skin.text());
    let inner = crate::chrome::draw(frame, panel, &crate::chrome::Ring {
        rounded: config.panel.rounded,
        border: Style::default().fg(skin.border()).bg(skin.paper()),
        ground,
        title: None,
    });
    // The rows go inside the ring rather than through a `Paragraph`'s own
    // block: the ring is `chrome`'s now, and one thing should draw it.
    frame.render_widget(Paragraph::new(lines).style(ground), inner);
}

#[cfg(test)]
mod tests {
/// 把一行畫出來的字裏「寬字後面那個空格」擠掉，好照字面對句子（#497）。
///
/// `row_text` 逐格取符號，而一個漢字佔兩格——第二格是空的。從前狀態欄那一句是
/// 純 ASCII 的 `Ln 1, Col 1`，對得上；換成「行 1, 列 1」之後每個漢字後面都多一個
/// 空格。擠掉連續空格再比，就不必把畫面上的間距寫進斷言裏。
fn squeezed(text: &str) -> String {
    let mut out = String::new();
    let mut blank = false;
    for c in text.chars() {
        match c == ' ' {
            true if blank => {}
            true => {
                blank = true;
                out.push(c);
            }
            false => {
                blank = false;
                out.push(c);
            }
        }
    }
    out
}

    /// A whole novel goes through `!` without deadlocking.
    ///
    /// The old `run_capturing` wrote all of stdin before reading any of
    /// stdout, so any filter that answers as it reads — `cat` included —
    /// blocked on its own full stdout while this side blocked filling its
    /// stdin. It hung **on the main thread**: frozen screen, no keys, no `:w`,
    /// and `kill -9` (which writes no recovery copy) as the only way out.
    ///
    /// One megabyte, which is a long chapter and well past every pipe buffer
    /// there is. The watchdog is the point of the test: without the fix this
    /// never returns, and a test that hangs a CI runner for six hours is a
    /// worse report than one that fails.
    #[test]
    fn a_long_manuscript_survives_the_pipe() {
        let text = "那年冬天，山路已經看不見了。\n".repeat(30_000);
        assert!(text.len() > 1_000_000, "{} bytes", text.len());
        let (tell, hear) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let ran = super::run_capturing("cat", Some(&text));
            let _ = tell.send(ran.map(|r| (r.ok, r.said.len(), text.len())));
        });
        match hear.recv_timeout(std::time::Duration::from_secs(60)) {
            Ok(Ok((ok, said, sent))) => {
                assert!(ok, "cat should succeed");
                assert_eq!(said, sent, "everything that went in came back");
            }
            Ok(Err(err)) => panic!("the run failed: {err}"),
            Err(_) => panic!("deadlocked: stdin was written before stdout was read"),
        }
    }

    /// A filter that stops reading early is not a failed run.
    ///
    /// `head -1` closes its stdin after the first line; the write that was
    /// still going then fails with `BrokenPipe`. That is the filter's choice,
    /// not an error to report — the exit status is what says whether the run
    /// worked.
    #[test]
    fn a_filter_that_reads_only_the_first_line_is_not_an_error() {
        let text = "第一行\n".to_string() + &"後面的\n".repeat(200_000);
        let ran = super::run_capturing("head -1", Some(&text)).expect("no error");
        assert!(ran.ok);
        assert_eq!(ran.said, "第一行\n");
    }

    /// 自動認詞裝上的詞要活過下一個按鍵（#491）。
    ///
    /// 每個 worker 送**兩次**：答案，以及線程結束時 `Unlatch` 補的那條空的。
    /// 從前收到空的那一輪會無條件清空，於是剛裝進去的兩百個詞在下一個按鍵就
    /// 沒了——`yume.md` 裏的「宇夢」掃多少遍都不著色，就是這一條。
    #[test]
    fn an_empty_second_answer_does_not_undo_the_first() {
        use yumete_core::discover::Found;
        let mut editor = yumete_core::Editor::new();
        let found = vec![Found { word: "宇夢".to_string(), count: 24, cohesion: 1.0, entropy: 1.0 }];
        super::install_detected(&mut editor, &found);
        assert_eq!(editor.detected_word_count(), 1, "第一條答案裝上了");
        assert!(editor.joins_as_one("宇夢"), "裝上了就併得起來");
        // 守衛那條空的，下一輪到。
        super::install_detected(&mut editor, &[]);
        assert_eq!(editor.detected_word_count(), 1, "空的一條不許抹掉上一條");
        assert!(editor.joins_as_one("宇夢"), "下一個按鍵之後還在");
    }

    /// `:diff` 的改動段是**紅刪綠增**，而四個記號不在頁面上（#499）。
    ///
    /// 原話：「能不能和 git 模式一样，红色表示删除，绿色表示新增，然后对于修改
    /// 的字加红/绿底色表示区别？」——所以驗三件事：記號撤了、兩段各有自己的底、
    /// 兩段的底彼此不同也都不同於紙。
    #[test]
    fn a_diff_listing_paints_what_changed_and_shows_no_markers() {
        let dir = std::env::temp_dir().join(format!("yumete-diff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = dir.join("old.md");
        std::fs::write(&old, "那年冬天，雪下得很早。\n").unwrap();
        let new = dir.join("new.md");
        std::fs::write(&new, "那年冬天，雪下得極早。\n").unwrap();

        let mut editor = Editor::new();
        editor.open_file(&new).unwrap();
        editor.execute(&format!(":diff {}", old.display())).unwrap();

        let config = Config::default();
        let buf = render(&editor, &config, 60, 8);
        let line = (0..8)
            .map(|y| row_text(&buf, y))
            .find(|row| row.contains('很'))
            .expect("改動那一行在頁面上");
        for marker in ["[-", "-]", "{+", "+}"] {
            assert!(!line.contains(marker), "記號撤下頁面了：{line:?}");
        }
        assert!(line.contains('很') && line.contains('極'), "兩邊都畫：{line:?}");

        let y = (0..8).position(|y| row_text(&buf, y).contains('很')).unwrap() as u16;
        let text = row_text(&buf, y);
        let gone = buf[(column_of(&text, "很"), y)].style().bg;
        let added = buf[(column_of(&text, "極"), y)].style().bg;
        let page = buf[(column_of(&text, "年"), y)].style().bg;
        assert_ne!(gone, page, "刪掉的那一段有自己的底");
        assert_ne!(added, page, "新增的那一段也有");
        assert_ne!(gone, added, "而且兩種底分得開");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// 色相：一半原墨，一半藍，**兩者一樣亮**（#501／#507）。
    ///
    /// 原話：「ink……有些字亮有些字暗，亮的像强调」——所以這一支的整個價值就在
    /// 那個亮度差要小。`ink` 量出來是 1.38:1；這裏要求 1.05:1 以內。
    ///
    /// ⚠️ **只換一半。** 兩半都上色試過（暖配冷），代價是整頁沒有一處是原墨：
    /// 「还使用 ink 色比较好，这样只有蓝色的那些词才變色」。
    ///
    /// ⚠️ **本來有顏色的段落一點不碰**——標題的金、連結的藍照舊：色相轉過去就把
    /// 人家的本色扔了（「标题本来是金色，现在变成兰黄」）。
    #[test]
    fn the_hue_mark_keeps_one_half_in_plain_ink_at_one_brightness() {
        let mut editor = editor_with("## 第一章\n\n那年冬天，山下起了大雪。\n");
        editor.set_word_mark(yumete_cjk::WordMark::Color);
        editor.set_segmentation_visible(true);
        let config = Config::default();
        let buffer = render(&editor, &config, 60, 8);

        let y = (0..8u16)
            .position(|y| row_text(&buffer, y).contains('那'))
            .expect("那 is on the page") as u16;
        let text = row_text(&buffer, y);
        let ink_at = |ch: &str| -> (u8, u8, u8) {
            match buffer[(column_of(&text, ch), y)].style().fg {
                Some(ratatui::style::Color::Rgb(r, g, b)) => (r, g, b),
                other => panic!("{ch} 的墨不是 RGB：{other:?}"),
            }
        };
        let mut seen: Vec<(u8, u8, u8)> = Vec::new();
        for ch in ["那", "年", "冬", "天", "山", "下", "起", "了", "大", "雪"] {
            let ink = ink_at(ch);
            if !seen.contains(&ink) {
                seen.push(ink);
            }
        }
        assert_eq!(seen.len(), 2, "整段漢字只有兩種墨：{seen:?}");

        let plain = match ink(&config).text() {
            ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
            other => panic!("原墨不是 RGB：{other:?}"),
        };
        assert!(seen.contains(&plain), "一半就是原墨：{seen:?} vs {plain:?}");

        let lum = |(r, g, b): (u8, u8, u8)| -> f64 {
            let f = |v: u8| {
                let v = f64::from(v) / 255.0;
                match v <= 0.03928 {
                    true => v / 12.92,
                    false => ((v + 0.055) / 1.055).powf(2.4),
                }
            };
            0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
        };
        let (hi, lo) = (lum(seen[0]).max(lum(seen[1])), lum(seen[0]).min(lum(seen[1])));
        let apart = (hi + 0.05) / (lo + 0.05);
        assert!(apart < 1.05, "明度幾乎不動，量到 {apart:.3}:1：{seen:?}");

        // 標題是金的，而且**整行一種墨**——色相沒有插手。
        let hy = (0..8u16)
            .position(|y| row_text(&buffer, y).contains('章'))
            .expect("the heading is on the page") as u16;
        let head = row_text(&buffer, hy);
        let gold = buffer[(column_of(&head, "第"), hy)].style().fg;
        assert_eq!(
            buffer[(column_of(&head, "章"), hy)].style().fg,
            gold,
            "標題一種墨，沒有詞界：{head:?}"
        );
        assert_eq!(gold, Some(ink(&config).gold()), "而且還是金的");
    }

    /// 線：字本身一點不動，只在詞下畫一條（#501）。
    #[test]
    fn the_line_mark_touches_the_writing_not_at_all() {
        let plain = {
            let mut editor = editor_with("那年冬天，山下起了大雪。\n");
            editor.set_segmentation_visible(false);
            render(&editor, &Config::default(), 60, 6)
        };
        let mut editor = editor_with("那年冬天，山下起了大雪。\n");
        editor.set_word_mark(yumete_cjk::WordMark::Line);
        editor.set_segmentation_visible(true);
        let marked = render(&editor, &Config::default(), 60, 6);

        let mut ruled = 0;
        for x in 0..24u16 {
            let (a, b) = (plain[(x, 0)].style(), marked[(x, 0)].style());
            assert_eq!(a.fg, b.fg, "第 {x} 格的字色沒動");
            assert_eq!(a.bg, b.bg, "第 {x} 格的底色沒動");
            if b.add_modifier.contains(Modifier::UNDERLINED) {
                ruled += 1;
                assert!(b.underline_color.is_some(), "線有自己的顏色");
            }
        }
        assert!(ruled > 0, "有詞被畫了線");
        assert!(ruled < 24, "不是整行都畫——那就沒有分界了");
    }

    /// 落款在提示行的右端，擠不下就沒有（#498）。
    ///
    /// 要的是一個 identity，而位置是有講究的：這一行的**左端**是全屏最有用的
    /// 一條——按了半個 `t` 就靠它說下一個鍵能按什麼——所以落款只能在右端，而且
    /// 一旦右端被佔就該讓開。
    #[test]
    fn the_signature_sits_at_the_right_and_yields_when_the_row_is_full() {
        let signature = yumete_core::messages::say("ui.signature", &[]);
        let editor = editor_with("那年冬天。\n");
        let config = Config::default();

        let wide = render(&editor, &config, 80, 8);
        let row = row_text(&wide, 7);
        assert!(row.contains(&signature), "寬的時候在：{row:?}");
        assert!(
            row.trim_end().ends_with(&signature),
            "而且在最右邊：{row:?}"
        );

        // 提示行滿了就讓開：表格那一行有三組鍵，四十格裝不下它再加落款。
        let mut editor = editor_with("| 星陳 | 卿雲 |\n| --- | --- |\n| 甲 | 乙 |\n");
        for key in ['j', 'j', 't', 'b'] {
            editor.on_key(Key::Char(key));
        }
        let narrow = render(&editor, &config, 40, 8);
        let row = row_text(&narrow, 7);
        assert!(row.contains('t'), "提示行確實在說話：{row:?}");
        assert!(!row.contains(&signature), "擠不下就讓開：{row:?}");
    }

    /// 輸入框要看得出是個框（#447）。
    ///
    /// 原話：「这里的输入框能不能画个上下框线什么的？不然还是不知道这里有个
    /// 可以输入的地方。」空的 尋找 框從前與面板同底色，只有一個光標浮在那裏，
    /// 看不出是一格能打字的地方。現在框裏是**紙色**（第 90 檔，比面板的第 81 檔
    /// 沉一階），上下各一道線把它封起來。
    #[test]
    fn the_search_box_is_drawn_as_a_box() {
        let mut ed = Editor::new();
        ed.on_key(Key::Char(' '));
        ed.on_key(Key::Char('/'));
        let config = Config::default();
        let (buf, _) = render_caret(&ed, &config, 60, 16);
        let row = |y: u16| -> String {
            (0..60).filter_map(|x| buf.cell((x, y)).map(|c| c.symbol().to_string())).collect()
        };
        // 標題、線、框、線 —— 這個次序就是「這裏能打字」的全部說法。
        // 一格一個字符，全角字的第二格是空的，所以比的是第一個字。
        let first = say!("label.panel.search").chars().next().unwrap();
        assert!(row(0).contains(first), "{:?}", row(0));
        assert!(row(1).contains("──"), "框上要有線：{:?}", row(1));
        assert!(row(3).contains("──"), "框下要有線：{:?}", row(3));

        // 框裏的底色不是面板的底色。空框沒有字，所以**只有底色說得出它在那裏**。
        let ink = crate::theme::Palette::of(&config);
        let inside = buf.cell((10, 2)).expect("框裏").style().bg;
        let panel = buf.cell((10, 4)).expect("面板上的別處").style().bg;
        assert_ne!(inside, panel, "框與面板同色，等於沒有框");
        assert_eq!(inside, Some(ink.ground(yumete_config::rung::PAPER).bg.unwrap()));
    }

    /// `:yume-where` names every layer, in order, with what it holds.
    ///
    /// The failure it exists for is silent: a reader installs 宇浩, types
    /// 漢字, gets the cut table anyway, and has no way to ask which of six
    /// directories the editor actually read. So the test is that the report
    /// **names all six kinds of place**, says something about each, and ends
    /// with the one answering now — not that any particular directory exists,
    /// which depends on the machine.
    #[test]
    fn where_names_every_layer_it_looked_in() {
        use yumete_ime::{ImeSession, Scheme};
        let ime = ImeSession::new(Scheme::LINGMING, vec![std::path::PathBuf::from("/no/such")]);
        let report = super::where_report(&ime);

        // Every layer that exists on this machine is numbered and labelled.
        let numbered: Vec<&str> = report
            .lines()
            .filter(|l| l.split_once(". ").is_some_and(|(n, _)| n.parse::<u32>().is_ok()))
            .collect();
        assert!(numbered.len() >= 4, "too few layers:\n{report}");
        for (nth, line) in numbered.iter().enumerate() {
            assert!(
                line.starts_with(&format!("{}. ", nth + 1)),
                "layers must be numbered in order, got {line:?}\n{report}"
            );
        }
        // The built-in tables are named last, because they are what answers
        // when none of the directories do.
        let builtin = crate::say!("yume.where.builtin");
        assert!(report.contains(&builtin), "{report}");
        assert!(
            report.rfind(&builtin) > report.find("yumete"),
            "the built-in layer comes after the directories\n{report}"
        );
        // …and the last thing it says is which one is answering.
        assert!(report.trim_end().ends_with(&ime.table_source()), "{report}");
        // A directory with nothing in it says so rather than being left blank.
        assert!(report.contains(&crate::say!("yume.where.nothing")), "{report}");
    }

    /// #220: an installed data file the core refuses names the *version*, not
    /// the symptom. Before this, `:yume` said nothing at all and the writer saw
    /// only 拆分 comments that had stopped appearing.
    #[test]
    fn yume_names_a_data_file_the_core_will_not_have() {
        let entry = yumete_ime::data_set(Scheme::LINGMING)
            .into_iter()
            .find(|f| f.kind == yumete_ime::DataKind::Annotations)
            .expect("拆分 is in the manifest");
        let dir = std::env::temp_dir().join(format!("yumete-yume-fault-{}", std::process::id()));
        let path = dir.join(entry.file.replace('/', std::path::MAIN_SEPARATOR_STR));
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("fixture dir");
        let mut stale = b"YDV20260828".to_vec();
        stale.extend_from_slice(&[0u8; 64]);
        std::fs::write(&path, &stale).expect("fixture file");

        let ime = ImeSession::new(Scheme::LINGMING, vec![dir]);
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
        let ime = ImeSession::new(Scheme::LINGMING, vec![std::path::PathBuf::from("/no/such/dir")]);
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
        ImeSession::new(Scheme::LINGMING, vec![])
    }

    /// Render `editor` with `config` and `ime` to an in-memory terminal buffer.
    fn render_with(
        editor: &Editor,
        config: &Config,
        ime: &ImeSession,
        w: u16,
        h: u16,
    ) -> ratatui::buffer::Buffer {
        // **The mood, before the first cell is painted** — the same omission
        // `:shot` once had, one floor down. `run` settles it at startup and
        // nothing else in a test does, so `MOOD` began light and turned dark
        // the first time a `:shot` test settled it: every other test that
        // compared a rendered colour against `Palette::of` was racing that one
        // flip, and about one run in four lost.
        crate::theme::settle(config, None);
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut viewport = Seats::default();
        terminal
            .draw(|frame| draw(frame, editor, config, ime, &mut viewport))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    /// **Which buffer a `:shot` is a picture of.** `Terminal::draw` swaps
    /// ratatui's two buffers on its way out and resets the one it swaps in, so
    /// `current_buffer_mut()` afterwards is blank paper, not the frame that was
    /// just painted. `:shot html` and `:shot txt` read from there from the day
    /// they landed (#189) until 2026-09-07, and wrote a page of empty rows with
    /// the version footer under it — a file that looks written and says
    /// nothing. `--shot` never showed it: that path asks the backend.
    #[test]
    fn a_picture_is_of_the_frame_draw_hands_back_not_the_one_it_reset() {
        let mut terminal = Terminal::new(TestBackend::new(16, 2)).unwrap();
        let drawn = terminal
            .draw(|frame| {
                frame.render_widget(ratatui::widgets::Paragraph::new("有字"), frame.area());
            })
            .map(|completed| completed.buffer.clone())
            .expect("draw one frame");
        assert!(buffer_to_text(&drawn).contains("有字"));
        let next = buffer_to_text(terminal.current_buffer_mut());
        let page: String = next.lines().take(2).collect();
        assert!(page.trim().is_empty(), "the next frame's paper is blank: {page:?}");
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
        let page = page_areas(editor, config, Rect::new(0, 0, w, h), 0).text;
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

    /// #375/#377: the status line never hands the terminal a character it
    /// would **act on**.
    ///
    /// The readout names the character under the cursor, and on a TAB it put
    /// the tab itself there: nought cells to every width this editor asks, a
    /// jump to the next stop to the terminal. Measured as exactly filling the
    /// line and drawn wider than it, so it overflowed, wrapped, and made the
    /// frame a row taller than the screen — which scrolls the terminal and
    /// leaves the previous frame standing on the page.
    #[test]
    fn the_status_line_holds_nothing_the_terminal_would_act_on() {
        let mut editor = editor_with("ch\t錐\n");
        editor.on_key(Key::Char('l'));
        editor.on_key(Key::Char('l'));
        assert_eq!(editor.char_at_cursor(), Some('\t'), "standing on the tab");
        for w in [40u16, 60, 61, 80, 99, 120] {
            let buffer = render_with(&editor, &Config::default(), &no_ime(), w, 8);
            let row = status_line(&buffer);
            assert!(
                !row.chars().any(char::is_control),
                "w={w}: a control character on the status line: {row:?}"
            );
            // What the terminal will consume is what the line was measured at,
            // which is the whole of this bug: the two used to differ by the
            // width of a tab stop.
            assert_eq!(
                yumete_cjk::drawn_width(&row),
                w as usize,
                "w={w}: {row:?}"
            );
            // Narrow, the readout gives way altogether — that is the two
            // stages of giving way doing their job, not this bug.
            if w >= 60 {
                assert!(row.contains("U+0009"), "w={w}: and it still names it: {row:?}");
            }
        }
    }

    /// The first latch on its own: the readout names a formless character
    /// rather than showing it.
    #[test]
    fn the_readout_names_a_character_it_cannot_show() {
        let config = Config::default();
        let mut editor = editor_with("ch\t錐\n");
        editor.on_key(Key::Char('l'));
        editor.on_key(Key::Char('l'));
        let [long, short, _] = char_info(&editor, &config);
        assert!(!long.contains('\t'), "{long:?}");
        assert!(!short.contains('\t'), "{short:?}");
        assert!(long.starts_with("U+0009"), "{long:?}");
    }

    /// The second latch on its own, which is the one that does not depend on
    /// anybody remembering: whatever reaches the status line, a character the
    /// terminal would act on does not.
    #[test]
    fn the_status_line_strips_what_it_is_handed() {
        assert_eq!(drawable("a\tb"), "ab");
        assert_eq!(drawable("a\rb\u{8}c"), "abc");
        // The common case pays nothing and is handed straight back — borrowed,
        // not rebuilt, which is every line of every real page.
        assert!(matches!(
            drawable("-- NORMAL --  file.txt"),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    /// The rule, set on 2026-09-11：「TAB 是个不稳定渲染。除了文本
    /// 区有 tab 外，我们在其他位置不应该有 tab 存在。太危险了。」
    ///
    /// Not one guard per drawing path — about thirty of them draw spans that
    /// never see [`drawable`] — but one sweep at the end, because every path
    /// ends in a cell. So this test does not ask a particular path to behave;
    /// it asks the *frame* whether anything got through, which is the only
    /// question that stays answered when a thirty-first path is written.
    ///
    /// **It passes with the sweep taken out**, and that is worth writing down
    /// rather than hiding: every path a control character can reach today is
    /// already stopped upstream, so the sweep closes no hole that is open now.
    /// What it buys is that the next span cannot open one, and that the rule
    /// has an address instead of thirty.
    #[test]
    fn no_cell_of_a_frame_holds_a_character_the_terminal_obeys() {
        // **A file whose name holds a tab**, which is the case no guard
        // upstream covers: the tab bar and the pane title take the name and
        // hand it straight to a span. It is a legal name on every system this
        // runs on, and one `curl` of a badly-made archive puts one on disk.
        let dir = std::env::temp_dir().join(format!("yumete-tab-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a directory to write in");
        let path = dir.join("ta\tb.txt");
        std::fs::write(&path, "ch\t錐\n").expect("a file to open");

        // …and a status line with a tab under the cursor, a message quoting
        // one, and a buffer whose text is full of them.
        let mut editor = Editor::new();
        editor.open_file(&path).expect("open it");
        editor.on_key(Key::Char('l'));
        editor.on_key(Key::Char('l'));
        editor.set_status("a\tb\u{1b}[31m".to_string());
        for (w, h) in [(40u16, 10u16), (80, 24), (120, 40)] {
            let buffer = render_with(&editor, &Config::default(), &no_ime(), w, h);
            for y in 0..h {
                for x in 0..w {
                    let symbol = buffer[(x, y)].symbol();
                    assert!(
                        !symbol.bytes().any(|b| b < 0x20 || b == 0x7f),
                        "{w}x{h} at ({x},{y}): {symbol:?}"
                    );
                }
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A character that *has* a form is still shown beside its code point —
    /// the readout exists to answer「is this the character I think it is」.
    #[test]
    fn a_character_with_a_form_is_still_shown() {
        let editor = editor_with("錐\n");
        let buffer = render_with(&editor, &Config::default(), &no_ime(), 80, 8);
        let row = status_line(&buffer);
        assert!(row.contains("錐 U+9310"), "{row:?}");
    }

    /// #379, the design: 「在顶部预留一个信息栏（两行：列号+列名）。
    /// 这个平常不显示，只是在下方是表格中间部分的时候显示。这样它是独立的，
    /// 也就不会侵扰文本的区域了。」
    ///
    /// The assertion that matters is not that the bar is *there* — it is that
    /// the names and numbers stand over the columns **as the row below them is
    /// drawn**, which is why they are placed by that row's own `Drawn` and not
    /// worked out again. A bar that names one column while pointing at another
    /// is worse than no bar.
    #[test]
    fn a_table_scrolled_past_its_head_is_given_a_bar_over_its_columns() {
        let mut source = String::from("| 地名 | 小傳 | 年代 |\n| --- | --- | --- |\n");
        for i in 0..40 {
            source.push_str(&format!("| 城{i} | 記事{i} | 年{i} |\n"));
        }
        let mut editor = editor_with(&source);
        for c in "tb".chars() {
            editor.on_key(Key::Char(c));
        }
        editor.execute("30").unwrap();
        let buffer = render_with(&editor, &Config::default(), &no_ime(), 44, 12);
        let numbers = row_text(&buffer, 0);
        let names = row_text(&buffer, 1);
        let row = row_text(&buffer, 2);
        assert!(names.contains("地名") && names.contains("年代"), "{names:?}");
        assert!(numbers.contains('1') && numbers.contains('3'), "{numbers:?}");

        // Each label ends where its column does, on the row under the bar.
        // The row opens with a wall of its own, so column 1 ends at the
        // *second* pipe — counted in screen cells, not in bytes.
        let mut walls = Vec::new();
        let mut at = 0u16;
        for c in row.chars() {
            if c == '|' {
                walls.push(at);
            }
            at += yumete_cjk::char_width(c) as u16;
        }
        assert!(walls.len() >= 2, "the row has its walls: {row:?}");
        assert_eq!(
            column_of(&numbers, "1") + 1,
            walls[1],
            "the number stands against its own wall: {numbers:?} over {row:?}"
        );
        assert_eq!(
            column_of(&names, "地名") + yumete_cjk::str_width("地名") as u16,
            walls[1],
            "and so does the name: {names:?} over {row:?}"
        );
    }

    /// The bar is the table's own top two rows brought up the page: the ruler
    /// above, the names under it, exactly as they stand when the head is in
    /// view. Drawn the other way round until it was caught.
    #[test]
    fn the_bar_is_ordered_the_way_the_table_top_is() {
        let mut source = String::from("| 地名 | 年代 |\n| --- | --- |\n");
        for i in 0..40 {
            source.push_str(&format!("| 城{i} | 年{i} |\n"));
        }
        let mut editor = editor_with(&source);
        for c in "tf".chars() {
            editor.on_key(Key::Char(c));
        }
        editor.execute("36").unwrap();
        let buffer = render_with(&editor, &Config::default(), &no_ime(), 44, 12);
        let (upper, lower) = (row_text(&buffer, 0), row_text(&buffer, 1));
        assert!(upper.contains('1') && !upper.contains("地名"), "numbers above: {upper:?}");
        assert!(lower.contains("地名"), "names under them: {lower:?}");
    }

    /// **Not while the head is in view.** Entering a long table puts the
    /// cursor a few rows below its head, and a bar naming columns that are
    /// named two rows above it is two rows spent saying nothing.
    #[test]
    fn a_table_whose_head_is_still_in_view_is_given_no_bar() {
        let mut source = String::from("| 地名 | 小傳 | 年代 |\n| --- | --- | --- |\n");
        for i in 0..40 {
            source.push_str(&format!("| 城{i} | 記事{i} | 年{i} |\n"));
        }
        let mut editor = editor_with(&source);
        for c in "tb".chars() {
            editor.on_key(Key::Char(c));
        }
        // Three rows into a forty-row table, with the page thirty tall: the
        // head is right there.
        editor.set_page(30, 100);
        editor.execute("5").unwrap();
        assert!(
            !editor.table_head_is_off_the_page(0),
            "the head is on the page, so no bar"
        );
        // …and once the cursor is further down than the page is tall, it
        // cannot be, whatever the scroll did.
        editor.execute("36").unwrap();
        assert!(editor.table_head_is_off_the_page(0), "now it cannot be");
    }

    /// 平常不显示: a table the page can show whole has its own head in view, so
    /// two rows spent repeating it would be two rows wasted.
    #[test]
    fn a_table_that_fits_is_given_no_bar() {
        let mut editor = editor_with("前文\n\n| 地名 | 年代 |\n| --- | --- |\n| 洛陽 | 春秋 |\n");
        for c in "tb".chars() {
            editor.on_key(Key::Char(c));
        }
        editor.execute("5").unwrap();
        assert!(!editor.table_head_is_off_the_page(0), "its head is in view");
        let buffer = render_with(&editor, &Config::default(), &no_ime(), 44, 12);
        assert!(
            row_text(&buffer, 0).contains("前文"),
            "the page starts at the file: {:?}",
            row_text(&buffer, 0)
        );
    }

    /// **A row on a page of prose never opens the panel unasked** — `to`,
    /// `tb` and `tf` alike (2026-09-11: 「tb 模式（basic）默认不用打开
    /// information panel」; 2026-09-15: 「tf 和 tb 在信息面板显示上保持一致」).
    /// Only `tt`, where the grid has the window, opens it: there the panel is
    /// the way to read a folded cell whole and there is no prose to disturb.
    ///
    /// ⚠️ `tf` used to open it, and the reason it must not is the **cost on a
    /// prose page**: the panel takes a fifth of the width, so the paragraphs
    /// above and below rewrap the moment it appears — 「markdown 中如果向下移动
    /// 遇到表格总是会发生 wrap 跳动」. Scrolling past a table made the page jump.
    ///
    /// And `t i` outranks all of it either way: what the reader asked for is
    /// not something a change of level may quietly undo.
    #[test]
    fn the_panel_opens_only_in_the_pane_and_never_unasked_on_a_page() {
        let long = "甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉戌亥天地玄黃";
        let source = format!("| 地名 | 備註 |\n| --- | --- |\n| 洛陽 | {long} |");
        let at = |level: &str| {
            let mut editor = editor_with(&source);
            for key in ['j', 'j'] {
                editor.on_key(Key::Char(key));
            }
            for c in level.chars() {
                editor.on_key(Key::Char(c));
            }
            editor
        };
        assert!(!at("to").detail_visible(), "源碼 leaves it shut");
        assert!(!at("tb").detail_visible(), "基本 leaves it shut");
        assert!(!at("tf").detail_visible(), "全 too — it is still a page of prose");
        assert!(at("tt").detail_visible(), "the pane is the one that opens it");

        // Asked for, it opens at 基本 too…
        let mut editor = at("tb");
        editor.on_key(Key::Char('t'));
        editor.on_key(Key::Char('i'));
        assert!(editor.detail_visible(), "{}", editor.status());
        // …and the answer travels: walking up to 全 and back does not undo it.
        for c in "tftb".chars() {
            editor.on_key(Key::Char(c));
        }
        assert!(editor.detail_visible(), "the reader's answer outlives the level");

        // The other way round: asked for at 全 and it opens there, which is
        // the whole of what 「按 ti 自己打开」 buys.
        let mut editor = at("tf");
        editor.on_key(Key::Char('t'));
        editor.on_key(Key::Char('i'));
        assert!(editor.detail_visible(), "{}", editor.status());
        // …and shutting the pane's own is equally the reader's to do.
        let mut editor = at("tt");
        editor.on_key(Key::Char('t'));
        editor.on_key(Key::Char('i'));
        assert!(!editor.detail_visible(), "{}", editor.status());
    }

    /// #379: 基本 numbers the columns too.    /// #379: 基本 numbers the columns too.
    ///
    /// 2026-09-11：「tb 模式可不可以也标注列号（和 tf 模式一样）」，
    /// which 基本's own law allows — 「tb 的原则是只能多字（标注）不能少字」 —
    /// because a strip above the table adds a row and hides nothing. 源碼 gets
    /// none: there the file is drawn as it is written.
    #[test]
    fn 基本_numbers_the_columns_and_源碼_does_not() {
        let strip = |level: &str| -> String {
            let mut editor = editor_with("ch\t錐\nlongcode\t蜘\n");
            for c in level.chars() {
                editor.on_key(Key::Char(c));
            }
            let buffer = render_with(&editor, &Config::default(), &no_ime(), 40, 8);
            row_text(&buffer, 0).trim_end().to_string()
        };
        // The numbers stand over their columns, so they are the row's whole
        // content — nothing of the file is on it.
        let basic = strip("tb");
        assert!(basic.contains('1') && basic.contains('2'), "基本: {basic:?}");
        assert!(!basic.contains("ch"), "and it is not the table's first row: {basic:?}");
        let full = strip("tf");
        assert_eq!(basic, full, "基本 numbers them exactly as 全 does");
        let source = strip("to");
        assert!(source.contains("ch"), "源碼 starts at the file: {source:?}");
    }

    /// #378 on the drawn frame: a TSV read as a table squares its columns up
    /// the way a Markdown table does, and the tab in it is a wall — one cell,
    /// like a pipe — rather than a stop.
    ///
    /// On the frame because the run list has lied once already (#374): it said
    /// the columns lined up while ratatui was giving the tab no cell at all.
    #[test]
    fn a_delimited_table_squares_its_columns_up_like_a_pipe_one() {
        let mut editor = editor_with("ch\t錐\nlongcode\t蜘\nbk\t裘\n");
        for c in "tf".chars() {
            editor.on_key(Key::Char(c));
        }
        let buffer = render_with(&editor, &Config::default(), &no_ime(), 40, 6);
        // Row 0 is the strip that numbers the columns; the rows follow it.
        let at: Vec<u16> = ["錐", "蜘", "裘"]
            .iter()
            .enumerate()
            .map(|(y, ch)| column_of(&row_text(&buffer, y as u16 + 1), ch))
            .collect();
        assert_eq!(at[0], at[1], "ch and longcode reach one column: {at:?}");
        assert_eq!(at[1], at[2], "and bk with them: {at:?}");
        // The wall is one cell, so the widest code is followed by exactly one
        // before its character — a tab advanced to a stop would be more.
        let row = row_text(&buffer, 2);
        let gutter = column_of(&row, "l");
        assert_eq!(at[1] - gutter, "longcode".len() as u16 + 1, "{row:?}");
    }

    /// #374, on the **drawn frame** rather than on the run list: a 碼表 lines
    /// its characters up in one column whatever the code before them is.
    ///
    /// The run list said so all along and the page still did not, because the
    /// tab kept its cell in every width the editor asks and lost it in
    /// ratatui: `unicode-width` answers `None` for a control character, so the
    /// row came out one cell short of the column the caret was told it stood
    /// in. Only a rendered frame catches that.
    #[test]
    fn a_code_table_puts_every_character_in_the_same_column() {
        let editor = editor_with("ch\t錐\nbkd\t蜘\nfvtf\t裘\n");
        let buffer = render_with(&editor, &Config::default(), &no_ime(), 40, 6);
        let at: Vec<u16> = ["錐", "蜘", "裘"]
            .iter()
            .enumerate()
            .map(|(y, ch)| column_of(&row_text(&buffer, y as u16), ch))
            .collect();
        assert_eq!(at[0], at[1], "two letters and three reach the same stop");
        assert_eq!(at[1], at[2], "and four, which needs a whole stop of its own");
        // The stop is eight, and the gutter is furniture in front of it.
        let gutter = column_of(&row_text(&buffer, 0), "c");
        assert_eq!(at[0] - gutter, 8, "{:?}", row_text(&buffer, 0));
    }

    /// **A stopwatch on the grid**, not a test — it is `#[ignore]`d because it
    /// asserts nothing about time; it prints what the clock said.
    ///
    /// ```text
    /// cargo test -p yumete-tui --release the_cost_of_drawing_a_grid \
    ///     -- --ignored --nocapture
    /// ```
    ///
    /// It walks the roadmap table in `docs/development.md` and a synthetic one
    /// whose 備註 column is 7,000 characters wide — the shape the roadmap had
    /// before #296's footnotes — twice each: one row at a time, and in jumps of
    /// a hundred. Keys and drawing are timed apart, because they answer
    /// different questions: the draw is what #289 is about, the keys are what a
    /// counted motion costs.
    #[test]
    #[ignore]
    fn the_cost_of_drawing_a_grid() {
        use std::time::Instant;
        let ime = no_ime();
        let config = Config::default();
        let (w, h) = (200u16, 50u16);

        let fat = std::env::temp_dir().join("yumete-bench-fat-cells.md");
        {
            let mut text = String::from("| # | 名 | 備註 |\n| --- | --- | --- |\n");
            let para = "這一格是一整段話，長到沒有任何視窗裝得下它，所以折行會把它攤成很多行。"
                .repeat(200);
            for i in 0..300 {
                text.push_str(&format!("| {i} | 第 {i} 條 | {para} |\n"));
            }
            std::fs::write(&fat, text).unwrap();
        }

        let run = |file: &str, wrap: bool, jump: bool| -> (f64, f64) {
            let mut editor = Editor::new();
            editor.open_file(file).expect("a table to walk");
            for c in ":20".chars() {
                editor.on_key(Key::Char(c));
            }
            editor.on_key(Key::Enter);
            editor.on_key(Key::Char('t'));
            editor.on_key(Key::Char('t'));
            assert!(editor.grid_has_the_pane(), "t t did not hand the grid the pane");
            if wrap {
                editor.on_key(Key::Char('t'));
                editor.on_key(Key::Char('a'));
                assert!(editor.cell_wrap(), "t a did not turn 折行 on");
            }
            let _ = render_with(&editor, &config, &ime, w, h);
            let steps = 100;
            let (mut keys, mut drawn) = (0u128, 0u128);
            for i in 0..steps {
                let began = Instant::now();
                if jump {
                    // Off the page every time, so the scroll rule drops the top
                    // to cursor − half a window and the 折行 loop pushes from
                    // there — the case #289 is about.
                    for c in "100".chars() {
                        editor.on_key(Key::Char(c));
                    }
                    editor.on_key(Key::Char(if i % 2 == 0 { 'j' } else { 'k' }));
                } else {
                    editor.on_key(Key::Char('j'));
                }
                keys += began.elapsed().as_micros();
                let began = Instant::now();
                let _ = render_with(&editor, &config, &ime, w, h);
                drawn += began.elapsed().as_micros();
            }
            let per = |t: u128| t as f64 / steps as f64 / 1000.0;
            (per(keys), per(drawn))
        };

        let fat = fat.to_string_lossy().to_string();
        println!("\n{:<20}{:>12}{:>12}{:>12}", "", "keys", "draw 摺", "draw 折行");
        for (name, file) in [("roadmap", "../../docs/development.md"), ("7,000 字一格", fat.as_str())] {
            for jump in [false, true] {
                let off = run(file, false, jump);
                let on = run(file, true, jump);
                println!(
                    "{:<20}{:>9.3} ms{:>9.3} ms{:>9.3} ms",
                    format!("{name}  {}", if jump { "100j" } else { "j" }),
                    off.0,
                    off.1,
                    on.1
                );
            }
        }
    }

    /// `cargo test -p yumete-tui --release the_cost_of_a_flick -- --ignored --nocapture`
    ///
    /// ⚠️ **This measures the page being computed, not the page being shown.**
    /// `render_with` draws into a `TestBackend`, which is memory: no diff into
    /// escape sequences, no write, no terminal. The numbers here came out at
    /// one to three milliseconds and were read as 「scrolling is cheap」, while
    /// the thing that actually stalled — `terminal.draw` blocking on a tty that
    /// had fallen behind — is not in them at all (#360). Trust it about the
    /// layout and about nothing else.
    #[test]
    #[ignore]
    fn the_cost_of_a_flick() {
        use std::time::Instant;
        let doc = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/development.md");
        let ime = no_ime();
        let config = Config::default();
        let (w, h) = (200u16, 50u16);
        for target in [500usize, 3000, 6000] {
            for notches in [64usize, 256, 1024] {
                for back in [false, true] {
                    let mut ed = Editor::new();
                    ed.open_file(doc).unwrap();
                    ed.set_wrap_width(w as usize);
                    ed.set_page(h as usize, w as usize);
                    ed.execute(&format!(":{target}")).unwrap();
                    let _ = render_with(&ed, &config, &ime, w, h);
                    let began = Instant::now();
                    ed.scroll(3 * notches, back);
                    let flick = began.elapsed().as_secs_f64() * 1000.0;
                    let began = Instant::now();
                    let _ = render_with(&ed, &config, &ime, w, h);
                    println!(
                        "line {target:>5}  {:>4} notches {}  scroll {flick:>9.1}  draw {:>7.1}  (ms)",
                        notches,
                        if back { "up  " } else { "down" },
                        began.elapsed().as_secs_f64() * 1000.0
                    );
                }
            }
        }
    }

    /// The symbol at a cell, for grid assertions.
    fn at(buffer: &ratatui::buffer::Buffer, x: u16, y: u16) -> String {
        buffer[(x, y)].symbol().to_string()
    }

    /// #283. 「折叠标志要不要加个下划线背景色什么的突出一下避免用户当作它是个
    /// 普通的 `>`」 (2026-09-07). The glyph itself cannot change — every
    /// ellipsis Unicode has is East Asian *Ambiguous*, and a table is where the
    /// two width tables must agree to the cell — so the ink carries it: 金, and
    /// bold, on a page that is otherwise a run of greys.
    #[test]
    fn the_fold_mark_is_not_in_the_ink_of_the_writing_it_stands_after() {
        let long = "甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉戌亥天地玄黃宇宙洪荒日月盈昃";
        let source = format!("| 地名 | 小傳 |\n| --- | --- |\n| 洛陽 | {long} |");
        let config = Config::default();
        // Both surfaces the mark is drawn on: the prose page, which folds
        // against the columns it squares up once `t w` asks it to, and the
        // pane, which draws its own grid and caps it unasked.
        for keys in [vec!['j', 'j', 't', 'b', 't', 'w'], vec!['j', 'j', 't', 't']] {
            let mut editor = editor_with(&source);
            for key in keys.iter().copied() {
                editor.on_key(Key::Char(key));
            }
            let how: String = keys.iter().collect();
            let frame = render(&editor, &config, 96, 10);
            // The grid alone: the pane's panel prints the whole cell down the
            // right, and the mark is never drawn in it.
            let rows: Vec<String> = (0..10)
                .map(|y| row_text(&frame, y).split('│').next().unwrap_or("").to_string())
                .collect();
            let y = rows
                .iter()
                // **The row this test is about**: the one that has the name
                // *and* a fold mark on it. A bare `洛陽` is on the page twice
                // — the panel beside the prose page lists the cells too — and
                // it matched the rule row's half of it. Asking for both is
                // also the only needle that works in either drawing, since the
                // pane re-glyphs the pipes away. (#379 moved the column strip
                // down a level, which is what turned this up.)
                .position(|row| row.contains("洛陽") && row.contains('>'))
                .unwrap_or_else(|| panic!("the row is on the page ({how}):\n{}", rows.join("\n")))
                as u16;
            let text = &rows[y as usize];
            let mark = frame[(column_of(text, ">"), y)].style();
            let word = frame[(column_of(text, "甲"), y)].style();
            assert!(
                mark.add_modifier.contains(Modifier::BOLD),
                "the mark is bold ({how}): {mark:?}"
            );
            assert!(
                !word.add_modifier.contains(Modifier::BOLD),
                "and the writing beside it is not ({how}): {word:?}"
            );
            assert_ne!(mark.fg, word.fg, "and it is not in the writing's ink ({how})");
        }
    }

    /// One rendered row, as the reader sees it.
    ///
    /// A wide glyph occupies two cells and ratatui blanks the second, so the
    /// row is walked by display width rather than by cell.
    /// #283. The panel's shape follows what it **holds**. A row of a Markdown
    /// table used to get the note's four-line strip along the bottom, because
    /// the shape was picked from whether the table had taken the window — so a
    /// five-column row showed two of its fields with no sign there were more.
    #[test]
    fn a_row_panel_goes_down_the_right_in_prose_too() {
        let mut editor = editor_with(
            "一段話。\n\n| A | B | C | D | E |\n| --- | --- | --- | --- | --- |\n| 1 | 2 | 3 | 4 | 5 |",
        );
        // `t i`, because a table on a page of prose does not open the panel
        // by itself any more (#495) — what is under test here is its *shape*
        // once it is open.
        for key in ['4', 'j', 't', 'f', 't', 'i'] {
            editor.on_key(Key::Char(key));
        }
        let config = Config::default();
        let frame = render(&editor, &config, 76, 24);
        let page: String = (0..24).map(|y| row_text(&frame, y)).collect::<Vec<_>>().join("\n");
        assert!(page.contains(" 5 E"), "every field is there:\n{page}");
    }

    /// #283, off `development.md` itself (2026-09-07): in the
    /// window the grid cut every over-wide cell at 32 cells, said nothing
    /// about having done it, and had no key to give the tail back — 「长单元格
    /// 被折叠的信息永远无法读取」.
    ///
    /// Three answers, and this is all three: the cut is marked, `t w` lifts
    /// the cap, and the cell **being typed in** is drawn whole.
    ///
    /// That last one used to open for the caret in Normal as well, and
    /// it was taken back to the prose page's rule on 2026-09-11：
    /// 「列宽容易跳……建议这个和 tf 保持一致」. Which leaves Normal three ways
    /// to read a cut cell and no moving columns: the panel down the right has
    /// it whole already, `i` opens it in place, and `t w` opens every one.
    #[test]
    fn the_grid_marks_a_cut_cell_t_w_lifts_the_cap_and_the_caret_reads_whole() {
        let long = "甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉戌亥天地玄黃";
        let mut editor = editor_with(&format!(
            "| 地名 | 備註 |\n| --- | --- |\n| 洛陽 | {long} |"
        ));
        for key in ['j', 'j', 't', 't'] {
            editor.on_key(Key::Char(key));
        }
        editor.on_key(Key::Char('T')); // #356: 這一條測的是格
        assert!(editor.grid_has_the_pane(), "{}", editor.status());
        let config = Config::default();
        // **The grid, not the window**: the panel down the right is showing
        // the same cell whole, which is its job, and reading the two together
        // would let the panel answer for the grid.
        let page = |editor: &Editor| -> String {
            let frame = render(editor, &config, 120, 10);
            (0..10)
                .map(|y| row_text(&frame, y).split('│').next().unwrap_or("").to_string())
                .collect::<Vec<_>>()
                .join("\n")
        };

        // The caret is in 地名, so 備註 is cut — and says so.
        let folded = page(&editor);
        assert!(!folded.contains("玄黃"), "the tail is off the grid:\n{folded}");
        assert!(
            folded.contains(&format!("{} >", "甲乙丙丁戊己庚辛壬癸子丑寅卯辰")),
            "and a mark stands where it stopped:\n{folded}"
        );

        // `t w` — the whole cell, the way it does in prose.
        editor.on_key(Key::Char('t'));
        editor.on_key(Key::Char('w'));
        let whole = page(&editor);
        assert!(whole.contains(long), "every character of it:\n{whole}");
        assert!(!whole.contains('>'), "and nothing left to unfold:\n{whole}");

        // Folded again, and walked into: in Normal the grid does not move.
        // **A column that rewrote itself on every `l`** is what this costs
        // otherwise — the key means「next cell」and the whole table shifted
        // sideways to answer it.
        editor.on_key(Key::Char('t'));
        editor.on_key(Key::Char('w'));
        editor.on_key(Key::Char('l'));
        assert_eq!(editor.cell_position().map(|(_, c)| c), Some(1));
        let stood_in = page(&editor);
        assert!(
            !stood_in.contains("玄黃"),
            "standing in it does not open it:\n{stood_in}"
        );
        // The grid itself, not the two rows at the foot: the hint and the
        // position readout name the cell the caret is in, and it moved.
        let grid = |page: &str| page.lines().take(8).collect::<Vec<_>>().join("\n");
        assert_eq!(grid(&stood_in), grid(&folded), "and the grid is exactly as it was");

        // …and it is still readable, three ways. The panel down the right had
        // it whole all along, which is the one that costs the grid nothing.
        let frame = render(&editor, &config, 120, 10);
        let whole_page: String = (0..10).map(|y| row_text(&frame, y)).collect::<Vec<_>>().join("");
        assert!(
            long.chars().all(|c| whole_page.contains(c)),
            "the panel is showing it:\n{whole_page}"
        );
        // Typing in it opens it in place, the way it does in prose.
        editor.on_key(Key::Char('i'));
        let typing = page(&editor);
        assert!(typing.contains(long), "the cell being typed in is whole:\n{typing}");
    }

    /// `t a` — 格內折行 (2026-09-07: 「把所有超长的单元格都在单元格下方
    /// 的空行中 soft wrap」). The grid can do this and the prose page cannot:
    /// it owns its own layout, so a row is as tall as its tallest cell and the
    /// rows under it move down.
    #[test]
    fn t_a_wraps_a_long_cell_under_itself_and_moves_the_rows_below_down() {
        let long = "甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉戌亥天地玄黃";
        let mut editor = editor_with(&format!(
            "| 地名 | 備註 |\n| --- | --- |\n| 洛陽 | {long} |\n| 長安 | 短 |"
        ));
        for key in ['j', 'j', 't', 't'] {
            editor.on_key(Key::Char(key));
        }
        let config = Config::default();
        let page = |editor: &Editor| -> Vec<String> {
            let frame = render(editor, &config, 120, 12);
            (0..12)
                .map(|y| row_text(&frame, y).split('│').next().unwrap_or("").to_string())
                .collect()
        };

        // Folded: one row each, and the tail is off the grid behind a `>`.
        let folded = page(&editor);
        let row_of = |page: &[String], needle: &str| {
            page.iter()
                .position(|line| line.contains(needle))
                .unwrap_or_else(|| panic!("{needle} is on the page: {page:?}"))
        };
        assert_eq!(
            row_of(&folded, "長安"),
            row_of(&folded, "洛陽") + 1,
            "one row each:\n{}",
            folded.join("\n")
        );

        // `t a`: the tail is drawn underneath, in its own column.
        editor.on_key(Key::Char('t'));
        editor.on_key(Key::Char('a'));
        let wrapped = page(&editor);
        assert!(
            !wrapped.iter().any(|line| line.contains('>')),
            "nothing is cut any more:\n{}",
            wrapped.join("\n")
        );
        assert!(
            wrapped.iter().any(|line| line.contains("玄黃")),
            "every character of the cell is on the page:\n{}",
            wrapped.join("\n")
        );
        let below = row_of(&wrapped, "長安");
        assert!(
            below > row_of(&wrapped, "洛陽") + 1,
            "and the row under it has moved down:\n{}",
            wrapped.join("\n")
        );
        // The row is numbered once: the lines under it are the same row.
        let carried = &wrapped[row_of(&wrapped, "洛陽") + 1];
        assert!(
            carried.trim_start().starts_with(|c: char| !c.is_ascii_digit()),
            "no second row number on the wrapped line: {carried:?}"
        );
    }

    /// The other half of the same complaint: 「信息面板也没有换行功能来显示这个
    /// 单元格的全部信息」. The panel is what the grid's cap points *at*, so a
    /// panel that cuts the value at its own edge leaves the reader nowhere to
    /// go at all.
    #[test]
    fn the_detail_panel_wraps_a_value_too_long_for_its_width() {
        let long = "甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉戌亥天地玄黃";
        let mut editor = editor_with(&format!(
            "| 地名 | 備註 |\n| --- | --- |\n| 洛陽 | {long} |"
        ));
        for key in ['j', 'j', 't', 't'] {
            editor.on_key(Key::Char(key));
        }
        let config = Config::default();
        let frame = render(&editor, &config, 120, 12);
        let page: String = (0..12).map(|y| row_text(&frame, y)).collect::<Vec<_>>().join("\n");
        // The caret is in 地名, so the grid has cut 備註 — the last character
        // of it can only be on the page because the panel wrapped to it.
        assert!(page.contains('黃'), "the tail is readable in the panel:\n{page}");
    }

    /// #283. The ruler and the names used to stop one column before the rows
    /// did: they broke on 「does the whole column fit」 and the rows break on
    /// 「does this column start inside the window」. The 32-cell cap kept most
    /// columns well inside the window and hid it; `t w` makes a column wider
    /// than the room left over the ordinary case — and the column it beheaded
    /// was the one the reader had just asked to see whole.
    #[test]
    fn the_ruler_and_the_names_stop_where_the_rows_stop() {
        let long = "甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉戌亥天地玄黃宇宙洪荒日月盈昃";
        let mut editor = editor_with(&format!(
            "| 地名 | 小傳 |\n| --- | --- |\n| 洛陽 | {long} |"
        ));
        for key in ['j', 'j', 't', 't', 't', 'w'] {
            editor.on_key(Key::Char(key));
        }
        let config = Config::default();
        // The grid alone: the panel down the right names every field, so
        // reading the two together would let the panel answer for the ruler.
        let frame = render(&editor, &config, 96, 10);
        let grid: Vec<String> = (0..10)
            .map(|y| row_text(&frame, y).split('│').next().unwrap_or("").to_string())
            .collect();
        let heading = grid
            .iter()
            .position(|row| row.contains("地名"))
            .unwrap_or_else(|| panic!("a heading row:\n{}", grid.join("\n")));
        assert!(
            grid[heading].contains("小傳"),
            "the second column keeps its name:\n{}",
            grid.join("\n")
        );
        assert!(
            grid[heading - 1].contains('2'),
            "and its number:\n{}",
            grid.join("\n")
        );
    }

    /// The status line — **second from the bottom**, because the command row
    /// is below it (#302).
    fn status_line(buffer: &ratatui::buffer::Buffer) -> String {
        row_text(buffer, buffer.area.height - 2)
    }

    /// The command row: the bottom row of all, and the one a `:` is typed into.
    fn command_line(buffer: &ratatui::buffer::Buffer) -> String {
        row_text(buffer, buffer.area.height - 1)
    }

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

    /// Which screen row holds `needle`.
    ///
    /// **Not a number written down.** How many screen rows a file line is
    /// drawn at depends on what else the page puts above it — a reading, a row
    /// of air, and since #379 the strip that numbers a table's columns at 基本
    /// as well as at 全. Five tests said 「row 3 is the fourth line of the
    /// file」 and all five broke the day the strip moved a level, which is a
    /// test failing for something it was not about.
    fn row_holding(buffer: &ratatui::buffer::Buffer, needle: &str) -> u16 {
        (0..buffer.area.height)
            .find(|&y| row_text(buffer, y).contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} is on the page"))
    }

    /// Which **screen column** a row's text holds `needle` at.
    ///
    /// Not `str::find`. [`row_text`] walks the buffer by display width, so its
    /// byte offsets and the buffer's columns part company at the first 漢字 on
    /// the row — aiming an assertion by one lands it a cell or two to the
    /// right of the character it names, which is how `assert_ne!` on a closing
    /// pipe came to be inspecting the blank past the end of the row.
    fn column_of(text: &str, needle: &str) -> u16 {
        let at = text
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} is on the page: {text:?}"));
        text[..at].chars().map(|c| yumete_cjk::char_width(c) as u16).sum()
    }

    /// The same, for the **last** time `needle` appears.
    fn last_column_of(text: &str, needle: &str) -> u16 {
        let at = text
            .rfind(needle)
            .unwrap_or_else(|| panic!("{needle:?} is on the page: {text:?}"));
        text[..at].chars().map(|c| yumete_cjk::char_width(c) as u16).sum()
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
        editor.set_candidate(vec![(0, 2, "候補".to_string())]);
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

    /// Feature #248: a note is drawn, and drawn in its own ink.
    ///
    /// The candidate is quiet because it is *about to be* the text; a note
    /// never will be, so it takes the marker's colour — the rung the editor
    /// uses when it is pointing at something rather than saying it.
    #[test]
    fn a_note_is_drawn_in_the_ink_of_something_pointed_at() {
        let mut editor = editor_with("他說,好");
        editor.execute(":view-punct on").unwrap();
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let buffer = render_wrapped(&mut editor, &config, 20, 4);
        assert_eq!(row_text(&buffer, 0).trim_end(), "他說,，好");

        let column = column_of(&row_text(&buffer, 0), "，");
        let ink = ink(&config);
        assert_eq!(buffer[(column, 0)].fg, ink.marker(), "the note");
        assert_ne!(buffer[(0, 0)].fg, ink.marker(), "…but not 他");
    }

    /// The same page set vertically, where these pages are actually read.
    ///
    /// 縱書 draws every slot off the line's own style, so a note would be set
    /// in the manuscript's ink unless the ink is carried down into the grid —
    /// which is why [`yumete_core::drawn::Ink`] reaches `zong::Slot` at all.
    #[test]
    fn a_note_set_vertically_is_not_read_as_the_manuscript() {
        let mut editor = editor_with("他說,好");
        editor.execute(":view-punct on").unwrap();
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 30, 12);
        // Down the rightmost 縱: 他 說 ␣ ︐ 好 — the half-width comma is hung
        // in the margin (its slot is blank) and the note is rotated like any
        // other 句讀 the 縱 carries.
        let column: Vec<String> = (0..5).map(|y| at(&buffer, 28, y)).collect();
        assert_eq!(column, ["他", "說", " ", "︐", "好"]);
        let ink = ink(&config);
        assert_eq!(buffer[(28, 3)].fg, ink.marker(), "the note");
        assert_ne!(buffer[(28, 0)].fg, ink.marker(), "…but not 他");
    }

    #[test]
    fn the_caret_lands_past_the_candidate_it_typed() {
        let mut editor = editor_with("春夏秋冬");
        editor.set_candidate(vec![(0, 2, "候補".to_string())]);
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

    /// With `:view-wrap off` the page follows the caret off the right edge
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
        // On the `z`, which is 25. This read `26, "past the last character"`
        // until 2026-09-11 — the assertion was writing #382 down rather than
        // catching it.
        assert_eq!(editor.cursor(), 25, "on the last character");
        let (buffer, at) = render_caret(&editor, &config, 10, 4);
        assert_eq!(
            row_text(&buffer, 0).trim_end(),
            // Ten columns of writing, `q` through `z`. It was nine —
            // `rstuvwxyz` and a blank column — for as long as the caret stood
            // one past the `z` (#382); now it stands *on* the `z`, so the
            // window ends on a character instead of on nothing.
            "qrstuvwxyz",
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
        // The leftmost cell is `q`, the seventeenth character counting from
        // zero. It was `r` while `gl` left the caret one past the `z` and the
        // window therefore ended on a blank column (#382).
        assert_eq!(click(0), Some(16));
        assert_eq!(click(8), Some(24));
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

    /// A click on drawn text means the character it stands before.
    ///
    /// The cells are on the page but not in the file, so they are the one
    /// thing a click cannot land *in* — and the candidate's own cells belong
    /// to the character being typed, which is the character after them.
    #[test]
    fn a_click_on_a_candidate_means_the_character_it_stands_before() {
        let mut editor = editor_with("春夏秋冬");
        editor.set_candidate(vec![(0, 2, "候補".to_string())]);
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
        editor.set_candidate(vec![(0, 2, "候補".to_string())]);
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
        let mut config = vertical_config();
        // A one-cell gap, which the factory page no longer has (`:view-margin`, 2026-09-16).
        config.editor.zong_gap = 1;
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
        // configured 縱 is. The command row is off: this is about the 縱, and a
        // row spent on the footer would only move every coordinate below.
        let mut editor = editor_with(&"字".repeat(8));
        let mut config = vertical_config();
        config.editor.command_line = false;
        // A one-cell gap, which the factory page no longer has (`:view-margin`, 2026-09-16).
        config.editor.zong_gap = 1;
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

    /// 漢字 do not lean. The Chinese setting of `<em>` is a dot beside every
    /// character of the run, and 縱書 puts it in the margin (Feature #236).
    #[test]
    fn an_emphasised_word_is_dotted_down_the_margin_beside_it() {
        let mut editor = editor_with("一*二三*四");
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 20, 12);

        // The markers are still on the page (`:render basic` is the default), so
        // the 縱 reads 一 * 二 三 * 四 and the run is the two in the middle.
        let x = (0..20).find(|&x| at(&buffer, x, 0) == "一").unwrap();
        assert_eq!(at(&buffer, x, 2), "二");
        assert_eq!(at(&buffer, x, 3), "三");
        assert_eq!(at(&buffer, x + 2, 2), "·", "着重號 beside 二");
        assert_eq!(at(&buffer, x + 2, 3), "·", "着重號 beside 三");
        // …and beside nothing else, the markers included.
        for row in [0u16, 1, 4, 5] {
            assert_eq!(at(&buffer, x + 2, row), " ", "row {row} is not emphasised");
        }
    }

    /// `**` is a weight, not a 着重號; dotting both would put dots down half a
    /// page. A line with no emphasis on it pays nothing for the feature — the
    /// rightmost 縱 stays flush against the edge of the page.
    #[test]
    fn strong_is_a_weight_and_costs_the_page_no_margin() {
        let mut editor = editor_with("一**二三**四");
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 20, 12);
        let x = (0..20).find(|&x| at(&buffer, x, 0) == "一").unwrap();
        assert_eq!(x, 18, "flush right: nothing asked for a margin");
    }

    /// A 着重號 is bought the way a reading is: the layout pays a cell for the
    /// 縱 that needs one. So on the factory page — `:view-margin dense`, no gap,
    /// no 稿紙 rule — the dots are there, and they never land on the next 縱's
    /// writing.
    #[test]
    fn a_dense_margin_buys_the_cell_a_dot_needs() {
        let mut editor = editor_with("甲乙丙丁\n一*二三*四");
        editor.set_margin(yumete_cjk::Margin::Dense);
        let mut config = vertical_config();
        config.editor.zong_gap = 0;
        let buffer = render_vertical(&mut editor, &config, 20, 12);

        // The first paragraph has no emphasis and pays nothing: flush right.
        let first = (0..20).find(|&x| at(&buffer, x, 0) == "甲").unwrap();
        assert_eq!(first, 18);
        // The second does, so the page opens a cell between the two — the only
        // thing that ever moves a 縱 on a dense page is a margin something
        // needs.
        let second = (0..20).find(|&x| at(&buffer, x, 0) == "一").unwrap();
        assert_eq!(first - second, 3, "two cells of writing and one of margin");
        assert_eq!(at(&buffer, second + 2, 2), "·", "着重號 beside 二");
        assert_eq!(at(&buffer, second + 2, 3), "·", "着重號 beside 三");
        // …and the 縱 to its right is untouched.
        for (row, ch) in ["甲", "乙", "丙", "丁"].iter().enumerate() {
            assert_eq!(at(&buffer, first, row as u16), *ch, "row {row}");
        }
    }

    /// `:view-margin`'s four answers, down the page: one paragraph folded into
    /// two 縱 with a 着重號 **only in the first**, then a short paragraph.
    ///
    /// Where each 縱 lands is the whole answer — a 縱 that buys the lane is a
    /// cell wider, and everything to its left moves over.
    #[test]
    fn view_margin_decides_which_zong_buy_the_lane() {
        use yumete_cjk::Margin;
        let lefts = |margin: Margin| -> (Vec<u16>, bool) {
            let mut editor = editor_with("*甲*乙丙丁戊己庚辛\n末");
            editor.set_margin(margin);
            let mut config = vertical_config();
            config.editor.command_line = false;
            config.editor.zong_gap = 0;
            let buffer = render_vertical(&mut editor, &config, 20, 8);
            let find = |ch: &str, row: u16| (0..20).find(|&x| at(&buffer, x, row) == ch).unwrap();
            let dotted = (0..20).any(|x| (0..8).any(|y| at(&buffer, x, y) == "·"));
            (vec![find("甲", 1), find("戊", 0), find("末", 0)], dotted)
        };
        // Six to a 縱 here, and a half-width `*` is a square of its own:
        // 「*甲*乙丙丁」 then 「戊己庚辛」 — the emphasis is in the first alone.
        assert_eq!(lefts(Margin::Never), (vec![18, 16, 14], false), "nothing bought, no dots");
        assert_eq!(
            lefts(Margin::Dense),
            (vec![17, 15, 13], true),
            "only the first 縱 carries the dot, so only it is a cell wider"
        );
        assert_eq!(
            lefts(Margin::Loose),
            (vec![17, 14, 12], true),
            "the paragraph carries one, so both its 縱 buy the lane; 末 does not"
        );
        assert_eq!(lefts(Margin::Always), (vec![17, 14, 11], true), "every 縱");
    }

    /// The cell a 縱 blanks is the one **it** bought, not the one the widest
    /// 縱 on the page bought. A full-width reading covers the cell after it and
    /// has to clear it; a 着重號 in a one-cell margin must not, or it rubs out
    /// the 縱 to its right — which on this page is the one carrying the reading.
    #[test]
    fn a_dot_blanks_only_the_cell_its_own_zong_bought() {
        let mut editor = editor_with("<ruby>永<rt>ㄩㄥˇ</rt></ruby>和平\n一*二三*四");
        let config = vertical_config();
        let buffer = render_vertical_ruby(&mut editor, &config, 24, 12);

        let on_page = |c: &str| {
            (0..12)
                .flat_map(|y| (0..24).map(move |x| (x, y)))
                .any(|(x, y)| at(&buffer, x, y) == c)
        };
        for ch in ["永", "和", "平"] {
            assert!(on_page(ch), "{ch} was rubbed out by the dots beside it");
        }
        assert!(on_page("·"), "and the dots are drawn");
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
        let mut config = vertical_config();
        // A one-cell gap, which the factory page no longer has (`:view-margin`, 2026-09-16).
        config.editor.zong_gap = 1;
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
        // A one-cell gap, which the factory page no longer has (`:view-margin`, 2026-09-16).
        config.editor.zong_gap = 1;
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
        // The bundled word list is a build input, not a tracked file
        // (`yumete-cjk/build.rs`), so a machine that has never installed 宇浩
        // has none and every word is one 漢字 — which this picture is not of.
        if !DictionarySegmenter::has_builtin() {
            return;
        }
        let mut editor = editor_with("你好世界");
        editor.set_segmenter(Box::new(DictionarySegmenter::builtin(0)));
        editor.set_segmentation_visible(true);
        let mut config = vertical_config();
        config.editor.show_segmentation = true;
        // 出廠是 ink 了（#456）；這一條驗的是 tint，所以明說。
        editor.set_word_mark(yumete_cjk::WordMark::Tint);
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "ajvy 奧\n");
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧\n");
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
            Scheme::LINGMING,
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
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

    /// **A click does not carry a half-typed word into another file** (#336).
    ///
    /// `Event::Mouse` and `Event::Paste` are siblings of `Event::Key` and never
    /// asked whether the IME was mid-word. Type half a 拆分, click another tab,
    /// and the preedit was still alive — redrawn at the caret of the **new**
    /// file, where the next space committed it. A few characters of somebody
    /// else's chapter, and nothing said so.
    #[test]
    fn ending_a_composition_leaves_nothing_behind_to_land_elsewhere() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
        ime.input('b');
        assert!(ime.is_composing(), "the fixture composes");

        end_the_composition(&mut ime, &mut editor);
        assert!(!ime.is_composing(), "nothing is left in flight");
        // Discarded, not committed: half a code has not chosen a character, and
        // guessing one would be putting a word in the writer's mouth.
        assert_eq!(editor.current_buffer().text(), "", "and nothing was typed");
        assert_eq!(ime.display_buffer(), "", "no preedit to redraw anywhere");

        // Idempotent, because the guard runs on every click and most clicks
        // arrive with nothing in flight.
        end_the_composition(&mut ime, &mut editor);
        assert_eq!(editor.current_buffer().text(), "");
    }

    /// **Whole fields give way; the line is never cut through a word** (#394).
    ///
    /// The left half used to be one `format!` with the comment 「the left side
    /// is never squeezed」 — so a narrow window simply sliced the end off it,
    /// and the first thing lost was the position, the one thing the status
    /// line exists to answer.
    #[test]
    fn a_narrow_status_line_drops_fields_rather_than_cutting_one() {
        let mut editor = editor_with("第一段。\n");
        editor.current_buffer_mut().name_as("long.csv");
        let config = Config::default();
        let status = |w: u16| {
            let buffer = render(&editor, &config, w, 6);
            status_line(&buffer).trim_end().to_string()
        };

        // Wide: everything, and the three-space gaps.
        let wide = status(70);
        assert!(wide.contains("long.csv"), "{wide:?}");
        assert!(wide.contains("行 1, 列 1"), "{wide:?}");

        // Narrow: **the position outlives the file name**, and what is left is
        // whole — no `Ln 1, C`, no `long.c`.
        let narrow = status(28);
        assert!(narrow.contains("行 1, 列 1"), "the position stays: {narrow:?}");
        assert!(!narrow.contains("long.csv"), "the name gave way: {narrow:?}");
        assert!(narrow.contains("NORMAL"), "{narrow:?}");
        assert!(
            yumete_cjk::str_width(&narrow) <= 28,
            "and it fits: {narrow:?}"
        );
    }

    /// **The panel stops above the status line** (#387).
    ///
    /// It was sized and placed against `frame.area()` — the whole window — so
    /// a full page of candidates on a twelve-row terminal ran over the last
    /// row and took the mode, the file name and the position with it. On a
    /// tall terminal it never reached the bottom, which is why nobody saw it.
    /// The command row below it is out of bounds too, and for a second reason:
    /// while a `:` or `/` is open the caret the panel hangs from is standing
    /// in that row, so the placement has to be clamped and not merely flipped.
    #[test]
    fn the_candidate_panel_never_covers_the_status_line() {
        let table = "b 吧 八 把 爸 罷 壩 霸 拔 跋\n";
        for height in [8u16, 10, 12, 16, 30] {
            let mut editor = Editor::new();
            editor.on_key(Key::Char('i'));
            let mut ime = ImeSession::from_table_text(Scheme::LINGMING, table);
            ime.input('b');
            let buffer = render_with(&mut editor, &Config::default(), &ime, 40, height);

            // **The panel is actually on the page.** Without this the test
            // would pass on a build that simply stopped drawing it.
            let page = buffer_text(&buffer);
            assert!(
                page.contains('╰') && page.contains('吧'),
                "{height} rows: no panel to speak of: {page:?}"
            );

            // The status line still reads as one: the mode is on it, and no
            // border is — and the command row under it is clear as well.
            let last = status_line(&buffer);
            assert!(
                last.contains("NORMAL") || last.contains("INSERT"),
                "{height} rows: the status line is gone: {last:?}"
            );
            let under = command_line(&buffer);
            for edge in ['╰', '╯', '╭', '╮', '│'] {
                assert!(
                    !last.contains(edge),
                    "{height} rows: the panel is on the status line: {last:?}"
                );
                assert!(
                    !under.contains(edge),
                    "{height} rows: the panel is on the command row: {under:?}"
                );
            }
        }
    }

    /// 快捷符號 is not a candidate list, and the panel must not say it is.
    ///
    /// 原話：「yume 的分号默认是快捷符号，他不是一个候选面板而是一个特殊面板，
    /// 是按字母、空格、分号等按键上屏的。」 Drawn through the ordinary candidate
    /// path it came out as `㊀ ：「 ㊁ ～ …` — nine of twenty-seven, numbered,
    /// and every number a key that commits **something else**. The labels have
    /// to be the keys, and the whole table has to be on the panel.
    #[test]
    fn the_shortcut_panel_is_labelled_with_keys_not_numbers() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
        ime.input(';');
        assert!(ime.is_shortcut(), "分號開的是快捷符號");

        let config = Config::default();
        let buffer = render_with(&editor, &config, &ime, 60, 20);
        let page = buffer_text(&buffer);
        assert!(page.contains('╰'), "面板沒畫出來：{page:?}");
        // The letters people actually press, with what they commit.
        assert!(page.contains("j 、"), "j 那一格：{page:?}");
        assert!(page.contains("n 《"), "n 那一格：{page:?}");
        // …and the fixed keys, which are the far end of the same table.
        assert!(page.contains("$ ￥"), "固定鍵也要在：{page:?}");
        // 全角空格畫成 ␣——空格子與「這個字母沒指派」得分得開。
        assert!(page.contains("o ␣"), "看不見的符號要有記號：{page:?}");
        // Nothing numbered: the markers are for 選重, and there is none here.
        for mark in config.panel.markers.chars() {
            assert!(!page.contains(mark), "快捷符號不該有 {mark:?}：{page:?}");
        }

        // **The panel comes up even under `bare`**, which has nothing to draw
        // inline: 「一個分號」 previews nothing, and the letters are only here.
        let mut bare = ime;
        // Out of the mode first: a second `;` *inside* it commits 「；」, which
        // is the `;;` exit and would leave nothing to draw.
        bare.escape();
        bare.set_panel_display(PanelDisplay::Bare);
        bare.input(';');
        assert!(bare.panel_is_full(), "bare 也要出這個面板");
    }

    /// 快捷符號 mode is the engine's, key by key — **驅動鍵盤**，不是讀那張表。
    ///
    /// 原話：「我按 `;;` 上屏的是第二個（b 對應的波浪號），而不是分號。」
    /// 那是把 `;` 交給 `key_action` 的下場：緩衝裏有引導鍵、表算作候選，狀態於是
    /// 讀成「組字中，有候選」，而靈明在那個狀態下的 `;` 是**選二**。
    ///
    /// ⚠️ **上一輪的兩條測試看不出這個。** 它們問的是面板畫成什麼樣、表怎麼排
    /// ——都對，而按鍵走的是另一條路。要驗這一族只能真按鍵。
    #[test]
    fn the_shortcut_mode_takes_every_key_from_the_engine() {
        let press = |ime: &mut ImeSession, editor: &mut Editor, code: KeyCode| {
            super::ime_handle(ime, editor, code, KeyModifiers::NONE);
        };
        let open = || {
            let mut editor = Editor::new();
            editor.on_key(Key::Char('i'));
            let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八 巴\n");
            ime.input(';');
            assert!(ime.is_shortcut());
            (editor, ime)
        };

        // `;;` ＝ 「；」，不是第二條。
        let (mut editor, mut ime) = open();
        press(&mut ime, &mut editor, KeyCode::Char(';'));
        assert_eq!(editor.current_buffer().text(), "；", "{}", editor.status());
        assert!(!ime.is_shortcut(), "上了屏就該關掉");

        // 空格同一個出口。
        let (mut editor, mut ime) = open();
        press(&mut ime, &mut editor, KeyCode::Char(' '));
        assert_eq!(editor.current_buffer().text(), "；");

        // `'` 在靈明是選三，而這個模式裏它不是——它不在表上，所以退出模式，
        // 那一鍵當作剛按下的新鍵重走一遍。無論如何**不能**上屏第三條。
        let (mut editor, mut ime) = open();
        press(&mut ime, &mut editor, KeyCode::Char('\''));
        let text = editor.current_buffer().text();
        assert!(!text.contains('！'), "選三跑出來了：{text:?}");
        assert!(!ime.is_shortcut());

        // 字母走表：`j` ＝ 、
        let (mut editor, mut ime) = open();
        press(&mut ime, &mut editor, KeyCode::Char('j'));
        assert_eq!(editor.current_buffer().text(), "、");

        // 固定鍵：`$` ＝ ￥
        let (mut editor, mut ime) = open();
        press(&mut ime, &mut editor, KeyCode::Char('$'));
        assert_eq!(editor.current_buffer().text(), "￥");

        // 沒有可導航的東西，方向鍵一律吞掉——漏到編輯器就是光標跑了。
        let (mut editor, mut ime) = open();
        for code in [KeyCode::Down, KeyCode::Tab, KeyCode::PageDown] {
            press(&mut ime, &mut editor, code);
        }
        assert!(ime.is_shortcut(), "面板不該被方向鍵關掉");
        assert_eq!(editor.current_buffer().text(), "", "有東西漏進了正文");

        // Esc 收面板，什麼都不上屏。
        let (mut editor, mut ime) = open();
        press(&mut ime, &mut editor, KeyCode::Esc);
        assert!(!ime.is_shortcut());
        assert_eq!(editor.current_buffer().text(), "");
    }

    /// 二十七格排成格子，鍵對齊成列。
    #[test]
    fn the_shortcut_table_is_laid_out_in_columns() {
        let rows: Vec<(String, String)> = [("a", "：「"), ("b", "～"), ("c", "！")]
            .iter()
            .map(|(k, t)| (k.to_string(), t.to_string()))
            .collect();
        // 最寬的一格是 `a ：「` ＝ 1 + 1 + 4 ＝ 6 欄，加兩欄間隔。
        let wide = super::shortcut_lines(&rows, 40);
        assert_eq!(wide, vec!["a ：「  b ～    c ！".to_string()]);
        // 窄到只放得下一格時，一行一格——而不是除以零。
        let narrow = super::shortcut_lines(&rows, 1);
        assert_eq!(narrow.len(), 3, "{narrow:?}");
        assert!(super::shortcut_lines(&[], 40).is_empty());
    }

    #[test]
    fn the_horizontal_panel_takes_the_same_markers_and_border() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八 巴 芭 疤\n");
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八 巴 芭 疤\n");
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
        // `:yume-commit` with nothing after it is the question (Feature #209).
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "a 啊\n");
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
        // 出廠是 `full` since 2026-09-08, and this asks about the candidate.
        editor.set_hud(Hud::Basic);
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八 巴
");
        ime.set_panel_display(PanelDisplay::Bare);
        ime.input('b');
        settle_inline_candidate(&mut editor, &ime);

        // The first candidate is on the page, at the caret, though the file
        // holds not one byte of it.
        assert_eq!(editor.current_buffer().text(), "");
        assert_eq!(editor.drawn_on_line(0), vec![(0, "吧".to_string())]);

        let config = Config::default();
        let buffer = render_with(&editor, &config, &ime, 40, 8);
        let rows: Vec<String> = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect()
            })
            .collect();
        // The gutter, the candidate — and the code two cells on, which is
        // where #269 put it: with the sentence empty, the nearest margin to
        // the caret is the one on the caret's own row. It used to be at the
        // far right of the row *below*, half a screen away, which is what
        // the reported screenshot was of.
        assert_eq!(rows[0].trim_end(), "1  吧   ─ b", "{:?}", rows[0]);
        // No panel: the second and third candidates are nowhere on the screen.
        assert!(
            !rows.iter().any(|r| r.contains('八') || r.contains('巴')),
            "{rows:#?}"
        );
        // …and the code has somewhere to be — beside the caret, and on the
        // status line's right edge.
        assert_eq!(hud_line(&editor, &ime), "b");
        assert!(rows.iter().any(|r| r.contains("─ b")), "{rows:#?}");
    }

    /// #213: a locked buffer says so standing, not once.
    #[test]
    fn a_locked_buffer_wears_it_on_the_status_line() {
        let mut editor = Editor::new();
        editor.current_buffer_mut().insert(0, "讀一讀\n").expect("the fixture buffer is writable");
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八 巴
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
        assert!(!editor.has_candidate());

        // The word lands, and the panel goes with it.
        ime.space();
        editor.insert_committed(&ime.take_committed());
        assert_eq!(editor.current_buffer().text(), "吧");
        assert!(!ime.panel_is_full());
        ime.input('b');
        assert!(!ime.panel_is_full(), "a new word starts空空如也 again");
    }

    /// #215: what the 字典 panel looks like on the page.
    ///
    /// The rows are the editor's, but only a drawn frame says whether the
    /// names line up — the padding is counted in columns, and every one of
    /// these names is CJK.
    #[test]
    fn the_dictionary_panel_lines_its_columns_up() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        for c in "那年冬天".chars() {
            editor.on_key(Key::Char(c));
        }
        editor.on_key(Key::Esc);
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('g'));
        editor.look_up('那', true);
        editor.take_dictionary_query();
        editor.set_dictionary('那', vec![
            ("拆分".to_string(), "刀二阝".to_string()),
            ("分節編碼".to_string(), "vf-b".to_string()),
        ]);

        let ime = ImeSession::from_table_text(Scheme::LINGMING, "a 啊\n");
        let buffer = render_with(&editor, &Config::default(), &ime, 60, 8);
        // The panel is the bottom layer of the right slot now (#293), so what
        // is being read here is the part past the rule down its left edge.
        let panel = |y: u16| match row_text(&buffer, y).split_once('│') {
            Some((_, panel)) => panel.to_string(),
            None => String::new(),
        };
        let head = panel(0);
        assert!(head.contains("字典"), "{head}");
        let rows: Vec<String> = (1..4).map(panel).collect();
        assert!(rows[0].starts_with(" 那"), "{:?}", rows[0]);
        // 拆分 is four columns and 分節編碼 is eight, so the shorter name is
        // padded by four — and the two values start in the same column.
        // In **columns**, which is the only measure a terminal lines things up
        // in: 拆分 is two characters and four columns.
        let at = |row: &str, what: &str| row.find(what).map(|b| yumete_cjk::str_width(&row[..b]));
        assert_eq!(at(&rows[1], "刀二阝"), at(&rows[2], "vf-b"), "{rows:?}");
    }

    /// #215: with the list up, `Tab` asks the 字典 about the highlighted one.
    ///
    /// Under `bare` that is the second press — the first brings the list up,
    /// and you cannot ask about a candidate you cannot see. Under `full` the
    /// list is always up, so it is the first.
    #[test]
    fn tab_on_a_candidate_asks_the_dictionary_about_it() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八 巴\n");
        ime.set_panel_display(PanelDisplay::Bare);
        ime.input('b');

        ime_handle(&mut ime, &mut editor, KeyCode::Tab, KeyModifiers::NONE);
        assert!(ime.panel_is_full(), "the first press is still the panel");
        assert_eq!(editor.take_dictionary_query(), None, "…and only the panel");

        ime_handle(&mut ime, &mut editor, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(editor.take_dictionary_query(), Some('吧'));
        assert_eq!(
            editor.transient(Side::Right),
            Some(yumete_core::sidebar::Transient::Dictionary)
        );
        // The keys stay with the word: the reader is mid-composition, and the
        // panel is only there to be glanced at.
        assert!(!editor.sidebar_focused());
        assert!(ime.is_composing(), "…and the word is still being written");

        // Under `full` there is nothing to summon, so one press does it.
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八 巴\n");
        ime.input('b');
        ime_handle(&mut ime, &mut editor, KeyCode::Tab, KeyModifiers::NONE);
        assert_eq!(editor.take_dictionary_query(), Some('吧'));
    }

    /// #211: the setting, the command, and the question, in the writer's words.
    #[test]
    fn the_panel_says_which_way_it_is_drawing() {
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "a 啊
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八 巴
");
        ime.set_panel_display(PanelDisplay::Bare);
        ime.input('b');
        settle_inline_candidate(&mut editor, &ime);
        assert!(!editor.has_candidate(), "nothing goes into the manuscript");
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
        ime.input('b');
        let config = vertical_config();
        let buffer = render_vertical_with(&mut editor, &config, &ime, 40, 14);

        // 墨香 dark: warm ink on a deep ground, ringed in a mid rung of the same
        // ladder. Nothing in the panel falls back to the terminal default.
        let paper = Color::Rgb(0x26, 0x2A, 0x27);
        // rung::RULE — 第 50 檔 exactly, since the ladder was renumbered to
        // one hundred and one stops of a hundred steps each (#451).
        let ring = Color::Rgb(0x71, 0x6F, 0x61);
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "dydn 靈 聯絡員
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
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

        // 出廠是 ink 了（#456）；這一條驗的是 tint，所以明說。
        editor.set_word_mark(yumete_cjk::WordMark::Tint);
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
    fn the_other_mark_moves_the_writing_and_leaves_the_paper_alone() {
        // 字色 (#278): the same alternation, said with ink instead of ground —
        // for a page that is tinted for something else already, and for a
        // reader to whom a band under the writing is heavier than the writing.
        let mut editor = Editor::new();
        editor.set_segmenter(Box::new(DictionarySegmenter::builtin(0)));
        editor.set_segmentation_visible(true);
        editor.on_key(Key::Char('i'));
        for c in "你好世界".chars() {
            editor.on_key(Key::Char(c));
        }
        editor.on_key(Key::Esc);
        assert!(editor.execute("word-show 字色").is_ok());
        assert_eq!(editor.word_mark(), yumete_cjk::WordMark::Ink);

        let config = Config::default();
        let buffer = render(&editor, &config, 40, 6);
        let quiet = ink(&config).word_ink();
        let tint = ink(&config).word();
        let mut marked = 0;
        let mut tinted = 0;
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                if buffer[(x, y)].style().fg == Some(quiet) {
                    marked += 1;
                }
                if buffer[(x, y)].style().bg == Some(tint) {
                    tinted += 1;
                }
            }
        }
        // 你好 is the first word. Two cells, not four: a 漢字 is two columns
        // wide and the buffer carries its style on the leading one.
        assert_eq!(marked, 2, "the first word is not in the second ink");
        assert_eq!(tinted, 0, "the paper was painted anyway");
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

    /// A control character in the file is a control character on the page
    /// (#398).
    ///
    /// A NUL handed straight to the terminal draws nothing, so the cell it was
    /// charged for came up blank and the file looked like a file with nothing
    /// there — the one failure a reader cannot report, because there is
    /// nothing to point at. It gets its Control Picture instead, on both
    /// pages, and the byte is untouched.
    #[test]
    fn a_control_character_is_drawn_as_its_picture() {
        let path = std::env::temp_dir().join(format!("yumete-nul-{}.txt", std::process::id()));
        std::fs::write(&path, "a\u{0}b\u{1}\u{7f}\n").unwrap();
        let mut editor = Editor::new();
        editor.open_file(&path).unwrap();

        let page = page_text(&render_with(&editor, &Config::default(), &no_ime(), 30, 8));
        assert!(page.contains("a␀b␁␡"), "{page:?}");

        // …and on the 縱書 page, where the same character had the same nothing.
        let config = vertical_config();
        let down = buffer_text(&render_vertical_with(&mut editor, &config, &no_ime(), 30, 12));
        assert!(down.contains('␀'), "{down:?}");

        // The buffer is unchanged: this is a face, not an edit.
        assert_eq!(editor.current_buffer().text(), "a\u{0}b\u{1}\u{7f}\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn candidate_panel_shows_while_composing() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i')); // Insert mode
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
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
        // Wide enough for all three fields **and** the readout — narrower, 字
        // gives way to it on purpose (#500).
        let buffer = render_vertical(&mut editor, &config, 80, 12);

        // A wide glyph leaves its continuation cell blank in the test backend,
        // so the run of spaces after 橫 is an artefact of reading the grid.
        let raw: String = (0..buffer.area.width)
            .map(|x| buffer[(x, buffer.area.height - 2)].symbol())
            .collect();
        let status = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        // **縱橫字 ＝ 行列字 轉了個身** (#500). 「竖排的纵＝横排的行，竖排的横＝
        // 横排的列」 — so 縱 is the absolute line, 橫 is how far along it, and
        // 字 is how far along in characters.
        assert!(status.contains("縱 1"), "which line: {status:?}");
        assert!(status.contains("橫 2"), "how far down it: {status:?}");
        assert!(status.contains("字 2"), "and in characters: {status:?}");
        assert!(!status.contains("Ln"), "no ambiguous line number");

        // ⚠️ **A wrapped paragraph keeps one 縱 number**, exactly as a wrapped
        // line on a horizontal page keeps one 行 number — 「一段折成几列时，几列
        // 同一个纵号」. It used to be written `2-2`, naming the piece; the piece
        // is a *visual* fact and this coordinate is not.
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
            .map(|x| buffer[(x, buffer.area.height - 2)].symbol())
            .collect();
        let status = raw.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(status.contains("縱 2,"), "still the one line: {status:?}");
        assert!(!status.contains("2-2"), "and no piece number: {status:?}");
        assert!(status.contains("橫 13"), "thirteen slots down: {status:?}");
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

    /// 2026-09-05: `:view-wrap 50` then `:view-wrap off` left a rule down
    /// the middle of the page with the writing running straight through it.
    ///
    /// The measure is kept on purpose — `:view-wrap` on its own has to be able to
    /// put it back — but a measure nothing is folded at is not a margin, and
    /// the furniture that says 「the paper ends here」 was still being drawn.
    #[test]
    fn a_measure_wrapping_stopped_at_rules_nothing() {
        let mut editor = editor_with(&"字".repeat(30));
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        // Nothing configured: every mark on this page comes from `:view-wrap 20`.
        assert_eq!(config.editor.ruler, 0);
        let tint = Some(ink(&config).at(yumete_config::rung::BAND));

        editor.execute("view-wrap 20").unwrap();
        let buffer = render_wrapped(&mut editor, &config, 40, 8);
        assert_eq!(buffer[(20, 0)].style().bg, tint, "the measure is in force");

        // `:view-wrap off`: the rows are no longer folded at twenty, so nothing on
        // the page may claim they are.
        editor.execute("view-wrap off").unwrap();
        let buffer = render_wrapped(&mut editor, &config, 40, 8);
        assert_ne!(buffer[(20, 0)].style().bg, tint, "no margin past it");
        assert!(
            (0..8).all(|y| at(&buffer, 20, y) != "│"),
            "and no rule at it"
        );

        // …and `:view-wrap` alone puts the measure — and its margin — back.
        editor.execute("view-wrap").unwrap();
        let buffer = render_wrapped(&mut editor, &config, 40, 8);
        assert_eq!(buffer[(20, 0)].style().bg, tint, "the number was kept");
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

        editor.execute("view-wrap 20").unwrap();
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
        editor.execute("view-wrap 0").unwrap();
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
        editor.on_key(Key::Char('T')); // #356: 這一條測的是格
        editor.on_key(Key::Char('l'));

        let caret = |e: &Editor, c: &Config| render_caret(e, c, 60, 10).1.unwrap().x;
        let at_start = caret(&editor, &config);

        // Reading by character, the caret has to move with the cursor — pinned
        // to the cell's first 字 it would say the cursor had not moved at all.
        editor.on_key(Key::Char('T'));
        editor.on_key(Key::Char('l'));
        let one = caret(&editor, &config);
        assert_eq!(one, at_start + 2, "one 漢字 further along the cell");
        editor.on_key(Key::Char('l'));
        assert_eq!(caret(&editor, &config), at_start + 4);
        editor.on_key(Key::Char('h'));
        assert_eq!(caret(&editor, &config), one, "and back");

        // The same in Insert, where it decides where the next 字 lands.
        editor.on_key(Key::Char('T'));
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
        editor.on_key(Key::Char('T')); // #356: 這一條測的是格
        editor.on_key(Key::Char('l'));
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        let buffer = render(&editor, &config, 60, 10);
        // ⚠️ **The box is a bracket now, not a ground** (#407, 2026-09-12).
        // The cell used to be filled with `ink.selection()`, which left a
        // selection *inside* it no colour to be drawn in; and the row it is on
        // is already lit with `HEAD`, so the cell cannot be told from its row
        // by a ground either. So the box moved into the seam: two 金 rules in
        // the gap the page keeps between columns. It is still a box — the
        // brackets stand at the column's edges, past the end of the text — and
        // the ground is now free for the selection alone.
        let gold = ink(&config).gold();
        let bracket: Vec<u16> = (0..60u16)
            .filter(|&x| buffer[(x, 2)].style().fg == Some(gold))
            .collect();
        assert_eq!(bracket.len(), 2, "one on each side: {bracket:?}");
        let (left, right) = (bracket[0], bracket[1]);
        // This grid draws its seams (`:table-rules line`), and a rule already
        // there is only recoloured — the glyph does not change, so the grid
        // reads exactly as it did. A blank seam gets a mark of its own instead.
        assert_eq!(at(&buffer, left, 2), "\u{2506}", "the seam before it, in 金");
        assert_eq!(at(&buffer, right, 2), "\u{2506}", "and the one after");
        assert_eq!(at(&buffer, left + 1, 2), "\u{2ff0}", "it is the 拆分 cell");
        // The brackets stand at the *column's* edges, not the text's — a cell
        // you are inside, not three highlighted characters. Counted, not
        // run-length: a wide glyph covers two terminal cells.
        assert!(
            right > left + 5,
            "the box is the column's width, not the text's: {left}..{right}"
        );
        let text_ends =
            (left..60).find(|&x| at(&buffer, x, 2) == " " && at(&buffer, x - 1, 2) == " ");
        assert!(text_ends.is_some_and(|e| right >= e), "the padding is inside it");
        // …and no other **data** row is bracketed. The frozen rows above are
        // a different matter: the column's number and its heading are lit in
        // the same 金 up there, which is the second and third place the grid
        // says which cell you are in.
        // Only as far as the grid: the detail pane to the right of it names
        // the cell's own fields in the same 金, which is its job.
        let elsewhere: Vec<(u16, String)> = (0..=right)
            .filter(|&x| buffer[(x, 3)].style().fg == Some(gold))
            .map(|x| (x, at(&buffer, x, 3)))
            .collect();
        assert!(elsewhere.is_empty(), "not another row: {elsewhere:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    // ---- The cell you are standing on (#229) ------------------------------

    /// A `|` table is drawn as part of its page, so until #229 the only sign
    /// that Insert was confined to a cell was the column name in the status
    /// line. The cell now carries a ground — **its whole box**, padding
    /// included, because the padding is the column's width and an empty cell
    /// has no content to tint.
    #[test]
    fn a_pipe_tables_cell_carries_a_ground_too() {
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let ink = ink(&config);
        let mut editor = Editor::new();
        editor
            .current_buffer_mut()
            .insert(0, "前文\n| 字 | 讀音 |\n| --- | --- |\n| 木 | mu |\n")
                .expect("the fixture buffer is writable");
        // Onto the `木` row. By line number, not by counting `j`: which rows a
        // `j` steps over depends on the grain (#356), and this test is about
        // how a cell is drawn.
        editor.execute("4").unwrap();
        // 表格操作 (#275): the pipes stay on the page, which is what the
        // assertions below read. `t t` re-glyphs them into a grid instead.
        assert!(
            editor.enter_table_as(false),
            "{}",
            editor.status()
        );
        editor.on_key(Key::Char('T')); // #356: 這一條測的是格
        // …and into the second cell, which is `mu`.
        editor.on_key(Key::Char('l'));
        let (line, at) = editor.cell_position().expect("in a cell");
        assert_eq!(at, 1, "the second cell");
        let buf = render(&editor, &config, 30, 8);
        let want = ink.at(yumete_config::rung::HEAD);
        // By the row's own text, not by a number: the pane on the right
        // lists the cells, so  appears twice on the page.
        let row = row_holding(&buf, "| 木");
        let text = row_text(&buf, row);
        let mu = column_of(&text, "mu");
        for x in mu..mu + 2 {
            assert_eq!(buf[(x, row)].bg, want, "column {x} of {text:?} is in the cell");
        }
        // The pipe that closes the cell is not in it.
        let bar = last_column_of(&text, "|");
        assert_ne!(buf[(bar, row)].bg, want, "the pipe is furniture");
        // And the cell on the same row that the cursor is *not* in stays plain.
        let mu_cell = editor.cell_box(line, at).expect("a box");
        let other = editor.cell_box(line, 0).expect("a box");
        assert!(other.1 <= mu_cell.0, "the first cell ends before the second");
        let wood = column_of(&text, "木");
        assert_ne!(buf[(wood, row)].bg, want, "the other cell is not tinted");
    }

    /// The cell ground survives a `::: danger`, and still gives way to a
    /// `==highlight==`.
    ///
    /// The guard was written as「the ground here is already `wash`」, and three
    /// things paint `wash`: a highlight, a danger callout, and the search hit.
    /// A `|` table inside a callout therefore had every character already
    /// washed and lost the ground entirely — table mode drew nothing at all.
    #[test]
    fn the_cell_ground_survives_a_callout_but_not_a_highlight() {
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let ink = ink(&config);
        let want = ink.at(yumete_config::rung::HEAD);
        let mut editor = Editor::new();
        editor.current_buffer_mut().insert(
            0,
            "::: danger\n| 字頭 | 讀音 |\n| --- | --- |\n| 木頭 | ==mu== |\n:::\n",
        ).expect("the fixture buffer is writable");
        // **The line, said out loud.** Since #283 the level builds the view
        // on its own, so `j` on a table row is a step down the *column*; this
        // setup means a line of the file.
        editor.on_key(Key::Char(':'));
        editor.on_key(Key::Char('4'));
        editor.on_key(Key::Enter);
        assert!(editor.enter_table(), "{}", editor.status());
        // **The page, and only the page.** The detail panel would show the
        // same row down the right, and since 2026-09-07 it *wraps* a value too
        // long for its width — so 「==mu==」 would land there in pieces, one of
        // which is on the rule row, and a search for `mu` across the whole
        // window would answer with a row of `┄` rather than with the cell.
        // Since #495 it is shut unless asked for, which is what this wants.
        assert!(!editor.detail_visible(), "{}", editor.status());
        let row_with = |buf: &ratatui::buffer::Buffer, needle: &str| {
            (0..8u16)
                .find(|&y| row_text(buf, y).contains(needle))
                .unwrap_or_else(|| {
                    panic!("{needle:?}: {:?}", (0..8).map(|y| row_text(buf, y)).collect::<Vec<_>>())
                })
        };
        // Inside the callout, and the ground is there. The character *beside*
        // the cursor, because the block cursor paints its own slot last.
        let buf = render(&editor, &config, 30, 8);
        let y = row_with(&buf, "木頭");
        let text = row_text(&buf, y);
        assert_eq!(buf[(column_of(&text, "頭"), y)].bg, want, "{text:?}");

        // Now into the highlighted cell. A highlighter's ground is the answer
        // to「this is marked」and the cell must not rub it out.
        editor.on_key(Key::Char('l'));
        let buf = render(&editor, &config, 30, 8);
        let y = row_with(&buf, "mu");
        let text = row_text(&buf, y);
        let at = column_of(&text, "mu");
        assert_eq!(buf[(at, y)].bg, ink.wash(), "{text:?}");
    }

    /// A ground that says「an edit lands here」has to be told the truth about
    /// where the cursor is. `:table` staying on is not that truth: `gg`, `G`,
    /// `:N` and a search all walk out of the table without putting it away,
    /// and the rule row is the drawing of the alignments rather than a row
    /// anything can be typed into.
    #[test]
    fn no_cell_is_drawn_off_the_rows() {
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let ink = ink(&config);
        let want = ink.at(yumete_config::rung::HEAD);
        let mut editor = Editor::new();
        editor
            .current_buffer_mut()
            // **The prose line holds a `|`.** With a pipe-free line under the
            // table `mdtable::boxes` returns nothing and the old renderer drew
            // no ground there either — the assertion below passed on the very
            // bug it was written to catch. A line that has a pipe but does not
            // open with one is a paragraph, and it used to be given a cell.
            .insert(0, "| 字 | 讀音 |\n| --- | --- |\n| 木 | mu |\n見上表 | 附註")
                .expect("the fixture buffer is writable");
        // **The line, said out loud.** Since #283 the level builds the view
        // on its own, so `j` on a table row is a step down the *column*; this
        // setup means a line of the file.
        editor.on_key(Key::Char(':'));
        editor.on_key(Key::Char('3'));
        editor.on_key(Key::Enter);
        // 表格操作 (#275): the pipes stay on the page, which is what the
        // assertions below read. `t t` re-glyphs them into a grid instead.
        assert!(
            editor.enter_table_as(false),
            "{}",
            editor.status()
        );
        // Found, not counted: `G` scrolls the header off, so a hard-coded
        // screen row would be asserting about a different line of the file
        // after the motion than before it.
        let row_with = |buf: &ratatui::buffer::Buffer, needle: &str| {
            (0..8u16)
                .find(|&y| row_text(buf, y).contains(needle))
                .unwrap_or_else(|| {
                    panic!("{needle:?}: {:?}", (0..8).map(|y| row_text(buf, y)).collect::<Vec<_>>())
                })
        };
        // Standing in the table, the ground is there — otherwise the two
        // assertions below would pass on a renderer that never draws it.
        let buf = render(&editor, &config, 30, 8);
        // With its pipe: the pane lists the cells, so a bare `木` is on the
        // page twice and the first one is not in the table.
        let y = row_with(&buf, "| 木");
        let text = row_text(&buf, y);
        assert_eq!(buf[(column_of(&text, "木"), y)].bg, want, "{text:?}");

        // Out of the table altogether. `G` is not a table motion, so this is
        // the ordinary prose under it — no cell of anything.
        editor.on_key(Key::Char('G'));
        assert_eq!(editor.cursor_line(), 3, "on the prose: {}", editor.status());
        let buf = render(&editor, &config, 30, 8);
        let y = row_with(&buf, "見");
        for x in 0..30u16 {
            assert_ne!(buf[(x, y)].bg, want, "column {x} of the prose row: {:?}", row_text(&buf, y));
        }

        // …and onto the rule. `clear_cell` refuses it and `move_cell_row`
        // steps over it, so nothing may say an edit lands in it.
        editor.on_key(Key::Char(':'));
        editor.on_key(Key::Char('2'));
        editor.on_key(Key::Enter);
        assert_eq!(editor.cursor_line(), 1, "on the rule: {}", editor.status());
        let buf = render(&editor, &config, 30, 8);
        let y = row_with(&buf, "---");
        for x in 0..30u16 {
            assert_ne!(buf[(x, y)].bg, want, "column {x} of the rule row");
        }
    }

    /// The cell is `HEAD` and a selection is `SELECTION`, one rung louder and
    /// patched on afterwards — which is the whole reason the cell was allowed
    /// to be a ground at all. Nothing asserted it, so a patch that reordered
    /// the two would have passed the suite.
    #[test]
    fn a_selection_inside_the_cell_still_reads_first() {
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let ink = ink(&config);
        let mut editor = Editor::new();
        editor
            .current_buffer_mut()
            .insert(0, "| 字 | 讀音 |\n| --- | --- |\n| 木 | mu |\n")
                .expect("the fixture buffer is writable");
        // Onto the `木` row by line number: which rows a `j` steps over depends
        // on the grain (#356), and this test is about how a cell is drawn.
        editor.execute("3").unwrap();
        // 表格操作 (#275): the pipes stay on the page, which is what the
        // assertions below read. `t t` re-glyphs them into a grid instead.
        assert!(
            editor.enter_table_as(false),
            "{}",
            editor.status()
        );
        editor.on_key(Key::Char('T')); // #356: 這一條測的是格
        editor.on_key(Key::Char('l'));
        assert_eq!(editor.cell_position().map(|(_, c)| c), Some(1), "on `mu`");
        // `T` is `Grain::Char`, so `l` walks inside the cell and the selection
        // covers two characters. It has to: the block cursor is painted over
        // its own slot last of all, so the column that answers this question is
        // the one **beside** the cursor.
        editor.on_key(Key::Char('T'));
        editor.on_key(Key::Char('v'));
        editor.on_key(Key::Char('l'));
        let buf = render(&editor, &config, 30, 8);
        // By the row's own text: the pane beside it lists the cells, so `mu`
        // is on the page twice, and what stands above the table is the page's
        // to decide (#379).
        let row = row_holding(&buf, "| 木");
        let text = row_text(&buf, row);
        let mu = column_of(&text, "mu");
        assert_eq!(
            buf[(mu, row)].bg,
            ink.at(yumete_config::rung::SELECTION),
            "the selection, not the cell, at column {mu} of {text:?}",
        );
    }

    /// The padding that squares the page up (#212) is *inside* the cell, so it
    /// wears the cell's ground. Without this the tint stops at the last
    /// character the file actually holds, and the one drawing that is about
    /// alignment is the one that looks ragged.
    #[test]
    fn the_padding_inside_the_cell_is_part_of_the_cell() {
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let ink = ink(&config);
        let mut editor = Editor::new();
        // `miao` is the widest cell of the second column, so the `mu` row is
        // padded by two columns the file does not hold.
        editor
            .current_buffer_mut()
            .insert(0, "| 字 | 讀音 |\n| --- | --- |\n| 木 | mu |\n| 目 | miao |\n")
                .expect("the fixture buffer is writable");
        // **The line, said out loud.** Since #283 the level builds the view
        // on its own, so `j` on a table row is a step down the *column*; this
        // setup means a line of the file.
        editor.on_key(Key::Char(':'));
        editor.on_key(Key::Char('3'));
        editor.on_key(Key::Enter);
        // 表格操作 (#275): the pipes stay on the page, which is what the
        // assertions below read. `t t` re-glyphs them into a grid instead.
        assert!(
            editor.enter_table_as(false),
            "{}",
            editor.status()
        );
        editor.on_key(Key::Char('T')); // #356: 這一條測的是格
        editor.on_key(Key::Char('l'));
        assert_eq!(editor.cell_position().map(|(_, c)| c), Some(1));
        let buf = render(&editor, &config, 30, 8);
        let want = ink.at(yumete_config::rung::HEAD);
        let row = row_holding(&buf, "| 木");
        let text = row_text(&buf, row);
        let mu = column_of(&text, "mu");
        // `mu` itself, then the two columns of padding after it, all one cell.
        for x in mu..mu + 4 {
            assert_eq!(buf[(x, row)].bg, want, "column {x} of {text:?}");
        }
        // …and it stops at the pipe. **The pipe**, not `mu + 4`: the box holds
        // the space on either side of the content as well as the padding, so
        // counting columns off the content lands inside it.
        let bar = last_column_of(&text, "|");
        assert_ne!(buf[(bar, row)].bg, want, "and it stops at the pipe: {text:?}");
    }

    /// The 縱書 page draws the ground under the same three rules as the
    /// horizontal one: not on prose, not on the rule row, not over a
    /// `==highlight==`.
    ///
    /// The region guard landed on both pages; the highlight guard landed only
    /// on the horizontal one, and nothing here would have said so.
    #[test]
    fn the_vertical_page_keeps_the_cell_to_the_table() {
        let mut editor = editor_with(
            "| 字 | 讀音 |\n| --- | --- |\n| 木頭 | ==mu== |\n見上表 | 附註",
        );
        editor.set_layout(WritingLayout::Vertical);
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('g'));
        // `h` walks to the next 縱, which is the next line of the file.
        editor.on_key(Key::Char('h'));
        editor.on_key(Key::Char('h'));
        assert!(editor.enter_table(), "{}", editor.status());
        editor.on_key(Key::Char('T')); // #356: 這一條測的是格
        let config = vertical_config();
        let ink = ink(&config);
        let want = ink.at(yumete_config::rung::HEAD);
        let find = |buffer: &ratatui::buffer::Buffer, ch: &str| {
            (0..24u16)
                .flat_map(|x| (0..14u16).map(move |y| (x, y)))
                .find(|&(x, y)| at(buffer, x, y) == ch)
                .unwrap_or_else(|| panic!("{ch} is on the page"))
        };

        // Into the highlighted cell — `l` walks across the row, because the
        // motions are turned with the text. The highlighter's ground is the
        // answer to 「this is marked」 and the cell may not rub it out.
        editor.on_key(Key::Char('l'));
        assert_eq!(editor.cell_position(), Some((2, 1)), "in the marked cell");
        let buffer = render_vertical(&mut editor, &config, 24, 14);
        let (x, y) = find(&buffer, "m");
        assert_eq!(buffer[(x, y)].bg, ink.wash(), "the highlight keeps its ground");

        // Onto the rule row: drawn, not edited.
        editor.on_key(Key::Char(':'));
        editor.on_key(Key::Char('2'));
        editor.on_key(Key::Enter);
        assert_eq!(editor.cursor_line(), 1, "on the rule: {}", editor.status());
        let buffer = render_vertical(&mut editor, &config, 24, 14);
        let (x, y) = find(&buffer, "-");
        assert_ne!(buffer[(x, y)].bg, want, "the rule row is not a cell");

        // And out of the table, onto a paragraph that merely holds a `|`.
        editor.on_key(Key::Char('G'));
        assert_eq!(editor.cursor_line(), 3, "on the prose: {}", editor.status());
        let buffer = render_vertical(&mut editor, &config, 24, 14);
        let (x, y) = find(&buffer, "附");
        assert_ne!(buffer[(x, y)].bg, want, "the prose is not a cell");
    }

    /// 縱書 keeps its page for **表格操作** — `turn_for_table` turns the page
    /// only for 真表格顯示, which draws a grid and cannot draw one down the
    /// page; `t n` leaves the pipes and the commas where they are, and turning
    /// a whole chapter sideways to mend three lines of it throws away
    /// everything around them. So the vertical page is the **only** surface
    /// that ever says which cell Insert is confined to, and #229 has to reach
    /// it too.
    #[test]
    fn the_vertical_page_draws_the_cell_as_well() {
        // Two characters in the first cell, because the cursor's own slot is
        // painted over by the block cursor last of all — the character *beside*
        // it is where the cell's ground has to show through.
        let mut editor = editor_with("| 字 | 讀音 |\n| --- | --- |\n| 木頭 | mu |\n");
        editor.set_layout(WritingLayout::Vertical);
        // 縱書 turns the motions with the text: `h` walks to the next 縱, which
        // is the next line of the file.
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('g'));
        editor.on_key(Key::Char('h'));
        editor.on_key(Key::Char('h'));
        assert!(
            editor.enter_table_as(false),
            "{}",
            editor.status()
        );
        assert_eq!(editor.layout(), WritingLayout::Vertical, "the page is not turned");
        let (line, cell) = editor.cell_position().expect("in a cell");
        assert_eq!(cell, 0, "the first cell, on 木頭");
        assert!(editor.cell_box(line, cell).is_some(), "the cell has a box");
        let config = vertical_config();
        let ink = ink(&config);
        let buffer = render_vertical(&mut editor, &config, 24, 14);
        let want = ink.at(yumete_config::rung::HEAD);
        let find = |ch: &str| {
            (0..24u16)
                .flat_map(|x| (0..14u16).map(move |y| (x, y)))
                .find(|&(x, y)| at(&buffer, x, y) == ch)
                .unwrap_or_else(|| panic!("{ch} is on the page"))
        };
        let head = find("頭");
        assert_eq!(buffer[(head.0, head.1)].bg, want, "the cell it is in is drawn");
        // …and the cell beside it, in the same row, is not.
        let other = find("m");
        assert_ne!(buffer[(other.0, other.1)].bg, want, "the cell beside it is not");
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
    fn the_row_below_says_what_would_finish_what_you_started() {
        let mut editor = editor_with("那年冬天");
        let config = Config::default();
        // A wide glyph covers two cells and only the first carries it, so the
        // row reads back with a gap after every 字.
        // The落款 at the right end is furniture, not something the row said
        // (#498) — it is there whatever is or is not going on.
        let signature = yumete_core::messages::say("ui.signature", &[]);
        let hint = move |e: &Editor| -> String {
            let b = render(e, &config, 100, 10);
            command_line(&b).replace(&signature, "").replace(' ', "")
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
        // **The gutter numbers the window's own rows** since 2026-09-07, so a
        // row number is read back through where the table starts.
        let base = editor.table_row_base();
        let numbered = |y: u16| -> Option<usize> {
            let n: String = (0..6).map(|x| at(&buffer, x, y)).collect();
            n.trim().parse::<usize>().ok().map(|n| base + n - 1)
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
        // 出廠是 `full` since 2026-09-08; this one is about the 藥丸.
        editor.set_hud(Hud::Basic);
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
        // **The status line's right end**, which is the row above the hint row
        // — and since #498 the hint row's own right end carries the 落款, so
        // 「the last line」 is no longer the same thing as 「the corner」.
        let status = |page: &str| -> String {
            let rows: Vec<&str> = page.lines().collect();
            rows[rows.len() - 2].to_string()
        };
        assert!(
            status(&drawn).ends_with('3'),
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

    /// **The nearest margin, not the first row that has one** (#269).
    ///
    /// The old rule was 「the row below, else the row above」 — it asked whether
    /// a row had room and never asked how far away the room was. So a short
    /// line being typed on with a long line under it put the mark at the far
    /// end of *that* line, half a screen from the caret, while the empty margin
    /// one row up went unused.
    #[test]
    fn the_hud_takes_the_nearest_margin_not_the_first_row_with_room() {
        let config = Config::default();
        // Which row the mark landed on, and which corner it drew — the corner
        // says where it thinks the caret is: `─` beside, `╰` below, `╭` above.
        let mark = |editor: &Editor, w: u16, h: u16| -> (Option<(usize, String)>, Vec<String>) {
            let b = render(editor, &config, w, h);
            let rows: Vec<String> = (0..b.area.height)
                .map(|y| (0..b.area.width).map(|x| at(&b, x, y)).collect())
                .collect();
            let found = rows.iter().enumerate().find_map(|(y, row)| {
                ["─ 3", "╰ 3", "╭ 3"]
                    .iter()
                    .find(|mark| row.contains(**mark))
                    .map(|mark| (y, mark.to_string()))
            });
            (found, rows)
        };

        // One: the caret's own row is a candidate, and usually the winner.
        // The line under it reaches the right edge, and the old rule — which
        // could only look below and above — drew nothing at all here.
        let full = "那年冬天山下起了大雪一直下到開春天氣才回暖起來了。";
        let mut editor = editor_with(&format!("短。\n{full}\n"));
        // 出廠是 `full` since 2026-09-08; this one is about the 藥丸's margin.
        editor.set_hud(Hud::Basic);
        editor.on_key(Key::Char('3'));
        let (found, rows) = mark(&editor, 30, 8);
        assert_eq!(
            found,
            Some((0, "─ 3".to_string())),
            "the margin on the caret's own row is the nearest there is: {rows:#?}",
        );

        // Two: when that row *is* full, it flips — to the nearer side. Both
        // sides have room: above's margin starts four columns from the caret
        // and below's eighteen, and the old rule took below every time, because
        // below was simply tried first.
        let brim = "那年冬天山下起了大雪一直下到";
        let mut editor = editor_with(&format!("短。\n{brim}\n回暖起來了天氣才好\n"));
        editor.set_hud(Hud::Basic);
        editor.on_key(Key::Char('j'));
        editor.on_key(Key::Char('3'));
        let (found, rows) = mark(&editor, 30, 8);
        assert_eq!(
            found,
            Some((0, "╭ 3".to_string())),
            "the short line above, not the far end of the wrapped line below: {rows:#?}",
        );
    }

    /// **Three levels, and the loud one is what a window opens at** (#284).
    ///
    /// 「不畫、藥丸、面板」. `off` leaves the status line's right edge and
    /// nothing else — #193's floor, which no level takes away. `full` is the
    /// one that was asked for by name, and since 2026-09-08 the one a window
    /// opens at: a ring, pinned under the caret, over whatever is written
    /// there. It does cover writing — in Normal the HUD carries the *count*,
    /// so the panel hides some of what `3` is counting — and `basic` is one
    /// word away for anyone who would rather keep the prose.
    #[test]
    fn the_hud_has_three_levels_and_only_the_loud_one_covers_the_writing() {
        let config = Config::default();
        // A wide glyph reads back as its own cell and an empty one beside it,
        // so the spaces are dropped before anything is looked for — every
        // 漢字 on the page would otherwise have one inside it.
        let rows = |editor: &Editor| -> Vec<String> {
            let b = render(editor, &config, 60, 10);
            (0..b.area.height)
                .map(|y| {
                    (0..b.area.width)
                        .map(|x| at(&b, x, y))
                        .collect::<String>()
                        .replace(' ', "")
                })
                .collect()
        };
        let mut editor = editor_with("那年冬天，山下起了大雪。\n短。\n回暖起來了天氣才好。\n");
        editor.on_key(Key::Char('3'));

        // 出廠 is the loud one since 2026-09-08. Ask for the 藥丸 first: it
        // sits in the margin and leaves the writing under it untouched.
        assert_eq!(editor.hud(), Hud::Full);
        editor.set_hud(Hud::Basic);
        let page = rows(&editor).concat();
        assert!(page.contains("╰3"), "beside the caret: {page}");
        assert!(page.contains("回暖起來了天氣才好。"), "{page}");

        // `off`: only the corner of the status line is left.
        editor.set_hud(Hud::Off);
        let drawn = rows(&editor);
        assert!(
            !drawn.concat().contains("╰ 3"),
            "nothing beside the caret: {drawn:#?}",
        );
        assert!(
            drawn[drawn.len() - 2].ends_with('3'),
            "and the status line still says it: {drawn:#?}",
        );

        // `full`: a ring under the caret, and it is over the writing — which
        // is the whole reason it is not the factory level.
        editor.set_hud(Hud::Full);
        let drawn = rows(&editor);
        let page = drawn.concat();
        assert!(page.contains("│3│"), "a bordered panel: {drawn:#?}");
        assert!(
            !page.contains("回暖起來了天氣才好。"),
            "and it covers what was there: {drawn:#?}",
        );
    }

    /// **A pinned HUD can no longer see what it would land on** (#284).
    ///
    /// `basic` reads the drawn buffer back, and that one question — 「is
    /// anything here」 — kept it off the writing *and* off the which-key panel,
    /// the command menu, the picker and the detail panel at the same time.
    /// `full` covers writing on purpose, so 「有字」 and 「有面板」 read alike
    /// and the answer stops working. Every panel now says where it went.
    #[test]
    fn the_pinned_hud_still_keeps_off_the_other_panels() {
        let config = Config::default();
        // Short enough that the which-key panel is right under the caret.
        let lines: String = (1..=12).map(|i| format!("第{i}行的字。\n")).collect();
        let mut editor = editor_with(&lines);
        editor.set_hud(Hud::Full);
        // The last line, so the pinned place — one row under the caret — is
        // inside the menu that is about to open.
        editor.on_key(Key::Char('G'));
        editor.on_key(Key::Char('3'));
        editor.on_key(Key::Char('t'));
        // One row per key in the `t` menu, and the menu has grown since (`t a`,
        // 2026-09-07) — so the window is sized to hold the menu **and** leave
        // the pinned HUD somewhere to stand, which is what this is testing.
        let b = render(&editor, &config, 46, 20);
        let drawn: Vec<String> = (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| at(&b, x, y)).collect())
            .collect();
        assert!(
            drawn.concat().contains("│3t│"),
            "the panel is drawn: {drawn:#?}",
        );
        // Every row of the menu still ends in its own right edge — a HUD
        // pinned over it would have taken one out.
        let ring = drawn
            .iter()
            .filter(|row| row.trim_end().ends_with('│'))
            .count();
        assert!(ring >= 7, "the menu's rows are whole: {drawn:#?}");
    }

    /// **The vertical reader had no HUD at all** (#284).
    ///
    /// 「margin」 was the column after the last glyph drawn on the row, which is
    /// the margin only on a page that fills left to right. 縱書 fills right to
    /// left, so on every row carrying any writing that column sat hard against
    /// the right edge, every candidate failed to fit, and what you had typed
    /// was never drawn beside the caret — a whole writing direction with only
    /// the status line. Blank *runs* are the same answer in both directions.
    #[test]
    fn the_vertical_page_gets_the_hud_too() {
        let mut editor = editor_with("那年冬天，山下起了大雪。\n短。\n回暖起來了天氣才好。\n");
        editor.on_key(Key::Char('3'));
        // 出廠是 `full` since 2026-09-08; this one is about the 藥丸.
        editor.set_hud(Hud::Basic);
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 40, 16);
        let rows: Vec<String> = (0..buffer.area.height)
            .map(|y| (0..buffer.area.width).map(|x| at(&buffer, x, y)).collect())
            .collect();
        // 縱書 puts the margin on the caret's *left*, so the corner hangs off
        // the far end and points back — `╰ 3` would point away from the caret.
        assert!(
            rows.iter().any(|row| row.contains("3 ─")),
            "what was typed is beside the caret: {rows:#?}",
        );
        // And it is still never over the writing: the 縱 it sits beside keeps
        // every one of its characters.
        let page: String = rows.concat();
        for ch in "那年冬天山下起了大雪短回暖來天氣才好".chars() {
            assert!(page.contains(ch), "{ch} was painted over: {rows:#?}");
        }
    }

    /// The panel says what can finish the key you pressed, and stands on the
    /// side of the page the cursor is not on.
    #[test]
    fn the_panel_lists_what_would_finish_the_sequence() {
        let mut editor = editor_with("那年冬天，山下起了大雪。\n");
        // 出廠是 `full` since 2026-09-08; this one is about the 藥丸.
        editor.set_hud(Hud::Basic);
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
        let status: String = (0..40u16).map(|x| at(&buffer, x, 8)).collect();
        assert!(status.contains("/20]"), "{status:?}");

        // Walk back to the first and the bar comes with you.
        for _ in 0..19 {
            editor.execute("buffer-previous").unwrap();
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
        // the keys that use it. They disagreed by the command row, the tab bar and
        // the detail panel, so the 縱 the cursor moved on was longer than the
        // 縱 on the screen and a click resolved to the wrong character.
        let mut editor = editor_with("一二三四五六七八九十\n");
        editor.set_layout(WritingLayout::Vertical);
        let config = vertical_config();
        let area = Rect::new(0, 0, 40, 20);
        let page = page_areas(&editor, &config, area, 0).text;
        // Twenty rows, less the status line and the command row.
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
        editor.current_buffer_mut().insert(0, &a_book_of(20_000))
            .expect("the fixture buffer is writable");
        println!("a book:     {:.2?} a frame", frame(&mut editor, 200));
        editor.set_segmentation_visible(true);
        println!("…segmented: {:.2?} a frame", frame(&mut editor, 200));
        editor.set_indent(2);
        println!("…indented:  {:.2?} a frame", frame(&mut editor, 200));

        // **兩半各一本書** (#281): the peek half draws its own document now, and
        // every whole-file memo behind it — `block_cache`, `md_tables`,
        // `pad_cache`, `fold_cache` — is a single slot keyed by buffer. Two
        // documents on one page could therefore mean two full walks a frame,
        // which is the shape #282 cost 8.26 s. This is the line that says
        // whether they need a second slot.
        let dir = std::env::temp_dir().join(format!("yumete-split-cost-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let other = dir.join("other.md");
        std::fs::write(&other, a_book_of(20_000)).unwrap();
        let mut editor = Editor::new();
        editor
            .current_buffer_mut()
            .insert(0, &a_book_of(20_000))
            .expect("the fixture buffer is writable");
        editor.execute(&format!(":open {}", other.display())).unwrap();
        editor.on_key(Key::Char(' '));
        editor.on_key(Key::Char('w'));
        println!("split, one book:  {:.2?} a frame", frame(&mut editor, 100));
        editor.execute(":buffer-previous").unwrap();
        println!("split, two books: {:.2?} a frame", frame(&mut editor, 100));
        std::fs::remove_dir_all(&dir).ok();
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
    fn an_always_margin_keeps_a_row_of_air_across_the_page() {
        let mut editor = editor_with("那年冬天。\n雪一直下。\n山路斷了。\n");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        config.editor.command_line = false;
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
        // The factory margin: nothing to read, so the rows are against each other.
        assert!(rows(&editor)[1].contains("雪"), "{:?}", rows(&editor));

        editor.execute(":view-margin always").unwrap();
        let loose = rows(&editor);
        assert!(loose[0].trim().is_empty(), "a row of air first: {loose:?}");
        assert!(loose[1].contains("那"), "{loose:?}");
        assert!(loose[2].trim().is_empty(), "and between them: {loose:?}");
        assert!(loose[3].contains("雪"), "{loose:?}");

        // …and the caret follows the page it is drawn on.
        editor.on_key(Key::Char('j'));
        let (_, caret) = render_caret(&editor, &config, 30, 10);
        assert_eq!(caret.map(|p| p.y), Some(3), "the second row is drawn at 3");

        editor.execute(":view-margin dense").unwrap();
        assert!(rows(&editor)[1].contains("雪"));
    }

    /// A reader that knows five characters of 春曉 and nothing else.
    struct Tones;

    impl yumete_cjk::Reader for Tones {
        fn read(&self, word: &str) -> Option<Vec<String>> {
            let mut out = Vec::new();
            for ch in word.chars() {
                out.push(
                    match ch {
                        '春' => "chūn",
                        '眠' => "mián",
                        '不' => "bù",
                        '覺' => "jué",
                        '曉' => "xiǎo",
                        _ => return None,
                    }
                    .to_string(),
                );
            }
            Some(out)
        }

        fn available(&self) -> bool {
            true
        }
    }

    fn metered(text: &str) -> yumete_core::Editor {
        let mut editor = editor_with(text);
        editor.set_reader(Box::new(Tones));
        editor.execute(":view-meter on").unwrap();
        editor
    }

    /// 平仄 in the 縱書 margin (#247): ○ 平, ● 仄, and a triangle at the 韻腳.
    #[test]
    fn the_meter_is_written_in_the_margin_beside_the_characters() {
        let mut editor = metered("春眠不覺曉。\n");
        let config = vertical_config();
        let buffer = render_vertical(&mut editor, &config, 20, 10);
        // Wherever the 縱 landed, the marks are in the cell to its right.
        let x = (0..20u16)
            .find(|&x| at(&buffer, x, 0) == "春")
            .expect("the 縱 is on the page");
        let margin: String = (0..5).map(|y| at(&buffer, x + 2, y)).collect();
        // 春 chūn 平, 眠 mián 平, 不 bù 仄, 覺 jué 平 (入聲 in the rule — see
        // `meter`'s own doc), 曉 xiǎo 仄 and last before 。, so a 韻腳.
        assert_eq!(margin, "○○●○▲");
    }

    /// The same marks, set horizontally, in the row a reading would have had.
    #[test]
    fn set_horizontally_the_meter_goes_in_the_row_above_the_line() {
        let mut editor = metered("春眠不覺曉。\n");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.command_line = false;
        config.editor.show_segmentation = false;
        let buffer = render_wrapped(&mut editor, &config, 30, 8);
        assert!(row_text(&buffer, 1).starts_with("春眠不覺曉。"), "{:?}", row_text(&buffer, 1));
        // One mark per character, each over the character it belongs to — so
        // the marks sit in the odd columns a full-width 漢字 begins at.
        let above = row_text(&buffer, 0);
        let marks: String = above.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(marks, "○○●○▲");
        assert_eq!(at(&buffer, 0, 0), "○", "{above:?}");
    }

    /// Turned off, the row is not bought at all — a manuscript that is not a
    /// poem pays nothing for the mode existing.
    #[test]
    fn with_the_meter_off_the_page_is_the_page() {
        let mut editor = metered("春眠不覺曉。\n");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.command_line = false;
        config.editor.show_segmentation = false;
        editor.execute(":view-meter off").unwrap();
        let buffer = render_wrapped(&mut editor, &config, 30, 8);
        assert!(row_text(&buffer, 0).starts_with("春眠不覺曉。"));
    }

    /// With no reader installed there are no tones to draw, and the mode says
    /// so rather than drawing an empty margin.
    #[test]
    fn without_a_reader_the_meter_has_nothing_to_say() {
        let mut editor = editor_with("春眠不覺曉。\n");
        editor.execute(":view-meter on").unwrap();
        assert!(editor.status().contains("讀音") || editor.status().contains("reading"));
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.command_line = false;
        config.editor.show_segmentation = false;
        let buffer = render_wrapped(&mut editor, &config, 30, 8);
        assert!(row_text(&buffer, 0).starts_with("春眠不覺曉。"));
    }

    /// Set vertically the margin is a **cell off every 縱**, and it is bought
    /// before a line is asked for its marks. With nothing to fill it the page
    /// used to reflow anyway — every 縱 paying for a column that can never
    /// hold anything, while the status line said there was no reading table.
    #[test]
    fn without_a_reader_the_page_does_not_pay_for_the_margin() {
        let config = vertical_config();
        let mut off = editor_with("春眠不覺曉。\n");
        let quiet = render_vertical(&mut off, &config, 20, 10);
        let plain = (0..20u16).find(|&x| at(&quiet, x, 0) == "春");

        let mut on = editor_with("春眠不覺曉。\n");
        on.execute(":view-meter on").unwrap();
        assert!(!on.meter_drawn(), "asked for, and not drawable");
        let buffer = render_vertical(&mut on, &config, 20, 10);
        assert_eq!(
            (0..20u16).find(|&x| at(&buffer, x, 0) == "春"),
            plain,
            "the 縱 is where it was"
        );
    }

    /// A reader that knows 了 two ways, and 為 one.
    struct Both;

    impl yumete_cjk::Reader for Both {
        fn read(&self, word: &str) -> Option<Vec<String>> {
            match word {
                // 為了 is a word, and its 了 is 輕聲 — neither 平 nor 仄.
                "為了" => Some(vec!["wèi".into(), "le".into()]),
                "為" => Some(vec!["wèi".into()]),
                "了" => Some(vec!["liǎo".into()]),
                "他" => Some(vec!["tā".into()]),
                _ => None,
            }
        }

        fn available(&self) -> bool {
            true
        }
    }

    /// 平仄 are read off a **word**, so the segmenter is an input to them —
    /// and the answers are kept against a hash of the line's *text*, which
    /// does not change when the dictionary does.
    ///
    /// The trigger in the field is the ordinary one: `:view-meter on` before the
    /// IME has finished loading its dictionary. Until this was fixed, the 了
    /// in 為了 stayed marked 仄 for the rest of the session — the exact
    /// mistake the feature exists to catch.
    #[test]
    fn a_new_dictionary_takes_the_meter_marks_with_it() {
        let mut editor = editor_with("為了他。\n");
        editor.set_reader(Box::new(Both));
        editor.execute(":view-meter on").unwrap();
        // One character at a time: 為 仄, 了 liǎo 仄, 他 平 and the 韻腳.
        let apart = editor.meter_on_line(0);
        assert_eq!(apart.len(), 3, "{apart:?}");

        editor.set_segmenter(Box::new(yumete_cjk::DictionarySegmenter::new(
            [("為了".to_string(), 100i64)],
            1,
        )));
        // 為了 is one word now, and its 了 is 輕聲: no mark at all.
        let joined = editor.meter_on_line(0);
        assert_eq!(joined.len(), 2, "{joined:?}");
    }

    /// 焦點模式: the 段 being written keeps the page's ink and the rest of it
    /// stands back a rung — the same recession a peeked pane is drawn at.
    #[test]
    fn focus_stands_the_rest_of_the_page_back_and_leaves_the_paragraph_lit() {
        let mut editor = editor_with("第一段。\n第二段。\n第三段。\n");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.command_line = false;
        config.editor.show_segmentation = false;
        editor.on_key(Key::Char('j'));

        let lit = render_wrapped(&mut editor, &config, 30, 8);
        let ink = |b: &ratatui::buffer::Buffer, y: u16| b[(0, y)].fg;
        // Nothing is stood back until it is asked for.
        assert_eq!(ink(&lit, 0), ink(&lit, 1), "{:?}", ink(&lit, 0));

        editor.execute(":view-focus").unwrap();
        let focused = render_wrapped(&mut editor, &config, 30, 8);
        // The cursor's own 段 is drawn exactly as it was.
        assert_eq!(ink(&focused, 1), ink(&lit, 1));
        // The others are not, and they are the *same* other — one rung, not a
        // gradient away from the cursor.
        assert_ne!(ink(&focused, 0), ink(&lit, 0));
        assert_eq!(ink(&focused, 0), ink(&focused, 2));

        editor.execute(":view-focus off").unwrap();
        let again = render_wrapped(&mut editor, &config, 30, 8);
        assert_eq!(ink(&again, 0), ink(&lit, 0));
    }

    /// …and on a page whose ground the reader kept.
    ///
    /// `[theme] ground = "terminal"` leaves the writing with no colour of its
    /// own, so swapping palettes moves nothing: `:view-focus` said 「開」 in the
    /// status line and drew an identical page. Standing a row back has to name
    /// the ink out loud, which is what 縱書 has always done.
    #[test]
    fn focus_is_not_a_no_op_on_a_page_that_kept_the_terminal_ground() {
        let mut editor = editor_with("第一段。\n第二段。\n第三段。\n");
        let mut config = Config::default();
        config.theme.ground = yumete_config::Ground::Terminal;
        config.editor.line_numbers = LineNumbers::None;
        config.editor.command_line = false;
        config.editor.show_segmentation = false;
        editor.on_key(Key::Char('j'));

        let lit = render_wrapped(&mut editor, &config, 30, 8);
        let ink = |b: &ratatui::buffer::Buffer, y: u16| b[(0, y)].fg;
        // The reader's own ink, on the reader's own ground.
        assert_eq!(ink(&lit, 0), ratatui::style::Color::Reset);

        editor.execute(":view-focus on").unwrap();
        let focused = render_wrapped(&mut editor, &config, 30, 8);
        // The cursor's 段 is still the reader's ink — that is the promise the
        // setting makes — and the rest of the page is now a colour.
        assert_eq!(ink(&focused, 1), ratatui::style::Color::Reset);
        assert_ne!(ink(&focused, 0), ratatui::style::Color::Reset);
        assert_eq!(ink(&focused, 0), ink(&focused, 2), "one rung, not a gradient");
    }

    /// The same, set vertically — where the unit is the 縱 the 段 is written
    /// down, and a paragraph that wraps keeps every 縱 it wraps into.
    #[test]
    fn focus_lights_the_whole_paragraph_even_where_it_wraps() {
        // Two paragraphs, the first long enough to need two 縱 at this height.
        let mut editor = editor_with(&format!("{}\n短。\n", "長".repeat(12)));
        let mut config = vertical_config();
        config.editor.command_line = false;
        // A one-cell gap, which the factory page no longer has (`:view-margin`, 2026-09-16).
        config.editor.zong_gap = 1;
        editor.execute(":view-focus on").unwrap();
        let buffer = render_vertical(&mut editor, &config, 20, 8);

        // The cursor is in the first paragraph, which wraps: the rightmost two
        // 縱 are both it, and both are lit.
        let ink = |x: u16| buffer[(x, 0)].fg;
        assert_eq!(at(&buffer, 18, 0), "長");
        assert_eq!(at(&buffer, 15, 0), "長", "the same 段, wrapped");
        assert_eq!(ink(18), ink(15), "a wrap point is not a unit of writing");
        // The next paragraph is another one, and stands back.
        assert_eq!(at(&buffer, 12, 0), "短");
        assert_ne!(ink(12), ink(18));
    }

    /// 焦點模式 dims the writing, not the page's furniture.
    ///
    /// The 縱書 number band is the page's own — it is painted before any 縱 is
    /// drawn and stays where it is — so the digits standing on it have to stay
    /// too. Dimmed against a band that did not move, a stood-back 縱's number
    /// measured 1.39:1.
    #[test]
    fn focus_leaves_the_number_band_where_it_is_digits_and_all() {
        let mut editor = editor_with("第一段。\n第二段。\n第三段。\n");
        let mut config = vertical_config();
        config.editor.line_numbers = LineNumbers::Absolute;
        config.editor.line_number_fill = true;
        config.editor.command_line = false;

        let lit = render_vertical(&mut editor, &config, 30, 12);
        editor.execute(":view-focus on").unwrap();
        let focused = render_vertical(&mut editor, &config, 30, 12);

        // The band's rows are above the text; find the one carrying a digit on
        // a 縱 that is not the cursor's.
        // ⚠️ **Not the last row.** The status line carries digits too
        // (`Ln 1, Col 1`), and this used to scan the whole frame — so which
        // columns it found depended on how the status line happened to be laid
        // out, and #394 (whole fields giving way on a narrow window) moved them
        // and broke a test that has nothing to do with the status line.
        let digit = |b: &ratatui::buffer::Buffer, x: u16| {
            (0..b.area.height.saturating_sub(1))
                .find(|&y| b[(x, y)].symbol().chars().any(|c| c.is_ascii_digit()))
                .map(|y| (b[(x, y)].fg, b[(x, y)].bg))
        };
        // The rightmost 縱 is the cursor's; the next 縱 leftward is not.
        let columns: Vec<u16> =
            (0..lit.area.width).filter(|&x| digit(&lit, x).is_some()).collect();
        let x = columns[columns.len() - 2];
        assert!(digit(&lit, x).is_some(), "a number is on the band");
        assert_eq!(
            digit(&focused, x),
            digit(&lit, x),
            "the band and its digits both stay"
        );
    }

    /// Typewriter mode: the row being written stays in the middle, and the
    /// paper moves under it.
    #[test]
    fn typewriter_keeps_the_line_you_are_writing_in_the_middle() {
        let mut editor = editor_with(&(1..=60).map(|n| format!("第{n}行。\n")).collect::<String>());
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        config.editor.command_line = false;
        let rows = 13u16;
        let middle = 5;
        let mut seats = Seats::default();

        editor.execute(":view-typewriter").unwrap();
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
        editor.execute(":view-typewriter off").unwrap();
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
        config.editor.command_line = false;
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
        editor.execute(":table-find 第55行").unwrap();
        let caret = caret_over_time(&editor, &config, &mut seats, 30, rows);
        assert_eq!(caret.map(|p| p.y), Some(middle), "forwards");
        editor.execute(":table-find 第9行").unwrap();
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
    /// the manual's own `:view-wrap` example.
    #[test]
    fn a_blocks_ground_reaches_the_edge_whatever_the_widths_say() {
        let editor = editor_with("前一段。\n\n```\n那年冬天。   ▓▓▓▓▓▓\n雪——一直下。\n```\n");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        // A fence has no ground of its own since #449 — 品色 says what it is
        // and the ``` says where it ends. This is about the *width* of a
        // ground, so it asks for one.
        config.theme.fill = true;
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
    fn the_other_half_draws_the_file_its_caption_names() {
        // #281. The other work area keeps a *buffer id*, and the caption on the
        // divider prints that buffer's name — but the text under it came off
        // whichever file the keys happen to be in. Open the split on 乙, walk
        // the live half back to 甲, and the page said 甲 twice under two
        // different names.
        let dir = std::env::temp_dir().join(format!("yumete-281-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("jia.md"), "甲甲甲\n").unwrap();
        std::fs::write(dir.join("yi.md"), "乙乙乙\n").unwrap();
        let mut editor = Editor::new();
        editor.execute(&format!(":open {}", dir.join("jia.md").display())).unwrap();
        editor.execute(&format!(":open {}", dir.join("yi.md").display())).unwrap();
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;

        // The split opens where you are standing: on 乙.
        editor.on_key(Key::Char(' '));
        editor.on_key(Key::Char('w'));
        assert!(editor.other_pane().is_some(), "the page is split");

        // …and then the live half goes back to 甲, leaving the other half
        // looking at a file that is no longer the current one.
        editor.execute(":buffer-previous").expect("two buffers to walk between");
        assert_eq!(editor.current_buffer().text(), "甲甲甲\n");

        let buffer = render(&editor, &config, 40, 9);
        let rows: Vec<String> = (0..9).map(|y| row_text(&buffer, y).trim_end().to_string()).collect();
        let divider = rows.iter().position(|r| r.starts_with('─')).expect("a rule between them");
        assert!(rows[divider].contains("yi.md"), "the caption still names 乙 的檔: {:?}", rows[divider]);
        assert!(
            rows[..divider].iter().any(|r| r == "甲甲甲"),
            "the live half is where the keys are: {rows:?}"
        );
        assert_eq!(rows[divider + 1], "乙乙乙", "and the other half is what it says it is: {rows:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_question_is_drawn_in_the_middle_with_its_three_answers() {
        // #295. Every other panel here dodges the caret; this one does not,
        // because the editor is stopped behind it. Centre, border, name in
        // gold — and the numbers on the page, so the answer can be checked.
        let dir = std::env::temp_dir().join(format!("yumete-295-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let chapter = dir.join("ch1.md");
        std::fs::write(&chapter, "第一稿。\n".repeat(2_000)).unwrap();

        let mut editor = Editor::new();
        editor.execute(&format!(":open {}", chapter.display())).unwrap();
        editor.paste_text(&"甲乙丙丁戊己庚辛。\n".repeat(40_000));
        editor.execute(":w").unwrap();
        assert!(editor.query().is_some(), "the save stopped to ask");

        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        let buffer = render(&editor, &config, 72, 20);
        let rows: Vec<String> =
            (0..20).map(|y| row_text(&buffer, y).trim_end().to_string()).collect();
        let page = rows.join("\n");

        assert!(page.contains("安全核驗"), "the panel is named: {page}");
        for answer in ["繼續保存", "查看區別", "取消保存"] {
            assert!(page.contains(answer), "{answer} is offered: {page}");
        }
        assert!(page.contains("MB"), "the sizes are on the page: {page}");
        // Middle, not a corner: the top and bottom rows of the window are the
        // manuscript's, and the ring is somewhere between them.
        let ring = rows.iter().position(|r| r.contains('─')).expect("a border");
        assert!(ring > 1 && ring < 18, "the panel is in the middle, at row {ring}: {page}");

        std::fs::remove_dir_all(&dir).ok();
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
            let y = b.area.height - 2;
            (0..w).map(|x| at(b, x, y)).collect::<String>()
        };

        editor.on_key(Key::Char('l'));
        let quiet = row(&render(&editor, &config, 100, 10), 100);
        assert!(squeezed(&quiet).contains("行 1, 列 3"), "{quiet:?}");

        // A yank says something, and it says it on the row below — the status
        // line goes on answering "where am I" while it does.
        editor.on_key(Key::Char('y'));
        let buffer = render(&editor, &config, 100, 10);
        let hint = command_line(&buffer);
        assert!(hint.contains("取"), "the message is below: {hint:?}");
        let status = row(&buffer, 100);
        assert!(squeezed(&status).contains("行 1, 列 3"), "position kept: {status:?}");
        assert!(!status.contains("取"), "and not repeated: {status:?}");

        // With the command row off the message comes back to the status line —
        // a message nobody can see is not a message. And the status line is
        // then the bottom row, there being no row under it.
        let mut plain = config.clone();
        plain.editor.command_line = false;
        let status = row_text(&render(&editor, &plain, 100, 10), 9);
        assert!(status.contains("取") && squeezed(&status).contains("行 1, 列 3"), "{status:?}");
    }

    #[test]
    fn the_status_line_names_the_character_under_the_cursor() {
        // The bundled word list is a build input, not a tracked file
        // (`yumete-cjk/build.rs`), so a machine that has never installed 宇浩
        // has none and every word is one 漢字 — which this picture is not of.
        if !DictionarySegmenter::has_builtin() {
            return;
        }
        let mut editor = editor_with("那年冬天");
        let config = Config::default();
        let row = |b: &ratatui::buffer::Buffer, w: u16| -> String {
            let y = b.area.height - 2;
            (0..w).map(|x| at(b, x, y)).collect::<String>()
        };

        // The cursor opens on the first 字.
        // **A hundred cells, not ninety** (#500). The position grew a third
        // field, and ninety cells with an IME tag on them can no longer hold
        // the block name as well — which is the readout's own third stage of
        // giving way working correctly, not this test's subject.
        let buffer = render(&editor, &config, 100, 10);
        let line = row(&buffer, 100);
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
        assert!(squeezed(&line).contains("行 1, 列 5"), "position kept: {line:?}");
        // And the standing 中／ABC tag (#337) is not what pays for it: the 字
        // itself is, because it is already on the page under the cursor.
        assert!(line.contains("靈"), "the language tag stands: {line:?}");

        // Narrower still and it says nothing rather than truncating.
        let buffer = render(&editor, &config, 44, 10);
        assert!(!row(&buffer, 44).contains("U+"), "nothing rather than a stub");
    }

    #[test]
    fn a_vertical_page_colours_its_markup_the_way_a_horizontal_one_does() {
        // `**那**` — a bold word between markers, and a heading above it.
        let mut editor = editor_with("# 卷一\n那年**冬天**，山下起了大雪。");
        let config = vertical_config();
        editor.set_render(yumete_core::editor::Render::Basic);
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

    /// `never` spends every column on writing: the 稿紙 ticks, which buy a cell
    /// beside every 縱, have nowhere to go. The gap between 縱 is **not** one of
    /// the things it takes — that is `zong_gap`, zero here so the ticks are the
    /// whole difference being measured.
    #[test]
    fn no_margin_spends_every_column_on_writing() {
        // More text than the page can hold, so what is measured is how much of
        // it fits.
        let mut editor = editor_with(&"字".repeat(600));
        let mut config = vertical_config();
        config.editor.zong_gap = 0;
        config.editor.paper_ticks = 10;

        // With ticks a 縱 costs three cells: two for the 字 and one for its tick.
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

        // With no margin a 縱 is two cells — one 漢字 — and nothing else is spent.
        editor.execute("view-margin never").unwrap();
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
            "no ticks without a margin"
        );

        // …and it comes back, because a toggle that does not is not one.
        editor.execute("view-margin dense").unwrap();
        let loose = render_vertical(&mut editor, &config, 40, 14);
        assert_eq!(fits(&loose), before, "back to where it was");
    }

    /// 竪排的號碼是**兩位一行**的，而隔一縱換一檔把它們分開（#512）。
    ///
    /// ⚠️ 這一條從前叫 `..._a_digit_at_a_time`，斷言的正是相反的事：一位一行，
    /// 理由是「密排時 `119` `118` 會讀成 `11` `11` 疊着 `9` `8`」。那個理由成立，
    /// 而另一個解法——把**鄰居**分開，不必把號碼拉長。
    #[test]
    fn a_zong_number_is_two_digits_to_a_row_and_its_neighbour_is_a_rung_back() {
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
        editor.execute("view-margin never").unwrap();
        editor.execute("1").unwrap();
        let buffer = render_vertical(&mut editor, &config, 40, 16);

        // 126 lines: three digits, so **two** header rows, not three — and the
        // numbers sit on the bottom one, the row against the text.
        let rightmost = 40 - 1;
        let head = 1u16;
        let numbers: String = (0..40u16).map(|x| at(&buffer, x, head)).collect();
        assert!(
            numbers.ends_with("1110 9 8 7 6 5 4 3 2 1"),
            "20 down to 1, two digits to a row: {numbers:?}"
        );
        assert_eq!(at(&buffer, rightmost, 0), " ", "the row above is blank");

        // 縱 12 is 「12」 — both cells of one slot, one row.
        let x12 = rightmost - 11 * 2;
        let pair = format!("{}{}", at(&buffer, x12 - 1, head), at(&buffer, x12, head));
        assert_eq!(pair, "12", "two digits, one row: {pair:?}");

        // …and 11 beside it is drawn a rung back, or the two would read as one
        // run of four digits.
        let ink11 = buffer[(rightmost - 10 * 2, head)].style().fg;
        let ink12 = buffer[(x12, head)].style().fg;
        assert_ne!(ink11, ink12, "neighbouring numbers are told apart by colour");
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
        editor.execute(":render basic").unwrap();
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

    /// Feature #249. Two sides, told apart by the ground under them — and the
    /// markers in 朱, because an unresolved conflict is 這裏不對.
    #[test]
    fn the_two_sides_of_a_merge_are_two_grounds() {
        let mut editor = editor_with(
            "第一段\n<<<<<<< HEAD\n我方寫的\n=======\n他方寫的\n>>>>>>> 枝\n最後一段",
        );
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let buffer = render(&editor, &config, 40, 10);

        let prose = buffer[(0, 0)].style().bg;
        let ours = buffer[(0, 2)].style().bg;
        let theirs = buffer[(0, 4)].style().bg;
        assert_ne!(ours, prose, "our side sits on a ground of its own");
        assert_ne!(theirs, prose, "and so does theirs");
        assert_ne!(ours, theirs, "which is the whole point of drawing them");
        assert_eq!(buffer[(38, 4)].style().bg, theirs, "all the way across");
        assert_eq!(buffer[(0, 6)].style().bg, prose, "and it ends where it says");

        // The markers are 朱 — the one thing on the page that says 這裏不對.
        assert_eq!(buffer[(0, 1)].style().fg, Some(ink(&config).mark()));
        assert_eq!(buffer[(0, 5)].style().fg, Some(ink(&config).mark()));

        // Under 所見即所得 the seven brackets come off and the branch stays,
        // the way a heading keeps its words and loses its hashes.
        editor.execute(":render full").unwrap();
        let buffer = render(&editor, &config, 40, 10);
        assert_eq!(row_text(&buffer, 1).trim_end(), "HEAD");
        assert_eq!(row_text(&buffer, 3).trim_end(), "");
        assert_eq!(row_text(&buffer, 5).trim_end(), "枝");
        assert_eq!(row_text(&buffer, 2).trim_end(), "我方寫的");
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

    /// **A table is a block, and blocks have a ground** (#270).
    #[test]
    fn a_table_in_a_chapter_has_a_ground_of_its_own() {
        let mut editor = editor_with("那年冬天\n| 字 | 碼 |\n| --- | --- |\n| 天 | ab |\n山下起了雪");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.show_segmentation = false;
        let buffer = render(&editor, &config, 40, 8);

        let prose = buffer[(0, 0)].style().bg;
        // **Two grounds, and one of them is the page** (#480, #486). The body
        // alternates so one row's cells can be told from the next's when a cell
        // wraps; the header and the `---` under it are one thing and take the
        // page, because the header is told apart by its **ink**.
        assert_eq!(buffer[(0, 1)].style().bg, prose, "the header is not banded");
        assert_eq!(buffer[(0, 2)].style().bg, prose, "…nor the rule under it");
        assert_ne!(buffer[(0, 3)].style().bg, prose, "the body is");
        let ink = ink(&config);
        assert_eq!(buffer[(2, 1)].style().fg, Some(ink.gold()), "the header is 金");
        assert_ne!(buffer[(2, 3)].style().fg, Some(ink.gold()), "a body row is not");
        let head = buffer[(0, 3)].style().bg;
        // **And the ground stops where the table does** (#460): a table is a
        // shape *on* the page with a width of its own, not a block *of* the
        // page taking the window's.
        assert_eq!(
            buffer[(38, 1)].style().bg,
            prose,
            "the band must not run past the last `|`"
        );
        assert_eq!(buffer[(0, 4)].style().bg, prose, "and it ends where it ends");

        // With the colouring off it is prose again, like every other block.
        editor.set_render(yumete_core::editor::Render::Off);
        let buffer = render(&editor, &config, 40, 8);
        assert_eq!(buffer[(0, 1)].style().bg, prose);
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
        editor.current_buffer_mut().insert(0, "第一篇").expect("the fixture buffer is writable");
        editor.execute(":new").unwrap();
        editor.current_buffer_mut().insert(0, "第二篇").expect("the fixture buffer is writable");
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
        alone.current_buffer_mut().insert(0, "第一篇").expect("the fixture buffer is writable");
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
        // The keys go on the row below, so the status line can go on saying
        // which file you are in and where in it you are.
        let hint = command_line(&buffer);
        assert!(hint.contains("邊欄"), "{hint:?}");
        assert!(hint.contains("C-w"), "how to get back: {hint:?}");
        let status = status_line(&buffer);
        assert!(status.contains("[scratch]"), "still says the file: {status:?}");

        // With the keys back in the text it says what it always said.
        editor.on_key(Key::Ctrl('w'));
        let buffer = render(&editor, &config, 80, 12);
        let status = status_line(&buffer);
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
        assert!(text.contains("打開文件"), "{text:?}");
        assert!(text.contains("切換緩衝區"), "{text:?}");

        // `b` replaces it with the picker, which names where you are in a list
        // it does not have to show all of.
        editor.on_key(Key::Char('b'));
        let buffer = render_with(&editor, &config, &no_ime(), 60, 24);
        let text = page_text(&buffer);
        assert!(text.contains("緩衝區"), "{text:?}");
        assert!(!text.contains("打開文件"), "the menu is gone: {text:?}");
        // And it is a box in the middle of the page, not the page: half the
        // window and three rows of furniture (2026-09-18 — it was eight rows
        // and a keyhole).
        //
        // ⚠️ **Counted off the ring, not off the ground.** This asked for a
        // hard-coded `Rgb(0x26, 0x2a, 0x27)` and got it nowhere — the palette
        // moved under it — while the assertion it carried was 「at most nine
        // rows」, which zero satisfies. A test that passes by measuring nothing
        // is worse than no test; the border is drawn by definition.
        let drawn = (0..buffer.area.height)
            .filter(|&y| {
                (0..buffer.area.width)
                    .any(|x| matches!(buffer[(x, y)].symbol(), "│" | "╭" | "╰" | "┌" | "└"))
            })
            .count();
        assert!(
            (10..=24 / 2 + 3).contains(&drawn),
            "half the page and its rings, and never fewer than ten rows: {drawn}"
        );
    }

    /// Which rows a command menu covers, and where its columns start.
    ///
    /// Counted off the colons every entry begins with, not off the ground it is
    /// drawn on: the menu's ground *is* the page's ground on a painted theme,
    /// so a `bg` test either matches the whole window or — with a dark colour
    /// written out by hand under a light default — nothing at all, and the two
    /// assertions it was carrying had never once run.
    fn menu_shape(
        buffer: &ratatui::buffer::Buffer,
    ) -> (
        std::collections::BTreeSet<u16>,
        std::collections::BTreeSet<u16>,
    ) {
        let mut colons: Vec<(u16, u16)> = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| buffer[(x, y)].symbol() == ":")
            .collect();
        // The prompt's own colon is the lowest of them; drop that row.
        if let Some(prompt) = colons.iter().map(|&(_, y)| y).max() {
            colons.retain(|&(_, y)| y < prompt);
        }
        // **The footer is not an entry**, however much it looks like one: it
        // is the highlighted row's help, and a help that names a command —
        // 「`:view-wrap 50` 定寬度」 — put a colon on the line and was counted
        // as a ninth row of eight. It is the last line inside the ring, so it
        // is the last row with a colon on it and nothing but the ring below.
        if let Some(footer) = colons.iter().map(|&(_, y)| y).max() {
            let entries = colons.iter().filter(|&&(_, y)| y == footer).count();
            if entries == 1 {
                colons.retain(|&(_, y)| y < footer);
            }
        }
        (
            colons.iter().map(|&(_, y)| y).collect(),
            colons.iter().map(|&(x, _)| x).collect(),
        )
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
        // The commands, and the aliases that name a whole line (#363) — they
        // are in the list beside the names, so they are in the count.
        // What the menu actually lists: the families are folded (#369).
        let total = yumete_core::command::complete("").len();
        assert!(
            text.contains(&format!("1/{total}")),
            "how much more there is"
        );
        // The menu is a handful of rows, not the screen: **half the window is
        // the ceiling**, footer and rings included (#372).
        let (rows, _) = menu_shape(&buffer);
        assert!(!rows.is_empty(), "a menu was drawn");
        assert!(
            rows.len() + 3 <= 24 / 2,
            "half of a 24-row window, and the menu took {}",
            rows.len()
        );

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
        assert!(text.contains("basic"), "the words `:ruby` takes: {text:?}");
        assert!(!text.contains(":basic"), "a word is not a command, so no colon");
        // A wide glyph covers two cells and only the first carries it. The
        // help shown is the highlighted word's, and the first word `:ruby`
        // takes is `off` — the dialects are commands of their own now
        // (`:ruby-html`), so what follows `:ruby ` is the level and nothing
        // else (#368).
        let squashed = text.replace(' ', "");
        assert!(squashed.contains("顯示源碼"), "and what each one does: {squashed:?}");
    }

    /// **The shape is two caps, and a taller window does not change them**
    /// (2026-09-10). Eight rows, or half the window when that is
    /// shorter; and as many columns as the width holds, up to six.
    ///
    /// It used to grow: the fewest columns that showed every entry, worked out
    /// from how many there were. So the menu was three columns of eighteen at
    /// one moment and eight of six at the next, and a place on it was not
    /// worth learning. A list too long for the caps scrolls, which is what a
    /// list too long for the window did anyway.
    #[test]
    fn the_command_menu_is_the_same_shape_however_tall_the_window() {
        let config = Config::default();
        let total = yumete_core::command::complete("").len();
        let mut editor = editor_with("那年冬天");
        editor.on_key(Key::Char(':'));

        let tall = render_with(&editor, &config, &no_ime(), 160, 60);
        let short = render_with(&editor, &config, &no_ime(), 160, 24);
        let (deep, wide) = menu_shape(&tall);
        assert_eq!(wide.len(), MENU_COLUMNS, "six columns at this width: {wide:?}");
        assert_eq!(
            deep.len(),
            total.div_ceil(MENU_COLUMNS),
            "as deep as six columns need to hold {total}: {deep:?}"
        );
        assert_eq!(
            menu_shape(&short).0.len(),
            deep.len(),
            "and a shorter window does not change it"
        );
        assert!(buffer_text(&tall).contains(&format!("1/{total}")));

        // …until half the window is less than that, which is where it stops:
        // on twelve rows, ten and a footer would leave nothing of the page the
        // menu is *for*.
        let squat = render_with(&editor, &config, &no_ime(), 160, 12);
        assert!(menu_shape(&squat).0.len() < deep.len(), "half of twelve");

        // **Typing narrows the panel without moving its rows**, which is what
        // measuring the shape off the whole list buys.
        for c in "view".chars() {
            editor.on_key(Key::Char(c));
        }
        let narrowed = render_with(&editor, &config, &no_ime(), 160, 60);
        assert_eq!(
            menu_shape(&narrowed).0.len(),
            deep.len(),
            "twelve `view-…` still stand on the rows the whole list settled"
        );
    }

    #[test]
    fn the_command_menu_leaves_a_short_window_something_to_look_at() {
        // The eight-row floor keeps a menu worth opening on a small window,
        // but on a twelve-row terminal eight rows and a footer is nine of the
        // twelve — the page it is a menu *for* would be gone. Half is where
        // the floor stops.
        let config = Config::default();
        let mut editor = editor_with("那年冬天");
        editor.on_key(Key::Char(':'));
        let buffer = render_with(&editor, &config, &no_ime(), 120, 12);
        let (rows, _) = menu_shape(&buffer);
        assert!(!rows.is_empty(), "a menu at all");
        assert!(rows.len() <= 5, "at most half the window, footer and all: {rows:?}");
    }

    /// A real 碼表 in each table mode, for looking at (#374).
    #[test]
    #[ignore = "a picture, not a test"]
    fn the_code_table_in_every_mode() {
        let dir = std::env::temp_dir().join(format!("yumete-mabiao-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("m.txt");
        std::fs::write(
            &path,
            "ch\t錐\ndl\t叭\nsbi\t埭\nsbfr\t壚\nsbjj\t埈\nfvtf\t裘\n",
        )
        .unwrap();
        let config = Config::default();
        for keys in ["", "tf", "tb", "tt", "to"] {
            let mut ed = Editor::new();
            ed.open_file(&path).unwrap();
            for c in keys.chars() {
                ed.on_key(Key::Char(c));
            }
            let buffer = render_with(&ed, &config, &no_ime(), 40, 12);
            println!("=== `{keys}` ===");
            for y in 0..8 {
                println!("|{}|", row_text(&buffer, y).trim_end());
            }
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Draw the `:` menu narrowed by typing, for looking at.
    #[test]
    #[ignore = "a picture, not a test"]
    fn the_menu_drawn_narrowed() {
        let config = Config::default();
        for typed in ["", "view", "t"] {
            let mut ed = editor_with("那年冬天");
            ed.on_key(Key::Char(':'));
            for c in typed.chars() {
                ed.on_key(Key::Char(c));
            }
            let buffer = render_with(&ed, &config, &no_ime(), 160, 60);
            println!("=== `:{typed}` ===");
            for y in 0..60 {
                let line = row_text(&buffer, y);
                if line.contains('│') || line.contains('╭') || line.contains('╰') {
                    println!("{}", line.trim_end());
                }
            }
        }
    }

    /// Draw the `:` menu at a real size, for looking at.
    #[test]
    #[ignore = "a picture, not a test"]
    fn the_menu_drawn_wide() {
        let config = Config::default();
        let mut wide = editor_with("那年冬天");
        wide.on_key(Key::Char(':'));
        for (w, h) in [(160u16, 40u16), (100, 30), (80, 24)] {
            let buffer = render_with(&wide, &config, &no_ime(), w, h);
            println!("=== {w}x{h} ===");
            for y in 0..h {
                let line = row_text(&buffer, y);
                if line.contains('│') || line.contains('╭') || line.contains('╰') {
                    println!("{}", line.trim_end());
                }
            }
        }
    }

    #[test]
    fn the_command_menu_spreads_across_a_wide_window() {
        let config = Config::default();
        // The commands, and the aliases that name a whole line (#363) — they
        // are in the list beside the names, so they are in the count.
        // What the menu actually lists: the families are folded (#369).
        let total = yumete_core::command::complete("").len();

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
        // more than one column, and no taller than a menu is allowed to be on
        // a window this short — half of it, footer and rings included.
        let (rows, columns) = menu_shape(&buffer);
        assert!(
            rows.len() <= 24 / 2 - 3,
            "a menu is glanced at, not read: {rows:?}"
        );
        assert!(columns.len() >= 3, "several columns: {columns:?}");
        assert!(
            columns.len() <= MENU_COLUMNS,
            "and not past the half-dozen a glance can hold: {columns:?}"
        );
        // Wide enough that the whole list fits without scrolling — which is
        // the point of the columns, and what folding the families keeps true
        // (#369): flattening the tree took the top level well past what a
        // window holds, and a menu is glanced at rather than read.
        assert!(
            rows.len() * columns.len() >= total,
            "all {total} of them: {} slots",
            rows.len() * columns.len()
        );
        assert!(
            total < yumete_core::command::COMMANDS.len(),
            "and it is the folded list: {total}"
        );

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
        // Above it is the hint row, then the menu's bottom rule, and above
        // *that* the menu's own footer — the count and what the highlighted
        // row means — with the rows above that again. The menu stacks upward
        // from the whole footer, not from the command line alone, so it never
        // covers either.
        let footer: String = (0..buffer.area.width)
            .map(|x| buffer[(x, buffer.area.height - 4)].symbol())
            .collect();
        assert!(footer.contains('/'), "a count of the matches: {footer:?}");
        let text = buffer_text(&buffer);
        assert!(text.contains("ruby") || text.contains("redo"), "{text:?}");
    }

    /// 2026-09-05: 「command 提示面板的設計感不如快捷鍵提示面板。
    /// 請你對齊一下：面板有個邊框 ＋ 左上有個「命令」文字。」
    ///
    /// It was a rectangle of ground with no edge and no name — a thing that
    /// appeared beside the writing, where the which-key panel two keystrokes
    /// away is a named ring. One panel, one idea.
    #[test]
    fn the_command_menu_wears_the_same_ring_as_the_which_key_panel() {
        let mut editor = editor_with("那年冬天");
        editor.on_key(Key::Char(':'));
        let config = Config::default();
        let buffer = render_with(&editor, &config, &no_ime(), 90, 24);
        // A wide glyph covers two cells and only the first carries it.
        let text = buffer_text(&buffer).replace(' ', "");
        assert!(text.contains("命令"), "the panel says what it is: {text:?}");
        // The rule runs down the left edge, beside the entries — which is
        // where the colons are, one cell in from it.
        let (rows, columns) = menu_shape(&buffer);
        let top = *rows.iter().next().expect("a menu was drawn");
        assert_eq!(*columns.iter().next().unwrap(), 2, "inside the ring");
        assert_eq!(buffer[(0, top)].symbol(), "│", "a ring around it");
        assert_eq!(buffer[(0, top - 1)].symbol(), "╭", "and a corner to it");
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
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

    /// #286: a panel places itself from its own width, so its left edge lands
    /// on an odd column for half of all terminal widths — and over 漢字 prose
    /// that is the second cell of somebody's character. The whole left wall
    /// (`╭`, every `│`, `╰`) was computed, stored, and never emitted; the
    /// reader saw the prose behind it showing through the gap.
    #[test]
    fn no_panel_wall_stands_in_the_second_half_of_a_漢字() {
        // Every row full of 漢字 — a page with blank rows under the panel
        // cannot show the fault, because there is no glyph to straddle.
        let line = "春夏秋冬花開花落又一年，江水東流不復回。".repeat(8);
        let prose = vec![line; 24].join("\n");
        let config = Config::default();
        // **Look for the corner that is missing, not for the one that is
        // wrong.** A wall written into the second half of a 漢字 never reaches
        // the terminal at all — `render_with` returns the backend's buffer,
        // which ratatui fills through `Buffer::diff`, and `diff` skips whatever
        // a wide glyph covers. So the broken frame holds no wall to inspect:
        // the only trace is a right corner whose left corner never arrived.
        for width in 70..110u16 {
            let mut editor = editor_with(&prose);
            // 出廠是 `full` since 2026-09-08; the panel under test is `t`'s.
            editor.set_hud(Hud::Basic);
            editor.on_key(Key::Char('t'));
            let buffer = render_with(&editor, &config, &no_ime(), width, 16);
            let mut tops = 0;
            let mut bottoms = 0;
            for y in 0..buffer.area.height {
                let row: Vec<&str> = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                let has = |s: &str| row.contains(&s);
                if has("╮") || has("┐") {
                    tops += 1;
                    assert!(
                        has("╭") || has("┌"),
                        "width {width}, row {y}: the panel's top-right corner is \
                         on the page and its top-left corner is not"
                    );
                }
                if has("╯") || has("┘") {
                    bottoms += 1;
                    assert!(
                        has("╰") || has("└"),
                        "width {width}, row {y}: the panel's bottom-right corner \
                         is on the page and its bottom-left corner is not"
                    );
                }
            }
            assert!(
                tops == 1 && bottoms == 1,
                "width {width}: expected exactly one panel on the page, \
                 found {tops} top edges and {bottoms} bottom edges"
            );
        }
    }

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

    /// #294, the first slice of #299: a note **takes no rows off the page**.
    ///
    /// It used to get a full-width four-row band along the bottom — a wrong
    /// price twice over: one short paragraph left most of that width blank,
    /// and the four rows came off the manuscript whether the note filled them
    /// or not. It floats now, in the same panel every other pop-up already is,
    /// so the page keeps every row it had and the note merely stands over one
    /// corner of it.
    #[test]
    fn a_note_floats_in_a_panel_and_takes_no_rows_off_the_page() {
        let mut body: Vec<String> = (1..=14)
            .map(|i| format!("第 {i} 段。那一年的雨下得久，他站在門口。"))
            .collect();
        body[1] = format!("末段的雨停了一次[^1]，{}", body[1]);
        let text = format!(
            "[^1]: 舊城的屋簷極寬，一到雨季，簷下就成了另一條街。\n\n{}",
            body.join("\n")
        );
        let config = Config::default();
        let mut editor = editor_with(&text);
        // 出廠是 `full`; this one reads the page's own rows.
        editor.set_hud(Hud::Basic);

        // Two rows down and onto the reference, which opens the panel by
        // itself — a footnote is not a table row, so it needs no `t i`.
        let quiet = render(&editor, &config, 70, 20);
        for _ in 0..3 {
            editor.on_key(Key::Char('j'));
        }
        editor.on_key(Key::Char('f'));
        editor.on_key(Key::Char('^'));
        let shown = render(&editor, &config, 70, 20);

        let rows = |b: &ratatui::buffer::Buffer| -> Vec<String> {
            (0..b.area.height).map(|y| row_text(b, y)).collect()
        };
        let page = rows(&shown);
        assert!(
            page.iter().any(|r| r.contains("舊城的屋簷極寬")),
            "the note is on the page: {page:?}"
        );
        assert!(
            page.iter().any(|r| r.contains('│')),
            "and it is in a ring, not a band: {page:?}"
        );
        // Its name in its own border, and where it is written said quietly on
        // the bottom edge.
        assert!(page.iter().any(|r| r.contains("[^1]")), "{page:?}");
        assert!(page.iter().any(|r| r.contains("第 1 行")), "{page:?}");

        // **The page kept its rows.** The panel stands over the bottom-right
        // corner, so the left of those same rows still holds the manuscript it
        // held before — a band would have blanked them to the full width.
        let before = rows(&quiet);
        for y in (shown.area.height - 6)..(shown.area.height - 3) {
            let cut = |r: &String| r.chars().take(8).collect::<String>();
            assert_eq!(
                cut(&before[y as usize]),
                cut(&page[y as usize]),
                "row {y} lost its manuscript to the note"
            );
        }
    }

    /// #294: the panel takes **the corner the caret is not in** — on both
    /// axes. A fixed corner is right half the time and covers the very line
    /// being read the other half.
    #[test]
    fn the_note_panel_takes_the_corner_the_caret_is_not_in() {
        let config = Config::default();
        // Where the panel's top-left corner landed, and how the page reads.
        let corner = |editor: &Editor| -> ((u16, u16), Vec<String>) {
            let b = render(editor, &config, 70, 20);
            let page: Vec<String> = (0..b.area.height).map(|y| row_text(&b, y)).collect();
            let at = (0..b.area.height)
                .flat_map(|y| (0..b.area.width).map(move |x| (x, y)))
                .find(|&(x, y)| b[(x, y)].symbol() == "\u{256d}" || b[(x, y)].symbol() == "\u{250c}");
            (
                at.unwrap_or_else(|| panic!("no panel on the page: {page:?}")),
                page,
            )
        };
        // A manuscript whose reference sits on line `mark` of `lines` body
        // rows, with the caret standing on it. The note is two rows long at
        // this width — see the flip below for why that matters.
        let note = |lines: usize, mark: usize| -> Editor {
            let mut body: Vec<String> = (1..=lines)
                .map(|i| format!("第 {i} 段。那一年的雨下得久，他站在門口。"))
                .collect();
            body[mark] = format!("末段的雨停了一次[^1]，{}", body[mark]);
            let text = format!(
                "[^1]: 舊城的屋簷極寬，一到雨季，簷下就成了另一條街，走過去要低頭。\n\n{}",
                body.join("\n")
            );
            let mut editor = editor_with(&text);
            // 出廠是 `full`; this one reads the page's own rows.
            editor.set_hud(Hud::Basic);
            for _ in 0..(mark + 2) {
                editor.on_key(Key::Char('j'));
            }
            editor.on_key(Key::Char('f'));
            editor.on_key(Key::Char('^'));
            editor
        };

        // Caret high on the page and hard left: the panel goes low and right.
        // 「Right」 is its **far** edge against the page's — a narrow panel's
        // left corner is nowhere near the middle of the screen.
        let ((x, y), page) = corner(&note(40, 0));
        assert!(x > 0, "panel should be off the left wall: {x}, {page:?}");
        assert!(
            page[y as usize].ends_with('\u{256e}') || page[y as usize].ends_with('\u{2510}'),
            "panel should be flush right: {page:?}"
        );
        assert!(y > 20 / 2, "panel should be low: {y}, {page:?}");

        // Caret down among the rows the panel wanted: it moves to the top.
        // Not 「the caret passed the middle」 — 「the caret is in the way」.
        //
        // **The page keeps three rows under the caret**, so how far down the
        // caret can be is fixed and what varies is how far up the panel
        // reaches: a two-row note is four rows of ring, and its top row is the
        // row the caret is on. That is the collision, and it is the common
        // one — a note of any length at all wraps.
        let ((_, y), page) = corner(&note(40, 15));
        assert_eq!(y, 0, "panel should have flipped to the top: {page:?}");
    }

    #[test]
    fn the_ime_composes_into_a_search_prompt() {
        let mut editor = editor_with("春江潮水連海平");
        editor.on_key(Key::Char('/'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");

        // Letters typed at a `/` prompt compose instead of landing literally.
        assert!(ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char('b'),
            KeyModifiers::NONE
        ));
        assert!(ime.is_composing());
        assert_eq!(editor.prompt(), Some(("/", "")), "nothing committed yet");

        // …and the committed candidate lands in the pattern, not the buffer.
        assert!(ime_handle(
            &mut ime,
            &mut editor,
            KeyCode::Char(' '),
            KeyModifiers::NONE
        ));
        assert_eq!(editor.prompt(), Some(("/", "吧")));
        assert_eq!(editor.current_buffer().text(), "春江潮水連海平");
    }

    #[test]
    fn the_prompt_shows_the_preedit_and_the_language_tag() {
        let mut editor = editor_with("春江潮水");
        editor.on_key(Key::Char('/'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧\n");
        ime.input('b');
        let config = Config::default();
        let buffer = render_with(&editor, &config, &ime, 60, 8);

        let status: String = (0..buffer.area.width)
            .map(|x| buffer[(x, buffer.area.height - 1)].symbol())
            .collect();
        // **`搜索:` now, not `/`** (#505) — the prompt says the word the way
        // helix's does; `/` is still the key that opens it.
        let label = yumete_core::messages::say("ui.prompt-search", &[]);
        // Spaces out, because a 漢字 leaves its continuation cell blank in the
        // test backend and the label is two of them.
        // Spaces out, because a 漢字 leaves its continuation cell blank in the
        // test backend and the label is two of them — **and the label ends in a
        // space of its own** (#505), so that `搜索:` and a typed `:s` do not run
        // together.
        assert!(
            status.replace(' ', "").starts_with(&label.replace(' ', "")),
            "the prompt says its name: {status:?}"
        );
        assert!(
            status.replace(' ', "").contains(&format!("{}b", label.trim_end())),
            "preedit missing: {status:?}"
        );
        assert!(status.contains("[中"), "language tag missing: {status:?}");
    }

    /// `:yume on` is an answer about the language, and the loop must see it
    /// as one (#412).
    ///
    /// It asked for `+` and `-` — the spelling from before there were three
    /// answers to give (#290) — and nothing has sent either since. So this
    /// runs the commands and reads what the core really puts on the channel.
    /// #341: 讀盤之前先畫一句，而問狀態、調設定、交還鍵盤都不讀盤。
    #[test]
    fn only_the_requests_that_read_a_table_say_they_are_loading() {
        let none = ImeSession::empty(Scheme::LINGMING);
        let loaded = ImeSession::from_table_text(Scheme::LINGMING, "a 啊\n");
        // 換方案、指一個檔、要系統那份、要出廠那份——手上有沒有碼表都要讀盤。
        for tag in ["", "lingming", "=~/my.txt", "~", "!"] {
            assert!(loading_the_table(tag, &none), "{tag:?}");
            assert!(loading_the_table(tag, &loaded), "{tag:?}");
        }
        for tag in ["?", "commit:unique", "panel:bare", "lang:abc", "lang:off"] {
            assert!(!loading_the_table(tag, &none), "{tag:?}");
        }
        // 開始打中文只在手上還沒有碼表的那一次讀盤。
        assert!(loading_the_table("lang:chinese", &none));
        assert!(!loading_the_table("lang:chinese", &loaded));
    }

    #[test]
    fn the_language_requests_are_spelled_the_way_the_loop_reads_them() {
        let mut editor = Editor::new();
        for line in ["yume on", "yume abc", "yume off"] {
            editor.execute(line).unwrap();
            let tag = editor
                .take_scheme_request()
                .unwrap_or_else(|| panic!("{line} asked the front end nothing"));
            assert!(answers_the_language(&tag), "{line} sent {tag:?}");
        }
        // …and the requests about the 碼表 are not answers about the
        // language: putting the borrow back after one of those is right.
        for line in ["yume-scheme", "yume-panel", "yume-commit"] {
            editor.execute(line).unwrap();
            if let Some(tag) = editor.take_scheme_request() {
                assert!(!answers_the_language(&tag), "{line} sent {tag:?}");
            }
        }
    }

    /// A prompt is left in the language it was handed, whichever prompt it is.
    ///
    /// The list of which modes count used to be written out here, and it said
    /// `:` and `::` (#340): `/` neither ended the composition on the way in
    /// nor gave anything back on the way out.
    #[test]
    fn every_prompt_borrows_the_language_and_gives_it_back() {
        for prompt in [Mode::Command, Mode::Lookfor, Mode::Search, Mode::Ruby, Mode::Picker] {
            // Out of 中文 Insert and into the prompt: the composition is
            // ended either way, and only `:` and `::` force 英.
            let mut borrow = Borrow::default();
            let door = borrow.crossing(Some(Mode::Insert), prompt, true);
            assert!(door.escape, "{prompt:?} has nothing to finish it with");
            assert_eq!(
                door.toggle,
                prompt.prompt_opens_in_english(),
                "{prompt:?} forcing 英"
            );
            // …and back out, into whatever the prompt left the engine in.
            let chinese = !door.toggle;
            let back = borrow.crossing(Some(prompt), Mode::Insert, chinese);
            assert!(!back.escape, "nothing to end on the way back");
            assert_eq!(back.toggle, !chinese, "{prompt:?} owes 中 back");
            assert_eq!(
                borrow.crossing(Some(Mode::Insert), Mode::Normal, true),
                AtTheDoor::default(),
                "and owes nothing twice"
            );
        }
    }

    /// `:` and `::` are one prompt: stepping between them is not leaving.
    #[test]
    fn the_command_line_and_its_lookup_are_one_prompt() {
        let mut borrow = Borrow::default();
        assert!(borrow.crossing(Some(Mode::Insert), Mode::Command, true).toggle);
        let across = borrow.crossing(Some(Mode::Command), Mode::Lookfor, false);
        assert_eq!(across, AtTheDoor::default(), "not a door at all");
        let out = borrow.crossing(Some(Mode::Lookfor), Mode::Normal, false);
        assert!(out.toggle, "中 was Insert's and comes back at the end");
    }

    /// A lone-Shift tap **on a prompt** changes that one line (#338).
    ///
    /// Repro: write English in Insert, `:`, `e `, tap Shift, `第三章.md`,
    /// Enter — and Insert came out speaking 中文, because the tap had thrown
    /// away the way back.
    #[test]
    fn a_tap_on_the_command_line_does_not_end_the_borrow() {
        let mut borrow = Borrow::default();
        // 英 Insert opens `:` — nothing to force, it is already 英.
        assert!(!borrow.crossing(Some(Mode::Insert), Mode::Command, false).toggle);
        // The tap that turns the file name 中文 is about this line only.
        borrow.answered_in(Mode::Command);
        let out = borrow.crossing(Some(Mode::Command), Mode::Insert, true);
        assert!(out.toggle, "英 is what Insert lent and what it gets back");
        // The same tap in Insert *is* the answer, and settles the debt.
        let mut borrow = Borrow::default();
        borrow.crossing(Some(Mode::Insert), Mode::Command, false);
        borrow.answered_in(Mode::Insert);
        assert_eq!(
            borrow.crossing(Some(Mode::Command), Mode::Insert, true),
            AtTheDoor::default(),
            "nothing owed"
        );
    }

    /// Modes that collect *prose* compose; Normal must not, or `/` itself would
    /// be swallowed by the IME, and the command line must not, because its
    /// whole vocabulary is ASCII.
    #[test]
    fn only_prose_modes_compose() {
        assert!(Mode::Insert.composes());
        assert!(Mode::Search.composes());
        assert!(Mode::Ruby.composes());
        assert!(!Mode::Normal.composes());
        assert!(!Mode::Command.composes());
    }

    /// Normal mode is not prose — except for the character `f`、`r`、`ms`、
    /// `mi`、`mr` are waiting for (§5.2.3 ②, #414), which in a Chinese
    /// manuscript is 中文 and needs the engine. Every gate in this file asks
    /// `composes_here`, so this one answer opens the preedit, the panel and
    /// the lone-Shift tap at once.
    #[test]
    fn a_key_waiting_for_a_character_composes_in_normal_mode() {
        let mut editor = Editor::new();
        assert!(!composes_here(&editor), "Normal mode is keys, not text");
        editor.on_key(Key::Char('r'));
        assert!(composes_here(&editor), "`r` is waiting for a character");
        editor.on_key(Key::Esc);
        assert!(!composes_here(&editor), "and it stops when `r` is answered");
        // The four that could not do this before #414.
        for keys in [&["f"][..], &["F"], &["m", "s"], &["m", "i"], &["m", "r"]] {
            let mut editor = Editor::new();
            for key in keys {
                editor.on_key(Key::Char(key.chars().next().unwrap()));
            }
            assert!(
                composes_here(&editor),
                "`{}` is waiting for a character of the manuscript",
                keys.concat()
            );
        }
        // …while the keys waiting for the *name* of something are still keys.
        for keys in [&["\""][..], &["m"], &["Z"], &["z"], &["g"], &[" "]] {
            let mut editor = Editor::new();
            for key in keys {
                editor.on_key(Key::Char(key.chars().next().unwrap()));
            }
            assert!(
                !composes_here(&editor),
                "`{}` is waiting for a letter that names a command",
                keys.concat()
            );
        }
    }

    /// **A query, then `Esc`, and the frame still draws** (2026-09-17).
    ///
    /// The hint that says what the list layer's keys are goes on the end of
    /// the footer, and the caret's column used to be worked out by subtracting
    /// byte lengths from the whole of it — so anything after the query put the
    /// index inside a 漢字 and the editor panicked **while drawing**, which is
    /// the one place a panic takes the session with it: 「space + f + j + j +
    /// Esc 我就退出 yumete 了」.
    #[test]
    fn the_picker_draws_with_a_query_and_the_keys_in_the_list() {
        let mut editor = Editor::new();
        let config = Config::default();
        editor.on_key(Key::Char(' '));
        editor.on_key(Key::Char('f'));
        editor.on_key(Key::Char('/'));
        for c in "jj".chars() {
            editor.on_key(Key::Char(c));
        }
        editor.on_key(Key::Esc);
        assert!(editor.picker().is_some_and(|p| !p.typing()));
        // Panicked here before the fix, whatever the query matched.
        let text = buffer_to_text(&render_with(&editor, &config, &no_ime(), 80, 24));
        assert!(text.contains("jj"), "the query is drawn: {text}");
        // **And the box keeps its shape with nothing in it** — the query
        // matches no file, and the panel is still a panel, with the reason
        // written in it.
        assert!(text.contains(&yumete_core::say!("picker.nothing-matched")), "{text}");
    }

    /// **One float at a time** (2026-09-18) — a `:` menu beats a half-pressed
    /// sequence, which beats the note the cursor is standing on.
    ///
    /// 作者, looking at a 百科 entry and the `:` menu crowding one screen:
    /// 「一次只會出現一個面板，那麽輸入命令的時候百科窗口自然就會消失」.
    #[test]
    fn only_one_thing_floats_over_the_page_at_a_time() {
        let config = Config::default();
        // ⚠️ **Wide rings only.** The HUD beside the caret is a ring too —
        // two cells of what has been typed so far — and it is not a panel;
        // counting it made 「one float」 read as two.
        let rings = |editor: &Editor| {
            let buffer = render_with(editor, &config, &no_ime(), 80, 24);
            (0..buffer.area.height)
                .filter(|&y| {
                    let corner = (0..buffer.area.width)
                        .any(|x| matches!(buffer[(x, y)].symbol(), "╭" | "┌"));
                    let wide = (0..buffer.area.width)
                        .filter(|&x| buffer[(x, y)].symbol() == "─")
                        .count()
                        >= 10;
                    corner && wide
                })
                .count()
        };

        // A half-pressed `空格` draws its key table…
        let mut editor = editor_with("那年冬天");
        editor.on_key(Key::Char(' '));
        assert_eq!(rings(&editor), 1, "the key table, and nothing else");

        // …and a `:` line draws the command menu instead of it. Two rings
        // would be two things wanting the same corner.
        let mut editor = editor_with("那年冬天");
        editor.on_key(Key::Char(':'));
        assert_eq!(rings(&editor), 1, "the command menu, alone");
    }

    /// **The picker's query composes; its list does not** (2026-09-17).
    ///
    /// The gate used to ask only 「is this mode a typing one」, and `Mode::Picker`
    /// is — so with the keys in the list, `j` and `k` went to the engine as a
    /// code instead of walking the list: 「我按了 space f 進入 picker，按 jk 他
    /// 開始輸入」. One line in `composes_here`, and this is the test that keeps
    /// it: a layer nobody asked about is how that bug happened.
    #[test]
    fn the_pickers_list_layer_does_not_compose() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char(' '));
        editor.on_key(Key::Char('f'));
        assert!(editor.picker().is_some(), "the picker is open");
        assert!(!composes_here(&editor), "it opens in the list, where jk walk");
        editor.on_key(Key::Char('/'));
        assert!(composes_here(&editor), "the query takes 中文");
        editor.on_key(Key::Esc);
        assert!(editor.picker().is_some(), "Esc is the layer, not the door out");
        assert!(!composes_here(&editor), "…and back in the list");
    }

    /// The command line takes no 中文 anywhere on it — not after the command
    /// name, not after a bang, not in a path (2026-09-16).
    ///
    /// The rule it replaced let the caret decide, which made the boundary a
    /// thing `←` could step over; with 模態掛起 behind this gate, every such
    /// step was a signal to the system's input method. One answer for the whole
    /// line is the point, so the test walks the shapes that used to say yes.
    #[test]
    fn the_command_line_never_composes() {
        let line = |text: &str| {
            let mut editor = Editor::new();
            editor.on_key(Key::Char(':'));
            for c in text.chars() {
                editor.on_key(Key::Char(c));
            }
            editor
        };
        for text in ["", "layout ", "s/照首行", "e ", "w! ", "yume-tab ", "e 第三章.md"] {
            let editor = line(text);
            assert!(
                !composes_here(&editor),
                "`:{text}` is a command line: {:?}",
                editor.command_line()
            );
        }
        // …while the one next door still does: a second `:` is 命令搜索, which
        // exists to be asked in 中文.
        let mut editor = Editor::new();
        editor.on_key(Key::Char(':'));
        editor.on_key(Key::Char(':'));
        assert_eq!(editor.mode(), Mode::Lookfor);
        assert!(composes_here(&editor), "命令搜索 is asked in 中文");
    }

    #[test]
    fn typing_then_space_commits_into_the_editor() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");

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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
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

    /// `:convert` from end to end, through the real `opencc` — Feature #241.
    ///
    /// The unit tests either side of this one check the plan and the rewrite;
    /// what only a run can check is that the plan produces a command line a
    /// real converter accepts, and that what comes back is what the manuscript
    /// should say. `--shot` cannot photograph this — it draws without turning
    /// the loop that services a shell request — so the loop's own arm is
    /// spelled out here instead.
    ///
    /// Skipped where `opencc` is not installed, which is most CI machines. A
    /// test that silently passes without the program would be worse than none.
    #[test]
    fn convert_runs_the_real_opencc_and_brings_back_the_manuscript() {
        use yumete_core::editor::How;
        let Some(_) = yumete_core::convert::opencc() else {
            return;
        };
        // 通規 in, 臺灣正體 out: 説 内 吴 are 字形 opencc has never heard of,
        // so this only works if the table was walked backwards first.
        let mut editor = editor_with("他説内人在裏面，吴先生録了一段。\n");
        editor.execute("convert c tw").unwrap();
        let want = editor.take_shell_request().expect("opencc has to be run");
        let How::Convert(input) = want.how else {
            panic!("a conversion is piped, not handed the screen");
        };
        assert_eq!(input, "他說內人在裏面，吳先生錄了一段。\n", "the way back");
        let ran = run_capturing(&want.line, Some(&input)).expect("opencc runs");
        assert!(ran.ok, "{}", ran.why());
        editor.provide_conversion(&ran.said);
        assert_eq!(
            editor.current_buffer().rope().to_string(),
            "他說內人在裡面，吳先生錄了一段。\n"
        );
        // And back the other way, where the table is walked forwards over
        // what opencc hands over.
        let mut editor = editor_with("他说内人在里面，吴先生录了一段。\n");
        editor.execute("convert s c").unwrap();
        let want = editor.take_shell_request().expect("opencc has to be run");
        let How::Convert(input) = want.how else {
            panic!("a conversion is piped, not handed the screen");
        };
        let ran = run_capturing(&want.line, Some(&input)).expect("opencc runs");
        assert!(ran.ok, "{}", ran.why());
        editor.provide_conversion(&ran.said);
        assert_eq!(
            editor.current_buffer().rope().to_string(),
            "他説内人在裏面，吴先生録了一段。\n"
        );
    }

    /// The headless picture: `--shot`, and what a reviewer sees.
    #[test]
    fn a_frame_can_be_drawn_without_a_terminal() {
        let mut editor = editor_with("那年冬天，雪下得早。\n山路斷了。\n");
        let config = Config::default();
        let ime = ImeSession::empty(Scheme::LINGMING);
        let shot = frame_to_text(&mut editor, &config, &ime, 40, 8);
        // The writing is in it, one 漢字 to two cells and no space between two
        // of them — the blank a wide glyph owns is not part of the picture.
        assert!(shot.contains("那年冬天，雪下得早。"), "{shot}");
        assert!(shot.contains("山路斷了。"), "{shot}");
        // …and so is the status line, which is half of what a picture is for.
        assert!(shot.contains("NORMAL"), "{shot}");
        assert_eq!(shot.lines().count(), 8, "one line per row: {shot}");
    }

    /// The three states (#290), and the two switches that reach them.
    ///
    /// **ABC and 關 are not the same state**, although the keyboard behaves
    /// the same way in both: only one of them is yume holding the keys, and
    /// the front end holds the terminal flag that reports a bare Shift for
    /// exactly as long as that is true.
    #[test]
    fn the_input_method_has_three_states_not_two() {
        let config = Config::default();
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
        assert!(ime.is_chinese(), "a loaded table starts in Chinese");
        assert!(ime.engaged(), "…and holding the keyboard");

        // Inner switch: 中文 ⇄ ABC, which is what a lone Shift tap does. yume
        // still has the keys.
        let said = engage(&mut ime, Engagement::Ascii, &config);
        assert!(!ime.is_chinese(), "{said}");
        assert!(ime.engaged(), "ABC is still yume holding the keyboard: {said}");

        // Outer switch: hand it back. The language is left alone on the way
        // out, so coming back comes back to what you were typing in.
        let said = engage(&mut ime, Engagement::Off, &config);
        assert!(!ime.engaged(), "{said}");
        assert!(!ime.is_chinese(), "handing it back is not a language answer");

        // …and `:yume on` from there is 中文, not ABC: the way back in is
        // the way you meant to type.
        let said = engage(&mut ime, Engagement::Chinese, &config);
        assert!(ime.engaged(), "{said}");
        assert!(ime.is_chinese(), "{said}");
    }

    /// What the status line says about each of the three (#290).
    #[test]
    fn the_language_tag_goes_quiet_when_yume_has_handed_the_keyboard_back() {
        let config = Config::default();
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");

        assert!(
            language_tag(&editor, &ime).contains("靈明"),
            "中文 names the scheme"
        );
        engage(&mut ime, Engagement::Ascii, &config);
        assert_eq!(language_tag(&editor, &ime), "[ABC]");
        engage(&mut ime, Engagement::Off, &config);
        assert_eq!(
            language_tag(&editor, &ime),
            "",
            "nothing to say about a keyboard yume does not have"
        );
    }

    /// Normal mode is where you need it most, and it was the one place it
    /// never showed (#337).
    #[test]
    fn the_status_line_says_which_language_the_next_i_lands_in() {
        let config = Config::default();
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
        let editor = Editor::new();

        assert_eq!(
            language_tag(&editor, &ime),
            "",
            "Normal mode composes nothing"
        );
        assert!(
            standing_language_tag(&ime).contains("靈明"),
            "but the status line still says what `i` would land in"
        );
        engage(&mut ime, Engagement::Ascii, &config);
        assert_eq!(standing_language_tag(&ime), "[ABC]");
        engage(&mut ime, Engagement::Off, &config);
        assert_eq!(
            standing_language_tag(&ime),
            "",
            "and nothing at all once yume has the keyboard back"
        );
    }

    /// The stand-in key, and the one thing about it that is not obvious: a
    /// legacy terminal — the only kind that needs this key — cannot tell `C-^`
    /// from `C-6`, so the two names are one key (#339).
    #[test]
    fn the_stand_in_for_shift_is_the_same_key_under_either_protocol() {
        let mut config = Config::default();
        let want = language_key(&config).expect("bound out of the box");

        // Apple Terminal sends 0x1E, which crossterm reads back as `6`.
        assert!(is_language_key(
            want,
            KeyCode::Char('6'),
            KeyModifiers::CONTROL
        ));
        // A Kitty-protocol terminal sends the character that was typed.
        assert!(is_language_key(
            want,
            KeyCode::Char('^'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT
        ));
        assert!(
            !is_language_key(want, KeyCode::Char('6'), KeyModifiers::NONE),
            "a bare 6 is a count, not a language switch"
        );

        config.editor.language_key = "A-l".to_string();
        let want = language_key(&config).expect("A- is a name too");
        assert!(is_language_key(want, KeyCode::Char('l'), KeyModifiers::ALT));
        assert!(!is_language_key(
            want,
            KeyCode::Char('l'),
            KeyModifiers::CONTROL
        ));

        config.editor.language_key = "F9".to_string();
        assert_eq!(
            language_key(&config),
            Some((KeyCode::F(9), KeyModifiers::NONE))
        );

        // No key at all, and a name that is not a key, are the same answer.
        for name in ["off", "", "  ", "C-", "Ctrl-x", "F13"] {
            config.editor.language_key = name.to_string();
            assert_eq!(language_key(&config), None, "{name:?}");
        }
    }

    #[test]
    fn ascii_mode_lets_keys_fall_through_to_the_editor() {
        let mut editor = Editor::new();
        editor.on_key(Key::Char('i'));
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧\n");
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

        // Press then release Shift with nothing in between → a tap.
        let mut tap = ShiftTap::default();
        assert!(matches!(
            tap.update(&key(shift(), KeyEventKind::Press)),
            ShiftResult::Consumed
        ));
        assert!(matches!(
            tap.update(&key(shift(), KeyEventKind::Release)),
            ShiftResult::Tap(FuncKey::ShiftL)
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
    fn the_two_shifts_are_tracked_apart() {
        // One shared `down` flag made this fire a toggle while the *left*
        // Shift was still held: the right one's release ended a tap nobody
        // started. Upstream watches one key at a time, so pressing the other
        // one re-targets and the stale half is dropped on the spot.
        let (l, r) = (
            KeyCode::Modifier(ModifierKeyCode::LeftShift),
            KeyCode::Modifier(ModifierKeyCode::RightShift),
        );
        let mut tap = ShiftTap::default();
        tap.update(&key(l, KeyEventKind::Press));
        tap.update(&key(r, KeyEventKind::Press));
        assert!(
            matches!(
                tap.update(&key(r, KeyEventKind::Release)),
                ShiftResult::Tap(FuncKey::ShiftR)
            ),
            "the release that ends a tap is the key that was being watched"
        );
    }

    #[test]
    fn a_lost_release_does_not_eat_the_next_tap() {
        // ⌘-Tab away with Shift held and the release never arrives; the next
        // ordinary key then taints a tap that is no longer being made. Without
        // the reset on FocusLost, the *following* genuine tap answered
        // `Consumed` — the writer presses Shift, nothing happens, and the next
        // word goes in in the wrong language.
        let shift = || KeyCode::Modifier(ModifierKeyCode::LeftShift);
        let mut tap = ShiftTap::default();
        tap.update(&key(shift(), KeyEventKind::Press));
        // …focus leaves, the release is lost, and the loop says so.
        tap.reset();
        tap.update(&key(KeyCode::Char('n'), KeyEventKind::Press));

        tap.update(&key(shift(), KeyEventKind::Press));
        assert!(matches!(
            tap.update(&key(shift(), KeyEventKind::Release)),
            ShiftResult::Tap(FuncKey::ShiftL)
        ));
    }

    #[test]
    fn a_frame_is_skipped_only_when_nobody_would_have_seen_it() {
        // The rule the loop applies, stated where it can be checked (#314).
        // Three things override the skip: a picture was asked for, the floor
        // has been reached, or nothing is waiting.
        let skip = |waiting: bool, stale: bool, shot: bool| !(shot || !waiting || stale);

        assert!(skip(true, false, false), "input waiting and the page is fresh");
        assert!(!skip(false, false, false), "nothing waiting — draw");
        assert!(!skip(true, true, false), "the floor keeps a held key visible");
        assert!(!skip(true, false, true), ":shot must never photograph a skipped frame");
    }

    #[test]
    fn the_floor_gives_way_to_a_terminal_that_cannot_keep_up() {
        // #360: a frame is mostly *writing to a terminal*, and that write
        // blocks when the emulator falls behind. A fixed floor then does the
        // wrong thing twice over — it forces frames a congested loop cannot
        // afford, and after a very slow one it would wait so long that the
        // page reads as frozen. So: at least the floor, at most the ceiling,
        // and in between a multiple of what the last frame actually cost.
        let floor = |last: std::time::Duration| {
            (last * FRAME_SLACK).clamp(FRAME_FLOOR, FRAME_CEILING)
        };
        use std::time::Duration;
        assert_eq!(floor(Duration::from_millis(1)), FRAME_FLOOR, "a cheap frame keeps 10 fps");
        assert_eq!(floor(Duration::from_millis(50)), Duration::from_millis(200), "a dear one backs off");
        assert_eq!(floor(Duration::from_secs(2)), FRAME_CEILING, "and never backs off out of sight");
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
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, "b 吧 八\n");
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
            .insert(0, "春夏秋冬春夏秋冬春夏").expect("the fixture buffer is writable");
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
            .insert(0, "春夏秋冬春夏秋冬春夏").expect("the fixture buffer is writable");
        editor.set_soft_wrap(false);
        let buf = render_wrapped(&mut editor, &wrap_config(), 8, 5);
        assert_eq!(row_text(&buf, 0), "春夏秋冬");
        assert_eq!(row_text(&buf, 1).trim(), "");
    }

    #[test]
    fn a_continuation_row_carries_no_line_number() {
        let mut editor = Editor::new();
        editor.current_buffer_mut().insert(0, "春夏秋冬春夏").expect("the fixture buffer is writable");
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
        // ⚠️ Twelve cells, a two-cell gutter and 漢字 two cells wide: four
        // characters fit on the first row now rather than five (#484).
        assert_eq!(second.trim(), "春夏", "{first:?} / {second:?}");
    }

    #[test]
    fn j_walks_the_rows_the_reader_sees() {
        let mut editor = Editor::new();
        editor.current_buffer_mut().insert(0, "春夏秋冬春夏秋冬").expect("the fixture buffer is writable");
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
        editor.current_buffer_mut().insert(0, "春夏秋冬春夏秋冬").expect("the fixture buffer is writable");
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
        editor.current_buffer_mut().insert(0, "那年冬天下雪以後").expect("the fixture buffer is writable");
        editor.set_segmentation_visible(true);
        let mut config = wrap_config();
        config.editor.show_segmentation = true;
        // 出廠是 ink 了（#456）；這一條驗的是 tint，所以明說。
        editor.set_word_mark(yumete_cjk::WordMark::Tint);
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
        editor.current_buffer_mut().insert(0, "甲\n\n乙\n").expect("the fixture buffer is writable");
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
        editor.current_buffer_mut().insert(0, &text).expect("the fixture buffer is writable");
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
    /// 疏排 and a reading over a paragraph that wraps: the row of air is bought
    /// for every row, and a row bought has to be a row drawn.
    ///
    /// `rows_on_screen` spends the screen row — the mouse, the scroll and the
    /// caret are placed off it — while the painter used to ask `reading_line`
    /// whether to push one. On the **tail** of a wrapped line that has ruby
    /// somewhere on it the two disagreed: the group is drawn over the row its
    /// base begins on, so the tail's answer was 「nothing to draw」 and its air
    /// row was never pushed. Everything below it then sat one row higher than
    /// the page believed, and a click landed a line off.
    /// `:view-margin`'s four answers, across the page: a line with a reading
    /// **on its first row only**, wrapped into two, then a line with none.
    #[test]
    fn view_margin_decides_which_rows_buy_the_row_above() {
        use yumete_cjk::Margin;
        let rows = |margin: Margin| -> Vec<String> {
            let mut editor =
                editor_with("<ruby>永<rt>ㄩㄥˇ</rt></ruby>和九年歲在癸丑暮春之初\n後面一行");
            let mut config = Config::default();
            config.editor.line_numbers = yumete_config::LineNumbers::None;
            config.editor.command_line = false;
            editor.set_margin(margin);
            let buffer = render_with_ruby(&mut editor, &config, 16, 10);
            (0..buffer.area.height)
                .map(|y| row_text(&buffer, y).trim_end().to_string())
                .take_while(|r| !r.contains("NORMAL"))
                .collect()
        };
        let head = |r: Vec<String>, n: usize| r.into_iter().take(n).collect::<Vec<_>>();
        assert_eq!(
            head(rows(Margin::Never), 3),
            ["永和九年歲在癸丑", "暮春之初", "後面一行"],
            "no row above anything, reading and all"
        );
        assert_eq!(
            head(rows(Margin::Dense), 4),
            ["ㄩㄥˇ", "永和九年歲在癸丑", "暮春之初", "後面一行"],
            "only the row the reading is over"
        );
        assert_eq!(
            head(rows(Margin::Loose), 5),
            ["ㄩㄥˇ", "永和九年歲在癸丑", "", "暮春之初", "後面一行"],
            "both rows of the read line; the unread line keeps none"
        );
        assert_eq!(
            head(rows(Margin::Always), 6),
            ["ㄩㄥˇ", "永和九年歲在癸丑", "", "暮春之初", "", "後面一行"],
            "every row"
        );
    }

    #[test]
    fn a_loose_page_keeps_its_air_over_the_tail_of_a_read_paragraph() {
        let mut editor =
            editor_with("<ruby>永<rt>ㄩㄥˇ</rt></ruby>和九年歲在癸丑暮春之初\n後面一行");
        let mut config = Config::default();
        config.editor.line_numbers = yumete_config::LineNumbers::None;
        config.editor.command_line = false;
        editor.execute(":view-margin always").unwrap();
        let buffer = render_with_ruby(&mut editor, &config, 16, 10);
        let rows: Vec<String> = (0..buffer.area.height)
            .map(|y| row_text(&buffer, y).trim_end().to_string())
            .collect();
        assert_eq!(
            &rows[..6],
            [
                "ㄩㄥˇ",
                "永和九年歲在癸丑",
                "",
                "暮春之初",
                "",
                "後面一行",
            ],
            "air over the tail as well as over the head: {rows:#?}"
        );
        // …and the caret is placed off the same count, so a page that lost the
        // row would put it on the wrong line of the terminal.
        editor.on_key(Key::Char('j'));
        editor.on_key(Key::Char('j'));
        let (_, caret) = render_caret(&editor, &config, 16, 10);
        assert_eq!(caret.map(|p| p.y), Some(5), "{rows:#?}");
    }

    /// A reading wins the row it is drawn in — **that** row, not the whole
    /// paragraph.
    ///
    /// 平仄 and ruby share one screen row and a reading takes it outright
    /// (interleaving them per character would leave a 詞譜 column with holes,
    /// and a hole reads as 輕聲 rather than as 「something else is written
    /// here」). But the question was asked of the *line*: one `<ruby>` at the
    /// end of a paragraph deleted the 平仄 from every row of it, including the
    /// rows the reading is nowhere near.
    #[test]
    fn a_reading_takes_its_own_row_from_the_meter_and_no_other() {
        let mut editor = metered("春眠不覺曉春眠不覺曉<ruby>春<rt>ㄔㄨㄣ</rt></ruby>\n");
        let mut config = Config::default();
        config.editor.line_numbers = LineNumbers::None;
        config.editor.command_line = false;
        config.editor.show_segmentation = false;
        let buffer = render_with_ruby(&mut editor, &config, 12, 10);
        let rows: Vec<String> = (0..buffer.area.height)
            .map(|y| row_text(&buffer, y).trim_end().to_string())
            .collect();
        assert_eq!(rows[0], "○ ○ ● ○ ● ○", "the first row keeps its 平仄: {rows:#?}");
        assert_eq!(rows[1], "春眠不覺曉春", "{rows:#?}");
        assert!(rows[2].contains('ㄔ'), "and the second row its reading: {rows:#?}");
        assert_eq!(rows[3], "眠不覺曉春", "{rows:#?}");
    }

}
