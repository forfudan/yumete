//! **The formula under the cursor, typeset**.
//!
//! When the cursor is in a `$…$` — Markdown's `$`/`$$`, Typst's `$` — a
//! float under it shows the formula set as maths, in a terminal that draws
//! pictures in its text ([`crate::graphics`]). Everywhere else nothing
//! changes: the coloured source stays what the page shows. `[editor]
//! maths_preview = false` turns it off.
//!
//! **Under `:render full` a display formula on lines of its own is drawn in
//! place** ([`in_place`]): its picture in the rows its source takes, while
//! the cursor is elsewhere. `[editor] maths_in_place = false` turns it off.
//! The editing region only — the preview and the exports never ask.
//!
//! **Typst's maths is drawn by the `typst` command**, the same one
//! `--export pdf` runs ([`yumete_core::export::typst_command`]), off the
//! drawing thread: a formula takes about 5 ms (20 ms with 漢字 in it), and
//! nothing waits for it. The frame asks ([`ask`]); a picture not made yet is
//! queued, and the loop is woken when it lands. Each picture is kept by what
//! it shows — the formula, its ink and its size — so walking the cursor back
//! over a formula does not run typst again. A formula that imports a
//! package typst has not cached makes typst download it, as typst always
//! does; the status line names the package while it comes ([`tell_downloads`]).
//!
//! **Markdown's maths is TeX, drawn by RaTeX** ([`crate::tex`]),
//! in the process, in well under a millisecond, so it is drawn when asked
//! (and kept, [`crate::tex::picture`]) and needs no
//! `typst` at all.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};

use yumete_core::markdown::Maths;

/// What a picture is made of: the formula, the ink it is drawn in, and how
/// big a cell is — a picture made for one font size is the wrong size for
/// another.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub maths: Maths,
    pub ink: (u8, u8, u8),
    /// A cell's height in pixels, which the type size is read off.
    pub cell_height: u16,
}

impl Key {
    /// A short name for this picture, the one the terminal's shelf knows it
    /// by ([`crate::graphics::show`]).
    pub fn hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        Hash::hash(self, &mut h);
        h.finish()
    }

    /// The type size, in points at 72 points to the inch — so one point is
    /// one pixel. A cell's height, which is a little over the text's own
    /// size: an inline formula's superscripts are small, and the float is
    /// there to be read.
    pub fn size(&self) -> u16 {
        self.cell_height.max(8)
    }
}

/// A picture, made.
#[derive(Debug)]
pub struct Picture {
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// What [`ask`] answers.
#[derive(Debug, Clone)]
pub enum Answer {
    /// Queued or being made; the loop is woken when it lands.
    Making,
    Ready(Arc<Picture>),
    /// typst said no — its own message.
    Failed(String),
    /// There is no `typst` to ask.
    NoTypst,
}

/// How many pictures are kept made.
const CACHED: usize = 128;

/// How long one formula may take before typst is stopped.
const PATIENCE: std::time::Duration = std::time::Duration::from_secs(10);

struct Studio {
    made: HashMap<Key, Answer>,
    /// The order they were made in, oldest first, for dropping.
    order: VecDeque<Key>,
    /// The float's picture, wanted before any other. A newer wish replaces
    /// an older one: the cursor has moved on, and a formula it left need not
    /// be made.
    wanted: Option<Key>,
    /// The pictures the page draws in place and does not have yet, top of
    /// the page first ([`ask_in_place`]). Each frame says the whole of it
    /// again, so a formula scrolled off before its turn is not made.
    queue: VecDeque<Key>,
    /// Whether the worker has been started.
    started: bool,
    /// No `typst` on this machine: every answer is [`Answer::NoTypst`].
    no_typst: bool,
    /// The formula whose last run took too long: answered so while it is
    /// still the one asked about, and tried again once another was.
    slow: Option<Key>,
    /// The packages typst downloaded for a formula, for the status line
    /// ([`tell_downloads`]).
    news: Vec<Download>,
}

/// A package typst fetched for a formula: started, or done (`true` when
/// typst made the picture).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Download {
    Started(String),
    Finished(String, bool),
}

fn hear(news: Download) {
    let (lock, _) = studio();
    lock.lock().unwrap_or_else(|e| e.into_inner()).news.push(news);
    if let Some(ring) = BELL.get() {
        ring();
    }
}

/// **Say on the status line what typst downloaded** for a formula: the
/// package while it comes, then that it came. Asked every frame; `true`
/// when something was said.
pub fn tell_downloads(editor: &mut yumete_core::editor::Editor) -> bool {
    let news = {
        let (lock, _) = studio();
        std::mem::take(&mut lock.lock().unwrap_or_else(|e| e.into_inner()).news)
    };
    for one in &news {
        match one {
            Download::Started(spec) => editor.typst_downloading(spec),
            Download::Finished(spec, ok) => editor.typst_downloaded(spec, *ok),
        }
    }
    !news.is_empty()
}

/// What a run that ran out of time says.
fn too_slow() -> String {
    format!("more than {} s", PATIENCE.as_secs())
}

fn studio() -> &'static (Mutex<Studio>, Condvar) {
    static STUDIO: OnceLock<(Mutex<Studio>, Condvar)> = OnceLock::new();
    STUDIO.get_or_init(|| {
        (
            Mutex::new(Studio {
                made: HashMap::new(),
                order: VecDeque::new(),
                wanted: None,
                queue: VecDeque::new(),
                started: false,
                no_typst: false,
                slow: None,
                news: Vec::new(),
            }),
            Condvar::new(),
        )
    })
}

static BELL: OnceLock<crate::server::Doorbell> = OnceLock::new();

/// Wake the loop this way when a picture lands — the same bell the language
/// servers ring.
pub fn ring_on_answers(bell: crate::server::Doorbell) {
    let _ = BELL.set(bell);
}

fn keep(studio: &mut Studio, key: Key, answer: Answer) {
    if studio.made.insert(key.clone(), answer).is_none() {
        studio.order.push_back(key);
    }
    while studio.order.len() > CACHED {
        if let Some(old) = studio.order.pop_front() {
            studio.made.remove(&old);
        }
    }
}

/// The RaTeX picture a TeX formula's `key` asks for.
pub fn tex_key(key: &Key) -> crate::tex::Key {
    crate::tex::Key {
        tex: key.maths.source.clone(),
        display: key.maths.display,
        ink: key.ink,
        size: key.size() as u32 * 10,
    }
}

/// **The picture for `key`**, or the news that it is on its way. A TeX
/// formula is drawn there and then, by RaTeX.
pub fn ask(key: &Key) -> Answer {
    if key.maths.latex {
        return match crate::tex::picture(&tex_key(key)) {
            Ok(picture) => Answer::Ready(picture),
            Err(why) => Answer::Failed(why),
        };
    }
    let (lock, wake) = studio();
    let mut studio = lock.lock().unwrap_or_else(|e| e.into_inner());
    if studio.no_typst {
        return Answer::NoTypst;
    }
    if let Some(answer) = studio.made.get(key) {
        return answer.clone();
    }
    match &studio.slow {
        Some(slow) if slow == key => return Answer::Failed(too_slow()),
        Some(_) => studio.slow = None,
        None => {}
    }
    studio.wanted = Some(key.clone());
    start(&mut studio);
    wake.notify_one();
    Answer::Making
}

/// Start the worker, once.
fn start(studio: &mut Studio) {
    if !studio.started {
        studio.started = true;
        let _ = std::thread::Builder::new().name("maths".into()).spawn(work);
    }
}

/// **The pictures for the formulas the page draws in place**, an answer a
/// key: [`ask`]'s answers, but a Typst formula not made yet joins a queue the
/// worker takes after the float's, instead of taking the float's place. The
/// queue becomes exactly the formulas asked for here that are still to make.
///
/// A formula whose last run took too long is answered so for good here: the
/// page shows its source, and only the float tries it again.
pub fn ask_in_place(keys: &[Key]) -> Vec<Answer> {
    let mut answers = Vec::with_capacity(keys.len());
    let mut queue = VecDeque::new();
    let (lock, wake) = studio();
    let mut studio = lock.lock().unwrap_or_else(|e| e.into_inner());
    for key in keys {
        let answer = match key.maths.latex {
            true => match crate::tex::picture(&tex_key(key)) {
                Ok(picture) => Answer::Ready(picture),
                Err(why) => Answer::Failed(why),
            },
            false if studio.no_typst => Answer::NoTypst,
            false => match studio.made.get(key) {
                Some(answer) => answer.clone(),
                None if studio.slow.as_ref() == Some(key) => Answer::Failed(too_slow()),
                None => {
                    if !queue.contains(key) {
                        queue.push_back(key.clone());
                    }
                    Answer::Making
                }
            },
        };
        answers.push(answer);
    }
    let waiting = !queue.is_empty();
    studio.queue = queue;
    if waiting {
        start(&mut studio);
        wake.notify_one();
    }
    answers
}

/// **Put a picture in by hand** — for a test, which has no typst to wait for.
pub fn preload(key: Key, png: Vec<u8>) {
    let (width, height) = crate::graphics::png_size(&png).unwrap_or((1, 1));
    let (lock, _) = studio();
    let mut studio = lock.lock().unwrap_or_else(|e| e.into_inner());
    keep(&mut studio, key, Answer::Ready(Arc::new(Picture { png, width, height })));
}

/// The worker: one picture at a time — the float's most recent wish first,
/// then the page's queue from the top.
fn work() {
    let (lock, wake) = studio();
    let place = Place::set_up();
    loop {
        let key = {
            let mut studio = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                let next = match studio.wanted.take() {
                    Some(key) => Some(key),
                    None => studio.queue.pop_front(),
                };
                match next {
                    Some(key) if !studio.made.contains_key(&key) => break key,
                    Some(_) => {}
                    None => studio = wake.wait(studio).unwrap_or_else(|e| e.into_inner()),
                }
            }
        };
        // Without a folder of its own typst would take the working folder as
        // its root, where a formula could read the project's files.
        let answer = match &place {
            Some(place) => render(&key, Some(place)),
            None => Answer::Failed("no private folder for typst to work in".to_string()),
        };
        {
            let mut studio = lock.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(answer, Answer::NoTypst) {
                studio.no_typst = true;
            }
            // A run that took too long may only have met a busy machine (the
            // first font scan): it is tried again when next asked, not kept.
            match &answer {
                Answer::Failed(why) if *why == too_slow() => {
                    studio.slow = Some(key);
                }
                _ => keep(&mut studio, key, answer),
            }
        }
        if let Some(ring) = BELL.get() {
            ring();
        }
    }
}

/// Where typst runs: a folder of this user's own under the system's
/// temporary one, holding a folder with one CJK font in it, and an empty
/// working folder for the root.
struct Place {
    work: PathBuf,
    /// A folder holding one CJK font, when this system has one where it was
    /// looked for — so a formula with 漢字 in it need not scan every font.
    fonts: Option<PathBuf>,
}

impl Place {
    fn set_up() -> Option<Place> {
        #[cfg(unix)]
        // SAFETY: `getuid` has no preconditions and cannot fail.
        let me = unsafe { libc::getuid() }.to_string();
        #[cfg(not(unix))]
        let me = std::env::var("USERNAME").unwrap_or_default();
        let base = std::env::temp_dir().join(format!("yumete-maths-{me}"));
        // **This user's own, and nobody else's**: on
        // Linux the temporary folder is the shared `/tmp`, where anyone could
        // make this folder first and plant a package or a link in it. It is
        // made private, and used only if it is a real folder of ours.
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, MetadataExt};
            let _ = std::fs::DirBuilder::new().mode(0o700).create(&base);
            let ours = std::fs::symlink_metadata(&base).ok()?;
            // SAFETY: as above.
            let uid = unsafe { libc::getuid() };
            if !ours.is_dir() || ours.uid() != uid {
                return None;
            }
            // One made by an earlier yumete is ours but readable by all.
            if ours.mode() & 0o077 != 0 {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).ok()?;
            }
        }
        let work = base.join("work");
        std::fs::create_dir_all(&work).ok()?;
        let fonts = Self::one_cjk_font(&base.join("fonts"));
        Some(Place { work, fonts })
    }

    /// A folder with a link to the first CJK font found where systems keep
    /// one. typst takes folders, not files, for `--font-path`.
    #[cfg(unix)]
    fn one_cjk_font(folder: &Path) -> Option<PathBuf> {
        let font = Path::new(crate::tex::cjk_font()?);
        let link = folder.join(format!("cjk.{}", font.extension()?.to_string_lossy()));
        if std::fs::read_link(&link).ok().as_deref() != Some(font) {
            std::fs::create_dir_all(folder).ok()?;
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(font, &link).ok()?;
        }
        Some(folder.to_path_buf())
    }

    /// No links without privileges on Windows: a formula with 漢字 in it
    /// scans the system's fonts instead, which costs a tenth of a second.
    #[cfg(not(unix))]
    fn one_cjk_font(_folder: &Path) -> Option<PathBuf> {
        None
    }
}

/// The name the formula is handed to typst under (`--input`), so it never has
/// to be quoted into the document.
const INPUT: &str = "yumete-maths";

/// **The document typst is given** for a Typst formula: a page the size of
/// the formula, no ground, the text in the page's own ink.
pub fn document(key: &Key) -> String {
    let (r, g, b) = key.ink;
    let size = key.size();
    let block = key.maths.display;
    let mut out = String::new();
    out.push_str("#set page(width: auto, height: auto, margin: 2pt, fill: none)\n");
    out.push_str(&format!("#set text(size: {size}pt, fill: rgb(\"#{r:02x}{g:02x}{b:02x}\"))\n"));
    // **The page is as tall as the ink, not the line**: by
    // default a line runs from cap height to baseline, and a page of
    // `height: auto` cut an inline `1/2` or `sqrt` off at both edges.
    out.push_str("#set text(top-edge: \"bounds\", bottom-edge: \"bounds\")\n");
    out.push_str(&format!("#math.equation(block: {block}, eval(sys.inputs.at(\"{INPUT}\"), mode: \"math\"))\n"));
    out
}

/// Whether `text` has a character the bundled maths fonts do not have — a
/// 漢字, a kana, a hangul — and so needs a CJK font.
fn wants_cjk(text: &str) -> bool {
    text.chars().any(|c| c as u32 >= 0x2E80)
}

/// Run typst on one formula — RaTeX for a TeX one.
fn render(key: &Key, place: Option<&Place>) -> Answer {
    use std::io::{Read, Write};
    if key.maths.latex {
        return ask(key);
    }
    let mut command = yumete_core::export::typst_command();
    command.args(["compile", "-", "-", "--format", "png", "--ppi", "72"]);
    if let Some(place) = place {
        command.arg("--root").arg(&place.work).current_dir(&place.work);
    }
    // The system's fonts are the slow part (0.1 s of a 0.12 s run): the
    // bundled maths fonts are enough, and one CJK font when the formula has
    // a 漢字 in it.
    match (wants_cjk(&key.maths.source), place.and_then(|p| p.fonts.as_ref())) {
        (false, _) => {
            command.arg("--ignore-system-fonts");
        }
        (true, Some(fonts)) => {
            command.arg("--ignore-system-fonts").arg("--font-path").arg(fonts);
        }
        (true, None) => {}
    }
    command.arg("--input").arg(format!("{INPUT}={}", key.maths.source));
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Answer::NoTypst,
        Err(e) => return Answer::Failed(e.to_string()),
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(document(key).as_bytes());
    }
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(pipe) = stdout.as_mut() {
            let _ = pipe.read_to_end(&mut bytes);
        }
        bytes
    });
    // Read as typst writes it, so a package it downloads is said while it
    // comes.
    let err = std::thread::spawn(move || {
        let mut last = None;
        let text = match stderr.take() {
            Some(pipe) => yumete_core::export::read_typst_stderr(pipe, |spec| {
                last = Some(spec.clone());
                hear(Download::Started(spec));
            }),
            None => String::new(),
        };
        (text, last)
    });
    let deadline = std::time::Instant::now() + PATIENCE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let png = out.join().unwrap_or_default();
    let (said, downloaded) = err.join().unwrap_or_default();
    if let Some(spec) = downloaded {
        hear(Download::Finished(spec, status.is_some_and(|s| s.success())));
    }
    match status {
        Some(status) if status.success() => match crate::graphics::png_size(&png) {
            Some((width, height)) => Answer::Ready(Arc::new(Picture { png, width, height })),
            None => Answer::Failed("no picture came back".to_string()),
        },
        Some(_) => Answer::Failed(first_error(&said)),
        None => Answer::Failed(too_slow()),
    }
}

/// The first thing typst said was wrong, without its `error:`.
fn first_error(said: &str) -> String {
    said.lines()
        .find_map(|line| line.trim().strip_prefix("error:"))
        .unwrap_or_else(|| said.lines().next().unwrap_or(""))
        .trim()
        .to_string()
}

/// What the float last showed — drawn again while the next picture is being
/// made, so typing inside a formula does not make the float blink.
/// Kept with the formula it showed (`Editor::maths_anchor`): another
/// formula never borrows it.
static LAST: Mutex<Option<Shown>> = Mutex::new(None);

/// A picture the float showed: which formula (buffer, first line), the
/// picture's key hash, and the picture.
type Shown = ((u64, usize), u64, Arc<Picture>);

/// **No `typst` is said at one formula only**, the first one stood in;
/// every other formula draws nothing, as in a terminal without pictures,
/// rather than saying it at every formula for good.
static TOLD_NO_TYPST: Mutex<Option<(u64, usize)>> = Mutex::new(None);

/// **The formula float**, if the cursor is in a formula and this terminal
/// can show one: `None` when it does not apply, so the next float may have
/// the place; `Some` with what it covered when it does.
#[allow(clippy::too_many_arguments)]
pub fn float(
    frame: &mut ratatui::Frame,
    editor: &yumete_core::editor::Editor,
    config: &yumete_config::Config,
    area: ratatui::layout::Rect,
    bottom: u16,
    caret: crate::chrome::Spot,
    vertical: bool,
    composing: bool,
) -> Option<Option<ratatui::layout::Rect>> {
    // A page window has no text to stand in.
    if !config.editor.maths_preview
        || composing
        || crate::graphics::support() == crate::graphics::Support::None
        || editor.page_window_has_the_keys()
    {
        return None;
    }
    let maths = editor.maths_here()?;
    let ink = crate::theme::Palette::of(config);
    let ink_rgb = match ink.text() {
        ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
        _ => (0x80, 0x80, 0x80),
    };
    let (cell_w, cell_h) = crate::graphics::cell_pixels();
    let key = Key { maths, ink: ink_rgb, cell_height: cell_h };
    let anchor = editor.maths_anchor();
    let last = || {
        LAST.lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .filter(|(at, ..)| *at == anchor)
            .map(|(_, hash, picture)| (hash, picture))
    };
    // **While typing, a formula is broken most of the time** — `\frac{` on
    // the way to `\frac{1}{2}` — so in Insert the last picture stays up
    // instead of typst's complaint, and the complaint is said once the hand
    // stops (Normal).
    let typing = editor.mode() == yumete_core::Mode::Insert;
    let picture = match ask(&key) {
        Answer::Ready(picture) => {
            *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((anchor, key.hash(), picture.clone()));
            (key.hash(), picture)
        }
        Answer::Making => match last() {
            Some(last) => last,
            None => return Some(None),
        },
        Answer::NoTypst => {
            let mut told = TOLD_NO_TYPST.lock().unwrap_or_else(|e| e.into_inner());
            if *told.get_or_insert(anchor) != anchor {
                return None;
            }
            return Some(said(frame, config, area, bottom, caret, vertical, say_no_typst()));
        }
        Answer::Failed(_) if typing && last().is_some() => last()?,
        Answer::Failed(why) => {
            let why = match key.maths.latex {
                true => yumete_core::say!("maths.tex-failed", why),
                false => yumete_core::say!("maths.failed", why),
            };
            return Some(said(frame, config, area, bottom, caret, vertical, why));
        }
    };
    let (hash, picture) = picture;
    Some(draw_picture(frame, config, area, bottom, caret, vertical, hash, &picture, (cell_w, cell_h)))
}

fn say_no_typst() -> String {
    yumete_core::say!("maths.no-typst")
}

/// The float as words — what typst said, or that there is no typst.
fn said(
    frame: &mut ratatui::Frame,
    config: &yumete_config::Config,
    area: ratatui::layout::Rect,
    bottom: u16,
    caret: crate::chrome::Spot,
    vertical: bool,
    text: String,
) -> Option<ratatui::layout::Rect> {
    crate::panel::draw(frame, config, area, bottom, caret, vertical, &crate::panel::Panel {
        stand: crate::panel::Stand::UnderCaret,
        accent: None,
        pages: false,
        reading: false,
        scroll: 0,
        title: String::new(),
        lede: None,
        entry: false,
        body: crate::panel::Body::Prose(text),
        tag: None,
        vertical_text: false,
        marked: false,
    })
}

/// **A display formula the page draws in place**: the formula, and the
/// screen rows its source takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spot {
    pub maths: Maths,
    /// The screen row the block's first row is on, counted from the top of
    /// the page: below zero when the page has scrolled past its top.
    pub top: i32,
    /// How many screen rows the block takes in all, on the page or not.
    pub rows: u16,
}

/// Whether the page draws display formulas in place at all: the setting is
/// on and this terminal can show a picture. Asked before the page works out
/// where its formulas are, so a frame that draws none pays nothing.
pub fn in_place_wanted(config: &yumete_config::Config) -> bool {
    config.editor.maths_in_place && crate::graphics::support() != crate::graphics::Support::None
}

/// **Draw each of `spots` as its typeset picture, in the rows its source
/// takes** on the page `area` (the text's columns and the page's rows).
///
/// The picture is no taller than the block and no wider than the text: one
/// that is bigger is shrunk as a whole, one that is smaller sits in the
/// middle of the block's rows. The rows are the block's own, so nothing
/// below moves when the cursor walks in and the source comes back. A
/// formula with no picture yet — Typst's, being made — or none at all — a
/// broken one, no `typst` — keeps its source, and the cells the source was
/// drawn in are left as they are. The page's other rows are not touched.
///
/// `palette` is the page's: the picture in its ink, on its ground.
pub fn in_place(
    buf: &mut ratatui::buffer::Buffer,
    palette: crate::theme::Palette,
    area: ratatui::layout::Rect,
    spots: &[Spot],
) {
    if spots.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    let ground = palette.page();
    let ink = match palette.text() {
        ratatui::style::Color::Rgb(r, g, b) => (r, g, b),
        _ => (0x80, 0x80, 0x80),
    };
    let cell = crate::graphics::cell_pixels();
    let keys: Vec<Key> = spots
        .iter()
        .map(|spot| Key { maths: spot.maths.clone(), ink, cell_height: cell.1 })
        .collect();
    let answers = ask_in_place(&keys);
    for ((spot, key), answer) in spots.iter().zip(&keys).zip(answers) {
        let Answer::Ready(picture) = answer else { continue };
        let (columns, rows) = cells_for((picture.width, picture.height), cell, (area.width, spot.rows));
        let block_top = area.y as i32 + spot.top;
        let block_end = block_top + spot.rows as i32;
        let seen_top = block_top.max(area.y as i32);
        let seen_end = block_end.min(area.bottom() as i32);
        if seen_top >= seen_end {
            continue;
        }
        for y in seen_top..seen_end {
            for x in area.x..area.right() {
                if let Some(cell) = buf.cell_mut((x, y as u16)) {
                    cell.reset();
                    cell.set_style(ground);
                }
            }
        }
        let picture_top = block_top + (spot.rows.saturating_sub(rows) / 2) as i32;
        let from = picture_top.max(seen_top);
        let to = (picture_top + rows as i32).min(seen_end);
        if from >= to {
            continue;
        }
        let id = crate::graphics::show_in_place(key.hash(), &picture.png, columns, rows);
        let spot_rect = ratatui::layout::Rect {
            x: area.x + area.width.saturating_sub(columns) / 2,
            y: from as u16,
            width: columns,
            height: (to - from) as u16,
        };
        crate::graphics::put_picture_from(buf, spot_rect, id, (from - picture_top) as u16, 0);
    }
}

/// **How many cells a picture takes**, at most `room`: its pixels over a
/// cell's, rounded up, then shrunk as a whole if that is more than the room —
/// the terminal fits the picture into the cells it is given and keeps its
/// shape.
pub fn cells_for(picture: (u32, u32), cell: (u16, u16), room: (u16, u16)) -> (u16, u16) {
    let (w, h) = picture;
    let (cw, ch) = (cell.0.max(1) as u32, cell.1.max(1) as u32);
    let mut columns = w.div_ceil(cw).max(1);
    let mut rows = h.div_ceil(ch).max(1);
    let (most_w, most_h) = (room.0.max(1) as u32, room.1.max(1) as u32);
    if columns > most_w || rows > most_h {
        // The same scale both ways, the smaller of the two that fit.
        let by_w = most_w as f64 / columns as f64;
        let by_h = most_h as f64 / rows as f64;
        let scale = by_w.min(by_h);
        columns = ((columns as f64 * scale).floor() as u32).max(1);
        rows = ((rows as f64 * scale).floor() as u32).max(1);
    }
    let most = crate::graphics::MOST_CELLS as u32;
    (columns.min(most) as u16, rows.min(most) as u16)
}

/// The float with the picture in it: a ring the size of the picture plus a
/// cell of air each side, under the caret (beside it on a vertical page).
#[allow(clippy::too_many_arguments)]
fn draw_picture(
    frame: &mut ratatui::Frame,
    config: &yumete_config::Config,
    area: ratatui::layout::Rect,
    bottom: u16,
    caret: crate::chrome::Spot,
    vertical: bool,
    hash: u64,
    picture: &Picture,
    cell: (u16, u16),
) -> Option<ratatui::layout::Rect> {
    use ratatui::style::Style;
    let ink = crate::theme::Palette::of(config);
    // As big as a reading float may be (`chrome::room`), less the ring and the
    // air.
    let (room_w, room_h) = crate::chrome::room(area, vertical, crate::chrome::Use::Read);
    let tall = bottom.saturating_sub(area.y).min(area.height);
    // The walls take `chrome::wall` cells each, and a cell of air is left
    // inside each.
    let wall = crate::chrome::wall();
    let room = (
        room_w.max(24).min(area.width).saturating_sub(2 + 2 * wall),
        room_h.max(3).min(tall).saturating_sub(2),
    );
    let (columns, rows) = cells_for((picture.width, picture.height), cell, room);
    let width = (columns + columns % wall + 2 + 2 * wall).max(8);
    let height = rows + 2;
    let anchor = match vertical {
        false => crate::chrome::Anchor::UnderCaret { at: caret.at, bottom },
        true => crate::chrome::Anchor::Caret { at: caret.at, keep: caret.keep, bottom, vertical },
    };
    let rect = crate::chrome::place(area, (width, height), anchor)?;
    let inner = crate::chrome::draw(frame, rect, &crate::chrome::Ring {
        foot: None,
        rounded: config.panel.rounded,
        open_sides: false,
        border: Style::default().fg(ink.rule()).bg(crate::chrome::panel_ground(ink)),
        ground: Style::default().bg(crate::chrome::panel_ground(ink)),
        title: None,
    });
    let spot = ratatui::layout::Rect {
        x: inner.x + inner.width.saturating_sub(columns) / 2,
        y: inner.y,
        width: columns.min(inner.width),
        height: rows.min(inner.height),
    };
    let id = crate::graphics::show(hash, &picture.png, spot.width, spot.height);
    crate::graphics::put_picture(frame.buffer_mut(), spot, id);
    Some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(source: &str, latex: bool, display: bool) -> Key {
        Key {
            maths: Maths { source: source.to_string(), latex, display },
            ink: (0xc8, 0xa2, 0xff),
            cell_height: 30,
        }
    }

    /// **A package typst downloads for a formula reaches the status line**:
    /// named while it comes, said to have come once typst is done; with no
    /// news, the status line is left as it was.
    #[test]
    fn a_formula_download_is_said_on_the_status_line() {
        let mut ed = yumete_core::editor::Editor::new();
        ed.set_status("as it was".to_string());
        assert!(!tell_downloads(&mut ed));
        assert_eq!(ed.status(), "as it was");
        hear(Download::Started("@preview/physica:0.9.5".to_string()));
        assert!(tell_downloads(&mut ed));
        assert_eq!(ed.status(), yumete_core::say!("export.typst-downloading", "@preview/physica:0.9.5"));
        hear(Download::Finished("@preview/physica:0.9.5".to_string(), true));
        assert!(tell_downloads(&mut ed));
        assert_eq!(ed.status(), yumete_core::say!("export.typst-downloaded", "@preview/physica:0.9.5"));
    }

    /// **A picture is kept by what it shows** — the formula, how it is set,
    /// the ink and the size — and by nothing else.
    #[test]
    fn the_cache_key_is_what_the_picture_shows() {
        let a = key("x^2", true, false);
        assert_eq!(a.hash(), key("x^2", true, false).hash());
        for other in [
            key("x^3", true, false),
            key("x^2", false, false),
            key("x^2", true, true),
            Key { ink: (0, 0, 0), ..a.clone() },
            Key { cell_height: 40, ..a.clone() },
        ] {
            assert_ne!(a, other);
            assert_ne!(a.hash(), other.hash(), "{other:?}");
        }
    }

    /// The document: Typst through `eval`, the formula never written into
    /// it.
    #[test]
    fn the_document_hands_the_formula_over_as_an_input() {
        let typ = document(&key("sum_(k=1)^n k", false, true));
        assert!(typ.contains("block: true"), "{typ}");
        assert!(typ.contains("rgb(\"#c8a2ff\")"), "{typ}");
        assert!(typ.contains("size: 30pt"), "{typ}");
        assert!(!typ.contains("sum"), "the formula is an input, not text: {typ}");
        let typ = document(&key("x^2", false, false));
        assert!(typ.contains("eval(sys.inputs.at(\"yumete-maths\"), mode: \"math\")"), "{typ}");
        assert!(typ.contains("block: false"), "{typ}");
    }

    /// **Markdown's TeX is drawn by RaTeX**, there and then,
    /// with no `typst` anywhere, and kept for the next time it is asked.
    #[test]
    fn a_tex_formula_is_drawn_by_ratex() {
        let k = key(r"\sqrt{\alpha^2+\beta^2}", true, false);
        match ask(&k) {
            Answer::Ready(picture) => {
                let same = crate::tex::picture(&tex_key(&k)).unwrap();
                assert!(Arc::ptr_eq(&picture, &same), "drawn once and kept");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(ask(&key(r"\frac{1}{", true, false)), Answer::Failed(_)));
    }

    /// **An inline fraction is not cut off**: the page is as
    /// tall as the ink. With the line's own edges, `1/2` came out exactly as
    /// tall as `x`, its numerator and denominator outside the picture.
    #[test]
    fn an_inline_fraction_is_as_tall_as_its_ink() {
        if !yumete_core::export::typst_is_installed() {
            return;
        }
        let tall = |source: &str| match render(&key(source, false, false), None) {
            Answer::Ready(picture) => picture.height,
            other => panic!("{other:?}"),
        };
        assert!(tall("1/2") > tall("x") + 4, "1/2 {} vs x {}", tall("1/2"), tall("x"));
    }

    /// A page of `x`, 32 × 8, and what [`in_place`] leaves on it for one
    /// formula whose block starts on screen row `top` and takes `rows`.
    fn page_with(source: &str, top: i32, rows: u16) -> ratatui::buffer::Buffer {
        use ratatui::layout::Rect;
        let mut buf = ratatui::buffer::Buffer::filled(Rect::new(0, 0, 32, 8), ratatui::buffer::Cell::new("x"));
        let spot = Spot { maths: Maths { source: source.into(), latex: true, display: true }, top, rows };
        // The text's columns: two of gutter on the left are not the page's.
        let palette = crate::theme::Palette::in_mood(&yumete_config::Config::default(), true);
        in_place(&mut buf, palette, Rect::new(2, 0, 30, 8), &[spot]);
        buf
    }

    /// The row of the picture a screen row shows, if it shows one.
    fn picture_row(buf: &ratatui::buffer::Buffer, y: u16) -> Option<u16> {
        let first = (0..32).find_map(|x| {
            let s = buf[(x, y)].symbol();
            s.starts_with(crate::graphics::PLACEHOLDER).then(|| s.to_string())
        })?;
        (0..crate::graphics::MOST_CELLS).find(|&r| crate::graphics::placeholder(r, 0).chars().nth(1) == first.chars().nth(1))
    }

    /// **The picture takes the block's rows and no others**: the rows above
    /// and below and the gutter keep what they had, the block's rows are
    /// the picture on the page's ground, and a block the page has scrolled
    /// into shows the picture's lower part at the same size.
    #[test]
    fn a_formula_in_place_takes_its_blocks_rows() {
        let formula = r"\frac{a}{b} + \sum_{k=1}^{n} k";
        let buf = page_with(formula, 1, 4);
        let shown: Vec<Option<u16>> = (0..8).map(|y| picture_row(&buf, y)).collect();
        for y in [0, 5, 6, 7] {
            assert!((0..32).all(|x| buf[(x, y)].symbol() == "x"), "row {y} is not the block's");
        }
        for y in 1..5 {
            assert_eq!(buf[(0, y)].symbol(), "x", "the gutter keeps row {y}");
            assert!((2..32).all(|x| buf[(x, y)].symbol() != "x"), "the source is gone from row {y}");
        }
        let rows: Vec<u16> = shown.iter().flatten().copied().collect();
        assert!(!rows.is_empty() && rows.len() <= 4, "{shown:?}");
        assert_eq!(rows, (0..rows.len() as u16).collect::<Vec<_>>(), "the picture's rows in order");
        // Cut at the top by one row: the same picture a row higher, its first
        // row off the page.
        for top in [0, -1, -2] {
            let cut = page_with(formula, top, 4);
            let up = (1 - top) as usize;
            for y in 0..8 - up {
                assert_eq!(picture_row(&cut, y as u16), shown[y + up], "top {top}, row {y}");
            }
        }
    }

    /// **A broken formula keeps its source**: nothing is drawn over it.
    #[test]
    fn a_broken_formula_in_place_keeps_its_source() {
        let buf = page_with(r"\frac{a}{", 1, 3);
        assert!((0..8).all(|y| (0..32).all(|x| buf[(x, y)].symbol() == "x")));
    }

    /// Cells: rounded up, and shrunk as a whole to the room.
    #[test]
    fn a_picture_takes_the_cells_it_needs_and_no_more_than_the_room() {
        assert_eq!(cells_for((284, 111), (10, 20), (80, 20)), (29, 6));
        // Too wide: both ways by the same share.
        assert_eq!(cells_for((1000, 100), (10, 20), (50, 20)), (50, 2));
        assert_eq!(cells_for((1, 1), (10, 20), (50, 20)), (1, 1));
    }

    #[test]
    fn typst_says_what_was_wrong() {
        assert_eq!(first_error("error: unknown variable: foo\n  ┌─ <stdin>:3:2\n"), "unknown variable: foo");
        assert_eq!(first_error("something else"), "something else");
    }

    /// **The real thing**, when this machine has typst: a Typst formula
    /// comes back as a PNG. Skipped where there is no typst.
    #[test]
    fn typst_makes_the_picture() {
        if !yumete_core::export::typst_is_installed() {
            eprintln!("no typst here: skipped");
            return;
        }
        let place = Place::set_up();
        for k in [key("sum_(k=1)^n k", false, true), key("\"面积\" = pi r^2", false, true)] {
            match render(&k, place.as_ref()) {
                Answer::Ready(p) => assert!(p.width > 10 && p.height > 10, "{k:?}: {}×{}", p.width, p.height),
                other => panic!("{k:?}: {other:?}"),
            }
        }
        match render(&key("foo", false, false), place.as_ref()) {
            Answer::Failed(why) => assert!(!why.is_empty()),
            other => panic!("a broken formula: {other:?}"),
        }
    }
}
