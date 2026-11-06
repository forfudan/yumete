//! **Pictures in the text, through the kitty graphics protocol** (2026-11-06).
//!
//! One use for now — the formula float ([`crate::maths`]) — and one route,
//! the protocol's **Unicode placeholders**: the PNG goes to the terminal once
//! (`a=T,U=1`, a *virtual placement*), and after that the picture is ordinary
//! text. Each of its cells holds U+10EEEE with two combining marks that say
//! which row and column of the picture it is, and the cell's foreground colour
//! carries the picture's id. ratatui's diff then moves, covers and erases it
//! like any other cell, and the picture survives tmux, since the cells are
//! plain text. Modelled on yazi's `kgp.rs` and ratatui-image's `kitty.rs`; the
//! survey is `local/survey-0.20/graphics.md`.
//!
//! **Decided by the terminal's name, not by asking it.** Konsole answers the
//! protocol's query and then ignores placeholders, and the protocol has no
//! query for them yet, so a terminal not known to draw them gets nothing —
//! not tofu where a formula should be.
//!
//! Two traps, both from the survey:
//!
//! - U+10EEEE is private use, which `yumete_cjk` counts as two cells and the
//!   page pads. The cells are written straight into the buffer
//!   ([`put_picture`]), never laid out as text.
//! - The upload is an escape sequence, not a cell. It goes out through the
//!   backend's writer, ahead of the cells of the frame that shows it
//!   ([`take_pending`], called by [`crate::backend::Dotted`]).

use std::sync::atomic::{AtomicU16, AtomicU8, Ordering};
use std::sync::Mutex;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};

/// Whether pictures can be drawn here, and how they get there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// No pictures: every terminal not known to draw placeholders, and every
    /// frame drawn with no terminal at all (`--shot`, the tests).
    None,
    /// Ghostty or kitty, the upload written as it is.
    Kitty,
    /// The same inside tmux with `allow-passthrough`: the upload is wrapped
    /// for tmux to pass on. The cells need nothing — they are text.
    KittyInTmux,
}

/// What tmux says about the client it is drawing on: the outer terminal's
/// `TERM` and the name it answers to (`client_termname`, `client_termtype`),
/// and whether `allow-passthrough` lets an escape sequence through.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tmux {
    pub client_term: String,
    pub passthrough: bool,
}

/// **Which terminal this is, from its environment** (2026-11-06).
///
/// `var` reads one environment variable; `tmux` is asked only inside tmux,
/// where the environment names tmux and not the terminal around it.
///
/// Warning: **a terminal opened inside another inherits the outer one's
/// variables.** VS Code started from kitty has `KITTY_WINDOW_ID` and draws
/// no placeholders, so a `TERM_PROGRAM` that names some other terminal wins
/// over the inherited markers.
pub fn support_from(var: impl Fn(&str) -> Option<String>, tmux: impl FnOnce() -> Option<Tmux>) -> Support {
    let set = |name: &str| var(name).is_some_and(|v| !v.is_empty());
    // A multiplexer that passes nothing through, and one that draws its own
    // screen: no pictures.
    if set("ZELLIJ") || set("STY") {
        return Support::None;
    }
    let draws = |term: &str| matches!(term, "xterm-ghostty" | "xterm-kitty");
    let inherited = || set("GHOSTTY_RESOURCES_DIR") || set("KITTY_WINDOW_ID");
    // **In tmux only tmux's word for the client counts** (review,
    // 2026-11-06): the pane's environment is the tmux server's, from
    // whichever terminal started it, and a session begun in Ghostty and
    // attached from iTerm2 still carries Ghostty's variables.
    if set("TMUX") {
        let Some(said) = tmux() else {
            return Support::None;
        };
        let client = said.client_term.to_ascii_lowercase();
        let named = client.contains("ghostty") || client.contains("kitty");
        return match said.passthrough && named {
            true => Support::KittyInTmux,
            false => Support::None,
        };
    }
    let term = var("TERM").unwrap_or_default();
    if draws(term.trim()) {
        return Support::Kitty;
    }
    match var("TERM_PROGRAM").unwrap_or_default().trim() {
        "ghostty" | "kitty" => return Support::Kitty,
        "" => {}
        _ => return Support::None,
    }
    if [
        "KONSOLE_VERSION",
        "WEZTERM_EXECUTABLE",
        "ITERM_SESSION_ID",
        "VSCODE_INJECTION",
        "WT_SESSION",
        "VTE_VERSION",
        "ALACRITTY_WINDOW_ID",
        "TERMINAL_EMULATOR",
    ]
    .iter()
    .any(|name| set(name))
    {
        return Support::None;
    }
    // A `TERM` naming some other terminal (`alacritty`, `foot`) outranks
    // variables inherited from a kitty or Ghostty it was started in; only a
    // plain `xterm` one leaves them to decide.
    if !matches!(term.trim(), "" | "xterm" | "xterm-256color") {
        return Support::None;
    }
    match inherited() {
        true => Support::Kitty,
        false => Support::None,
    }
}

/// [`support_from`] on this process's environment, asking tmux if it is in
/// one. Called once, by the interactive loop; nothing else turns pictures on.
pub fn detect() -> Support {
    support_from(
        |name| std::env::var(name).ok(),
        || {
            let ask = |args: &[&str]| -> Option<String> {
                let out = std::process::Command::new("tmux").args(args).output().ok()?;
                out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
            };
            let client_term = ask(&["display-message", "-p", "#{client_termname} #{client_termtype}"])?;
            // The pane's own setting first, then the global one.
            let on = |v: Option<String>| v.is_some_and(|v| v == "on" || v == "all");
            let passthrough = on(ask(&["show-options", "-pqv", "allow-passthrough"]))
                || on(ask(&["show-options", "-gqv", "allow-passthrough"]));
            Some(Tmux { client_term, passthrough })
        },
    )
}

static SUPPORT: AtomicU8 = AtomicU8::new(0);

/// Say what this terminal can do — [`detect`]'s answer, or a test's.
pub fn set_support(support: Support) {
    let n = match support {
        Support::None => 0,
        Support::Kitty => 1,
        Support::KittyInTmux => 2,
    };
    SUPPORT.store(n, Ordering::Relaxed);
}

/// What this terminal can do. [`Support::None`] until somebody says otherwise,
/// so a frame drawn with no terminal never has a picture in it.
pub fn support() -> Support {
    match SUPPORT.load(Ordering::Relaxed) {
        1 => Support::Kitty,
        2 => Support::KittyInTmux,
        _ => Support::None,
    }
}

static CELL_W: AtomicU16 = AtomicU16::new(0);
static CELL_H: AtomicU16 = AtomicU16::new(0);

/// **How big a cell is, in pixels** — from the window size the terminal
/// reports with its rows and columns (`TIOCGWINSZ`), which Ghostty, kitty and
/// tmux 3.3 and later all fill in. Asked every frame, so a font made bigger
/// takes effect on the next one.
pub fn note_window(columns: u16, rows: u16, width_px: u16, height_px: u16) {
    if columns == 0 || rows == 0 || width_px == 0 || height_px == 0 {
        return;
    }
    CELL_W.store(width_px / columns, Ordering::Relaxed);
    CELL_H.store(height_px / rows, Ordering::Relaxed);
}

/// A cell's size in pixels, or a guess when the terminal has not said: the
/// guess only changes how much the terminal scales the picture.
pub fn cell_pixels() -> (u16, u16) {
    match (CELL_W.load(Ordering::Relaxed), CELL_H.load(Ordering::Relaxed)) {
        (w, h) if w > 0 && h > 0 => (w, h),
        _ => (10, 20),
    }
}

/// The marks that number a placeholder's row and column, in order: kitty's
/// `rowcolumn-diacritics.txt`. A picture may be this many cells each way.
const DIACRITICS: [char; 297] = [
    '\u{0305}', '\u{030D}', '\u{030E}', '\u{0310}', '\u{0312}', '\u{033D}', '\u{033E}', '\u{033F}',
    '\u{0346}', '\u{034A}', '\u{034B}', '\u{034C}', '\u{0350}', '\u{0351}', '\u{0352}', '\u{0357}',
    '\u{035B}', '\u{0363}', '\u{0364}', '\u{0365}', '\u{0366}', '\u{0367}', '\u{0368}', '\u{0369}',
    '\u{036A}', '\u{036B}', '\u{036C}', '\u{036D}', '\u{036E}', '\u{036F}', '\u{0483}', '\u{0484}',
    '\u{0485}', '\u{0486}', '\u{0487}', '\u{0592}', '\u{0593}', '\u{0594}', '\u{0595}', '\u{0597}',
    '\u{0598}', '\u{0599}', '\u{059C}', '\u{059D}', '\u{059E}', '\u{059F}', '\u{05A0}', '\u{05A1}',
    '\u{05A8}', '\u{05A9}', '\u{05AB}', '\u{05AC}', '\u{05AF}', '\u{05C4}', '\u{0610}', '\u{0611}',
    '\u{0612}', '\u{0613}', '\u{0614}', '\u{0615}', '\u{0616}', '\u{0617}', '\u{0657}', '\u{0658}',
    '\u{0659}', '\u{065A}', '\u{065B}', '\u{065D}', '\u{065E}', '\u{06D6}', '\u{06D7}', '\u{06D8}',
    '\u{06D9}', '\u{06DA}', '\u{06DB}', '\u{06DC}', '\u{06DF}', '\u{06E0}', '\u{06E1}', '\u{06E2}',
    '\u{06E4}', '\u{06E7}', '\u{06E8}', '\u{06EB}', '\u{06EC}', '\u{0730}', '\u{0732}', '\u{0733}',
    '\u{0735}', '\u{0736}', '\u{073A}', '\u{073D}', '\u{073F}', '\u{0740}', '\u{0741}', '\u{0743}',
    '\u{0745}', '\u{0747}', '\u{0749}', '\u{074A}', '\u{07EB}', '\u{07EC}', '\u{07ED}', '\u{07EE}',
    '\u{07EF}', '\u{07F0}', '\u{07F1}', '\u{07F3}', '\u{0816}', '\u{0817}', '\u{0818}', '\u{0819}',
    '\u{081B}', '\u{081C}', '\u{081D}', '\u{081E}', '\u{081F}', '\u{0820}', '\u{0821}', '\u{0822}',
    '\u{0823}', '\u{0825}', '\u{0826}', '\u{0827}', '\u{0829}', '\u{082A}', '\u{082B}', '\u{082C}',
    '\u{082D}', '\u{0951}', '\u{0953}', '\u{0954}', '\u{0F82}', '\u{0F83}', '\u{0F86}', '\u{0F87}',
    '\u{135D}', '\u{135E}', '\u{135F}', '\u{17DD}', '\u{193A}', '\u{1A17}', '\u{1A75}', '\u{1A76}',
    '\u{1A77}', '\u{1A78}', '\u{1A79}', '\u{1A7A}', '\u{1A7B}', '\u{1A7C}', '\u{1B6B}', '\u{1B6D}',
    '\u{1B6E}', '\u{1B6F}', '\u{1B70}', '\u{1B71}', '\u{1B72}', '\u{1B73}', '\u{1CD0}', '\u{1CD1}',
    '\u{1CD2}', '\u{1CDA}', '\u{1CDB}', '\u{1CE0}', '\u{1DC0}', '\u{1DC1}', '\u{1DC3}', '\u{1DC4}',
    '\u{1DC5}', '\u{1DC6}', '\u{1DC7}', '\u{1DC8}', '\u{1DC9}', '\u{1DCB}', '\u{1DCC}', '\u{1DD1}',
    '\u{1DD2}', '\u{1DD3}', '\u{1DD4}', '\u{1DD5}', '\u{1DD6}', '\u{1DD7}', '\u{1DD8}', '\u{1DD9}',
    '\u{1DDA}', '\u{1DDB}', '\u{1DDC}', '\u{1DDD}', '\u{1DDE}', '\u{1DDF}', '\u{1DE0}', '\u{1DE1}',
    '\u{1DE2}', '\u{1DE3}', '\u{1DE4}', '\u{1DE5}', '\u{1DE6}', '\u{1DFE}', '\u{20D0}', '\u{20D1}',
    '\u{20D4}', '\u{20D5}', '\u{20D6}', '\u{20D7}', '\u{20DB}', '\u{20DC}', '\u{20E1}', '\u{20E7}',
    '\u{20E9}', '\u{20F0}', '\u{2CEF}', '\u{2CF0}', '\u{2CF1}', '\u{2DE0}', '\u{2DE1}', '\u{2DE2}',
    '\u{2DE3}', '\u{2DE4}', '\u{2DE5}', '\u{2DE6}', '\u{2DE7}', '\u{2DE8}', '\u{2DE9}', '\u{2DEA}',
    '\u{2DEB}', '\u{2DEC}', '\u{2DED}', '\u{2DEE}', '\u{2DEF}', '\u{2DF0}', '\u{2DF1}', '\u{2DF2}',
    '\u{2DF3}', '\u{2DF4}', '\u{2DF5}', '\u{2DF6}', '\u{2DF7}', '\u{2DF8}', '\u{2DF9}', '\u{2DFA}',
    '\u{2DFB}', '\u{2DFC}', '\u{2DFD}', '\u{2DFE}', '\u{2DFF}', '\u{A66F}', '\u{A67C}', '\u{A67D}',
    '\u{A6F0}', '\u{A6F1}', '\u{A8E0}', '\u{A8E1}', '\u{A8E2}', '\u{A8E3}', '\u{A8E4}', '\u{A8E5}',
    '\u{A8E6}', '\u{A8E7}', '\u{A8E8}', '\u{A8E9}', '\u{A8EA}', '\u{A8EB}', '\u{A8EC}', '\u{A8ED}',
    '\u{A8EE}', '\u{A8EF}', '\u{A8F0}', '\u{A8F1}', '\u{AAB0}', '\u{AAB2}', '\u{AAB3}', '\u{AAB7}',
    '\u{AAB8}', '\u{AABE}', '\u{AABF}', '\u{AAC1}', '\u{FE20}', '\u{FE21}', '\u{FE22}', '\u{FE23}',
    '\u{FE24}', '\u{FE25}', '\u{FE26}', '\u{10A0F}', '\u{10A38}', '\u{1D185}', '\u{1D186}',
    '\u{1D187}', '\u{1D188}', '\u{1D189}', '\u{1D1AA}', '\u{1D1AB}', '\u{1D1AC}', '\u{1D1AD}',
    '\u{1D242}', '\u{1D243}', '\u{1D244}',
];

/// The most cells a picture may take each way: one mark per row and column.
pub const MOST_CELLS: u16 = DIACRITICS.len() as u16;

/// The character every placeholder cell starts with.
pub const PLACEHOLDER: char = '\u{10EEEE}';

/// One placeholder cell: the picture's `row`, `column`.
pub fn placeholder(row: u16, column: u16) -> String {
    let mut out = String::with_capacity(12);
    out.push(PLACEHOLDER);
    out.push(DIACRITICS[row as usize % DIACRITICS.len()]);
    out.push(DIACRITICS[column as usize % DIACRITICS.len()]);
    out
}

/// The colour that names picture `id` to the terminal: its low 24 bits as
/// red, green and blue. Ids stay under 2^24, so no third mark is needed.
pub fn id_colour(id: u32) -> Color {
    Color::Rgb((id >> 16) as u8, (id >> 8) as u8, id as u8)
}

/// **Lay picture `id` into `area` of the buffer**, a placeholder a cell.
///
/// The cells keep their ground; only the symbol and the ink change, and no
/// modifier is left on them — an underline under a picture would be drawn.
pub fn put_picture(buf: &mut Buffer, area: Rect, id: u32) {
    let area = area.intersection(buf.area);
    for row in 0..area.height.min(MOST_CELLS) {
        for column in 0..area.width.min(MOST_CELLS) {
            if let Some(cell) = buf.cell_mut((area.x + column, area.y + row)) {
                cell.set_symbol(&placeholder(row, column));
                cell.fg = id_colour(id);
                cell.modifier = Modifier::empty();
                cell.skip = false;
            }
        }
    }
}

/// Standard base64, padded — what the protocol's payload is written in.
pub fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (chunk[0] as u32) << 16
            | (chunk.get(1).copied().unwrap_or(0) as u32) << 8
            | chunk.get(2).copied().unwrap_or(0) as u32;
        for i in 0..4 {
            match i <= chunk.len() {
                true => out.push(ABC[(n >> (18 - 6 * i)) as usize & 63] as char),
                false => out.push('='),
            }
        }
    }
    out
}

/// How much of the payload one escape sequence carries, as the protocol asks.
const CHUNK: usize = 4096;

/// One escape sequence, wrapped for tmux if it has to pass through tmux:
/// `ESC P tmux;` and every `ESC` inside doubled.
fn wrapped(sequence: &str, tmux: bool) -> String {
    match tmux {
        false => sequence.to_string(),
        true => format!("\x1bPtmux;{}\x1b\\", sequence.replace('\x1b', "\x1b\x1b")),
    }
}

/// **The bytes that upload a PNG as picture `id`, to be shown `columns` ×
/// `rows` cells big wherever its placeholders are** — a virtual placement
/// (`U=1`). `q=2` keeps the terminal from answering into the keyboard, and
/// `C=1` keeps the cursor where it is.
pub fn upload(id: u32, png: &[u8], columns: u16, rows: u16, tmux: bool) -> Vec<u8> {
    let data = base64(png);
    let chunks: Vec<&str> = data
        .as_bytes()
        .chunks(CHUNK)
        .map(|c| std::str::from_utf8(c).expect("base64 is ASCII"))
        .collect();
    let mut out = String::with_capacity(data.len() + chunks.len() * 40 + 64);
    let last = chunks.len().saturating_sub(1);
    for (n, chunk) in chunks.iter().enumerate() {
        let more = u8::from(n < last);
        let sequence = match n {
            0 => format!("\x1b_Ga=T,U=1,f=100,i={id},c={columns},r={rows},C=1,q=2,m={more};{chunk}\x1b\\"),
            _ => format!("\x1b_Gm={more};{chunk}\x1b\\"),
        };
        out.push_str(&wrapped(&sequence, tmux));
    }
    out.into_bytes()
}

/// The bytes that free picture `id` in the terminal, data and all.
pub fn delete(id: u32, tmux: bool) -> Vec<u8> {
    wrapped(&format!("\x1b_Ga=d,d=I,i={id},q=2\x1b\\"), tmux).into_bytes()
}

/// How many pictures stay in the terminal before the oldest is freed. Each is
/// a formula, a few kilobytes; the cap keeps a long session from leaving the
/// terminal a library of every formula ever stood on.
const KEPT: usize = 16;

/// **The pictures the terminal holds**, the most recently shown last, and the
/// bytes still to be written before the next frame's cells.
struct Shelf {
    held: Vec<(u64, u32, u16, u16)>,
    next: u8,
    pending: Vec<u8>,
}

static SHELF: Mutex<Shelf> = Mutex::new(Shelf { held: Vec::new(), next: 0, pending: Vec::new() });

/// The id this process gives its `n`th picture slot: the process id in the
/// high bits, so two programs in one terminal are unlikely to share one, and
/// never 0, which the protocol reserves.
fn id_for(n: u8) -> u32 {
    ((std::process::id() & 0x7FFF) << 8) | (n as u32 % 255 + 1)
}

/// **The id picture `key` is shown under**, uploading it first if the
/// terminal does not have it at this size. The upload waits in the shelf for
/// [`take_pending`].
pub fn show(key: u64, png: &[u8], columns: u16, rows: u16) -> u32 {
    let tmux = support() == Support::KittyInTmux;
    let mut shelf = SHELF.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(at) = shelf.held.iter().position(|&(k, _, c, r)| k == key && c == columns && r == rows) {
        let entry = shelf.held.remove(at);
        shelf.held.push(entry);
        return entry.1;
    }
    while shelf.held.len() >= KEPT {
        let (_, old, _, _) = shelf.held.remove(0);
        let bytes = delete(old, tmux);
        shelf.pending.extend(bytes);
    }
    let id = loop {
        let id = id_for(shelf.next);
        shelf.next = shelf.next.wrapping_add(1);
        if !shelf.held.iter().any(|&(_, held, _, _)| held == id) {
            break id;
        }
    };
    let bytes = upload(id, png, columns, rows, tmux);
    shelf.pending.extend(bytes);
    shelf.held.push((key, id, columns, rows));
    id
}

/// The uploads and deletions owed, taken — written by the backend before the
/// cells of the frame that needs them.
pub fn take_pending() -> Vec<u8> {
    std::mem::take(&mut SHELF.lock().unwrap_or_else(|e| e.into_inner()).pending)
}

/// **Forget what the terminal holds** — after the screen was handed to a
/// shell and back, when the pictures may have gone with the screen they were
/// on. The next frame that wants one uploads it again.
pub fn forget() {
    SHELF.lock().unwrap_or_else(|e| e.into_inner()).held.clear();
}

/// The bytes that free every picture this process left in the terminal, for
/// the way out.
pub fn release_all() -> Vec<u8> {
    let tmux = support() == Support::KittyInTmux;
    let mut shelf = SHELF.lock().unwrap_or_else(|e| e.into_inner());
    let mut out = std::mem::take(&mut shelf.pending);
    for (_, id, _, _) in std::mem::take(&mut shelf.held) {
        out.extend(delete(id, tmux));
    }
    out
}

/// A PNG's size in pixels, read off its header, or `None` for anything that
/// is not a PNG.
pub fn png_size(png: &[u8]) -> Option<(u32, u32)> {
    if png.len() < 24 || &png[..8] != b"\x89PNG\r\n\x1a\n" || &png[12..16] != b"IHDR" {
        return None;
    }
    let word = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
    Some((word(16), word(20)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_is_the_standard_alphabet_padded() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }

    /// **The exact bytes**, for a payload small enough to be one chunk.
    #[test]
    fn a_small_upload_is_one_sequence() {
        let bytes = upload(42, b"foo", 12, 3, false);
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "\x1b_Ga=T,U=1,f=100,i=42,c=12,r=3,C=1,q=2,m=0;Zm9v\x1b\\"
        );
        // Inside tmux: the whole of it wrapped once, every ESC doubled.
        let bytes = upload(42, b"foo", 12, 3, true);
        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "\x1bPtmux;\x1b\x1b_Ga=T,U=1,f=100,i=42,c=12,r=3,C=1,q=2,m=0;Zm9v\x1b\x1b\\\x1b\\"
        );
        assert_eq!(String::from_utf8(delete(42, false)).unwrap(), "\x1b_Ga=d,d=I,i=42,q=2\x1b\\");
    }

    /// A payload past 4096 base64 characters goes in chunks, `m=1` on every
    /// one but the last, and only the first carries the keys.
    #[test]
    fn a_large_upload_is_sent_in_chunks() {
        let png = vec![7u8; 4000];
        let text = String::from_utf8(upload(9, &png, 2, 1, false)).unwrap();
        let sequences: Vec<&str> = text.split("\x1b\\").filter(|s| !s.is_empty()).collect();
        assert_eq!(sequences.len(), 2, "5336 characters is two chunks");
        assert!(sequences[0].starts_with("\x1b_Ga=T,U=1,f=100,i=9,c=2,r=1,C=1,q=2,m=1;"));
        assert!(sequences[1].starts_with("\x1b_Gm=0;"));
        let payload: String = sequences.iter().map(|s| s.split_once(';').unwrap().1).collect();
        assert_eq!(payload, base64(&png));
    }

    /// The cell is the placeholder and the marks for its row and column; the
    /// id rides in the ink.
    #[test]
    fn a_placeholder_names_its_row_and_column() {
        assert_eq!(placeholder(0, 0), "\u{10EEEE}\u{0305}\u{0305}");
        assert_eq!(placeholder(1, 2), "\u{10EEEE}\u{030D}\u{030E}");
        assert_eq!(id_colour(0x01_02_03), Color::Rgb(1, 2, 3));
        let mut buf = Buffer::empty(Rect::new(0, 0, 4, 2));
        put_picture(&mut buf, Rect::new(1, 0, 2, 2), 0x2A);
        assert_eq!(buf[(0, 0)].symbol(), " ");
        assert_eq!(buf[(2, 1)].symbol(), placeholder(1, 1));
        assert_eq!(buf[(2, 1)].fg, Color::Rgb(0, 0, 0x2A));
        // Each placeholder is one cell to the renderer — the private-use
        // character is not padded to two.
        assert_eq!(yumete_cjk::drawn_width(&placeholder(0, 0)), 1);
    }

    fn env<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
    }

    /// **By name, and a nested terminal is not the one around it.**
    #[test]
    fn the_terminal_is_known_by_its_name() {
        let none = || None;
        assert_eq!(support_from(env(&[("TERM", "xterm-ghostty")]), none), Support::Kitty);
        assert_eq!(support_from(env(&[("TERM_PROGRAM", "ghostty"), ("TERM", "xterm-256color")]), none), Support::Kitty);
        assert_eq!(support_from(env(&[("TERM", "xterm-kitty")]), none), Support::Kitty);
        assert_eq!(support_from(env(&[("KITTY_WINDOW_ID", "1"), ("TERM", "xterm-256color")]), none), Support::Kitty);
        // VS Code opened from kitty inherits its variables and draws nothing.
        assert_eq!(
            support_from(env(&[("KITTY_WINDOW_ID", "1"), ("TERM_PROGRAM", "vscode")]), none),
            Support::None
        );
        // Konsole answers the query and ignores placeholders.
        assert_eq!(support_from(env(&[("KONSOLE_VERSION", "230800"), ("TERM", "xterm-256color")]), none), Support::None);
        for other in ["WezTerm", "iTerm.app", "Apple_Terminal"] {
            assert_eq!(support_from(env(&[("TERM_PROGRAM", other)]), none), Support::None, "{other}");
        }
        assert_eq!(support_from(env(&[("TERM", "xterm-256color")]), none), Support::None);
        // Alacritty and gnome-terminal started from a kitty shell.
        assert_eq!(support_from(env(&[("KITTY_WINDOW_ID", "1"), ("TERM", "alacritty")]), none), Support::None);
        assert_eq!(
            support_from(env(&[("KITTY_WINDOW_ID", "1"), ("TERM", "xterm-256color"), ("VTE_VERSION", "7600")]), none),
            Support::None
        );
        assert_eq!(support_from(env(&[("TERM", "xterm-ghostty"), ("ZELLIJ", "0")]), none), Support::None);
    }

    /// Inside tmux: the outer terminal by tmux's word, and only with
    /// passthrough on.
    #[test]
    fn inside_tmux_passthrough_decides() {
        let vars = [("TMUX", "/tmp/tmux-501/default,1,0"), ("TERM", "tmux-256color"), ("TERM_PROGRAM", "tmux")];
        let said = |client: &str, passthrough: bool| {
            let client = client.to_string();
            move || Some(Tmux { client_term: client, passthrough })
        };
        assert_eq!(support_from(env(&vars), said("xterm-ghostty", true)), Support::KittyInTmux);
        assert_eq!(support_from(env(&vars), said("xterm-ghostty", false)), Support::None);
        assert_eq!(support_from(env(&vars), said("xterm-256color", true)), Support::None);
        // The terminal's own name when its TERM is a plain one.
        assert_eq!(support_from(env(&vars), said("xterm-256color ghostty 1.2.0", true)), Support::KittyInTmux);
        assert_eq!(support_from(env(&vars), || None), Support::None, "tmux would not say");
        // Begun in Ghostty, attached from iTerm2: the server's variables are
        // Ghostty's, the client is not.
        let begun = [("TMUX", "/tmp/tmux-501/default,1,0"), ("GHOSTTY_RESOURCES_DIR", "/x")];
        assert_eq!(support_from(env(&begun), said("xterm-256color iTerm2 3.5", true)), Support::None);
    }

    #[test]
    fn a_png_says_its_size() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(284u32.to_be_bytes());
        png.extend(111u32.to_be_bytes());
        assert_eq!(png_size(&png), Some((284, 111)));
        assert_eq!(png_size(b"GIF89a"), None);
    }
}
