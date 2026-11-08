//! **TeX formulas as pictures, by RaTeX** (2026-11-08).
//!
//! Markdown writes its maths in TeX (`$…$`, `$$…$$`). The formula float
//! under the cursor ([`crate::maths`]) draws it here, and anything else that
//! shows a TeX formula set should too, so a formula looks the same
//! everywhere. Typst's own maths stays with the `typst` command
//! (`maths.rs`): it is a different language.
//!
//! [RaTeX](https://github.com/erweixin/RaTeX) (MIT) is KaTeX's layout in
//! Rust, with KaTeX's fonts (SIL OFL) built into the binary: no process, no
//! download, and a formula in well under a millisecond once warm, so it is
//! drawn where it is asked for.
//!
//! Two things it does that a terminal editor cannot have:
//!
//! - **It writes to standard error** (`ratex-unicode-font`) the first time it
//!   looks for a font for a character KaTeX's fonts lack — a 漢字 in
//!   `\text{}` — and standard error is the screen. Every render runs with
//!   standard error pointed at a file of its own ([`quietly`]); whatever was
//!   said goes to `yumete.log`. Per render rather than once at start-up,
//!   because which formula first needs the font, and whether any ever does,
//!   is not known, and pointing it away costs a few microseconds. A lock
//!   keeps two renders from crossing.
//! - **It may panic** on input it was not written for. A panic in it is
//!   caught ([`render`]) and said as the formula's error; the front end's
//!   panic hook leaves the screen alone for it ([`panicking_quietly`]).
//!
//! **The font for 漢字 in a formula** is RaTeX's own choice, and the only way
//! to make it is the `RATEX_UNICODE_FONT` variable: [`choose_cjk_font`] sets
//! it, once at start-up, to the CJK font the Typst formulas use
//! ([`CJK_FONTS`]), unless it is already set. On a Linux without any CJK
//! font, 漢字 in a formula come out as empty boxes.

use std::cell::Cell;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::maths::Picture;

/// Where systems keep a CJK font: the first one found is the one formulas
/// use, Typst's ([`crate::maths`]) and RaTeX's alike.
pub const CJK_FONTS: &[&str] = &[
    "/System/Library/Fonts/STHeiti Medium.ttc",
    "/System/Library/Fonts/Hiragino Sans GB.ttc",
    "/System/Library/Fonts/Supplemental/Songti.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    "/usr/share/fonts/opentype/noto/NotoSerifCJK-Regular.ttc",
    "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
    "/usr/share/fonts/wenquanyi/wqy-microhei/wqy-microhei.ttc",
    "C:\\Windows\\Fonts\\msyh.ttc",
    "C:\\Windows\\Fonts\\simsun.ttc",
];

/// The first CJK font in [`CJK_FONTS`] this system has.
pub fn cjk_font() -> Option<&'static str> {
    CJK_FONTS.iter().copied().find(|p| std::path::Path::new(p).is_file())
}

/// **Tell RaTeX which font to set 漢字 in**: the one the Typst formulas use,
/// unless `RATEX_UNICODE_FONT` already names one. Called once by `main`,
/// before any thread is started — changing the environment is safe only
/// then.
pub fn choose_cjk_font() {
    if std::env::var_os("RATEX_UNICODE_FONT").is_some() {
        return;
    }
    if let Some(font) = cjk_font() {
        std::env::set_var("RATEX_UNICODE_FONT", font);
    }
}

thread_local! {
    static QUIET_PANIC: Cell<bool> = const { Cell::new(false) };
}

/// Whether the panic under way is one [`render`] catches: the front end's
/// panic hook leaves the screen as it is for it.
pub fn panicking_quietly() -> bool {
    QUIET_PANIC.with(Cell::get)
}

/// The colour ink is drawn in: red, green, blue.
pub type Ink = (u8, u8, u8);

/// **Draw `tex`** in `ink`, `size` pixels to the em, set on its own when
/// `display`: a PNG with a clear ground, or what was wrong with it. Never
/// panics, and writes nothing to the screen.
pub fn render(tex: &str, display: bool, ink: Ink, size: f32) -> Result<Picture, String> {
    render_on_baseline(tex, display, ink, size).map(|(picture, _)| picture)
}

/// **[`render`], and where the formula's baseline is**: pixels from the
/// picture's top, so a formula in a line of proportional type can sit on
/// the line's baseline (the rendered Markdown preview, `yumete-mdview`).
pub fn render_on_baseline(tex: &str, display: bool, ink: Ink, size: f32) -> Result<(Picture, f32), String> {
    let (size, padding) = (size.max(4.0), (size / 8.0).clamp(1.0, 4.0));
    let (png, [_, height, _]) = draw(tex, display, ink, size, padding, 1.0)?;
    let baseline = padding + height as f32 * size;
    let (width, height) = crate::graphics::png_size(&png).ok_or_else(|| "no picture came back".to_string())?;
    Ok((Picture { png, width, height }, baseline))
}

/// **A formula for `:export html`** (2026-11-08): in `ink` on clear — black
/// for a printed page, the preview's writing for `:export-as-preview` — drawn at
/// three times a 40-pixel em so it stays sharp when printed, and measured in
/// `em` of the text around it — 1.1 of it, a little larger than the text as
/// KaTeX sets it (1.21), since a CJK line is already a full square tall.
/// Handed to the editor by `main` ([`yumete_core::Editor::set_formula_painter`]).
///
/// **`turned`** (2026-11-08), for a formula down a vertical page: the
/// picture is turned a quarter turn clockwise in the bitmap, and measured as
/// it is then shown — as wide as it was tall and as tall as it was wide —
/// so the page shows an ordinary picture and turns nothing itself, which
/// every browser and reader app does alike.
pub fn for_export(tex: &str, display: bool, turned: bool, ink: [u8; 3]) -> Result<yumete_core::export::Formula, String> {
    const SIZE: f32 = 40.0;
    const PAD: f32 = 1.0;
    const SCALE: f64 = 1.1;
    let (png, [width, height, depth]) = draw(tex, display, (ink[0], ink[1], ink[2]), SIZE, PAD, 3.0)?;
    let pad = f64::from(PAD / SIZE);
    let (width, height, depth) = ((width + 2.0 * pad) * SCALE, (height + depth + 2.0 * pad) * SCALE, (depth + pad) * SCALE);
    Ok(match turned {
        false => yumete_core::export::Formula { png, width, height, depth },
        true => yumete_core::export::Formula {
            png: turn_clockwise(&png).ok_or_else(|| "the picture could not be turned".to_string())?,
            width: height,
            height: width,
            depth,
        },
    })
}

/// A PNG turned a quarter turn clockwise, as RGBA; `None` for bytes that are
/// not a PNG.
fn turn_clockwise(png: &[u8]) -> Option<Vec<u8>> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(png));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut buf).ok()?;
    let channels = match reader.output_color_type().0 {
        png::ColorType::Grayscale => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        png::ColorType::Indexed => return None,
    };
    let (w, h) = (frame.width as usize, frame.height as usize);
    let rgba = |x: usize, y: usize| -> [u8; 4] {
        let p = &buf[(y * w + x) * channels..][..channels];
        match channels {
            1 => [p[0], p[0], p[0], 255],
            2 => [p[0], p[0], p[0], p[1]],
            3 => [p[0], p[1], p[2], 255],
            _ => [p[0], p[1], p[2], p[3]],
        }
    };
    // Row `y` of the turned picture is column `y` of the upright one, read
    // from its foot up.
    let mut turned = Vec::with_capacity(w * h * 4);
    for y in 0..w {
        for x in 0..h {
            turned.extend_from_slice(&rgba(y, h - 1 - x));
        }
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, h as u32, w as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(&turned).ok()?;
    }
    Some(out)
}

/// The PNG, and the formula's width and how far it reaches above and below
/// its baseline, in `em`; `padding` pixels of clear ground round it, all of
/// it `dpr` times.
fn draw(tex: &str, display: bool, ink: Ink, size: f32, padding: f32, dpr: f32) -> Result<(Vec<u8>, [f64; 3]), String> {
    use ratex_types::color::Color;
    use ratex_types::math_style::MathStyle;
    let tex = tex.trim();
    if tex.is_empty() {
        return Err("empty formula".to_string());
    }
    let colour = Color { r: ink.0 as f32 / 255.0, g: ink.1 as f32 / 255.0, b: ink.2 as f32 / 255.0, a: 1.0 };
    let style = if display { MathStyle::Display } else { MathStyle::Text };
    let options = ratex_render::RenderOptions {
        font_size: size,
        padding,
        background_color: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
        font_dir: String::new(),
        device_pixel_ratio: dpr,
    };
    let made = quietly(|| {
        QUIET_PANIC.with(|q| q.set(true));
        let made = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let ast = ratex_parser::parser::parse(tex).map_err(|e| e.to_string())?;
            let layout = ratex_layout::LayoutOptions::default().with_style(style).with_color(colour);
            let laid = ratex_layout::layout(&ast, &layout);
            ratex_layout::validate_layout(&laid)?;
            let list = ratex_layout::to_display_list(&laid);
            ratex_render::render_to_png(&list, &options).map(|png| (png, [list.width, list.height, list.depth]))
        }));
        QUIET_PANIC.with(|q| q.set(false));
        made
    });
    match made {
        Ok(result) => result.map_err(|e| tidy(&e)),
        Err(_) => Err("RaTeX could not lay this formula out".to_string()),
    }
}

/// RaTeX's message, without the type name it opens with.
fn tidy(said: &str) -> String {
    let said = said.lines().next().unwrap_or(said).trim();
    said.strip_prefix("ParseError: ")
        .or_else(|| said.strip_prefix("ParseError "))
        .unwrap_or(said)
        .trim()
        .to_string()
}

/// One lock for every render: standard error is the process's, and two
/// renders pointing it away at once would each put back the other's file.
static QUIET: Mutex<()> = Mutex::new(());

/// **Run `f` with standard error pointed into a file of its own**, then put
/// it back, and note in the log whatever was written there.
fn quietly<T>(f: impl FnOnce() -> T) -> T {
    let _held = QUIET.lock().unwrap_or_else(|e| e.into_inner());
    let Some(away) = Away::start() else { return f() };
    let out = f();
    let said = away.finish();
    if !said.trim().is_empty() {
        yumete_core::diag::note("formula", said.trim());
    }
    out
}

/// Standard error, pointed away.
struct Away {
    file: std::fs::File,
    #[cfg(unix)]
    saved: libc::c_int,
    #[cfg(windows)]
    saved: windows_sys::Win32::Foundation::HANDLE,
    done: bool,
}

/// A file no one else can have opened: made new, in this user's temporary
/// folder, and taken off the disk as soon as it is open.
fn private_file() -> Option<std::fs::File> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let n = N.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("yumete-tex-{}-{n}.txt", std::process::id()));
    let file = std::fs::OpenOptions::new().read(true).write(true).create_new(true).open(&path).ok()?;
    // Unlinked at once on Unix: the file lives on through the descriptor.
    #[cfg(unix)]
    let _ = std::fs::remove_file(&path);
    #[cfg(not(unix))]
    PENDING.with(|p| p.borrow_mut().push(path));
    Some(file)
}

#[cfg(not(unix))]
thread_local! {
    /// Files Windows will not let go of while open, removed once closed.
    static PENDING: std::cell::RefCell<Vec<std::path::PathBuf>> = const { std::cell::RefCell::new(Vec::new()) };
}

impl Away {
    #[cfg(unix)]
    fn start() -> Option<Away> {
        use std::io::Write;
        use std::os::unix::io::AsRawFd;
        let file = private_file()?;
        let _ = std::io::stderr().flush();
        // SAFETY: plain descriptor calls on descriptors this process owns;
        // each result is checked before it is used.
        unsafe {
            let saved = libc::dup(libc::STDERR_FILENO);
            if saved < 0 {
                return None;
            }
            if libc::dup2(file.as_raw_fd(), libc::STDERR_FILENO) < 0 {
                libc::close(saved);
                return None;
            }
            Some(Away { file, saved, done: false })
        }
    }

    #[cfg(windows)]
    fn start() -> Option<Away> {
        use std::io::Write;
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_ERROR_HANDLE};
        let file = private_file()?;
        let _ = std::io::stderr().flush();
        // SAFETY: the standard handle is read and replaced by a handle this
        // process owns, and put back when this is dropped.
        unsafe {
            let saved = GetStdHandle(STD_ERROR_HANDLE);
            if SetStdHandle(STD_ERROR_HANDLE, file.as_raw_handle() as _) == 0 {
                return None;
            }
            Some(Away { file, saved, done: false })
        }
    }

    #[cfg(not(any(unix, windows)))]
    fn start() -> Option<Away> {
        None
    }

    /// Put standard error back.
    fn restore(&mut self) {
        if std::mem::replace(&mut self.done, true) {
            return;
        }
        use std::io::Write;
        let _ = std::io::stderr().flush();
        #[cfg(unix)]
        // SAFETY: `saved` is the descriptor `start` duplicated.
        unsafe {
            libc::dup2(self.saved, libc::STDERR_FILENO);
            libc::close(self.saved);
        }
        #[cfg(windows)]
        // SAFETY: `saved` is the handle `start` read.
        unsafe {
            windows_sys::Win32::System::Console::SetStdHandle(windows_sys::Win32::System::Console::STD_ERROR_HANDLE, self.saved);
        }
    }

    /// Put standard error back and read what was written.
    fn finish(mut self) -> String {
        use std::io::{Read, Seek};
        self.restore();
        let mut said = String::new();
        if self.file.seek(std::io::SeekFrom::Start(0)).is_ok() {
            let _ = (&self.file).take(64 * 1024).read_to_string(&mut said);
        }
        said
    }
}

impl Drop for Away {
    fn drop(&mut self) {
        self.restore();
        #[cfg(not(unix))]
        PENDING.with(|p| {
            for path in p.borrow_mut().drain(..) {
                let _ = std::fs::remove_file(path);
            }
        });
    }
}

/// **Formulas for the rendered Markdown preview** (2026-11-08): RaTeX, as
/// the formula float draws them, with where each one's baseline is.
pub struct Ratex;

impl yumete_mdview::Formulas for Ratex {
    fn draw(&self, tex: &str, display: bool, ink: [u8; 3], size: f32) -> Result<yumete_mdview::Formula, String> {
        let (picture, baseline) = render_on_baseline(tex, display, (ink[0], ink[1], ink[2]), size)?;
        Ok(yumete_mdview::Formula { png: picture.png, baseline })
    }
}

// ── Kept pictures ──────────────────────────────────────────────────────────

/// What a picture is a picture of: the formula, how it is set, its ink and
/// its size in tenths of a pixel to the em.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Key {
    pub tex: String,
    pub display: bool,
    pub ink: Ink,
    pub size: u32,
}

impl Key {
    /// The name the terminal's shelf knows the picture by.
    pub fn hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        ("tex", self).hash(&mut h);
        h.finish()
    }
}

/// How many formulas are kept drawn.
const KEPT: usize = 512;

#[derive(Default)]
struct Shelf {
    made: HashMap<Key, Result<Arc<Picture>, String>>,
    order: VecDeque<Key>,
}

static SHELF: Mutex<Option<Shelf>> = Mutex::new(None);

/// **The picture of `key`**, drawn now if it is not kept already.
pub fn picture(key: &Key) -> Result<Arc<Picture>, String> {
    {
        let shelf = SHELF.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(made) = shelf.as_ref().and_then(|s| s.made.get(key)) {
            return made.clone();
        }
    }
    let made = render(&key.tex, key.display, key.ink, key.size as f32 / 10.0).map(Arc::new);
    let mut shelf = SHELF.lock().unwrap_or_else(|e| e.into_inner());
    let shelf = shelf.get_or_insert_with(Shelf::default);
    if shelf.made.insert(key.clone(), made.clone()).is_none() {
        shelf.order.push_back(key.clone());
    }
    while shelf.order.len() > KEPT {
        if let Some(old) = shelf.order.pop_front() {
            shelf.made.remove(&old);
        }
    }
    made
}

#[cfg(test)]
mod tests {
    use super::*;

    const INK: Ink = (0x20, 0x20, 0x20);

    #[test]
    fn a_formula_comes_back_as_a_picture() {
        let inline = render(r"x^2", false, INK, 20.0).expect("x^2");
        let display = render(r"\sum_{k=1}^{n} k = \frac{n(n+1)}{2}", true, INK, 20.0).expect("a sum");
        assert!(inline.width > 5 && inline.height > 5, "{}×{}", inline.width, inline.height);
        assert!(display.height > inline.height, "set on its own it is taller");
        assert_eq!(crate::graphics::png_size(&display.png), Some((display.width, display.height)));
        // Bigger type, a bigger picture.
        let big = render(r"x^2", false, INK, 40.0).unwrap();
        assert!(big.width > inline.width && big.height > inline.height);
    }

    /// **The baseline comes back with the picture**: inside it, below the
    /// middle of a formula that only rises (`x^2`), above the bottom of one
    /// that also descends (`y_1`).
    #[test]
    fn a_formula_says_where_its_baseline_is() {
        let (up, base_up) = render_on_baseline(r"x^2", false, INK, 20.0).unwrap();
        assert!(base_up > up.height as f32 / 2.0 && base_up <= up.height as f32, "{base_up} in {}", up.height);
        let (down, base_down) = render_on_baseline(r"y_1", false, INK, 20.0).unwrap();
        assert!(base_down < down.height as f32 - 2.0, "{base_down} in {}", down.height);
    }

    /// **A formula for an HTML export** (2026-11-08) is measured in `em` of
    /// the text: about a line tall for `x^2`, reaching below the baseline
    /// for a fraction, and drawn three times finer than it is shown.
    #[test]
    fn a_formula_for_a_page_is_measured_in_em() {
        let x = for_export(r"x^2", false, false, [0, 0, 0]).expect("x^2");
        assert!(x.height > 0.8 && x.height < 1.6, "{}", x.height);
        assert!(x.depth >= 0.0 && x.depth < x.height, "{}", x.depth);
        let (_, tall) = crate::graphics::png_size(&x.png).expect("a PNG");
        // 1.1 of an em of text is one em of the 40-pixel drawing, three times.
        let shown = x.height / 1.1 * 40.0 * 3.0;
        assert!((f64::from(tall) - shown).abs() <= 2.0, "{tall} pixels for {shown}");
        let frac = for_export(r"\frac{a}{b}", true, false, [0, 0, 0]).unwrap();
        assert!(frac.height > x.height && frac.depth > x.depth);
        assert!(for_export(r"\frac{a", false, false, [0, 0, 0]).is_err());
    }

    /// **A formula turned for a vertical page** (2026-11-08) is turned in
    /// the bitmap, clockwise: the picture as tall as the upright one is wide,
    /// and what was its top row its right-hand column.
    #[test]
    fn a_formula_for_a_vertical_page_is_turned_in_the_bitmap() {
        let up = for_export(r"x^2+y^2", false, false, [0, 0, 0]).unwrap();
        let down = for_export(r"x^2+y^2", false, true, [0, 0, 0]).unwrap();
        assert_eq!((down.width, down.height), (up.height, up.width));
        let (w, h) = crate::graphics::png_size(&up.png).unwrap();
        assert_eq!(crate::graphics::png_size(&down.png), Some((h, w)));
        // Two pixels, red over blue, become blue beside red.
        let mut made = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut made, 1, 2);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.write_header().unwrap().write_image_data(&[255, 0, 0, 0, 0, 255]).unwrap();
        }
        let turned = turn_clockwise(&made).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(&turned)).read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buf).unwrap();
        assert_eq!((frame.width, frame.height), (2, 1));
        assert_eq!(&buf[..8], &[0, 0, 255, 255, 255, 0, 0, 255]);
    }

    /// **A broken formula is said, never a panic**: an unknown command, an
    /// argument never closed, an empty one.
    #[test]
    fn a_broken_formula_says_what_is_wrong() {
        for tex in [r"\nosuchcommand{x}", r"\frac{a}{", "", "   ", r"\begin{matrix} a", "}}}"] {
            match render(tex, true, INK, 20.0) {
                Err(why) => assert!(!why.is_empty(), "{tex:?}"),
                Ok(_) => panic!("{tex:?} drew"),
            }
        }
        assert!(tidy("ParseError: Undefined control sequence: \\foo").starts_with("Undefined control sequence"));
    }

    #[test]
    fn a_drawn_formula_is_kept() {
        let key = Key { tex: r"\alpha+\beta".to_string(), display: false, ink: INK, size: 200 };
        let first = picture(&key).unwrap();
        let again = picture(&key).unwrap();
        assert!(Arc::ptr_eq(&first, &again), "drawn once");
        assert_ne!(key.hash(), Key { size: 210, ..key.clone() }.hash());
    }

    /// **Nothing reaches the screen** (2026-11-08): RaTeX says which font it
    /// found for a 漢字 on standard error, and standard error is the
    /// terminal. Run in a process of its own, with nothing capturing its
    /// output, and its standard error read back.
    #[test]
    fn a_formula_writes_nothing_to_standard_error() {
        if std::env::var_os("YUMETE_TEX_STDERR_CHILD").is_some() {
            let _ = render(r"\text{面积} = \pi r^2", true, INK, 24.0);
            let _ = render(r"\text{한국어とかな}", false, INK, 24.0);
            let _ = render(r"\broken{", false, INK, 24.0);
            return;
        }
        let me = std::env::current_exe().unwrap();
        let out = std::process::Command::new(me)
            .args(["--exact", "tex::tests::a_formula_writes_nothing_to_standard_error", "--nocapture", "--test-threads=1"])
            .env("YUMETE_TEX_STDERR_CHILD", "1")
            .output()
            .unwrap();
        let said = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "the child failed: {said}");
        assert!(!said.contains("ratex"), "standard error got: {said}");
        assert!(said.trim().is_empty(), "standard error got: {said}");
    }
}
