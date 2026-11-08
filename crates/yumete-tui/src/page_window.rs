//! **The preview, drawn** (2026-11-07; a tab of its own since 2026-11-08).
//!
//! The editor keeps the preview tabs and which windows show them
//! (`editor/page_preview.rs`); this asks tinymist for each one's page and
//! draws it. Two halves, as the formula float has ([`crate::maths`]):
//!
//! - [`PagePreview`], owned by the loop, talks to the tinymist the editor
//!   already runs as its Typst language server ([`crate::server::Servers`]):
//!   `tinymist.exportPng` with `write: false` for the page, made from the
//!   unsaved text; and, for each preview tab, a preview task of its own whose
//!   data plane says where the cursor's text is on the page (`jump,<page>
//!   <x> <y>`), asked with `tinymist.scrollPreview`. When to ask again is
//!   [`yumete_core::preview::Refresh`]: one question out at a time, new text
//!   once the typing has been still for 300 ms. **Only previews on the
//!   screen are asked anything**; a task, once started, runs until its tab
//!   closes.
//! - [`draw`], called by the frame for each window showing a preview, lays
//!   that preview's page into the window as placeholder cells
//!   ([`crate::graphics`]), scrolled to the cursor's place when the page is
//!   as wide as the window.
//!
//! **Shown as printed** unless `:view-preview-theme` says otherwise. **Only
//! tinymist** draws it, and only one new enough to hand the page over in
//! memory ([`yumete_core::preview::LEAST_TINYMIST`]); otherwise the window
//! says so and `:view-preview` opens the browser's preview instead. The
//! research is `local/survey-0.20/graphics.md` and its follow-up.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ratatui::layout::{Alignment, Rect};
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};

use yumete_core::editor::{Editor, PageStep, PreviewFit};
use yumete_core::lsp::Executed;
use yumete_core::preview::{self, Jump, Refresh, Want};
use yumete_core::say;
use yumete_core::syntax::Syntax;
use yumete_core::window::WindowId;

use crate::server::{Doorbell, Servers, Typesetter};

/// **A page, made**: the PNG and what it is a picture of.
#[derive(Debug)]
pub struct Page {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// The pixels to the inch it was made at.
    pub ppi: u32,
    /// Which page, from 1.
    pub number: usize,
    /// How many pages the document has, when tinymist said.
    pub total: Option<usize>,
    /// The PNG's own hash: the terminal is not handed the same picture twice.
    pub hash: u64,
}

impl Page {
    /// A page from a PNG, or `None` for bytes that are not one.
    pub fn from_png(png: Vec<u8>, ppi: u32, number: usize, total: Option<usize>) -> Option<Page> {
        use std::hash::{Hash, Hasher};
        let (width, height) = crate::graphics::png_size(&png)?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        png.hash(&mut hasher);
        Some(Page { width, height, ppi: ppi.max(1), number, total, hash: hasher.finish(), png })
    }

    /// The page's size in inches.
    pub fn inches(&self) -> (f64, f64) {
        (self.width as f64 / self.ppi as f64, self.height as f64 / self.ppi as f64)
    }
}

/// **One pixel of the page in the theme's colours** (2026-11-08,
/// `:view-preview-theme`): its lightness, from black ink to white paper,
/// taken from the theme's text colour to its background, and whatever
/// colour it had over its grey carried across, so a red heading stays red
/// and a blue link blue in light themes and dark alike.
pub fn recolour(pixel: [u8; 3], paper: [u8; 3], ink: [u8; 3]) -> [u8; 3] {
    let [r, g, b] = pixel.map(f32::from);
    let grey = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let light = grey / 255.0;
    let mut out = [0u8; 3];
    for (i, channel) in out.iter_mut().enumerate() {
        let mapped = f32::from(ink[i]) + light * (f32::from(paper[i]) - f32::from(ink[i]));
        *channel = (mapped + f32::from(pixel[i]) - grey).round().clamp(0.0, 255.0) as u8;
    }
    out
}

/// **A page's PNG in the theme's colours** ([`recolour`], each pixel), an
/// opaque RGB PNG; what was transparent is paper. `None` for bytes that are
/// not a PNG.
pub fn recolour_png(png: &[u8], paper: [u8; 3], ink: [u8; 3]) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).ok()?;
    let (colour, _) = reader.output_color_type();
    let channels = match colour {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    let pixels = &buf[..frame.buffer_size()];
    let mut rgb = Vec::with_capacity(pixels.len() / channels * 3);
    // Pages are mostly paper and a few inks: each colour worked out once.
    let mut seen: HashMap<[u8; 3], [u8; 3]> = HashMap::new();
    for pixel in pixels.chunks_exact(channels) {
        let (colour, alpha) = match channels {
            1 => ([pixel[0]; 3], 255),
            2 => ([pixel[0]; 3], pixel[1]),
            3 => ([pixel[0], pixel[1], pixel[2]], 255),
            _ => ([pixel[0], pixel[1], pixel[2]], pixel[3]),
        };
        // Over white, as the page would print.
        let over = colour.map(|c| ((u32::from(c) * u32::from(alpha) + 255 * (255 - u32::from(alpha))) / 255) as u8);
        let made = *seen.entry(over).or_insert_with(|| recolour(over, paper, ink));
        rgb.extend_from_slice(&made);
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, frame.width, frame.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&rgb).ok()?;
    }
    Some(out)
}

/// What a page in the theme's colours was made from: the page's hash, the
/// paper and the ink.
type Tint = (u64, [u8; 3], [u8; 3]);

/// **What a preview's window draws**, shared by the loop that fetches it
/// and the frame that draws it — the frame is drawn from places that know
/// nothing of the loop (`--shot`, the tests), as the formula float's is.
#[derive(Debug, Default)]
struct Board {
    page: Option<Arc<Page>>,
    /// What to say instead of the page, or under it: tinymist starting, too
    /// old, missing, or what it said was wrong.
    note: Option<String>,
    /// Where the cursor's text is, as the data plane said last.
    jump: Option<Jump>,
    /// Rows scrolled by hand since the cursor last moved.
    nudge: i32,
    /// The page turned to by hand (`J`, `K`, `5G`) since the cursor last
    /// moved; `None` shows the cursor's.
    turned: Option<usize>,
    /// Where the scroll stood before the hand moved it: the cursor's row less
    /// a third of the window, or what it was when the cursor's page is not
    /// the one drawn.
    base: i32,
    /// The window's picture area in cells, when last drawn.
    cells: Option<(u16, u16)>,
    /// The page in the theme's colours, made once from the page whose hash
    /// it names, in the paper and ink it names (`:view-preview-theme`).
    tinted: Option<(Tint, Arc<Page>)>,
}

impl Board {
    /// **The page in the theme's `paper` and `ink`**, made the first time it
    /// is asked for and kept until the page or the colours change; the page
    /// as it is when it cannot be read.
    fn tinted(&mut self, page: &Arc<Page>, paper: [u8; 3], ink: [u8; 3]) -> Arc<Page> {
        let key = (page.hash, paper, ink);
        if let Some((made_for, made)) = &self.tinted {
            if *made_for == key {
                return made.clone();
            }
        }
        let made = recolour_png(&page.png, paper, ink)
            .and_then(|png| Page::from_png(png, page.ppi, page.number, page.total))
            .map(Arc::new)
            .unwrap_or_else(|| page.clone());
        self.tinted = Some((key, made.clone()));
        made
    }
}

/// **Every preview's board**, by its tab's id (2026-11-08: one a preview,
/// where there was one for the editor).
static BOARDS: Mutex<Option<HashMap<u64, Board>>> = Mutex::new(None);

/// **The page `J`, `K` or `5G` turns to** from page `now`: never before
/// the first, nor past the last when the total is known.
pub fn turn(now: usize, total: Option<usize>, step: PageStep) -> usize {
    let to = match step {
        PageStep::Goto(n) => n,
        PageStep::Pages(n) => now.saturating_add_signed(n as isize),
        _ => now,
    };
    total.map_or(to, |t| to.min(t.max(1))).max(1)
}

impl Board {
    /// The page being shown: the one turned to by hand, else the cursor's.
    fn shown(&self) -> usize {
        self.turned.or(self.jump.map(|j| j.page)).unwrap_or(1)
    }
}

/// Every board, locked.
fn boards() -> std::sync::MutexGuard<'static, Option<HashMap<u64, Board>>> {
    BOARDS.lock().unwrap_or_else(|e| e.into_inner())
}

/// **Preview `id`'s board**, made empty when it has none yet, for `work`.
fn with_board<T>(id: u64, work: impl FnOnce(&mut Board) -> T) -> T {
    let mut all = boards();
    work(all.get_or_insert_with(HashMap::new).entry(id).or_default())
}

/// **Put a page on preview `id`'s board by hand** — for a test, which has no
/// tinymist.
pub fn preload(id: u64, page: Page, jump: Option<Jump>) {
    with_board(id, |board| {
        board.page = Some(Arc::new(page));
        board.jump = jump;
        board.note = None;
        board.nudge = 0;
        board.turned = None;
    });
}

/// Clear every board — a test's.
pub fn clear() {
    *boards() = None;
}

/// Whether preview `id` has a board — what a closed tab leaves none of.
pub fn has_a_board(id: u64) -> bool {
    boards().as_ref().is_some_and(|all| all.contains_key(&id))
}

static BELL: OnceLock<Doorbell> = OnceLock::new();

/// Wake the loop this way when the data plane says where the cursor is — the
/// bell the language servers ring.
pub fn ring_on_news(bell: Doorbell) {
    let _ = BELL.set(bell);
}

/// How long the cursor has to stay put before tinymist is asked where it is
/// on the page: a held `j` is not a question a line.
const FOLLOW_AFTER: Duration = Duration::from_millis(80);

/// The page size taken before the first page says its own: A4.
const A4: (f64, f64) = (8.27, 11.69);

/// What the loop does after a turn of [`PagePreview::follow`].
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    /// Something new to draw.
    Redraw,
    /// **Preview `preview` cannot be drawn**, and it was opened by
    /// `:view-preview`: close it and open the browser's preview of `path`,
    /// saying `why`.
    Browser { preview: u64, path: PathBuf, why: String },
}

/// The preview task inside tinymist: asked for, running, or not to be had.
#[derive(Debug, Default, PartialEq, Eq)]
enum Task {
    #[default]
    Off,
    Asked(i64),
    Running,
    Broken,
}

/// The data plane: where the jumps arrive, and the socket, held so closing
/// the window can close it.
struct Plane {
    jumps: mpsc::Receiver<Jump>,
    socket: Option<std::net::TcpStream>,
}

impl Drop for Plane {
    fn drop(&mut self) {
        if let Some(socket) = &self.socket {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// **The previews' conversations with tinymist**, owned by the loop: one a
/// preview tab, by its id.
#[derive(Default)]
pub struct PagePreview {
    talks: HashMap<u64, Talk>,
}

/// **One preview's conversation with tinymist.**
#[derive(Default)]
struct Talk {
    /// The task's name in tinymist ([`preview::task_id`]).
    task_id: String,
    /// The file it typesets.
    main: PathBuf,
    /// Whether it was opened by `:view-preview`, which goes to the browser
    /// when the window cannot be drawn.
    fallback: bool,
    /// Whether it was on the screen at the last turn.
    shown: bool,
    refresh: Refresh,
    /// The page asked for and not yet answered: its id and what it was.
    asked: Option<(i64, Want)>,
    /// What would be asked for now — kept for [`PagePreview::due_in`].
    want: Option<Want>,
    task: Task,
    plane: Option<Plane>,
    /// The cursor's place as last told to tinymist, and as last seen with
    /// when it got there.
    told: Option<(PathBuf, usize, usize)>,
    seen: Option<((PathBuf, usize, usize), Instant)>,
}

impl PagePreview {
    /// **Preview `id` was just opened beside the text**, by `:view-preview`
    /// (`fallback`) or by `:view-preview-window`.
    pub fn opened(&mut self, id: u64, fallback: bool) {
        self.talks.entry(id).or_default().fallback = fallback;
    }

    /// **One turn**: let the closed previews go, then for each preview on
    /// the screen start its task, take the answers, follow the cursor, ask
    /// for the page when it is due. Never waits.
    pub fn follow(&mut self, editor: &mut Editor, servers: &mut Servers, config: &yumete_config::Config) -> Outcome {
        // Typst's only: a Markdown preview is [`crate::md_window`]'s.
        let typst = |editor: &Editor, id: u64| editor.preview_kind_of(id) == Some(yumete_core::buffer::PreviewKind::Typst);
        let previews: Vec<(u64, PathBuf)> = editor.previews().into_iter().filter(|(id, _)| typst(editor, *id)).collect();
        let mut redraw = false;
        let gone: Vec<u64> = self.talks.keys().copied().filter(|id| !previews.iter().any(|(p, _)| p == id)).collect();
        for id in gone {
            if let Some(mut talk) = self.talks.remove(&id) {
                talk.stop_the_task(servers);
            }
            crate::graphics::drop_pages_of(id);
            if let Some(all) = boards().as_mut() {
                all.remove(&id);
            }
            redraw = true;
        }
        // The scroll keys move the preview the keys are in.
        let keys = editor.preview_with_the_keys().filter(|&id| typst(editor, id));
        let steps = match keys {
            Some(_) => editor.take_page_steps(),
            None => Vec::new(),
        };
        if let Some(id) = keys.filter(|_| !steps.is_empty()) {
            with_board(id, |board| {
                let rows = board.cells.map_or(1, |(_, rows)| rows.max(1) as i32);
                for step in steps {
                    board.nudge = match step {
                        PageStep::Rows(n) => board.nudge.saturating_add(n),
                        PageStep::Halves(n) => board.nudge.saturating_add(n * (rows / 2).max(1)),
                        PageStep::Screens(n) => board.nudge.saturating_add(n * rows),
                        PageStep::Top => i32::MIN / 4,
                        PageStep::Bottom => i32::MAX / 4,
                        // A page set across does not move sideways.
                        PageStep::Across(_) => board.nudge,
                        // **Another page, from its top.** The cursor stays
                        // where it is; the page it is on comes back when it
                        // moves.
                        PageStep::Pages(_) | PageStep::Goto(_) => {
                            let total = board.page.as_ref().and_then(|p| p.total);
                            board.turned = Some(turn(board.shown(), total, step));
                            board.base = 0;
                            i32::MIN / 4
                        }
                    };
                }
            });
            redraw = true;
        }
        for (id, main) in previews {
            let shown = editor.preview_on_screen(id);
            let talk = self.talks.entry(id).or_default();
            if talk.task_id.is_empty() {
                talk.task_id = preview::task_id(id);
            }
            talk.main = main;
            talk.shown = shown;
            if !shown {
                continue;
            }
            match talk.follow(id, editor, servers, config) {
                Outcome::Nothing => {}
                Outcome::Redraw => redraw = true,
                browser => return browser,
            }
        }
        match redraw {
            true => Outcome::Redraw,
            false => Outcome::Nothing,
        }
    }

    /// **How long the loop may sleep**: until the typing has been still long
    /// enough, or the cursor has stayed put long enough to be followed, in
    /// any preview on the screen. Past that it waits on others: the server's
    /// own wake while it is being handed the text, or the bell when an
    /// answer comes — never a spin.
    pub fn due_in(&self) -> Option<Duration> {
        self.talks.values().filter(|t| t.shown).filter_map(Talk::due_in).min()
    }

    /// The task names of the previews whose task tinymist was asked to
    /// start — for a test.
    pub fn tasks(&self) -> Vec<String> {
        let mut out: Vec<String> =
            self.talks.values().filter(|t| t.task != Task::Off).map(|t| t.task_id.clone()).collect();
        out.sort();
        out
    }
}

impl Talk {
    /// **One turn for preview `id`**, on the screen.
    fn follow(&mut self, id: u64, editor: &mut Editor, servers: &mut Servers, config: &yumete_config::Config) -> Outcome {
        let now = Instant::now();
        let mut redraw = false;
        // **Whether tinymist can draw it.** A server that went takes its
        // answers with it: what was asked is asked again once one is back.
        let why = match servers.typesetter(config, editor) {
            Typesetter::Ready => None,
            Typesetter::Starting => Some((say!("preview.typst-starting"), false)),
            Typesetter::Missing => Some((say!("preview.no-tinymist"), true)),
            Typesetter::TooOld(name) => Some((say!("preview.tinymist-too-old", name), true)),
        };
        if let Some((why, for_good)) = why {
            if self.asked.take().is_some() {
                self.refresh.lost();
            }
            self.task = Task::Off;
            self.plane = None;
            if for_good && self.fallback {
                return Outcome::Browser { preview: id, path: self.main.clone(), why };
            }
            let why = match for_good {
                true => say!("preview.or-the-browser", why),
                false => why,
            };
            return match self.note(id, Some(why)) {
                true => Outcome::Redraw,
                false => Outcome::Nothing,
            };
        }
        if self.task == Task::Off {
            let arguments = preview::start_preview_arguments(&self.task_id, &self.main);
            if let Some(ask) = servers.execute("tinymist.doStartPreview", &arguments, true) {
                self.task = Task::Asked(ask);
            }
        }
        if let Task::Asked(ask) = self.task {
            match servers.take_executed(ask) {
                Some(Ok(Some(Executed::Preview { data_plane }))) => {
                    self.plane = Some(dial(data_plane));
                    self.task = Task::Running;
                }
                // No data plane: the page is drawn and does not follow.
                Some(_) => self.task = Task::Broken,
                None => {}
            }
        }
        if let Some((ask, want)) = self.asked.clone() {
            if let Some(answer) = servers.take_executed(ask) {
                self.asked = None;
                self.refresh.answered();
                redraw = true;
                self.landed(id, answer, &want);
            }
        }
        if let Some(plane) = &self.plane {
            with_board(id, |board| {
                while let Ok(jump) = plane.jumps.try_recv() {
                    if board.jump != Some(jump) {
                        board.jump = Some(jump);
                        board.nudge = 0;
                        board.turned = None;
                        redraw = true;
                    }
                }
            });
        }
        // **Follow the cursor**: where its text is on the page, asked once it
        // has stayed put a moment and the server has the text it stands in —
        // only while the keys are on this preview's book; otherwise the
        // preview keeps its place.
        let target = editor.page_target(id);
        if self.task == Task::Running {
            if let Some(path) = target.as_ref().and_then(|t| t.path.as_ref()) {
                let target = target.as_ref().expect("a target with a path");
                // **The place after the character the cursor stands on**:
                // tinymist looks at what comes before the place it is given,
                // and at the start of a line that is the line break, which is
                // on no page (measured with 0.15.12: no jump at character 0).
                let character = (target.character + 1).min(target.line_length);
                let place = (path.clone(), target.line, character);
                match &self.seen {
                    Some((seen, at)) if *seen == place => {
                        let settled = now.saturating_duration_since(*at) >= FOLLOW_AFTER;
                        if settled && self.told.as_ref() != Some(&place) && server_has_it(editor, servers) {
                            let arguments = preview::scroll_preview_arguments(&self.task_id, path, target.line, character);
                            if servers.execute("tinymist.scrollPreview", &arguments, false).is_some() {
                                self.told = Some(place);
                            }
                        }
                    }
                    _ => self.seen = Some((place, now)),
                }
            }
        }
        // **The page, when it is due.**
        let buffer = editor.current_buffer();
        self.refresh.edited(buffer.id().wrapping_mul(1_000_003) ^ buffer.revision(), now);
        let want = with_board(id, |board| {
            let total = board.page.as_ref().and_then(|p| p.total);
            let number = board.shown();
            let number = total.map_or(number, |t| number.min(t.max(1)));
            let inches = board.page.as_ref().map_or(A4, |p| p.inches());
            let shown = board.page.as_ref().filter(|p| p.number == number).map(|p| p.ppi);
            let ppi = ppi_to_ask(editor.preview_fit(), board.cells, crate::graphics::cell_pixels(), inches, shown);
            Want { main: self.main.clone(), page: number, ppi, text: servers.typst_told() }
        });
        if self.refresh.due(&want, now) && server_has_it(editor, servers) {
            let arguments = preview::export_png_arguments(&want.main, want.page, want.ppi);
            if let Some(ask) = servers.execute("tinymist.exportPng", &arguments, true) {
                self.refresh.ask(want.clone());
                self.asked = Some((ask, want.clone()));
                if with_board(id, |board| board.page.is_none()) {
                    redraw |= self.note(id, Some(say!("preview.typesetting")));
                }
            }
        }
        self.want = Some(want);
        match redraw {
            true => Outcome::Redraw,
            false => Outcome::Nothing,
        }
    }

    /// What came back for `want`: the page, or what went wrong.
    fn landed(&mut self, id: u64, answer: Result<Option<Executed>, String>, want: &Want) {
        let page = match answer {
            Ok(Some(Executed::Pages { total, pages })) => match pages.into_iter().next() {
                Some((_, data)) => preview::decode_base64(&data)
                    .and_then(|png| Page::from_png(png, want.ppi, want.page, total))
                    .ok_or_else(|| say!("preview.failed", "PNG")),
                // Past the end (the last page went): the next want asks for
                // the last one, which the total now bounds.
                None => {
                    with_board(id, |board| {
                        if let (Some(total), Some(jump)) = (total, board.jump.as_mut()) {
                            jump.page = jump.page.min(total.max(1));
                        }
                    });
                    return;
                }
            },
            Ok(_) => Err(say!("preview.failed", "tinymist")),
            Err(why) => Err(say!("preview.failed", why.lines().next().unwrap_or(&why).trim())),
        };
        match page {
            Ok(page) => with_board(id, |board| {
                board.page = Some(Arc::new(page));
                board.note = None;
            }),
            Err(why) => {
                self.note(id, Some(why));
            }
        }
    }

    /// Say `note` in preview `id`'s window; whether that changed anything.
    fn note(&mut self, id: u64, note: Option<String>) -> bool {
        with_board(id, |board| {
            let changed = board.note != note;
            board.note = note;
            changed
        })
    }

    fn stop_the_task(&mut self, servers: &mut Servers) {
        if matches!(self.task, Task::Asked(_) | Task::Running) {
            servers.execute("tinymist.doKillPreview", &preview::kill_preview_arguments(&self.task_id), false);
        }
        self.task = Task::Off;
        self.plane = None;
        self.told = None;
    }

    /// How long the loop may sleep for this preview — see
    /// [`PagePreview::due_in`].
    fn due_in(&self) -> Option<Duration> {
        let now = Instant::now();
        let refresh = self.want.as_ref().and_then(|want| self.refresh.wake_in(want, now));
        let follow = match (&self.seen, self.task == Task::Running) {
            (Some((place, at)), true) if self.told.as_ref() != Some(place) => {
                FOLLOW_AFTER.checked_sub(now.saturating_duration_since(*at)).filter(|left| !left.is_zero())
            }
            _ => None,
        };
        [refresh, follow].into_iter().flatten().min()
    }
}

/// Whether tinymist holds the text being typed: a page or a place asked
/// before the edit reaches it is about the text before the edit.
fn server_has_it(editor: &Editor, servers: &Servers) -> bool {
    let buffer = editor.current_buffer();
    buffer.syntax() != Syntax::Typst || buffer.path().is_none() || servers.have_the_latest(editor)
}

/// **Dial the data plane** on `port` and listen to it on a thread of its own:
/// each `jump` goes down the channel and rings the bell. The socket is closed
/// with the [`Plane`].
fn dial(port: u16) -> Plane {
    let (send, jumps) = mpsc::channel();
    let at = format!("127.0.0.1:{port}");
    let socket = std::net::TcpStream::connect(&at).ok();
    let held = socket.as_ref().and_then(|s| s.try_clone().ok());
    if let Some(socket) = socket {
        let _ = std::thread::Builder::new().name("page-plane".into()).spawn(move || listen(socket, &at, send));
    }
    Plane { jumps, socket: held }
}

/// The data plane's reader: the handshake, then every message, of which only
/// the jumps are kept — the document itself is read and thrown away.
fn listen(mut socket: std::net::TcpStream, at: &str, send: mpsc::Sender<Jump>) {
    use std::io::{Read, Write};
    let key = preview::key(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(7, |d| d.as_nanos()));
    if socket.write_all(preview::handshake(at, &key).as_bytes()).is_err() {
        return;
    }
    let mut head = Vec::new();
    let mut chunk = [0u8; 16384];
    let rest = loop {
        let Ok(n) = socket.read(&mut chunk) else { return };
        if n == 0 {
            return;
        }
        head.extend_from_slice(&chunk[..n]);
        if let Some(end) = head.windows(4).position(|w| w == b"\r\n\r\n") {
            if !preview::accepted(&String::from_utf8_lossy(&head[..end])) {
                return;
            }
            break head.split_off(end + 4);
        }
        if head.len() > 65536 {
            return;
        }
    };
    let mut incoming = preview::Incoming::default();
    incoming.push(&rest);
    loop {
        while let Some((opcode, body)) = incoming.next_message() {
            match opcode {
                1 | 2 if body.starts_with(b"jump,") => {
                    if let Some(jump) = std::str::from_utf8(&body).ok().and_then(preview::jump) {
                        if send.send(jump).is_err() {
                            return;
                        }
                        if let Some(ring) = BELL.get() {
                            ring();
                        }
                    }
                }
                preview::CLOSE => return,
                _ => {}
            }
        }
        match socket.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(n) => incoming.push(&chunk[..n]),
        }
    }
}

/// **The pixels to the inch to make the page at**, so the terminal draws it
/// at the size it was made and never resamples it: as wide as the window's
/// pixels, or, for the whole page, as large as fits. Kept between 24 and 600,
/// and under 8000 pixels a side. `cells` is the picture area, `None` before
/// the window was first drawn.
pub fn ppi_for(fit: PreviewFit, cells: Option<(u16, u16)>, cell: (u16, u16), inches: (f64, f64)) -> u32 {
    let Some((columns, rows)) = cells else { return 96 };
    let (w, h) = (inches.0.max(0.5), inches.1.max(0.5));
    let wide = columns.max(1) as f64 * cell.0.max(1) as f64 / w;
    let ppi = match fit {
        PreviewFit::Width => wide,
        PreviewFit::Page => wide.min(rows.max(1) as f64 * cell.1.max(1) as f64 / h),
    };
    let most = (8000.0 / w).min(8000.0 / h).min(600.0);
    ppi.clamp(24.0, most.max(24.0)).round() as u32
}

/// **The ppi to ask for, holding on to the one shown** (2026-11-08): the page
/// size is read back from a PNG Typst rounded to whole pixels, so near a
/// half-integer the ideal ppi came out P from P+1 and P+1 from P, and the
/// window asked for one and then the other forever, the page shaking between
/// two sizes. Rounding moves it by less than 0.06, so a difference of one is
/// that and nothing else.
pub fn ppi_to_ask(
    fit: PreviewFit,
    cells: Option<(u16, u16)>,
    cell: (u16, u16),
    inches: (f64, f64),
    shown: Option<u32>,
) -> u32 {
    let ppi = ppi_for(fit, cells, cell, inches);
    match shown {
        Some(shown) if ppi.abs_diff(shown) <= 1 => shown,
        _ => ppi,
    }
}

/// **The cells a page takes when it is as wide as the window**: `columns`
/// wide, and as many rows as keep its shape — at most what the placeholders
/// can number, the width shrinking with them past that.
pub fn width_cells(picture: (u32, u32), cell: (u16, u16), columns: u16) -> (u16, u16) {
    let most = crate::graphics::MOST_CELLS;
    let (w, h) = (picture.0.max(1) as f64, picture.1.max(1) as f64);
    let (cw, ch) = (cell.0.max(1) as f64, cell.1.max(1) as f64);
    let columns = columns.clamp(1, most) as f64;
    let rows = (columns * cw * h / (w * ch)).round().max(1.0);
    match rows > most as f64 {
        true => (((most as f64) * ch * w / (h * cw)).floor().max(1.0) as u16, most),
        false => (columns as u16, rows as u16),
    }
}

/// **The first row to draw**: the cursor's row a third of the way down the
/// window, moved by hand, and kept inside the page. Returns it and the base
/// the hand's rows count from.
pub fn scroll_top(cursor_row: Option<i32>, base: i32, nudge: i32, rows: u16, window: u16) -> (u16, i32) {
    let base = cursor_row.map_or(base, |row| row - window as i32 / 3);
    let last = rows.saturating_sub(window) as i32;
    let top = base.saturating_add(nudge).clamp(0, last.max(0));
    (top as u16, base)
}

/// **Draw the preview window `id` shows into `area`.**
pub fn draw(frame: &mut ratatui::Frame, editor: &Editor, config: &yumete_config::Config, area: Rect, id: WindowId) {
    let Some(preview) = editor.preview_in(id) else { return };
    let ink = crate::theme::Palette::of(config);
    frame.render_widget(Paragraph::new("").style(ink.page()), area);
    // The last row says which page; the rest is the picture's.
    let (room, foot) = match area.height >= 3 {
        true => (
            Rect { height: area.height - 1, ..area },
            Some(Rect { y: area.y + area.height - 1, height: 1, ..area }),
        ),
        false => (area, None),
    };
    let mut all = boards();
    let board = all.get_or_insert_with(HashMap::new).entry(preview).or_default();
    board.cells = Some((room.width, room.height));
    let can = crate::graphics::support() != crate::graphics::Support::None;
    let page = board.page.clone().filter(|_| can);
    // **In the theme's colours** (2026-11-08): the paper the page's ground
    // and the ink its writing, made once a page.
    let page = match page {
        Some(page) if editor.preview_theme() => {
            let (paper, text) = (ink.paper_bytes(), ink.caret());
            Some(board.tinted(&page, [paper.0, paper.1, paper.2], [text.0, text.1, text.2]))
        }
        page => page,
    };
    let quiet = ink.page().fg(ink.quiet());
    let Some(page) = page else {
        let said = match can {
            false => say!("preview.no-pictures"),
            true => board.note.clone().unwrap_or_default(),
        };
        let lines = (said.chars().count() as u16 / room.width.max(1)) + 1;
        let middle = Rect { y: room.y + room.height.saturating_sub(lines) / 2, height: lines.min(room.height), ..room };
        frame.render_widget(Paragraph::new(said).style(quiet).alignment(Alignment::Center).wrap(Wrap { trim: true }), middle);
        return;
    };
    let cell = crate::graphics::cell_pixels();
    match editor.preview_fit() {
        PreviewFit::Width => {
            let (columns, rows) = width_cells((page.width, page.height), cell, room.width);
            let cursor_row = board
                .jump
                .filter(|j| j.page == page.number)
                .map(|j| (j.y * page.ppi as f64 / 72.0 / page.height as f64 * rows as f64) as i32);
            let (top, base) = scroll_top(cursor_row, board.base, board.nudge, rows, room.height);
            // Only a hand scroll is folded back into the page: storing the
            // clamp as one made a one-row wobble of the cursor's row move the
            // whole page.
            if board.nudge != 0 {
                board.nudge = top as i32 - base;
            }
            board.base = base;
            let picture = crate::graphics::show_page(preview, page.hash, &page.png, columns, rows);
            let spot = Rect {
                x: room.x + room.width.saturating_sub(columns) / 2,
                y: room.y,
                width: columns,
                height: (rows - top).min(room.height),
            };
            crate::graphics::put_picture_from(frame.buffer_mut(), spot, picture, top, 0);
        }
        PreviewFit::Page => {
            let (columns, rows) = crate::maths::cells_for((page.width, page.height), cell, (room.width, room.height));
            let picture = crate::graphics::show_page(preview, page.hash, &page.png, columns, rows);
            let spot = Rect { x: room.x + room.width.saturating_sub(columns) / 2, y: room.y, width: columns, height: rows };
            crate::graphics::put_picture(frame.buffer_mut(), spot, picture);
        }
    }
    if let Some(foot) = foot {
        let number = match page.total {
            Some(total) => format!("{}/{total} ", page.number),
            None => format!("{} ", page.number),
        };
        frame.render_widget(Paragraph::new(Line::from(number)).style(quiet).alignment(Alignment::Right), foot);
        if let Some(note) = &board.note {
            let left = Rect { width: foot.width.saturating_sub(8), ..foot };
            frame.render_widget(Paragraph::new(Line::from(format!(" {note}"))).style(quiet), left);
        }
    }
}

/// **Why the page window cannot be drawn for a file of `syntax`**, or `None`
/// when it can — as far as can be known before it is open: a tinymist still
/// starting may yet turn out too old, and the window says so then.
pub fn why_not(syntax: Syntax, servers: &Servers, config: &yumete_config::Config, editor: &Editor) -> Option<String> {
    if syntax != Syntax::Typst {
        return Some(say!("preview.typst-only"));
    }
    if crate::graphics::support() == crate::graphics::Support::None {
        return Some(say!("preview.no-pictures"));
    }
    match servers.typesetter(config, editor) {
        Typesetter::Missing => Some(say!("preview.no-tinymist")),
        Typesetter::TooOld(name) => Some(say!("preview.tinymist-too-old", name)),
        Typesetter::Starting | Typesetter::Ready => None,
    }
}

/// A path made absolute the way the language server was handed it.
pub fn absolute(path: &Path) -> PathBuf {
    match path.is_absolute() {
        true => path.to_path_buf(),
        false => std::env::current_dir().unwrap_or_default().join(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **As wide as the window's pixels**, so the terminal never resamples
    /// the page; the whole page as large as fits; kept in bounds.
    #[test]
    fn the_page_is_made_at_the_window_s_size() {
        // 80 columns of 10 px over an A5 page 5.83 in wide: 137 ppi.
        assert_eq!(ppi_for(PreviewFit::Width, Some((80, 40)), (10, 20), (5.83, 8.27)), 137);
        // The whole page: the height decides, 40 rows of 20 px over 8.27 in.
        assert_eq!(ppi_for(PreviewFit::Page, Some((80, 40)), (10, 20), (5.83, 8.27)), 97);
        assert_eq!(ppi_for(PreviewFit::Width, None, (10, 20), A4), 96, "not drawn yet");
        assert_eq!(ppi_for(PreviewFit::Width, Some((1, 1)), (10, 20), A4), 24, "never below 24");
        assert!(ppi_for(PreviewFit::Width, Some((2000, 40)), (40, 80), A4) <= 600);
    }

    #[test]
    fn a_page_as_wide_as_the_window_keeps_its_shape() {
        // 839 × 1191 px (A5 at 144 ppi), 10 × 20 px cells, 60 columns: 43 rows.
        assert_eq!(width_cells((839, 1191), (10, 20), 60), (60, 43));
        // Too tall to number: the rows stop at 297 and the width shrinks.
        let (columns, rows) = width_cells((100, 10_000), (10, 20), 200);
        assert_eq!(rows, crate::graphics::MOST_CELLS);
        assert!((1..200).contains(&columns), "{columns}");
    }

    /// The cursor's row a third of the way down, moved by hand, inside the
    /// page.
    #[test]
    fn the_page_scrolls_to_the_cursor_and_by_hand() {
        // 100 rows of page in a 30-row window, the cursor on row 50.
        assert_eq!(scroll_top(Some(50), 0, 0, 100, 30), (40, 40));
        assert_eq!(scroll_top(Some(50), 0, 5, 100, 30), (45, 40));
        assert_eq!(scroll_top(Some(5), 0, 0, 100, 30), (0, -5), "not above the page");
        assert_eq!(scroll_top(Some(99), 0, 0, 100, 30), (70, 89), "not below it");
        assert_eq!(scroll_top(None, 12, 3, 100, 30), (15, 12), "no cursor here: where it was");
        assert_eq!(scroll_top(None, 0, i32::MAX / 4, 100, 30), (70, 0));
        assert_eq!(scroll_top(Some(10), 0, 0, 20, 30), (0, 0), "a page shorter than the window");
    }

    #[test]
    fn a_page_is_its_png() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(839u32.to_be_bytes());
        png.extend(1191u32.to_be_bytes());
        let page = Page::from_png(png.clone(), 144, 21, Some(49)).unwrap();
        assert_eq!((page.width, page.height, page.number), (839, 1191, 21));
        let (w, h) = page.inches();
        assert!((w - 5.826).abs() < 0.01 && (h - 8.27).abs() < 0.01, "{w} {h}");
        assert_eq!(page.hash, Page::from_png(png, 72, 1, None).unwrap().hash, "the hash is the picture's");
        assert!(Page::from_png(b"GIF89a".to_vec(), 144, 1, None).is_none());
    }

    /// A tinymist new enough to draw the page window: `YUMETE_TINYMIST`, the
    /// one on `PATH`, or the one VS Code's extension carries — whichever
    /// says a version of 0.13.30 or later first.
    fn a_new_tinymist() -> Option<PathBuf> {
        let said = |exe: &Path| -> Option<String> {
            let out = std::process::Command::new(exe).arg("--version").output().ok()?;
            let text = String::from_utf8_lossy(&out.stdout).to_string();
            text.lines()
                .find_map(|line| line.trim().strip_prefix("Build Git Describe:"))
                .map(|v| v.trim().to_string())
        };
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Some(named) = std::env::var_os("YUMETE_TINYMIST") {
            candidates.push(named.into());
        }
        if let Some(path) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&path).map(|dir| dir.join("tinymist")));
        }
        if let Some(home) = std::env::var_os("HOME") {
            let extensions = Path::new(&home).join(".vscode/extensions");
            let mut found: Vec<PathBuf> = std::fs::read_dir(&extensions)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.file_name().is_some_and(|n| n.to_string_lossy().contains("tinymist")))
                .map(|p| p.join("out/tinymist"))
                .collect();
            found.sort();
            found.reverse();
            candidates.extend(found);
        }
        candidates
            .into_iter()
            .filter(|exe| exe.is_file())
            .find(|exe| said(exe).is_some_and(|v| preview::new_enough(Some(&v))))
    }

    /// **The real thing**, when this machine has a new enough tinymist: the
    /// language server started for a Typst file hands the preview its page
    /// from the unsaved text, and the data plane says where the cursor is. A
    /// second document's preview runs a task of its own beside it, and a
    /// closed tab takes its task and board with it. Skipped where there is
    /// none.
    #[test]
    fn tinymist_draws_the_page_and_follows_the_cursor() {
        let Some(tinymist) = a_new_tinymist() else {
            eprintln!("no tinymist of 0.13.30 or later here: skipped");
            return;
        };
        let dir = std::env::temp_dir().join(format!("yumete-page-live-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Another book, beside this one rather than under it.
        let elsewhere = std::env::temp_dir().join(format!("yumete-page-live-other-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&elsewhere);
        std::fs::create_dir_all(&elsewhere).unwrap();
        let main = dir.join("main.typ");
        let mut text = String::from("#set page(width: 10cm, height: 8cm)\n= One\n\nFirst page.\n#pagebreak()\n= Two\n\n");
        for n in 0..12 {
            text.push_str(&format!("Line {n} of the second page.\n\n"));
        }
        std::fs::write(&main, &text).unwrap();
        let other = elsewhere.join("note.typ");
        std::fs::write(&other, "#set page(width: 8cm, height: 6cm)\n= A note\n\nOne page.\n").unwrap();
        let mut editor = Editor::new();
        editor.open_file(&main).unwrap();
        let mut config = yumete_config::Config::default();
        config.lsp.insert(
            "typst".into(),
            vec![yumete_config::Server { command: tinymist.display().to_string(), ..Default::default() }],
        );
        let mut servers = Servers::default();
        let mut pages = PagePreview::default();
        let deadline = Instant::now() + Duration::from_secs(60);
        let turn = |editor: &mut Editor, servers: &mut Servers, pages: &mut PagePreview| {
            servers.follow(editor, &config);
            servers.collect(editor);
            let _ = pages.follow(editor, servers, &config);
            std::thread::sleep(Duration::from_millis(20));
        };
        while servers.typesetter(&config, &editor) != Typesetter::Ready {
            assert!(Instant::now() < deadline, "tinymist never got through its handshake");
            turn(&mut editor, &mut servers, &mut pages);
        }
        // An unsaved edit: the page is made from the buffer, not the file.
        editor.execute(":2").unwrap();
        for key in "A Unsaved".chars() {
            editor.on_key(yumete_core::input::Key::Char(key));
        }
        editor.on_key(yumete_core::input::Key::Esc);
        assert!(editor.current_buffer().is_modified());
        let window = editor.open_page_window(main.clone()).expect("a page window");
        let first = editor.preview_in(window).expect("a preview tab");
        pages.opened(first, false);
        with_board(first, |b| b.cells = Some((60, 30)));
        while with_board(first, |b| b.page.is_none()) {
            assert!(Instant::now() < deadline, "no page came: {:?}", with_board(first, |b| b.note.clone()));
            turn(&mut editor, &mut servers, &mut pages);
        }
        with_board(first, |board| {
            let page = board.page.as_ref().unwrap();
            assert_eq!(page.number, 1);
            assert!(page.total.is_some_and(|t| t >= 2), "{:?}", page.total);
            assert!(page.width > 100 && page.height > 50, "{}×{}", page.width, page.height);
        });
        // The cursor past the first page: the data plane says which, and that
        // is asked for.
        editor.execute(":12").unwrap();
        loop {
            assert!(Instant::now() < deadline, "the cursor was never followed: {:?}", with_board(first, |b| b.jump));
            turn(&mut editor, &mut servers, &mut pages);
            let followed = with_board(first, |board| {
                let jump = board.jump.filter(|j| j.page >= 2);
                jump.is_some_and(|j| board.page.as_ref().is_some_and(|p| p.number == j.page))
            });
            if followed {
                break;
            }
        }
        // **A second document, a second preview and a second task**: the
        // first keeps its page, the cursor being in another book now.
        editor.open_file(&other).unwrap();
        let window = editor.open_page_window(other.clone()).expect("a second page window");
        let second = editor.preview_in(window).expect("a second preview tab");
        assert_ne!(first, second);
        with_board(second, |b| b.cells = Some((60, 30)));
        while with_board(second, |b| b.page.is_none()) || pages.tasks().len() < 2 {
            assert!(Instant::now() < deadline, "no second page: {:?}", with_board(second, |b| b.note.clone()));
            turn(&mut editor, &mut servers, &mut pages);
        }
        assert_eq!(pages.tasks(), {
            let mut both = vec![preview::task_id(first), preview::task_id(second)];
            both.sort();
            both
        });
        assert!(with_board(first, |b| b.jump.is_some_and(|j| j.page >= 2)), "the first kept its place");
        // Closed: its task stopped, its board gone; the other runs on.
        assert!(editor.close_preview(first));
        turn(&mut editor, &mut servers, &mut pages);
        assert!(!has_a_board(first));
        assert_eq!(pages.tasks(), vec![preview::task_id(second)]);
        servers.stop();
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&elsewhere).ok();
    }

    /// **The page in the theme's colours** (2026-11-08): white paper becomes
    /// the background and black ink the text colour, a grey falls between,
    /// and a colour keeps its hue — in a light theme and a dark one.
    #[test]
    fn the_page_takes_the_theme_s_paper_and_ink() {
        let light = ([0xF7, 0xF5, 0xEE], [0x26, 0x2A, 0x27]);
        let dark = ([0x1C, 0x1E, 0x1D], [0xE0, 0xDE, 0xD6]);
        for (paper, ink) in [light, dark] {
            assert_eq!(recolour([255, 255, 255], paper, ink), paper, "paper");
            assert_eq!(recolour([0, 0, 0], paper, ink), ink, "ink");
            let mid = recolour([128, 128, 128], paper, ink);
            for i in 0..3 {
                let (lo, hi) = (paper[i].min(ink[i]), paper[i].max(ink[i]));
                assert!((lo..=hi).contains(&mid[i]), "a grey falls between: {mid:?}");
            }
            let red = recolour([200, 30, 30], paper, ink);
            assert!(red[0] > red[1] + 60 && red[0] > red[2] + 60, "red stays red: {red:?}");
            let blue = recolour([20, 60, 200], paper, ink);
            assert!(blue[2] > blue[0] + 60, "blue stays blue: {blue:?}");
        }
        // Dark: the paper is dark and the ink light.
        let (paper, ink) = dark;
        assert!(recolour([250, 250, 250], paper, ink)[0] < 60);
        assert!(recolour([10, 10, 10], paper, ink)[0] > 200);
    }

    /// **A whole PNG**, decoded, mapped and written again: its size kept,
    /// transparent as paper.
    #[test]
    fn a_png_comes_back_in_the_theme_s_colours() {
        let (paper, ink) = ([0x1C, 0x1E, 0x1D], [0xE0, 0xDE, 0xD6]);
        // 2 × 2 RGBA: white, black, transparent, half-transparent black.
        let pixels = [255, 255, 255, 255, 0, 0, 0, 255, 0, 0, 0, 0, 0, 0, 0, 128];
        let mut made = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut made, 2, 2);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
        }
        let out = recolour_png(&made, paper, ink).expect("a PNG");
        assert_eq!(crate::graphics::png_size(&out), Some((2, 2)));
        let mut reader = png::Decoder::new(std::io::Cursor::new(&out)).read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buf).unwrap();
        assert_eq!(frame.color_type, png::ColorType::Rgb);
        let px: Vec<[u8; 3]> = buf[..frame.buffer_size()].chunks(3).map(|c| [c[0], c[1], c[2]]).collect();
        assert_eq!(px[0], paper);
        assert_eq!(px[1], ink);
        assert_eq!(px[2], paper, "transparent is paper");
        assert!(px[3][0] > paper[0] && px[3][0] < ink[0], "half ink is between: {:?}", px[3]);
        assert!(recolour_png(b"not a png", paper, ink).is_none());
    }

    /// **Made once a page**: the same page and colours give the same
    /// picture back without making it again; another colour makes another.
    #[test]
    fn the_tinted_page_is_made_once() {
        let mut made = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut made, 1, 1);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[255, 255, 255]).unwrap();
        }
        let page = Arc::new(Page::from_png(made, 72, 1, Some(1)).unwrap());
        let mut board = Board::default();
        let one = board.tinted(&page, [0, 0, 0], [255, 255, 255]);
        let again = board.tinted(&page, [0, 0, 0], [255, 255, 255]);
        assert!(Arc::ptr_eq(&one, &again), "kept");
        assert_ne!(one.hash, page.hash);
        let other = board.tinted(&page, [10, 10, 10], [255, 255, 255]);
        assert!(!Arc::ptr_eq(&one, &other));
    }

    /// **`J`, `K` and `5G` turn pages** (2026-11-07): never before the first,
    /// nor past the last once the total is known.
    #[test]
    fn a_page_turns_within_the_book() {
        assert_eq!(turn(3, Some(10), PageStep::Pages(1)), 4);
        assert_eq!(turn(3, Some(10), PageStep::Pages(-2)), 1);
        assert_eq!(turn(1, Some(10), PageStep::Pages(-1)), 1, "not before the first");
        assert_eq!(turn(10, Some(10), PageStep::Pages(1)), 10, "nor past the last");
        assert_eq!(turn(3, Some(10), PageStep::Goto(5)), 5);
        assert_eq!(turn(3, Some(10), PageStep::Goto(50)), 10);
        assert_eq!(turn(3, None, PageStep::Goto(50)), 50, "the total is not known yet");
    }

    /// **The asked ppi settles** (2026-11-08): with Typst rounding the PNG to
    /// whole pixels, an A5 page 47 columns of 8 px wide asked for 65 and 64
    /// in turn forever, and the page shook between the two sizes.
    #[test]
    fn the_asked_ppi_settles_instead_of_flipping() {
        let made = |ppi: u32| -> (u32, u32) {
            ((419.5276 * ppi as f64 / 72.0).round() as u32, (595.2756 * ppi as f64 / 72.0).round() as u32)
        };
        let mut shown: Option<u32> = None;
        let mut asked = Vec::new();
        for _ in 0..6 {
            let inches = match shown {
                Some(p) => {
                    let (w, h) = made(p);
                    (w as f64 / p as f64, h as f64 / p as f64)
                }
                None => (8.27, 11.69),
            };
            let ppi = ppi_to_ask(PreviewFit::Width, Some((47, 30)), (8, 17), inches, shown);
            asked.push(ppi);
            shown = Some(ppi);
        }
        assert!(asked[2..].windows(2).all(|w| w[0] == w[1]), "{asked:?}");
    }

    /// **A wobble of the cursor's row does not move the page** when nothing
    /// was scrolled by hand: the clamp at the top is not a hand scroll.
    #[test]
    fn a_clamped_top_is_not_kept_as_a_hand_scroll() {
        let mut base = 0;
        let mut nudge = 0;
        for row in [5, 6, 5, 6] {
            let (top, b) = scroll_top(Some(row), base, nudge, 100, 30);
            assert_eq!(top, 0, "row {row}");
            if nudge != 0 {
                nudge = top as i32 - b;
            }
            base = b;
        }
    }
}
