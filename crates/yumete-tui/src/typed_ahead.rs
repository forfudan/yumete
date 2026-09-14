//! Keys the reader typed before the editor was on screen.
//!
//! Two start-up probes ask the terminal a question and read the answer
//! **straight off the descriptor** — `theme::ask_the_terminal` (「你的底色是
//! 什麼」) and `ambiguous::ask_the_terminal_about_width` (「這個字佔幾格」).
//! Reading a descriptor takes whatever is in it, and what is in it at that
//! moment is the terminal's reply *and anything the reader has already typed*.
//!
//! ⚠️ **That was thrown away, and it is the first four tenths of a second of a
//! session.** `yumete 第三章.md` followed straight away by `iHELLO` left the
//! file untouched and the editor still in Normal mode: measured at 0.0 s, 0.1,
//! 0.2 and 0.3 (gone) against 0.4 (kept). The comment above the probe claimed
//! the opposite — 「the reply is read straight off the descriptor rather than
//! through the event reader, so nothing the reader types can be eaten by it」 —
//! and reading the descriptor is exactly how it was eaten.
//!
//! So the probes hand the bytes that were **not** part of the reply to
//! [`keep`], and the front end plays them back through the same door `--keys`
//! uses before it enters the loop.
//!
//! # What is kept, and what is not
//!
//! Only what a person types: printable characters, `Tab`, `Enter`. **An `\x1b`
//! ends the batch** — everything from it on is dropped. A terminal answers
//! with escape sequences, and a session that replayed one as keystrokes would
//! be worse than the bug: `\x1b[2;3R` is `Esc [ 2 ; 3 R`, and `R` in Normal
//! mode opens a replace. Losing a typed `Esc` costs nothing; guessing at one
//! costs the manuscript.

use std::sync::Mutex;

static PENDING: Mutex<Vec<char>> = Mutex::new(Vec::new());

/// Keep the part of `seen` that lies outside `reply`, as far as it is plainly
/// something a person typed.
///
/// `reply` is the byte range the probe recognised as the terminal's answer;
/// pass an empty range when nothing was recognised, and the whole buffer is
/// considered — a probe that timed out was very likely reading a person.
pub fn keep(seen: &[u8], reply: std::ops::Range<usize>) {
    let mut typed: Vec<u8> = Vec::new();
    typed.extend_from_slice(&seen[..reply.start.min(seen.len())]);
    if reply.end < seen.len() {
        typed.extend_from_slice(&seen[reply.end..]);
    }
    let chars = readable(&typed);
    if chars.is_empty() {
        return;
    }
    if let Ok(mut pending) = PENDING.lock() {
        pending.extend(chars);
    }
}

/// Everything kept so far, and the buffer is empty afterwards.
pub fn take() -> Vec<char> {
    match PENDING.lock() {
        Ok(mut pending) => std::mem::take(&mut *pending),
        Err(_) => Vec::new(),
    }
}

/// The leading run of `bytes` that reads as typing, as characters.
///
/// Stops at the first `\x1b` — see the module note — and at any other control
/// byte apart from `\t` and the two newlines. Invalid UTF-8 stops it too: a
/// half-arrived multibyte character is not a keystroke yet.
fn readable(bytes: &[u8]) -> Vec<char> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        // Up to the bad byte, which is where a partially read character sits.
        Err(err) => std::str::from_utf8(&bytes[..err.valid_up_to()]).unwrap_or(""),
    };
    text.chars()
        .take_while(|&c| c == '\t' || c == '\r' || c == '\n' || !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::readable;

    #[test]
    fn what_a_person_typed_is_kept_and_what_a_terminal_said_is_not() {
        assert_eq!(readable(b"iHELLO"), "iHELLO".chars().collect::<Vec<_>>());
        assert_eq!(readable("i那年冬天".as_bytes()).len(), 5);
        assert_eq!(readable(b":w\r"), vec![':', 'w', '\r']);
        // **The batch ends at the first escape.** Replaying `\x1b[2;3R` as
        // keys would press `R` in Normal mode, which opens a replace.
        assert_eq!(readable(b"ab\x1b[2;3R"), vec!['a', 'b']);
        assert_eq!(readable(b"\x1b]11;rgb:1c1c/1c1c/1c1c\x1b\\"), Vec::<char>::new());
        // A character that has only half arrived is not a keystroke yet.
        assert_eq!(readable(&[b'a', 0xE4, 0xB8]), vec!['a']);
    }
}
