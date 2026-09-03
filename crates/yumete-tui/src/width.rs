//! Asking the terminal how wide an ambiguous character is.
//!
//! East-Asian **Ambiguous** characters — `—` `…` `“” ‘’` `·` `※` `▓` — are one
//! cell in a Latin font and two in a CJK one, and Annex #11 says the
//! environment decides. Every width question in the editor asks one global
//! ([`yumete_cjk::set_ambiguous_wide`]), so getting it wrong does not misplace
//! one character: it shifts the caret, the ground, the wrap and the click map
//! by one cell for **every one of them on the line**, and a Chinese paragraph
//! has dozens.
//!
//! The config used to answer this, wide by default, and a writer whose terminal
//! draws them narrow had no way to know that was the setting that had the
//! cursor sitting two cells past the character it was on. So the terminal is
//! asked instead — it is the only thing that knows, and it can be asked in a
//! tenth of a second: print one ambiguous character, ask where the cursor
//! ended up (CPR, `\x1b[6n`), and count.

/// The character the question is asked with.
///
/// `—` (U+2014 EM DASH) rather than a box-drawing character or a quote mark:
/// it is the one that appears in every chapter of Chinese prose, so it is the
/// one whose width the writer will actually notice being wrong, and a font
/// that draws it wide draws the rest of the ambiguous block wide too.
const ASK_WITH: &str = "—";

/// Ask the terminal whether an ambiguous character takes two cells.
///
/// `None` if there is nothing to ask (not a terminal) or it does not answer.
/// The reply is read straight off the descriptor rather than through the event
/// reader, the way the background is asked in [`crate::theme::ask_the_terminal`]
/// — nothing the writer types can be eaten by it, and a silent terminal costs a
/// tenth of a second at start-up.
///
/// The row is left as it was found: the probe writes at the start of the
/// current line and erases it afterwards. It has to run **before** the
/// alternate screen is entered, while that line is still scratch.
#[cfg(unix)]
pub fn ask_the_terminal_about_width() -> Option<bool> {
    use std::io::{IsTerminal, Read, Write};
    use std::os::fd::AsRawFd;
    use std::time::{Duration, Instant};

    let mut out = std::io::stdout();
    if !out.is_terminal() || !std::io::stdin().is_terminal() {
        return None;
    }
    // Raw, or the terminal echoes the reply onto the screen and hands it over a
    // line at a time — which is to say, never.
    let raw = ratatui::crossterm::terminal::enable_raw_mode().is_ok();
    let answer = (|| {
        // Column 1, a clean line, the character, and then: where are you?
        out.write_all(b"\r\x1b[2K").ok()?;
        out.write_all(ASK_WITH.as_bytes()).ok()?;
        out.write_all(b"\x1b[6n").ok()?;
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
            if let Some(wide) = cursor_moved_two(&reply) {
                return Some(wide);
            }
            // A terminal that is answering something else entirely.
            if reply.len() > 256 {
                return None;
            }
        }
    })();
    // Whatever happened, take the line back: a probe that leaves its own `—`
    // on the screen is a probe the writer can see.
    let _ = out.write_all(b"\r\x1b[2K");
    let _ = out.flush();
    if raw {
        let _ = ratatui::crossterm::terminal::disable_raw_mode();
    }
    answer
}

#[cfg(not(unix))]
pub fn ask_the_terminal_about_width() -> Option<bool> {
    None
}

/// Read `\x1b[row;colR` out of a terminal's reply: is the cursor in column 3?
///
/// The character was printed in column 1, so the cursor lands in column 2 if it
/// took one cell and column 3 if it took two. Anything else — a terminal that
/// wrapped, or answered about a different row — is no answer at all rather than
/// a guess, because a guess here shifts every line on the page.
fn cursor_moved_two(reply: &[u8]) -> Option<bool> {
    let text = std::str::from_utf8(reply).ok()?;
    let rest = text.split(';').nth(1)?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    // Not finished arriving: the column may still be growing a digit.
    if digits.is_empty() || !rest[digits.len()..].starts_with('R') {
        return None;
    }
    match digits.parse::<usize>().ok()? {
        2 => Some(false),
        3 => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::cursor_moved_two;

    #[test]
    fn a_cursor_in_column_two_means_one_cell() {
        assert_eq!(cursor_moved_two(b"\x1b[1;2R"), Some(false));
        assert_eq!(cursor_moved_two(b"\x1b[24;2R"), Some(false));
    }

    #[test]
    fn a_cursor_in_column_three_means_two_cells() {
        assert_eq!(cursor_moved_two(b"\x1b[1;3R"), Some(true));
    }

    #[test]
    fn half_a_reply_is_not_an_answer() {
        // The column is still arriving: `2` here is the first digit of `24`,
        // and answering now would say narrow about a terminal that wrapped.
        assert_eq!(cursor_moved_two(b"\x1b[1;2"), None);
        assert_eq!(cursor_moved_two(b"\x1b[1;"), None);
        assert_eq!(cursor_moved_two(b""), None);
    }

    #[test]
    fn a_cursor_anywhere_else_is_no_answer() {
        // Column 1 (the terminal wrapped, or drew nothing), or column 24 (it is
        // answering about something else entirely).
        assert_eq!(cursor_moved_two(b"\x1b[1;1R"), None);
        assert_eq!(cursor_moved_two(b"\x1b[1;24R"), None);
    }
}
