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
    match CHOSEN.read().ok().and_then(|c| c.clone()) {
        Some(theme) => theme,
        None => config.theme.clone(),
    }
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

/// Relative luminance (WCAG), for the palette's own contrast questions.
fn luminance(c: Color) -> f64 {
    let Color::Rgb(r, g, b) = c else { panic!("not an rgb colour") };
    let f = |v: u8| {
        let v = v as f64 / 255.0;
        match v <= 0.03928 {
            true => v / 12.92,
            false => ((v + 0.055) / 1.055).powf(2.4),
        }
    };
    0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b)
}

/// The WCAG contrast ratio between two colours.
fn contrast(a: Color, b: Color) -> f64 {
    let (x, y) = (luminance(a), luminance(b));
    let (hi, lo) = if x > y { (x, y) } else { (y, x) };
    (hi + 0.05) / (lo + 0.05)
}

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
    /// One shade back: a reading, a 拆分, a candidate's number.
    pub fn quiet(self) -> Color {
        self.at(rung::QUIET)
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
    /// [`yumete_config::rung::WORD`]. Being on the ladder, it follows the mood
    /// and the theme without carrying any of the accents' meanings: a word
    /// boundary is structure, not a mark somebody made.
    pub fn word(self) -> Color {
        self.at(yumete_config::rung::WORD)
    }

    /// The other way to mark a word: the **writing** a shade back, on paper
    /// left alone (Feature #278).
    ///
    /// [`yumete_config::rung::QUIET`] — 「one shade back」 is exactly what this
    /// has to say, and it is the rung a reading beside its base already uses,
    /// so an alternated page never has two kinds of quiet on it. It is a rung
    /// and not a hue for the same reason [`Ink::word`] is: a boundary is
    /// structure, not a mark somebody made.
    pub fn word_ink(self) -> Color {
        self.at(yumete_config::rung::QUIET)
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

    /// The **第 20 檔** ground — as far into the theme's own colour as the
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

#[cfg(test)]
mod tests {
    use super::*;

    fn palette(dark: bool) -> Palette {
        Palette::in_mood(&Config::default(), dark)
    }

    /// **A stood-back half recedes; it does not disappear.**
    ///
    /// The fade used to add a fixed 260 rungs and clamp, so every rung from
    /// 740 up — the word tint at 962, the tint past the measure at 940, the
    /// 縱書 opening squares — landed exactly on the paper. 焦點模式 with
    /// 分詞著色 on therefore did not stand the word tint back, it rubbed it
    /// out, and the reader lost a whole overlay by asking for another.
    #[test]
    fn nothing_but_the_paper_fades_to_the_paper() {
        for name in ["ink", "bw", "cyanotype", "amber", "mogao", "morandi", "meridian", "kiln"] {
            let theme = yumete_config::ThemeConfig::named(name).expect(name);
            for dark in [true, false] {
                let config = Config {
                    theme: theme.clone(),
                    ..Config::default()
                };
                let lit = Palette::in_mood(&config, dark);
                let back = lit.faded();
                let paper = lit.paper();
                for rung in [
                    yumete_config::rung::SELECTION,
                    yumete_config::rung::HEAD,
                    yumete_config::rung::BAND,
                    yumete_config::rung::WORD,
                    yumete_config::rung::CHROME,
                ] {
                    assert_ne!(
                        back.at(rung),
                        paper,
                        "{name} dark={dark}: rung {rung} still has to be seen"
                    );
                    // …and it did move: receding is the other half of it.
                    assert_ne!(
                        back.at(rung),
                        lit.at(rung),
                        "{name} dark={dark}: rung {rung} stands back"
                    );
                }
            }
        }
    }

    /// **The grounds a reader meets on one screen are told apart.**
    ///
    /// Near the paper end of the ladder the rungs compress, so two grounds
    /// sixty rungs apart can be the same colour: a table's cursor row (`head`)
    /// against its alternating columns (`band`) measured 1.11–1.20:1 in every
    /// theme, and a reader could not see which row they were on.
    #[test]
    fn the_grounds_that_meet_are_a_rung_apart() {
        for name in ["ink", "bw", "cyanotype", "amber", "mogao", "morandi", "meridian", "kiln"] {
            let theme = yumete_config::ThemeConfig::named(name).expect(name);
            for dark in [true, false] {
                let config = Config {
                    theme: theme.clone(),
                    ..Config::default()
                };
                let ink = Palette::in_mood(&config, dark);
                // The three that stack inside a table: the column band, the
                // cursor's row, and a selection within it.
                let pairs = [
                    ("欄底 / 光標行", ink.band(), ink.at(yumete_config::rung::HEAD)),
                    ("光標行 / 選區", ink.at(yumete_config::rung::HEAD), ink.selection()),
                ];
                for (what, a, b) in pairs {
                    assert!(
                        contrast(a, b) >= 1.24,
                        "{name} {dark}: {what} are the same colour ({:.2}:1)",
                        contrast(a, b)
                    );
                }
                // …and the writing still reads on the loudest of them.
                assert!(
                    contrast(ink.text(), ink.selection()) >= 4.5,
                    "{name} {dark}: {:.2}:1 on a selection",
                    contrast(ink.text(), ink.selection())
                );
            }
        }
    }

    /// **A word boundary is not a highlighter.**
    ///
    /// Both used to be 朱 washed toward the page — 78% for `==marked==`, 91%
    /// for the word tint — and on a dark ground thirteen points of one hue is
    /// not a difference: a reader could not tell which of the two they were
    /// looking at (reported with a screenshot, 2026-09-04).
    #[test]
    fn the_word_tint_and_the_highlighter_are_told_apart() {
        for name in ["ink", "bw", "cyanotype", "amber", "mogao", "firefly"] {
            let theme = yumete_config::ThemeConfig::named(name).expect(name);
            for dark in [true, false] {
                let config = Config {
                    theme: theme.clone(),
                    ..Config::default()
                };
                let ink = Palette::in_mood(&config, dark);
                let (paper, tint, wash) = (ink.paper(), ink.word(), ink.wash());
                // Each is a *ground*, so they are compared to the page and to
                // each other by contrast, the way the eye compares them.
                let off_page = |c| contrast(c, paper);
                assert!(
                    off_page(tint) < 1.25,
                    "{name} {dark}: the word tint shouts ({:.2}:1 off the page)",
                    off_page(tint)
                );
                assert!(
                    contrast(wash, tint) > 1.4,
                    "{name} {dark}: the highlighter and the word tint are the same colour \
                     ({:.2}:1 apart)",
                    contrast(wash, tint)
                );
                // …and the writing still reads on the highlighter.
                assert!(
                    contrast(ink.text(), wash) >= 4.5,
                    "{name} {dark}: {:.2}:1 on the highlighter",
                    contrast(ink.text(), wash)
                );
            }
        }
    }

    #[test]
    fn everything_meant_to_be_read_can_be() {
        // The bar a scheme has to clear, and the reason this file exists: three
        // colours in the old scheme measured 1.00, 1.04 and 1.11 against what
        // they were drawn on — a rule painted in its own background, a 稿紙
        // tick, and the 縱書 number band.
        for dark in [true, false] {
            let p = palette(dark);
            for (name, ink, bar) in [
                ("the writing", p.text(), 4.5),
                ("a reading", p.quiet(), 4.5),
                ("furniture", p.furniture(), 3.0),
                ("the markup", p.marker(), 3.0),
                ("a rule", p.rule(), 2.5),
                ("朱", p.mark(), 3.0),
                ("金", p.gold(), 4.5),
            ] {
                let got = contrast(ink, p.paper());
                assert!(got >= bar, "dark={dark}: {name} is {got:.2}:1 on the page");
            }
            // …and ink still reads on every ground.
            for (name, ground) in [
                ("chrome", p.chrome()),
                ("a band", p.band()),
                ("a selection", p.selection()),
            ] {
                let got = contrast(p.text(), ground);
                assert!(got >= 4.5, "dark={dark}: the writing is {got:.2}:1 on {name}");
            }
        }
    }

    /// **The ruler over a table's columns is read, so it is measured as text**
    /// — Feature #232.
    ///
    /// ⚠️ **Measured on the chrome it is drawn on, not on the paper.** That is
    /// the whole of what made this a decision rather than a patch: the numbers
    /// sit on the gutter's ground, so moving `CHROME` moves them, and at
    /// `FURNITURE` the worst of the twenty-one ladders was 3.15:1 — clearing
    /// what a border needs and missing what small text needs. This test is the
    /// thing that will notice the next time a ground moves under it.
    #[test]
    fn the_column_ruler_is_dark_enough_to_count() {
        let mut worst = f64::MAX;
        for name in ["ink", "bw", "cyanotype", "amber", "mogao", "morandi", "meridian", "kiln"] {
            let theme = yumete_config::ThemeConfig::named(name).expect(name);
            for dark in [true, false] {
                let p = Palette::of_theme(&theme, dark);
                let got = contrast(p.ruler(), p.chrome());
                assert!(
                    got >= 4.5,
                    "{name} dark={dark}: the column ruler is {got:.2}:1 on the gutter"
                );
                worst = worst.min(got);
                // …and it stays a ruler: quieter than the heading it sits over.
                assert!(
                    contrast(p.ruler(), p.chrome()) <= contrast(p.text(), p.chrome()),
                    "{name} dark={dark}: a ruler is not louder than the writing"
                );
            }
        }
        assert!(worst >= 4.5, "worst was {worst:.2}");
    }

    #[test]
    fn the_black_and_white_theme_says_it_all_without_a_hue() {
        // 黑白 is not 墨香 with the colour turned down: what warmth said there,
        // *position* has to say here — so 金 and 朱 leave the ladder by going
        // past its ends rather than by leaving its hue.
        for dark in [true, false] {
            let theme = yumete_config::ThemeConfig::named("bw").expect("黑白");
            let p = Palette::of_theme(&theme, dark);
            for (name, ink, bar) in [
                ("the writing", p.text(), 4.5),
                ("a reading", p.quiet(), 4.5),
                ("furniture", p.furniture(), 3.0),
                ("金", p.gold(), 4.5),
                ("朱", p.mark(), 3.0),
            ] {
                let got = contrast(ink, p.paper());
                assert!(got >= bar, "dark={dark}: {name} is {got:.2}:1 on the page");
            }
            // Nothing in it has a hue at all.
            for colour in [p.text(), p.quiet(), p.paper(), p.gold(), p.mark()] {
                let Color::Rgb(r, g, b) = colour else { panic!() };
                assert!(r == g && g == b, "dark={dark}: {colour:?} is not a grey");
            }
            // 金 is past the writing, not beside it: the one thing brighter
            // than the prose is the thing that is not prose.
            assert!(
                contrast(p.gold(), p.paper()) > contrast(p.text(), p.paper()),
                "dark={dark}: 金 does not stand out"
            );
        }
    }

    #[test]
    fn a_ground_is_raised_toward_the_ink_in_both_moods() {
        // The one formulation that survives a light/dark flip. A band defined
        // as "darker than the page" inverts its meaning in light mode; "one
        // notch toward the ink" does not.
        for dark in [true, false] {
            let p = palette(dark);
            for ground in [p.chrome(), p.band(), p.selection()] {
                let toward_ink = (luminance(ground) - luminance(p.paper())).abs();
                assert!(toward_ink > 0.0, "dark={dark}: a ground that is the page");
                let closer = contrast(ground, p.text()) < contrast(p.paper(), p.text());
                assert!(closer, "dark={dark}: a ground that is not raised");
            }
        }
    }

    #[test]
    fn the_ladder_is_evenly_spaced_enough_to_place_an_interface_on() {
        // sRGB mixing, and the reason for it: over this pair it steps 5.8–7.4
        // L* per tenth of the ink→paper run, where mixing in linear light steps
        // 3.2–16.8 and piles nine tenths of the rungs into the light half.
        //
        // ⚠️ A tenth is `PAPER / 10`, not 100: the ladder was restretched to
        // 0–10000 with 紙 at 9000 (第 18 檔), and sampling `i * 100` afterwards
        // measured the first ninth of it and called the whole ladder uneven.
        let p = palette(true);
        let l = |c: Color| {
            let y = luminance(c);
            match y > 0.008856 {
                true => 116.0 * y.cbrt() - 16.0,
                false => 903.3 * y,
            }
        };
        let tenth = yumete_config::rung::PAPER / 10;
        let steps: Vec<f64> = (0..10)
            .map(|i| l(p.at(i * tenth)) - l(p.at((i + 1) * tenth)))
            .collect();
        let (lo, hi) = steps.iter().fold((f64::MAX, 0.0f64), |(a, b), &s| (a.min(s), b.max(s)));
        // ⚠️ **Evenness, not size.** This used to assert `hi < 9.0`, which is a
        // number about *how far apart this theme puts its ink and paper* — a
        // theme's own decision. 墨香's page went deeper on 2026-09-14 (#24262C
        // → #1A1E19, the reader's 「太灰，没有松烟的深沉」) and every tenth of
        // the run grew with it: 5.8–7.4 L* became 7.2–9.8. Nothing was less
        // even — the spread is 1.37× against 1.28× — but the absolute bound
        // failed, and widening it to 10 would only move the same mistake.
        //
        // What the interface actually needs is that no tenth is invisible and
        // none swallows its neighbours, which is a floor and a ratio.
        assert!(lo > 3.0, "a tenth of the ladder nobody can see: {steps:?}");
        assert!(hi / lo < 1.6, "uneven ladder: {steps:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_terminals_answer_is_weighed_not_just_read() {
        let dark = |reply: &str| ground_is_dark(reply.as_bytes());
        // 墨香's own two grounds, as three terminals write them.
        assert_eq!(dark("\x1b]11;rgb:2626/2a2a/2727\x1b\\"), Some(true));
        assert_eq!(dark("\x1b]11;rgb:f1f1/ebeb/d9d9\x07"), Some(false));
        assert_eq!(dark("\x1b]11;rgb:26/2a/27\x07"), Some(true));
        // A mid grey looks light well before its luminance is a half.
        assert_eq!(dark("\x1b]11;rgb:8080/8080/8080\x07"), Some(false));
        assert_eq!(dark("\x1b]11;rgb:4040/4040/4040\x07"), Some(true));
        // Half an answer is not an answer: the reply is read as it arrives and
        // must not be weighed until the last component is whole.
        assert_eq!(dark("\x1b]11;rgb:f1f1/ebeb/d9"), None);
        assert_eq!(dark("\x1b]11;rgb:f1f1/ebeb"), None);
        assert_eq!(dark("\x1b]10;rgb:cfcf/c6c6/a9a9\x07"), Some(false));
        assert_eq!(dark(""), None);
    }

    #[test]
    fn giving_the_ground_back_leaves_the_meaning_alone() {
        let mut config = Config::default();
        config.theme.ground = Ground::Terminal;
        let p = Palette::in_mood(&config, true);
        assert_eq!(p.page(), Style::default(), "the page is the terminal's");
        // …but how far back a thing reads is not about the ground.
        assert_eq!(p.quiet(), palette(true).quiet());
    }
}
