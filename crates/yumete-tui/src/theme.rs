//! 【墨香】 — one theme, computed, and everywhere (Feature #152).
//!
//! Every colour the editor draws comes from here, and here has **three
//! numbers**: 墨, 紙, and 朱. Everything between the first two is a rung on a
//! ladder ([`Ladder::step`]); 朱 is the one thing that is not a quantity of ink
//! and so cannot be on it.
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
use yumete_config::{rung, Config, Ground, Ladder, Mode};

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
/// Terminals that do not answer are left to the config's own fallback. The
/// wait is bounded and the reply is read straight off the descriptor rather
/// than through the event reader, so nothing the reader types can be eaten by
/// it and an unanswering terminal costs a tenth of a second at start-up.
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
                return None;
            }
            let mut chunk = [0u8; 64];
            match std::io::stdin().read(&mut chunk) {
                Ok(0) | Err(_) => return None,
                Ok(n) => reply.extend_from_slice(&chunk[..n]),
            }
            if let Some(dark) = ground_is_dark(&reply) {
                return Some(dark);
            }
            // A terminal that is answering something else entirely.
            if reply.len() > 256 {
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
    paint: bool,
}

impl Palette {
    /// The palette in force.
    pub fn of(config: &Config) -> Palette {
        Palette::in_mood(config, dark())
    }

    /// The palette a config would give in the stated mood.
    ///
    /// The mood is a global because everything that draws needs it and nothing
    /// that draws is handed it; this is the way in that does not consult it,
    /// which is what lets a test say what it means.
    pub fn in_mood(config: &Config, dark: bool) -> Palette {
        Palette {
            ladder: config.theme.ladder(dark),
            mark: config.theme.mark(dark),
            paint: config.theme.ground == Ground::Paint,
        }
    }

    /// One rung, as a colour.
    pub fn at(self, rung: u16) -> Color {
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
    /// 朱 — 這裏不對.
    pub fn mark(self) -> Color {
        let (r, g, b) = self.mark;
        Color::Rgb(r, g, b)
    }
    /// 朱 washed most of the way to the page — a highlighter's ground.
    ///
    /// The one place 朱 is not saying 這裏不對: `==marked==` is the reader's own
    /// pen, which is what 朱 has always been. Washed, so the ink still reads on
    /// it and it cannot be mistaken for a selection.
    pub fn wash(self) -> Color {
        self.washed(78)
    }

    /// The word tint, which alternates with bare paper (Feature #24).
    ///
    /// 朱 again, washed until almost nothing is left of it: a square that
    /// belongs to the same word as its neighbour is a *hair* warmer than the
    /// page, never a colour in its own right. Derived rather than configured,
    /// so it follows the mood — a tint picked for a dark page is a smear on a
    /// light one.
    pub fn word(self) -> Color {
        self.washed(91)
    }

    /// 朱, `percent` of the way to the page.
    fn washed(self, percent: i64) -> Color {
        let (paper, mark) = (self.ladder.paper, self.mark);
        let mix = |a: u8, b: u8| -> u8 {
            let (a, b) = (a as i64, b as i64);
            ((a * 100 + (b - a) * percent) / 100) as u8
        };
        Color::Rgb(
            mix(mark.0, paper.0),
            mix(mark.1, paper.1),
            mix(mark.2, paper.2),
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

    /// Whether the page is painted.
    pub fn paints(self) -> bool {
        self.paint
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative luminance, for the contrast checks below.
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

    fn contrast(a: Color, b: Color) -> f64 {
        let (x, y) = (luminance(a), luminance(b));
        let (hi, lo) = if x > y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    fn palette(dark: bool) -> Palette {
        Palette::in_mood(&Config::default(), dark)
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
        // L* per hundred rungs, where mixing in linear light steps 3.2–16.8 and
        // piles nine tenths of the rungs into the light half.
        let p = palette(true);
        let l = |c: Color| {
            let y = luminance(c);
            match y > 0.008856 {
                true => 116.0 * y.cbrt() - 16.0,
                false => 903.3 * y,
            }
        };
        let steps: Vec<f64> = (0..10)
            .map(|i| l(p.at(i * 100)) - l(p.at((i + 1) * 100)))
            .collect();
        let (lo, hi) = steps.iter().fold((f64::MAX, 0.0f64), |(a, b), &s| (a.min(s), b.max(s)));
        assert!(lo > 4.0 && hi < 9.0, "uneven ladder: {steps:?}");
    }

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
