//! 【墨香】 — one theme, computed, and everywhere (Feature #152).
//!
//! Every colour the editor draws comes from here, and here is **four inks**:
//!
//! - **黑墨** — the ground the page is written on.
//! - **白金墨** — the writing. A white with a little gold in it, not a bone:
//!   over a whole screen a bone ink turns every grey on the ladder warm, and
//!   then the prose, the headings and the furniture are one colour and the
//!   page has no ranks in it.
//! - **金墨** — what is **not** prose: a heading, a table's header row, the
//!   name beside a value. Worth finding precisely because the rest is grey.
//! - **紅墨** — what is **wrong**: a torn row, a component with no row, a
//!   footnote's mark. Never emphasis.
//!
//! Everything between the first two is a rung on a ladder ([`Ladder::step`]);
//! 金 and 紅 are not quantities of ink and so cannot be on it.
//!
//! Before this, the two anchors dressed the candidate panel, the sidebar, the
//! tab bar, the `:` menu and the detail panel — and the manuscript itself was
//! drawn with no colour at all, in the terminal's own ink on the terminal's own
//! ground. Every other tint in the program was a hand-picked RGB triple in one
//! of four files, measured against a ground the editor did not control. Two of
//! them measured 1.00:1 and 1.11:1 against what they were drawn on: a rule
//! painted in exactly its own background, and a 稿紙 tick that has never been
//! seen.
//!
//! ## Where things sit
//!
//! The rungs are not evenly useful and [`rung`] says why: below 350 a colour is
//! ink you can read, above 730 it is a ground you can write on, and the third
//! of the ladder between the two carries nothing except a rule, which is
//! neither. Anything placed there by eye — and several things were — comes out
//! too faint to read and too pale to write on.

use ratatui::style::{Color, Style};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::RwLock;
use yumete_config::{rung, Config, Ground, Ladder, Mode, ThemeConfig};

/// The theme `:theme` picked, if it picked one.
///
/// The config's own theme is what the editor starts in; this is the one a
/// reader named while it was running, and it outranks it for as long as the
/// session lasts. Not written back to the config file — that is for what you
/// want every day.
static CHOSEN: RwLock<Option<ThemeConfig>> = RwLock::new(None);

/// What the theme in force is called.
pub fn name(config: &Config) -> String {
    in_force(config).name
}

/// Wear this theme from now on.
pub fn choose(theme: ThemeConfig) {
    if let Ok(mut chosen) = CHOSEN.write() {
        *chosen = Some(theme);
    }
}

/// The theme in force: the one `:theme` named, else the config's.
fn in_force(config: &Config) -> ThemeConfig {
    let mut theme = match CHOSEN.read().ok().and_then(|c| c.clone()) {
        Some(theme) => theme,
        None => config.theme.clone(),
    };
    if let Some(fill) = FILL.load(Ordering::Relaxed).checked_sub(1) {
        theme.fill = fill == 1;
    }
    theme
}

/// What `:theme-fill` last said: `0` nothing, `1` off, `2` on.
///
/// Carried beside the theme rather than in it, so that changing the theme does
/// not silently undo the answer — 「代碼要不要坐在框裏」 is a habit of the
/// reader's, not a property of 墨香.
static FILL: AtomicU8 = AtomicU8::new(0);

/// Say whether a coloured run gets a ground, and answer what is in force now.
pub fn set_fill(on: Option<bool>, config: &Config) -> bool {
    let now = in_force(config).fill;
    let want = on.unwrap_or(!now);
    FILL.store(1 + u8::from(want), Ordering::Relaxed);
    want
}

/// Which mood is in force, once the terminal has been asked.
///
/// A global, like the language: it is settled once at startup, and every part
/// of the drawing needs it without having anywhere to have been handed it.
static MOOD: AtomicU8 = AtomicU8::new(0);

/// What the terminal said when it was asked, so `system` can be gone back to.
///
/// Asked once, at start-up, and remembered: the question cannot be put again
/// from inside the alternate screen without racing the key reader for the
/// answer, and a terminal's ground rarely changes under a running program.
/// `0` is「沒問到」.
static ANSWERED: AtomicU8 = AtomicU8::new(0);

/// Remember what the terminal turned out to be.
pub fn set_dark(dark: bool) {
    MOOD.store(u8::from(dark), Ordering::Relaxed);
}

/// Whether the mood was chosen out loud (`:theme-mode`), rather than settled
/// from the config or the terminal.
static CHOSEN_MOOD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Say the mood **and remember that it was asked for**, so nothing settles over
/// it afterwards. What `:theme-mode` calls; `settle` is what start-up calls.
pub fn choose_mood(dark: bool) {
    CHOSEN_MOOD.store(true, Ordering::Relaxed);
    set_dark(dark);
}

/// Whether the page is dark.
pub fn dark() -> bool {
    MOOD.load(Ordering::Relaxed) == 1
}

/// Settle the mood from the config and, when it says `auto`, from the terminal.
///
/// `auto` asks the **terminal**, not the operating system, and that is the
/// right question: a dark terminal on a light desktop wants a dark editor, and
/// the reader who set it that way has already answered.
pub fn settle(config: &Config, terminal_is_dark: Option<bool>) {
    // ⚠️ **A mood somebody asked for out loud is not re-settled** (#470).
    // `frame_to` settles before every offscreen frame, which is right for the
    // ordinary case and wrong the moment `--keys=':theme-mode light'` has
    // already answered: the config's `mode` would be applied straight over the
    // reader's own word, and the picture would show the theme they did not
    // ask for.
    if CHOSEN_MOOD.load(Ordering::Relaxed) {
        return;
    }
    ANSWERED.store(
        match terminal_is_dark {
            None => 0,
            Some(true) => 1,
            Some(false) => 2,
        },
        Ordering::Relaxed,
    );
    set_dark(match config.theme.mode {
        Mode::Dark => true,
        Mode::Light => false,
        // No answer from the terminal means an old one, and an old terminal is
        // far likelier to be dark than light.
        Mode::Auto => terminal_is_dark.unwrap_or(true),
    });
}

/// Settle the mood, asking the terminal only if the config has not decided.
///
/// The one call the editor makes at start-up. A reader who wrote `mode =
/// "dark"` is not asked anything, and neither is their terminal — the question
/// costs a round trip and the answer would be thrown away.
pub fn settle_at_startup(config: &Config) {
    let asked = match config.theme.mode {
        Mode::Auto => ask_the_terminal(),
        _ => None,
    };
    settle(config, asked);
}

/// What the terminal answered at start-up, if it answered.
pub fn terminal_answer() -> Option<bool> {
    match ANSWERED.load(Ordering::Relaxed) {
        1 => Some(true),
        2 => Some(false),
        _ => None,
    }
}

/// Ask the terminal what colour its ground is (OSC 11).
///
/// `mode = "auto"` has to be answered before the first frame, and the terminal
/// is the one that knows: it is asked `\x1b]11;?` and replies with its own
/// background as `rgb:RRRR/GGGG/BBBB`. This is a better question than the
/// desktop's appearance setting, which is what "follows the system" usually
/// means — a reader with a dark terminal on a light desktop has already said
/// which they want, and their terminal is the thing whose ground the editor is
/// about to paint over.
///
/// Terminals that do not answer are left to the config's own fallback, and an
/// unanswering one costs a tenth of a second at start-up.
///
/// ⚠️ The reply is read **straight off the descriptor**, which takes whatever
/// is in it — the terminal's answer *and anything the reader has already
/// typed*. That used to be dropped, and it is the first tenth of a second of
/// the session: `yumete 第三章.md` followed straight away by `iHELLO` left the
/// file untouched. What is not part of the reply now goes to
/// [`crate::typed_ahead`], which the front end plays back before it enters the
/// loop.
#[cfg(unix)]
pub fn ask_the_terminal() -> Option<bool> {
    use std::io::{IsTerminal, Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    let mut out = std::io::stdout();
    if !out.is_terminal() || !std::io::stdin().is_terminal() {
        return None;
    }
    // Raw, or the terminal would echo the reply onto the screen and hand it
    // over a line at a time — which is to say, never.
    let raw = ratatui::crossterm::terminal::enable_raw_mode().is_ok();
    let answer = (|| {
        out.write_all(b"\x1b]11;?\x1b\\").ok()?;
        out.flush().ok()?;
        let deadline = Instant::now() + Duration::from_millis(120);
        let fd = std::io::stdin().as_raw_fd();
        let mut reply = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                // Nothing came that was an answer, so all of it was a person.
                crate::typed_ahead::keep(&reply, 0..0);
                return None;
            }
            let mut watch = libc::pollfd {
                fd,
                events: libc::POLLIN,
                revents: 0,
            };
            // SAFETY: one initialised `pollfd` describing a descriptor this
            // process owns, and a timeout in milliseconds.
            if unsafe { libc::poll(&mut watch, 1, left.as_millis() as libc::c_int) } <= 0 {
                // ⚠️ **The common path, and the one that drops keystrokes.**
                // A terminal that does not answer times out here, and
                // everything read on the way — which is the reader typing —
                // was thrown away with the buffer.
                crate::typed_ahead::keep(&reply, 0..0);
                return None;
            }
            let mut chunk = [0u8; 64];
            match std::io::stdin().read(&mut chunk) {
                Ok(0) | Err(_) => {
                    crate::typed_ahead::keep(&reply, 0..0);
                    return None;
                }
                Ok(n) => reply.extend_from_slice(&chunk[..n]),
            }
            if let Some(dark) = ground_is_dark(&reply) {
                crate::typed_ahead::keep(&reply, osc_span(&reply));
                return Some(dark);
            }
            // A terminal that is answering something else entirely.
            if reply.len() > 256 {
                crate::typed_ahead::keep(&reply, 0..0);
                return None;
            }
        }
    })();
    if raw {
        let _ = ratatui::crossterm::terminal::disable_raw_mode();
    }
    answer
}

#[cfg(not(unix))]
pub fn ask_the_terminal() -> Option<bool> {
    None
}

/// Read `…rgb:RRRR/GGGG/BBBB…` out of a terminal's reply, and weigh it.
///
/// The components are hexadecimal and may be one to four digits wide, which is
/// a scale and not a number: `f` and `ffff` are both full. Each is read as a
/// fraction of its own width.
///
/// Only the probe above calls it, and the probe is the unix one — so on a
/// platform without it this parser is dead code rather than a warning.
#[cfg(unix)]
/// Where the `\x1b]11;…rgb:…` answer sits inside everything that was read.
///
/// Everything outside it was typed by a person — see [`crate::typed_ahead`].
/// An answer with no terminator yet runs to the end of what has arrived.
fn osc_span(reply: &[u8]) -> std::ops::Range<usize> {
    let Some(rgb) = find(reply, b"rgb:") else {
        return 0..0;
    };
    // Back to the `\x1b]` that introduced it, and forward to `\x1b\` or BEL.
    let start = reply[..rgb]
        .windows(2)
        .rposition(|w| w == b"\x1b]")
        .unwrap_or(rgb);
    let tail = &reply[rgb..];
    let end = tail
        .iter()
        .position(|&b| b == 0x07)
        .map(|i| rgb + i + 1)
        .or_else(|| find(tail, b"\x1b\\").map(|i| rgb + i + 2))
        .unwrap_or(reply.len());
    start..end
}

/// The first index at which `needle` appears in `hay`.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn ground_is_dark(reply: &[u8]) -> Option<bool> {
    let text = std::str::from_utf8(reply).ok()?;
    let rest = text.split("rgb:").nth(1)?;
    let mut parts = rest.split('/');
    let mut channel = || -> Option<f64> {
        let digits: String = parts
            .next()?
            .chars()
            .take_while(char::is_ascii_hexdigit)
            .collect();
        if digits.is_empty() {
            return None;
        }
        let full = 16f64.powi(digits.len() as i32) - 1.0;
        Some(u32::from_str_radix(&digits, 16).ok()? as f64 / full)
    };
    let (r, g, b) = (channel()?, channel()?, channel()?);
    // The reply is not complete until the third component has all its digits,
    // and a `/` after it — or a terminator — is what says so.
    if !rest.split('/').nth(2)?.chars().any(|c| !c.is_ascii_hexdigit()) {
        return None;
    }
    let light = |v: f64| match v <= 0.04045 {
        true => v / 12.92,
        false => ((v + 0.055) / 1.055).powf(2.4),
    };
    // Halfway up in *perceived* lightness (L\* 50), not halfway up the numbers:
    // a mid grey looks light long before its luminance reaches a half.
    Some(0.2126 * light(r) + 0.7152 * light(g) + 0.0722 * light(b) < 0.1833)
}

/// Every colour, from three numbers.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    ladder: Ladder,
    mark: (u8, u8, u8),
    gold: (u8, u8, u8),
    /// 官服品色 — 紫（一三品）、綠（六七品）、藍（八九品）, for the things on
    /// the page that are **not prose**: a literal, a quotation, an address.
    purple: (u8, u8, u8),
    green: (u8, u8, u8),
    azure: (u8, u8, u8),
    amber: (u8, u8, u8),
    /// 變調 — 橙、粉、青、黃綠.
    orange: (u8, u8, u8),
    pink: (u8, u8, u8),
    cyan: (u8, u8, u8),
    lime: (u8, u8, u8),
    /// Whether a coloured run also gets a ground (`:theme-fill`).
    fill: bool,
    paint: bool,
    /// Whether this is the work area that is only being *read* (Feature #176).
    ///
    /// Everything in it steps one rung back toward the page — the writing, the
    /// furniture, 金 and 朱 alike — so which half the keys are in is answered
    /// by *weight*, at a glance, without a second colour and without painting
    /// the two halves different grounds. It is what tmux does to an inactive
    /// pane and what a printed page does to a facing note: the thing you are
    /// not reading is not louder, it is quieter.
    faded: bool,
}

/// One of the colours that is **not** a quantity of ink — 金, 朱, and the three
/// 官服品色 — named so a ground can be asked for by which one it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    /// 龍袍.
    Gold,
    /// 朱 — 這裏不對.
    Mark,
    /// 一至三品.
    Purple,
    /// 六至七品.
    Green,
    /// 八至九品.
    Azure,
    /// 皇室 — under 龍袍's 金.
    Amber,
}

/// A colour as hue (degrees), lightness and saturation (both per cent).
fn to_hsl((r, g, b): (u8, u8, u8)) -> (f64, f64, f64) {
    let (r, g, b) = (f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let light = (max + min) / 2.0;
    let span = max - min;
    if span.abs() < f64::EPSILON {
        return (0.0, light * 100.0, 0.0);
    }
    let sat = match light > 0.5 {
        true => span / (2.0 - max - min),
        false => span / (max + min),
    };
    let hue = if max == r {
        ((g - b) / span) % 6.0
    } else if max == g {
        (b - r) / span + 2.0
    } else {
        (r - g) / span + 4.0
    };
    ((hue * 60.0).rem_euclid(360.0), light * 100.0, sat * 100.0)
}

/// The colour that hue, lightness and saturation name.
fn from_hsl(hue: f64, light: f64, sat: f64) -> Color {
    let (light, sat) = (light / 100.0, sat / 100.0);
    if sat.abs() < f64::EPSILON {
        let v = (light * 255.0).round().clamp(0.0, 255.0) as u8;
        return Color::Rgb(v, v, v);
    }
    let q = match light < 0.5 {
        true => light * (1.0 + sat),
        false => light + sat - light * sat,
    };
    let p = 2.0 * light - q;
    let channel = |mut t: f64| -> u8 {
        t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 {
            p + (q - p) * 6.0 * t
        } else if t < 0.5 {
            q
        } else if t < 2.0 / 3.0 {
            p + (q - p) * (2.0 / 3.0 - t) * 6.0
        } else {
            p
        };
        (v * 255.0).round().clamp(0.0, 255.0) as u8
    };
    let h = hue / 360.0;
    Color::Rgb(channel(h + 1.0 / 3.0), channel(h), channel(h - 1.0 / 3.0))
}

/// Relative luminance (WCAG), for the palette's own contrast questions.
fn luminance(c: Color) -> f64 {
    let Color::Rgb(r, g, b) = c else { panic!("not an rgb colour") };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// One channel, out of what the terminal is handed and into **light** — the
/// space colours may be added in (#508).
fn linear(v: u8) -> f64 {
    let v = f64::from(v) / 255.0;
    match v <= 0.03928 {
        true => v / 12.92,
        false => ((v + 0.055) / 1.055).powf(2.4),
    }
}

/// How much colour a 分詞 `color` mark carries, as CIELAB chroma distance
/// (Δab), for an ink of this `lightness` (#510).
///
/// ⚠️ **It depends on the ink, and it has to.** Equal measured chroma is not
/// equal *visibility*: the eye tells hues apart worse the darker they get, and
/// a light page's ink sits at L\* 17 where 「light 模式下墨色都是黑色的」 —
/// 10.8, which reads clearly against the dark page's L\* 83 ink, was invisible
/// there. The two ends were picked by eye, a straight line
/// between them.
///
/// ⚠️ It is **not** that the dark ink has no room: the walk lowers red as well
/// as raising blue, and red carries little luminance, so 19.5 on the light page
/// costs 1.047:1 of brightness — against the 1.38:1 that made `ink` read as
/// emphasis in the first place.
fn word_hue_chroma(lightness: f64) -> f64 {
    // L* 82.8 → 10.8, L* 16.6 → 19.5.
    (21.7 - 0.1314 * lightness).clamp(8.0, 26.0)
}

/// CIELAB, for asking how far apart two colours *look* rather than how far
/// apart their bytes are (#510). D65, the sRGB white.
fn lab((r, g, b): (u8, u8, u8)) -> (f64, f64, f64) {
    let (rl, gl, bl) = (linear(r), linear(g), linear(b));
    let x = (0.4124 * rl + 0.3576 * gl + 0.1805 * bl) / 0.95047;
    let y = 0.2126 * rl + 0.7152 * gl + 0.0722 * bl;
    let z = (0.0193 * rl + 0.1192 * gl + 0.9505 * bl) / 1.08883;
    let f = |t: f64| match t > 0.008_856 {
        true => t.cbrt(),
        false => 7.787 * t + 16.0 / 116.0,
    };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    (116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz))
}

/// And back. Out of range is clamped: a walk through light can leave the cube.
fn srgb(v: f64) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let v = match v <= 0.003_130_8 {
        true => v * 12.92,
        false => 1.055 * v.powf(1.0 / 2.4) - 0.055,
    };
    (v * 255.0).round() as u8
}

/// The WCAG contrast ratio between two colours.
fn contrast(a: Color, b: Color) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

/// How far back a 分詞 `line` sits from the writing (#501).
///
/// 第 50 檔. It went 35 → 70 → 50: 35 was 「不够淡」, and 70 — picked off a
/// picture of the five candidates — turned out to disappear in use, because
/// the rule sits **hard against the bottom half of the characters** rather
/// than in clear space: 「他紧挨着字的下半部分，7000 看不出来」. A swatch and a
/// page of prose are not the same test.
const RULE_BACK: u16 = 5000;

impl Palette {
    /// The palette in force.
    pub fn of(config: &Config) -> Palette {
        Palette::of_theme(&in_force(config), dark())
    }

    /// The palette a config would give in the stated mood.
    ///
    /// The mood is a global because everything that draws needs it and nothing
    /// that draws is handed it; this is the way in that does not consult it,
    /// which is what lets a test say what it means.
    pub fn in_mood(config: &Config, dark: bool) -> Palette {
        Palette::of_theme(&config.theme, dark)
    }

    /// One theme, in one mood.
    pub fn of_theme(theme: &ThemeConfig, dark: bool) -> Palette {
        Palette {
            ladder: theme.ladder(dark),
            mark: theme.mark(dark),
            gold: theme.gold(dark),
            purple: theme.purple(dark),
            green: theme.green(dark),
            azure: theme.azure(dark),
            amber: theme.amber(dark),
            orange: theme.orange(dark),
            pink: theme.pink(dark),
            cyan: theme.cyan(dark),
            lime: theme.lime(dark),
            fill: theme.fill,
            paint: theme.ground == Ground::Paint,
            faded: false,
        }
    }

    /// How far a stood-back half is moved toward the page, per thousand of the
    /// distance it still has to go.
    ///
    /// 260 of the writing's own 1000, which is the step this was calibrated at
    /// when it was a fixed one.
    const FADE: u32 = 260;

    /// **`under`, marked `depth` rungs toward the ink** — the one way anything
    /// is laid over anything else (#488).
    ///
    /// ⚠️ **A mark is a displacement, not a position.** A rung is an absolute
    /// place on the line from 墨 to 紙, so 第 86 檔 means 「four rungs off the
    /// paper」 *and only on the paper*: a table banded at 第 86 inside a `:::`
    /// kept the colour it would have had on the page, and read as a patch
    /// glued onto the callout rather than a row of it. 「不是紙色的行還保持着
    /// 原來的顏色，造成視覺上的不舒服。」
    ///
    /// The fix is the model a designer would reach for — a translucent sheet
    /// laid over whatever is behind — and it is the same arithmetic the ladder
    /// already uses, applied as a **vector**: `depth` rungs of the 墨→紙
    /// distance, subtracted from `under`. On the paper that is exactly
    /// [`Ink::at`] of the rung it names, so nothing on an ordinary page moves;
    /// over a wash it is that wash, the same amount deeper.
    ///
    /// Negative `depth` lifts instead, toward the paper: that is what a quiet
    /// ink is, and why [`Ink::stepped`] is this function with the sign flipped.
    pub fn over(self, under: Color, depth: i32) -> Color {
        let Color::Rgb(r, g, b) = under else {
            return under;
        };
        let (ir, ig, ib) = self.ladder.ink;
        let (pr, pg, pb) = self.ladder.paper;
        // ⚠️ **Rounded, not truncated.** `Ladder::step` rounds half up, and a
        // mark that is one unit off it is a second colour for the same rung —
        // `over(paper, PAPER - WORD_TINT)` has to *be* `at(WORD_TINT)`, or the
        // claim that an ordinary page does not move is one unit false.
        let full = f64::from(rung::PAPER);
        let shift = |c: u8, ink: u8, paper: u8| -> u8 {
            let step = (f64::from(paper) - f64::from(ink)) * f64::from(depth) / full;
            (f64::from(c) - step).round().clamp(0.0, 255.0) as u8
        };
        Color::Rgb(
            shift(r, ir, pr),
            shift(g, ig, pg),
            shift(b, ib, pb),
        )
    }

    /// The word mark, laid over whatever the writing already carries.
    ///
    /// The same displacement [`Ink::over`] does for a ground, with the sign the
    /// other way: 「fifteen rungs quieter than what is there」. On plain prose
    /// that is [`rung::WORD_INK`] exactly; on a link it is that link's 藍, a
    /// step back — which is how a coloured run can show word boundaries and
    /// stay the colour it is.
    pub fn marked(self, ink: Color, depth: u16) -> Color {
        self.over(ink, -i32::from(depth))
    }

    /// The same palette, a rung back: the half that is only being read.
    pub fn faded(self) -> Palette {
        Palette {
            faded: true,
            ..self
        }
    }

    /// One rung, as a colour.
    pub fn at(self, rung: u16) -> Color {
        let rung = match self.faded {
            // Toward the page, not toward the ink: a quiet pane is *further
            // away*, and a ground it carries (a band, a selection) goes with
            // it so the whole half recedes together.
            //
            // **A fraction of what is left, not a fixed 260 rungs.** Adding
            // ran out of ladder: everything at 740 and above landed exactly on
            // the paper, so on a stood-back row the word tint, the tint past
            // the measure, the search band and the colour indent squares did
            // not recede — they vanished, which is a different statement.
            // Scaling the *remaining* distance keeps the rungs in order and in
            // sight, and leaves the ink end exactly where it was calibrated:
            // the writing still fades 260 rungs, because that is all of them.
            true => {
                let rung = rung.min(yumete_config::rung::PAPER);
                let room = (yumete_config::rung::PAPER - rung) as u32;
                (rung as u32 + room * Self::FADE / 1000) as u16
            }
            false => rung,
        };
        let (r, g, b) = self.ladder.step(rung);
        Color::Rgb(r, g, b)
    }

    /// The writing, and anything that *is* the writing.
    pub fn text(self) -> Color {
        self.at(rung::TEXT)
    }

    /// The terminal's own caret, as three bytes for OSC 12 (#493).
    ///
    /// **Rung 0 — the ink itself**, which is the one thing on the page it has
    /// to be: the block is drawn *over* a character and the character has to
    /// stay readable against it, and the ladder's two ends are calibrated to
    /// exactly that. It is also the only cursor colour that is right in both
    /// moods without a second rule — white caret on a dark page, black caret
    /// on a light one, which is what the reader asked for.
    ///
    /// Not a `Color`: OSC 12 wants the numbers, and `Color::Rgb` is the wrong
    /// shape to carry them out of here.
    pub fn caret(self) -> (u8, u8, u8) {
        self.ladder.step(rung::TEXT)
    }

    /// The paper, as three bytes — the other half of what OSC 10/11 tell the
    /// terminal (#502). Same reason [`Self::caret`] is not a `Color`.
    pub fn paper_bytes(self) -> (u8, u8, u8) {
        self.ladder.paper
    }
    /// 旁註 — read it, but it is not the prose: a reading, a 拆分, a
    /// candidate's number. [`yumete_config::rung::ASIDE`].
    pub fn quiet(self) -> Color {
        self.at(rung::ASIDE)
    }
    /// Furniture: line numbers, an unlit tab, a key's label, a 批注.
    pub fn furniture(self) -> Color {
        self.at(rung::FURNITURE)
    }

    /// The ruler over a table's columns — read, not glanced at (#232).
    pub fn ruler(self) -> Color {
        self.at(rung::RULER)
    }
    /// The markup itself.
    pub fn marker(self) -> Color {
        self.at(rung::MARKER)
    }
    /// A rule: a ring, an edge, the ruler's line, a 稿紙 tick.
    pub fn rule(self) -> Color {
        self.at(rung::RULE)
    }
    /// Chrome, raised one notch off the page.
    pub fn chrome(self) -> Color {
        self.at(rung::CHROME)
    }
    /// A quiet ground.
    pub fn band(self) -> Color {
        self.at(rung::BAND)
    }
    /// The selection's ground — and only its ground.
    pub fn selection(self) -> Color {
        self.at(rung::SELECTION)
    }
    /// The page.
    pub fn paper(self) -> Color {
        self.at(rung::PAPER)
    }
    /// 金 — 這不是正文.
    ///
    /// The page's ladder is a run of greys, on purpose, so the one warm colour
    /// on the screen is worth looking at: a heading, a table's header row, the
    /// name beside a value. It is the only thing here that is not a quantity
    /// of ink and not 朱.
    pub fn gold(self) -> Color {
        self.accent(self.gold)
    }

    /// 朱 — 這裏不對.
    pub fn mark(self) -> Color {
        self.accent(self.mark)
    }

    /// 紫（一至三品）— a literal: 行内代碼, a fence, anything that must be
    /// read character for character.
    pub fn purple(self) -> Color {
        self.accent(self.purple)
    }

    /// 綠（六至七品）— 引用: someone else's words.
    pub fn green(self) -> Color {
        self.accent(self.green)
    }

    /// 藍（八至九品）— a 鏈接: an address, the lowest rank on the page.
    pub fn azure(self) -> Color {
        self.accent(self.azure)
    }

    /// 黃（皇室）— a `[!WARNING]`. Under 金, which is the emperor's.
    pub fn amber(self) -> Color {
        self.accent(self.amber)
    }

    /// 橙 — 朱's second tone: a number in a fence. Never 「這裏不對」.
    pub fn orange(self) -> Color {
        self.accent(self.orange)
    }

    /// 粉 — 紫's second tone.
    pub fn pink(self) -> Color {
        self.accent(self.pink)
    }

    /// 青 — 藍's second tone.
    pub fn cyan(self) -> Color {
        self.accent(self.cyan)
    }

    /// 黃綠 — 綠's second tone.
    pub fn lime(self) -> Color {
        self.accent(self.lime)
    }

    /// Whether coloured runs are also given a ground (`:theme-fill`).
    ///
    /// Off by default: the backtick and the `>` are drawn, so a run's extent
    /// is already on the page, and a ground behind it answers a question that
    /// has been answered. On for a reader who wants the code in a box.
    pub fn fill(self) -> bool {
        self.fill
    }

    /// An accent, taken back with the rest when the half is only being read.
    fn accent(self, (r, g, b): (u8, u8, u8)) -> Color {
        if !self.faded {
            return Color::Rgb(r, g, b);
        }
        let paper = self.ladder.paper;
        let mix = |a: u8, b: u8| -> u8 { ((a as i64) + (b as i64 - a as i64) * 55 / 100) as u8 };
        Color::Rgb(mix(r, paper.0), mix(g, paper.1), mix(b, paper.2))
    }
    /// A highlighter's ground — **金**, washed toward the page.
    ///
    /// `==marked==` is the reader's own pen, and a pen is not a correction: 朱
    /// says 這裏不對 and belongs to errors and hits, 金 says 「not the prose」
    /// and belongs to everything a reader puts *on* the prose. They were both
    /// 朱 — one washed 78%, the other 91% — and thirteen points of the same hue
    /// is not a difference anybody can see on a dark ground.
    ///
    /// It also puts the highlight where a highlighter actually is: yellow in
    /// 墨香, malachite in 莫高, ash-green in 陶窯 — the theme's own second
    /// colour, whatever that theme decided it was.
    pub fn wash(self) -> Color {
        // **A fixed *look*, not a fixed mix.** 72% of the way to the page is a
        // different amount of visible in every theme — on 藍曬 it landed a
        // hair from the word tint, which is the very confusion this separation
        // was made to end. So the wash is chosen to sit a set distance off the
        // page (about 1.9:1, a ground a reader sees without reading it), and
        // is pushed further toward the page only if the writing on it would
        // otherwise fall under 4.5:1.
        self.washed_to(self.gold, 1.9, 4.5)
    }

    /// The same wash, for any of the 品色 — what a `:::` block sits on (#459).
    ///
    /// ⚠️ **This is why the four callouts can differ by hue now and could not
    /// before.** They used to be four *greys* 1.4–2.4 ΔE apart, which is below
    /// the threshold at which two flat grounds can be told apart at all. A hue
    /// held at a fixed distance off the page is a different proposition: the
    /// distance does the work of 「這是一塊」 and the hue does the work of
    /// 「哪一塊」, and neither is asked to do the other's job.
    /// ⚠️ **1.5:1, not [`Ink::wash`]'s 1.9** (#465). That number was set when
    /// 朱 was the only wash on the page and it is right for a small one — a
    /// `==標記==` is a few characters and needs to be found. A callout is a
    /// paragraph of prose the reader is meant to *read*, four of them are now
    /// coloured, and a saturated hue reads louder than a grey at the same
    /// distance: at 1.9 the writing inside came out at 5.8:1, the lowest
    /// contrast anywhere on a page whose prose is otherwise over 9. 1.5 puts it
    /// back to 7.4 and the block is still unmistakably a block — the table
    /// stripe beside it is 1.09.
    pub fn washed(self, accent: Accent) -> Color {
        let colour = self.accent_colour(accent);
        // ⚠️ **Close to the page, and closer on a light one** (#489).
        //
        // This began at 1.5:1 both ways, which is a *field* of colour, not a
        // pane of glass over the paper: 「太濃了……髒的要死」, 「侵略性太强」.
        // Two measurements settled where it belongs.
        //
        // **One.** The same ratio is the same ΔL* on either page — 1.5:1 is
        // fifteen points of L* in both moods — and nothing like the same
        // weight. A tint on a page the eye has adapted to *takes light away*
        // from a field twenty lines tall; the same step on a dark page adds a
        // little to one that was giving off none. So the light page gets the
        // smaller number.
        //
        // **Two.** A table's alternating rows sit at 1.13:1 and are read at a
        // glance across one row — and a callout is fifteen times that area.
        // 「面積大，對比可以低」 sets the floor, and these are under it, which
        // is why they can be this quiet and still be seen.
        //
        // ⚠️ **The saturation does not come down with them.** Lowering both is
        // what made the first attempt grey: the colour has to stay itself and
        // only move toward the page. 「保證本色的狀態下讓他更淡。」
        let off_page = match luminance(self.page_colour()) < luminance(self.text()) {
            true => 1.12,
            false => 1.08,
        };
        self.tinted(colour, off_page)
    }

    /// The raw three bytes behind one of the 品色.
    fn accent_colour(self, accent: Accent) -> (u8, u8, u8) {
        match accent {
            Accent::Gold => self.gold,
            Accent::Mark => self.mark,
            Accent::Purple => self.purple,
            Accent::Green => self.green,
            Accent::Azure => self.azure,
            Accent::Amber => self.amber,
        }
    }

    /// A ground for a **short run** of one of the 品色 — a diff's changed
    /// word, not a callout's field (#499).
    ///
    /// ⚠️ **Louder than [`Self::washed`], and that is not an inconsistency:
    /// a ground's loudness has to be set by its area.** 1.5:1 over a paragraph
    /// of prose was 「侵略性太强」 and became 1.08–1.12 for the callouts; the
    /// same 1.5 under two characters is barely a tint, and those two characters
    /// are the entire point of the line they are on. The reader is *hunting*
    /// here, not reading — the nearest thing on the page is `==標記==`, which
    /// sits at 1.9 for exactly that reason.
    pub fn short_wash(self, accent: Accent) -> Color {
        self.tinted(self.accent_colour(accent), 1.5)
    }

    /// 改動條那一格的底色——行號旁邊一欄寬的那一條（#55／#298）。
    ///
    /// ⚠️ **比 [`Self::short_wash`] 還響，理由還是面積。** 一段 `:::` 的底色
    /// 攤在二十行上，1.08–1.12 就夠了；`==標記==` 是幾個字，1.5；這一條是
    /// **一欄寬**，全頁最小的一塊有顏色的地方，而且它旁邊永遠是行號那一片灰。
    /// 3:1 是「一欄寬的東西還認得出是綠是藍」的下限，也就是 WCAG 給非文字元素
    /// 定的那個數——這一格正是一個非文字元素。
    ///
    /// 走 [`Self::tinted`] 而不是直接拿 `accent()` 當底色：原色那一檔是**給字
    /// 用的**，鋪成底色在亮暗兩個心情下響度差得遠，而 `tinted` 只動明度、把飽
    /// 和度夾在同一條帶子裏，亮頁往下走、暗頁往上走，兩邊一樣重。
    pub fn vcs(self, accent: Accent) -> Color {
        self.tinted(self.accent_colour(accent), 3.0)
    }

    /// The paper, as a colour — the ground everything else is measured against.
    fn page_colour(self) -> Color {
        Color::Rgb(self.ladder.paper.0, self.ladder.paper.1, self.ladder.paper.2)
    }

    /// A ground of `accent`'s **hue**, set `off_page` away from the paper.
    ///
    /// ⚠️ **Not a mix toward the paper** (#487). `washed_to` walks the straight
    /// line from the accent to the page, which works when the accent is on the
    /// far side of the page in luminance — a bright colour on a dark page — and
    /// falls apart the other way round. In the light mood the accents are
    /// *dark* (`#8A5F12` for 黃), so the walk from there to cream goes through
    /// brown-grey: measured, the five callouts came out at **10–20 saturation
    /// against the paper's 46**, twelve to seventeen points of lightness below
    /// it. They were not pale colours, they were dirty greys. 「亮色模式下……底
    /// 色太暗，不是亮色。」
    ///
    /// So the hue is kept and only the **lightness** moves, away from the page
    /// until the ground sits `off_page` from it — up on a dark page, down on a
    /// light one. Saturation is clamped into a band: under it a ground is a
    /// grey whatever its hue says, over it the luminous hues (green, yellow)
    /// shout while blue and purple whisper, because equal saturation is not
    /// equal loudness.
    fn tinted(self, accent: (u8, u8, u8), off_page: f64) -> Color {
        /// A ground below this is a grey; above it, the luminous hues shout.
        const BAND: (f64, f64) = (22.0, 42.0);
        let paper = self.page_colour();
        let (hue, _, sat) = to_hsl(accent);
        let (page_hue, page_light, page_sat) = to_hsl(self.ladder.paper);
        // ⚠️ **A tint the paper's own hue has to out-saturate the paper**
        // (#490). 墨香's light page is a warm cream at 45° and 46 saturation,
        // and 黃 sits at 39° — six degrees away, and *under* the paper in
        // saturation. So it did not read as yellow, it read as cream with the
        // light knocked out of it: 「黃色有些不夠純」. The other four are a
        // hundred degrees off and never meet this.
        //
        // Only the hue near the page's needs it, and only enough to be seen as
        // a colour of its own — so the floor is the page's saturation with a
        // margin, and every other accent keeps the band.
        //
        // ⚠️ **And only when the page is a colour at all.** The dark page is a
        // near-neutral that happens to compute a hue (216°, at 9 saturation),
        // and 藍 lands 16° from it — so without this the dark mood lifted the
        // blue to 58 and the rule fired where there was nothing to collide
        // with. A page nobody would call yellow cannot swallow a yellow.
        let near = page_sat > 25.0 && {
            let apart = (hue - page_hue).abs();
            apart.min(360.0 - apart) < 30.0
        };
        let sat = match near {
            true => sat.clamp(page_sat + 15.0, 100.0),
            false => sat.clamp(BAND.0, BAND.1),
        };
        // Away from the page: a dark page is lifted, a light one is lowered.
        let up = luminance(paper) < luminance(self.text());
        let step = if up { 0.4 } else { -0.4 };
        let mut light = page_light;
        for _ in 0..300 {
            light += step;
            if !(0.0..=100.0).contains(&light) {
                break;
            }
            let ground = from_hsl(hue, light, sat);
            if contrast(ground, paper) >= off_page {
                return ground;
            }
        }
        from_hsl(hue, page_light, sat)
    }

    /// An accent washed toward the page until it sits `off_page` from it —
    /// and further, if the writing on it would not clear `readable`.
    fn washed_to(self, accent: (u8, u8, u8), off_page: f64, readable: f64) -> Color {
        let paper = Color::Rgb(self.ladder.paper.0, self.ladder.paper.1, self.ladder.paper.2);
        let text = self.text();
        let mut chosen = self.washed_toward(accent, 90);
        // From the page outward: the first mix that is far enough off the page
        // and still carries the writing.
        for percent in (30..=90).rev().step_by(2) {
            let ground = self.washed_toward(accent, percent);
            if contrast(ground, paper) >= off_page {
                if contrast(text, ground) >= readable {
                    return ground;
                }
                // Too dim for the writing: keep the last one that was not.
                return chosen;
            }
            chosen = ground;
        }
        chosen
    }

    /// The word tint, which alternates with bare paper (Feature #24).
    ///
    /// **A rung, not a hue.** A square that belongs to the same word as its
    /// neighbour is a hair off the paper and nothing more — see
    /// [`yumete_config::rung::WORD_TINT`]. Being on the ladder, it follows the mood
    /// and the theme without carrying any of the accents' meanings: a word
    /// boundary is structure, not a mark somebody made.
    pub fn word(self) -> Color {
        self.at(yumete_config::rung::WORD_TINT)
    }

    /// The other way to mark a word: the **writing** a shade back, on paper
    /// left alone (Feature #278).
    ///
    /// [`yumete_config::rung::WORD_INK`] — 第 15 檔, a small step and no more.
    /// It is a rung and not a hue for the same reason [`Ink::word`] is: a word
    /// boundary is structure, not a mark somebody made.
    ///
    /// ⚠️ **Not `ASIDE`**, which is where this began. That rung means 「not
    /// the prose」, and borrowing it made every second word on the page look
    /// demoted rather than merely bounded.
    pub fn word_ink(self) -> Color {
        self.at(yumete_config::rung::WORD_INK)
    }

    /// The third way: **a second hue at the ink's own brightness** (#501).
    ///
    /// ⚠️ **One half is the plain ink; only the other takes a colour** (#507).
    /// It gave both halves a hue for a while — warm against cool — on the
    /// reasoning that blue against the near-grey ink reads as 「blue against
    /// white」. True, and the cost was that a page of prose had no plain ink
    /// anywhere on it. 「我觉得还使用 ink 色比较好，这样只有蓝色的那些词才變
    /// 色。」 So the pair is 第 0 檔 against one cool hue, and the page is still
    /// a page of ink.
    ///
    /// ⚠️ **Lightness is held and only the hue moves** — the opposite of
    /// [`Self::tinted`], and for the mirror reason. `word_ink` steps a word
    /// *darker*, and on a page of prose a darker run reads as emphasis whether
    /// or not it was meant to: 「有些字亮有些字暗，亮的像强调」. Hue carries the
    /// same one bit and says nothing about weight — so the blue is bisected to
    /// the ink's **measured** brightness, not given the ink's HSL lightness:
    /// HSL's L is `(max+min)/2` and knows nothing about the eye.
    ///
    /// ⚠️ **The hue is the theme's 藍, never its 朱 or 綠.** About eight men in
    /// a hundred cannot tell red from green, and this is the one mark on the
    /// page whose entire job is to be told apart. Blue against the page's own
    /// warm ink is a 藍↔黃 difference, which every common form of colour
    /// blindness leaves intact.
    ///
    /// ⚠️ **Only where the writing is the plain ink.** The answer for a heading
    /// or a link is `None`, and the renderers then leave that run alone.
    /// `word_ink` can step *any* colour back a rung and keep it 金 or 藍; a hue
    /// cannot — rotating a heading's 金 to 藍 does not mark a word boundary, it
    /// throws the heading's own colour away: 「标题本来是金色，现在变成兰黄。连
    /// 接本来是蓝色，现在是兰黄」. A coloured run already stands apart from the
    /// prose, which is most of what the mark buys.
    pub fn word_hue(self, from: Color) -> Option<Color> {
        if from != self.text() {
            return None;
        }
        let Color::Rgb(r, g, b) = from else { return None };
        // **A luminance-neutral direction, walked in linear light.** Summed
        // against the luminance weights this vector comes to 0.026 — so moving
        // along it changes the hue and leaves the brightness where it was,
        // which is the whole point of this mark. Its direction is the one
        // approved by eye (`#D2CEC4` → `#C4D1D7`), normalised: less red,
        // a little more green, more blue.
        const TOWARD_BLUE: (f64, f64, f64) = (-0.5821, 0.1284, 0.8029);
        let base = (linear(r), linear(g), linear(b));
        let walk = |k: f64| -> (u8, u8, u8) {
            (
                srgb(base.0 + k * TOWARD_BLUE.0),
                srgb(base.1 + k * TOWARD_BLUE.1),
                srgb(base.2 + k * TOWARD_BLUE.2),
            )
        };
        let here = lab((r, g, b));
        let want = word_hue_chroma(here.0);
        // How far to walk is found, not fixed: near black a small step in
        // linear light is a large step in what the terminal draws.
        let mut lo = 0.0;
        let mut hi = 2.0;
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            let there = lab(walk(mid));
            let apart = ((there.1 - here.1).powi(2) + (there.2 - here.2).powi(2)).sqrt();
            match apart < want {
                true => lo = mid,
                false => hi = mid,
            }
        }
        let (nr, ng, nb) = walk((lo + hi) / 2.0);
        Some(Color::Rgb(nr, ng, nb))
    }

    /// The rule a `line` word mark draws — see [`RULE_BACK`] (#501).
    ///
    /// ⚠️ **Not 35.** It began on FURNITURE — line numbers, an unlit tab — and
    /// the answer was 「不够淡」. Markdown already spends the solid underline on
    /// a link, so this one has to be visibly the quieter of the two.
    ///
    /// ⚠️ **And it is not drawn on a link at all.** Composing it over the
    /// link's own 藍 the way a table band composes over a callout was tried and
    /// measured `#002857`, which on a dark page is a black line: 「那个蓝色线，
    /// 深的看不出来了都」. So the rule that stands is the simple one — 「链接就
    /// 是用链接颜色 overwrite 掉分词下划线」 — and it is the renderers that
    /// enforce it, by leaving alone any cell that is underlined already.
    pub fn word_rule(self) -> Color {
        self.at(RULE_BACK)
    }


    /// An accent, `percent` of the way to the page.
    fn washed_toward(self, accent: (u8, u8, u8), percent: i64) -> Color {
        // A read-only half takes its accents back with everything else.
        let percent = match self.faded {
            true => percent + (100 - percent) * 55 / 100,
            false => percent,
        };
        let (paper, accent) = (self.ladder.paper, accent);
        let mix = |a: u8, b: u8| -> u8 {
            let (a, b) = (a as i64, b as i64);
            ((a * 100 + (b - a) * percent) / 100) as u8
        };
        Color::Rgb(
            mix(accent.0, paper.0),
            mix(accent.1, paper.1),
            mix(accent.2, paper.2),
        )
    }

    /// The page's own style: ink on paper, or nothing at all.
    ///
    /// `[theme] ground = "terminal"` gives the ground back to the reader, for
    /// somebody whose terminal palette is a decision they have already made.
    /// Nothing else in here changes — the ladder still says how far back a
    /// thing reads, which is the part that carries meaning.
    pub fn page(self) -> Style {
        match self.paint {
            true => Style::default().fg(self.text()).bg(self.paper()),
            false => Style::default(),
        }
    }

    /// A ground, on a page that may not have one of its own.
    ///
    /// With the page unpainted a band still has to be *a* colour or it would be
    /// invisible, so it keeps its rung; only the page itself is given back.
    pub fn ground(self, rung: u16) -> Style {
        Style::default().bg(self.at(rung))
    }

    /// The **第 100 檔** ground — as far into the theme's own colour as the
    /// ladder goes, and the only place that is not the page.
    ///
    /// See `rung::DEEP`: the ladder runs 墨 → 紙 → 更沉, which is 反主題色 →
    /// 主題色, so this is darker on a dark theme and lighter on a light one
    /// without anything here having to ask which.
    pub fn sunken(self) -> Color {
        self.at(yumete_config::rung::DEEP)
    }

    /// Whether the page is painted.
    pub fn paints(self) -> bool {
        self.paint
    }
}

#[cfg(all(test, unix))]
mod span_tests {
    use super::osc_span;

    /// What is left when the OSC 11 answer is cut out is what a person typed.
    #[test]
    fn the_answer_is_cut_out_and_the_typing_is_not() {
        let reply = b"\x1b]11;rgb:1c1c/1c1c/1c1c\x1b\\";
        assert_eq!(&reply[osc_span(reply)], &reply[..]);
        // Typed before the answer arrived, and after it.
        let mixed = b"i\x1b]11;rgb:ffff/ffff/ffff\x07HELLO";
        let span = osc_span(mixed);
        assert_eq!(&mixed[..span.start], b"i");
        assert_eq!(&mixed[span.end..], b"HELLO");
        // A terminal that said nothing recognisable claims nothing.
        assert_eq!(osc_span(b"iHELLO"), 0..0);
    }
}

