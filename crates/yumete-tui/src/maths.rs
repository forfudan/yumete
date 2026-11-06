//! **The formula under the cursor, typeset** (2026-11-06).
//!
//! When the cursor is in a `$…$` — Markdown's `$`/`$$`, Typst's `$` — a
//! float under it shows the formula set as maths, in a terminal that draws
//! pictures in its text ([`crate::graphics`]). Everywhere else nothing
//! changes: the coloured source stays what the page shows. `[editor]
//! maths_preview = false` turns it off.
//!
//! **Rendered by the `typst` command**, the same one `--export pdf` runs
//! ([`yumete_core::export::typst_command`]), off the drawing thread: a
//! formula takes about 5 ms (20 ms with 漢字 in it), and nothing waits for
//! it. The frame asks ([`ask`]); a picture not made yet is queued, and the
//! loop is woken when it lands. Each picture is kept by what it shows — the
//! formula, its ink and its size — so walking the cursor back over a formula
//! does not run typst again.
//!
//! **Markdown's maths is LaTeX**, which Typst reads through the `mitex`
//! package. It is bundled (`assets/mitex/`, Apache-2.0) and handed to typst
//! as a local package, so nothing is ever downloaded.

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
    /// The one picture wanted next. A newer wish replaces an older one: the
    /// cursor has moved on, and a formula it left need not be made.
    wanted: Option<Key>,
    /// Whether the worker has been started.
    started: bool,
    /// No `typst` on this machine: every answer is [`Answer::NoTypst`].
    no_typst: bool,
}

fn studio() -> &'static (Mutex<Studio>, Condvar) {
    static STUDIO: OnceLock<(Mutex<Studio>, Condvar)> = OnceLock::new();
    STUDIO.get_or_init(|| {
        (
            Mutex::new(Studio {
                made: HashMap::new(),
                order: VecDeque::new(),
                wanted: None,
                started: false,
                no_typst: false,
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

/// **The picture for `key`**, or the news that it is on its way.
pub fn ask(key: &Key) -> Answer {
    let (lock, wake) = studio();
    let mut studio = lock.lock().unwrap_or_else(|e| e.into_inner());
    if studio.no_typst {
        return Answer::NoTypst;
    }
    if let Some(answer) = studio.made.get(key) {
        return answer.clone();
    }
    studio.wanted = Some(key.clone());
    if !studio.started {
        studio.started = true;
        let _ = std::thread::Builder::new().name("maths".into()).spawn(work);
    }
    wake.notify_one();
    Answer::Making
}

/// **Put a picture in by hand** — for a test, which has no typst to wait for.
pub fn preload(key: Key, png: Vec<u8>) {
    let (width, height) = crate::graphics::png_size(&png).unwrap_or((1, 1));
    let (lock, _) = studio();
    let mut studio = lock.lock().unwrap_or_else(|e| e.into_inner());
    keep(&mut studio, key, Answer::Ready(Arc::new(Picture { png, width, height })));
}

/// The worker: one picture at a time, the most recently wanted.
fn work() {
    let (lock, wake) = studio();
    let place = Place::set_up();
    loop {
        let key = {
            let mut studio = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                match studio.wanted.take() {
                    Some(key) if !studio.made.contains_key(&key) => break key,
                    Some(_) => {}
                    None => studio = wake.wait(studio).unwrap_or_else(|e| e.into_inner()),
                }
            }
        };
        let answer = render(&key, place.as_ref());
        {
            let mut studio = lock.lock().unwrap_or_else(|e| e.into_inner());
            if matches!(answer, Answer::NoTypst) {
                studio.no_typst = true;
            }
            keep(&mut studio, key, answer);
        }
        if let Some(ring) = BELL.get() {
            ring();
        }
    }
}

/// The files `mitex` is made of, as typst wants them laid out.
const MITEX: &[(&str, &[u8])] = &[
    ("typst.toml", include_bytes!("../assets/mitex/0.2.6/typst.toml")),
    ("LICENSE", include_bytes!("../assets/mitex/0.2.6/LICENSE")),
    ("lib.typ", include_bytes!("../assets/mitex/0.2.6/lib.typ")),
    ("mitex.typ", include_bytes!("../assets/mitex/0.2.6/mitex.typ")),
    ("mitex.wasm", include_bytes!("../assets/mitex/0.2.6/mitex.wasm")),
    ("specs/mod.typ", include_bytes!("../assets/mitex/0.2.6/specs/mod.typ")),
    ("specs/prelude.typ", include_bytes!("../assets/mitex/0.2.6/specs/prelude.typ")),
    ("specs/latex/standard.typ", include_bytes!("../assets/mitex/0.2.6/specs/latex/standard.typ")),
];

/// Where typst runs: a folder of this user's own under the system's
/// temporary one, holding the bundled package, a folder with one CJK font in
/// it, and an empty working folder for the root.
struct Place {
    packages: PathBuf,
    cache: PathBuf,
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
        let packages = base.join("packages");
        let package = packages.join("local").join("mitex").join("0.2.6");
        for (name, bytes) in MITEX {
            let path = package.join(name);
            let same = std::fs::metadata(&path).is_ok_and(|m| m.len() == bytes.len() as u64);
            if !same {
                std::fs::create_dir_all(path.parent()?).ok()?;
                // Written beside and moved into place, so a second yumete
                // reading it halfway never sees half a file.
                let part = path.with_extension(format!("part{}", std::process::id()));
                std::fs::write(&part, bytes).ok()?;
                std::fs::rename(&part, &path).ok()?;
            }
        }
        let work = base.join("work");
        let cache = base.join("cache");
        std::fs::create_dir_all(&work).ok()?;
        let fonts = Self::one_cjk_font(&base.join("fonts"));
        Some(Place { packages, cache, work, fonts })
    }

    /// A folder with a link to the first CJK font found where systems keep
    /// one. typst takes folders, not files, for `--font-path`.
    #[cfg(unix)]
    fn one_cjk_font(folder: &Path) -> Option<PathBuf> {
        const WHERE: &[&str] = &[
            "/System/Library/Fonts/STHeiti Medium.ttc",
            "/System/Library/Fonts/Hiragino Sans GB.ttc",
            "/System/Library/Fonts/Supplemental/Songti.ttc",
            "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
            "/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc",
            "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
            "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
        ];
        let font = WHERE.iter().map(Path::new).find(|p| p.is_file())?;
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

/// **The document typst is given**: a page the size of the formula, no
/// ground, the text in the page's own ink.
pub fn document(key: &Key) -> String {
    let (r, g, b) = key.ink;
    let size = key.size();
    let block = key.maths.display;
    let mut out = String::new();
    if key.maths.latex {
        out.push_str("#import \"@local/mitex:0.2.6\": mimath\n");
    }
    out.push_str("#set page(width: auto, height: auto, margin: 2pt, fill: none)\n");
    out.push_str(&format!("#set text(size: {size}pt, fill: rgb(\"#{r:02x}{g:02x}{b:02x}\"))\n"));
    // **The page is as tall as the ink, not the line** (2026-11-06): by
    // default a line runs from cap height to baseline, and a page of
    // `height: auto` cut an inline `1/2` or `sqrt` off at both edges.
    out.push_str("#set text(top-edge: \"bounds\", bottom-edge: \"bounds\")\n");
    match key.maths.latex {
        true => out.push_str(&format!("#mimath(sys.inputs.at(\"{INPUT}\"), block: {block})\n")),
        false => out.push_str(&format!(
            "#math.equation(block: {block}, eval(sys.inputs.at(\"{INPUT}\"), mode: \"math\"))\n"
        )),
    }
    out
}

/// Whether `text` has a character the bundled maths fonts do not have — a
/// 漢字, a kana, a hangul — and so needs a CJK font.
fn wants_cjk(text: &str) -> bool {
    text.chars().any(|c| c as u32 >= 0x2E80)
}

/// Run typst on one formula.
fn render(key: &Key, place: Option<&Place>) -> Answer {
    use std::io::{Read, Write};
    let mut command = yumete_core::export::typst_command();
    command.args(["compile", "-", "-", "--format", "png", "--ppi", "72"]);
    if let Some(place) = place {
        command.arg("--root").arg(&place.work).current_dir(&place.work);
        command.arg("--package-path").arg(&place.packages);
        command.arg("--package-cache-path").arg(&place.cache);
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
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(pipe) = stderr.as_mut() {
            let _ = pipe.read_to_string(&mut text);
        }
        text
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
    let said = err.join().unwrap_or_default();
    match status {
        Some(status) if status.success() => match crate::graphics::png_size(&png) {
            Some((width, height)) => Answer::Ready(Arc::new(Picture { png, width, height })),
            None => Answer::Failed("no picture came back".to_string()),
        },
        Some(_) => Answer::Failed(first_error(&said)),
        None => Answer::Failed(format!("more than {} s", PATIENCE.as_secs())),
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
static LAST: Mutex<Option<(u64, Arc<Picture>)>> = Mutex::new(None);

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
    if !config.editor.maths_preview || composing || crate::graphics::support() == crate::graphics::Support::None {
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
    let last = || LAST.lock().unwrap_or_else(|e| e.into_inner()).clone();
    // **While typing, a formula is broken most of the time** — `\frac{` on
    // the way to `\frac{1}{2}` — so in Insert the last picture stays up
    // instead of typst's complaint, and the complaint is said once the hand
    // stops (Normal).
    let typing = editor.mode() == yumete_core::Mode::Insert;
    let picture = match ask(&key) {
        Answer::Ready(picture) => {
            *LAST.lock().unwrap_or_else(|e| e.into_inner()) = Some((key.hash(), picture.clone()));
            (key.hash(), picture)
        }
        Answer::Making => match last() {
            Some(last) => last,
            None => return Some(None),
        },
        Answer::NoTypst => return Some(said(frame, config, area, bottom, caret, vertical, say_no_typst())),
        Answer::Failed(_) if typing && last().is_some() => last()?,
        Answer::Failed(why) => {
            return Some(said(frame, config, area, bottom, caret, vertical, yumete_core::say!("maths.failed", why)))
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
    let room = (
        room_w.max(24).min(area.width).saturating_sub(4),
        room_h.max(3).min(tall).saturating_sub(2),
    );
    let (columns, rows) = cells_for((picture.width, picture.height), cell, room);
    let width = (columns + 4).max(8);
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

    /// The document: LaTeX through the bundled package, Typst through
    /// `eval`, the formula never written into it.
    #[test]
    fn the_document_hands_the_formula_over_as_an_input() {
        let tex = document(&key(r"\frac{1}{2}", true, true));
        assert!(tex.contains("#import \"@local/mitex:0.2.6\": mimath"), "{tex}");
        assert!(tex.contains("block: true"), "{tex}");
        assert!(tex.contains("rgb(\"#c8a2ff\")"), "{tex}");
        assert!(tex.contains("size: 30pt"), "{tex}");
        assert!(!tex.contains("frac"), "the formula is an input, not text: {tex}");
        let typ = document(&key("x^2", false, false));
        assert!(!typ.contains("mitex"), "{typ}");
        assert!(typ.contains("eval(sys.inputs.at(\"yumete-maths\"), mode: \"math\")"), "{typ}");
        assert!(typ.contains("block: false"), "{typ}");
    }

    /// **An inline fraction is not cut off** (2026-11-06): the page is as
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

    /// **The real thing**, when this machine has typst: a formula comes back
    /// as a PNG, LaTeX through the bundled package with nothing downloaded.
    /// Skipped where there is no typst.
    #[test]
    fn typst_makes_the_picture() {
        if !yumete_core::export::typst_is_installed() {
            eprintln!("no typst here: skipped");
            return;
        }
        let place = Place::set_up();
        for k in [key("sum_(k=1)^n k", false, true), key(r"\sqrt{\alpha^2+\beta^2}", true, false), key("\"面积\" = pi r^2", false, true)] {
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
