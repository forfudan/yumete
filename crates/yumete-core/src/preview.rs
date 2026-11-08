//! **The wire to a preview server's control plane** (2026-09-23).
//!
//! `tinymist preview` opens two ports. One of them is the page the browser
//! reads (`crate::…`/`yumete_tui::server_address` picks it). The other is a
//! **websocket the editor talks on**, and on it the editor can hand the
//! typesetter the text of a buffer that has never been saved:
//!
//! ```text
//! {"event":"updateMemoryFiles","files":{"/書/第一章.typ":"…全文…"}}
//! ```
//!
//! That is how the VS Code extension keeps its preview a keystroke behind the
//! caret, and 2026-09-23 報的就是這個：「vscode 的 tinymist 預覽，每次按鍵他都
//! 會刷新一下（而且似乎是增量編譯，所以反應很快）。我們是不是也可以做到？」
//!
//! Warning: **It is not 「save, then let it notice」.** Measured against the tinymist
//! on this machine (build 2025-07-25): push a *deliberately broken* text over
//! the socket while the file on disk stays good, and the server answers
//! `error: unknown variable: undefined_function_xyz` — it compiled what was
//! pushed, in 921 µs, and the disk was never touched.
//!
//! **Split the way [`crate::lsp`] is split**, and for the same reason: this
//! half is string in, string out, so a protocol mistake is caught by a test
//! that costs a string literal rather than by a browser and a stopwatch. The
//! socket, the thread and the process live in the front end.
//!
//! Warning: **No websocket crate.** A client that only ever *sends* needs the
//! handshake, the masking, and nothing else: it may skip `Sec-WebSocket-Accept`
//! (a 101 is the answer), which is the only part that would want sha1. That is
//! the whole of why this file is 150 lines and adds no dependency — the same
//! bargain #53 took (「新依賴只有 serde_json」).

use std::path::Path;

/// **The opening request**, ready to write to a freshly connected socket.
///
/// Warning: **`Origin` is not optional here.** The server logs 「websocket connection
/// is not set `Origin` header, which will be a hard error in the future」 when
/// it is left off (seen in its own log, 2026-09-23), so it goes in now rather
/// than on the day it starts refusing us.
pub fn handshake(host: &str, key: &str) -> String {
    format!(
        "GET / HTTP/1.1\r\n\
         Host: {host}\r\n\
         Origin: http://{host}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {key}\r\n\
         Sec-WebSocket-Version: 13\r\n\
         \r\n"
    )
}

/// Did the server agree to speak websocket?
///
/// Warning: **The status line, not the accept key.** Verifying `Sec-WebSocket-Accept`
/// proves the answer came from something that read our key — worth having on a
/// public network, worth nothing against a loopback port we spawned ourselves,
/// and it is the one thing on this wire that would cost a sha1.
pub fn accepted(head: &str) -> bool {
    head.lines()
        .next()
        .is_some_and(|line| line.starts_with("HTTP/1.1 101") || line.starts_with("HTTP/1.0 101"))
}

/// **One client text frame** — `FIN`, opcode 1, masked, with `text` inside.
///
/// Warning: **A client frame must be masked** (RFC 6455 §5.3): a server is required
/// to close the connection on an unmasked one. The mask is four bytes the
/// caller supplies; nothing here is a secret, so anything that varies will do.
///
/// Warning: **Three length forms, and a manuscript needs the third.** Under 126 the
/// length is in the second byte; under 64 KiB it is two more bytes; past that
/// it is eight. A chapter is past the first form on its first paragraph and a
/// book is past the second, so the 64-bit arm is the one that carries the
/// feature, not a corner.
pub fn frame(text: &str, mask: [u8; 4]) -> Vec<u8> {
    let body = text.as_bytes();
    let n = body.len();
    let mut out = Vec::with_capacity(n + 14);
    out.push(0x81);
    match n {
        0..=125 => out.push(0x80 | n as u8),
        126..=0xFFFF => {
            out.push(0x80 | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        _ => {
            out.push(0x80 | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    out.extend_from_slice(&mask);
    out.extend(body.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    out
}

/// **「這個檔現在長這樣」** — the message that makes the preview redraw.
///
/// The path is the one the typesetter knows the file by, so it must be the
/// absolute path it was started on; a relative one names nothing in its world.
pub fn update_memory_files(path: &Path, text: &str) -> String {
    format!(
        r#"{{"event":"updateMemoryFiles","files":{{{path}:{text}}}}}"#,
        path = json_string(&path.to_string_lossy()),
        text = json_string(text),
    )
}

/// **「光標在這裏」** — the preview scrolls to match.
///
/// Free with the socket, and the reason the socket is worth opening even for a
/// document that is not being typed into.
///
/// Warning: **`panelScrollTo`, not `changeCursorPosition`** (2026-11-07). The
/// second only ever drew a caret mark on the page, and tinymist has ignored
/// it since 0.15.2 (#2586: 「cursor indicator … temporarily disabled. Request
/// ignored」), so the browser preview had stopped following the caret.
/// Scrolling was always `panelScrollTo`, and the page window sends the same
/// message through the language server (`tinymist.scrollPreview`).
///
/// Warning: **`line` and `character` are 0-based**, and tinymist's own comment says
/// 「fixme: character is 0-based, UTF-16 code unit. We treat it as UTF-8 now」 —
/// it counts characters (`line_column_to_byte`), so this passes characters,
/// not UTF-16 units, matching what it does rather than what it says it means
/// to do one day.
pub fn panel_scroll_to(path: &Path, line: usize, character: usize) -> String {
    format!(
        r#"{{"event":"panelScrollTo","filepath":{path},"line":{line},"character":{character}}}"#,
        path = json_string(&path.to_string_lossy()),
    )
}

/// 16 bytes of `seed`, base64'd — what `Sec-WebSocket-Key` is.
///
/// The value is echoed back hashed and we do not check it, so all it has to be
/// is **sixteen bytes and valid base64**: a server that reads the length is
/// within its rights to refuse a shorter one.
pub fn key(seed: u128) -> String {
    base64(&seed.to_be_bytes())
}

/// Plain base64, no line breaks — twelve lines rather than a dependency.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for group in bytes.chunks(3) {
        let mut packed = 0u32;
        for (i, b) in group.iter().enumerate() {
            packed |= u32::from(*b) << (16 - 8 * i);
        }
        for i in 0..4 {
            match i <= group.len() {
                true => out.push(ALPHABET[(packed >> (18 - 6 * i)) as usize & 0x3F] as char),
                false => out.push('='),
            }
        }
    }
    out
}

/// A JSON string, escaped — the same rule [`crate::lsp`] writes by, for the
/// same reason: the payload is a whole manuscript and a stray control
/// character in it must not be able to make the message unparseable.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}


// ---- The page window: tinymist through the language server ----------------
//
// **The page beside the text** (2026-11-07). The window draws the page the
// cursor is on, as a picture, and asks the tinymist the editor already runs
// as its Typst language server for two things: the page as a PNG made from
// the unsaved text (`tinymist.exportPng` with `write: false`), and where on
// which page the cursor's text landed (a preview task in the server, whose
// data plane answers `panelScrollTo` with `jump,<page> <x> <y>`). The
// research is `local/survey-0.20/graphics.md` and its follow-up; this half is
// the strings, as above, and the process and the socket live in the front end.

/// **The oldest tinymist that hands a page over in memory**: `exportPng`
/// with `write: false` came with PR #2098, first released in v0.13.30. The
/// one before it ignored the option, wrote `doc.png` beside the document and
/// answered with a path.
pub const LEAST_TINYMIST: (u32, u32, u32) = (0, 13, 30);

/// A version as tinymist writes it in `serverInfo` — `0.15.12`, with or
/// without a leading `v` and a `-rc1` after it — as three numbers. A missing
/// part is 0; anything else that is not a number is `None`.
pub fn version_of(text: &str) -> Option<(u32, u32, u32)> {
    let text = text.trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let core = text.split(['-', '+', ' ']).next()?;
    let mut parts = core.split('.');
    let mut next = || -> Option<u32> {
        match parts.next() {
            None => Some(0),
            Some(part) => part.parse().ok(),
        }
    };
    let found = (next()?, next()?, next()?);
    (!core.is_empty()).then_some(found)
}

/// **Whether this tinymist can draw the page window.** `version` is what its
/// `serverInfo` said; a tinymist that said nothing is older than the version
/// that started saying it, and so too old.
pub fn new_enough(version: Option<&str>) -> bool {
    version.and_then(version_of).is_some_and(|v| v >= LEAST_TINYMIST)
}

/// **The name of a preview's task inside tinymist** (2026-11-08): one task
/// a preview tab, named by the tab's id, since several may be open and
/// tinymist keeps its tasks by name. The server is the editor's own, so the
/// names need only differ within this process.
pub fn task_id(preview: u64) -> String {
    format!("yumete-{preview}")
}

/// **`tinymist.exportPng`'s arguments for page `page` (from 1) of `main`, at
/// `ppi` pixels to the inch**, kept in memory: the answer carries the PNG as
/// base64 and nothing is written beside the document.
pub fn export_png_arguments(main: &Path, page: usize, ppi: u32) -> String {
    format!(
        r#"[{main},{{"pages":["{page}"],"ppi":{ppi}}},{{"write":false}}]"#,
        main = json_string(&main.to_string_lossy()),
    )
}

/// **`tinymist.doStartPreview`'s arguments**: the task, a data plane on a
/// loopback port the system picks, no browser, and not the server's primary
/// preview (that one follows whichever file the server thinks is main).
pub fn start_preview_arguments(task: &str, main: &Path) -> String {
    format!(
        r#"[["--task-id",{task},"--data-plane-host","127.0.0.1:0","--no-open","--not-primary",{main}]]"#,
        task = json_string(task),
        main = json_string(&main.to_string_lossy()),
    )
}

/// **`tinymist.scrollPreview`'s arguments**: the cursor's place, which the
/// task answers on its data plane with where that text is on the page. The
/// numbers count as [`panel_scroll_to`]'s do.
pub fn scroll_preview_arguments(task: &str, path: &Path, line: usize, character: usize) -> String {
    format!(r#"[{},{}]"#, json_string(task), panel_scroll_to(path, line, character))
}

/// **`tinymist.doKillPreview`'s arguments**: the task to stop.
pub fn kill_preview_arguments(task: &str) -> String {
    format!(r#"[{}]"#, json_string(task))
}

/// **Where a piece of text is on the page**: the page from 1, and the point
/// in typographic points from the page's top left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Jump {
    pub page: usize,
    pub x: f64,
    pub y: f64,
}

/// **Read a `jump` frame off the data plane** — `jump,16 49.94 476.19`, or
/// several places after the comma when the text is in more than one; the
/// first is the one taken. Anything else on the plane (the document itself,
/// `viewport,…`) is `None`.
pub fn jump(frame: &str) -> Option<Jump> {
    let rest = frame.strip_prefix("jump,")?;
    let first = rest.split(',').next()?;
    let mut numbers = first.split_whitespace();
    let page: usize = numbers.next()?.parse().ok()?;
    let x: f64 = numbers.next()?.parse().ok()?;
    let y: f64 = numbers.next()?.parse().ok()?;
    (page > 0 && x.is_finite() && y.is_finite()).then_some(Jump { page, x, y })
}

/// **Standard base64 back to bytes**, padded or not — what a PNG comes in.
/// Anything outside the alphabet is `None`.
pub fn decode_base64(text: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let bytes = text.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= value(c)? << (18 - 6 * i);
        }
        match chunk.len() {
            4 => out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8, n as u8]),
            3 => out.extend_from_slice(&[(n >> 16) as u8, (n >> 8) as u8]),
            2 => out.push((n >> 16) as u8),
            _ => return None,
        }
    }
    Some(out)
}

/// **Whole messages off a server's websocket**, put together from its frames.
///
/// The data plane talks back, unlike the control plane above: a `jump` is a
/// few bytes, and the document itself, which yumete reads and throws away, is
/// a hundred kilobytes at first. A server's frames are not masked, may be
/// split (opcode 0 carries on the one before), and a ping or a close may come
/// between the pieces.
#[derive(Debug, Default)]
pub struct Incoming {
    buf: Vec<u8>,
    /// The opcode and the pieces so far of a message split across frames.
    partial: Option<(u8, Vec<u8>)>,
}

/// Opcode 8: the server is closing the connection.
pub const CLOSE: u8 = 8;

impl Incoming {
    /// Bytes as they came off the socket.
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// **The next whole message**: its opcode (1 text, 2 binary, 8 close, 9
    /// ping, 10 pong) and its body. `None` means 「not all here yet」.
    pub fn next_message(&mut self) -> Option<(u8, Vec<u8>)> {
        loop {
            let (fin, opcode, body) = self.take_frame()?;
            match opcode {
                0 => {
                    let (code, mut pieces) = self.partial.take()?;
                    pieces.extend_from_slice(&body);
                    match fin {
                        true => return Some((code, pieces)),
                        false => self.partial = Some((code, pieces)),
                    }
                }
                // A control frame is never split, and may come in between.
                8..=10 => return Some((opcode, body)),
                _ if fin => return Some((opcode, body)),
                _ => self.partial = Some((opcode, body)),
            }
        }
    }

    /// One frame off the front of the buffer, unmasked.
    fn take_frame(&mut self) -> Option<(bool, u8, Vec<u8>)> {
        let buf = &self.buf;
        if buf.len() < 2 {
            return None;
        }
        let fin = buf[0] & 0x80 != 0;
        let opcode = buf[0] & 0x0F;
        let masked = buf[1] & 0x80 != 0;
        let (length, mut at) = match buf[1] & 0x7F {
            126 if buf.len() >= 4 => (u16::from_be_bytes([buf[2], buf[3]]) as usize, 4),
            127 if buf.len() >= 10 => (u64::from_be_bytes(buf[2..10].try_into().ok()?) as usize, 10),
            126 | 127 => return None,
            n => (n as usize, 2),
        };
        let mask = match masked {
            true if buf.len() >= at + 4 => {
                let m = [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]];
                at += 4;
                Some(m)
            }
            true => return None,
            false => None,
        };
        if buf.len() < at + length {
            return None;
        }
        let mut body = buf[at..at + length].to_vec();
        if let Some(mask) = mask {
            body.iter_mut().enumerate().for_each(|(i, b)| *b ^= mask[i % 4]);
        }
        self.buf.drain(..at + length);
        Some((fin, opcode, body))
    }
}

/// **What the page window asks tinymist for**: which file's page, which
/// page, how sharp, and which text — `text` counts the times the server was
/// handed the buffer, so a new count is new text in its hands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Want {
    pub main: std::path::PathBuf,
    pub page: usize,
    pub ppi: u32,
    pub text: u64,
}

/// **When to ask for the page again** (2026-11-07): one request out at a
/// time; a new page or a new size at once; new text only once the typing has
/// been still for a while ([`Refresh::QUIET`]), since a page made in the
/// middle of a word is a page nobody reads. Pure, so the clock is handed in.
#[derive(Debug, Clone, Default)]
pub struct Refresh {
    /// The last edit seen: a signature of the text and when it changed.
    edited: Option<(u64, std::time::Instant)>,
    /// What is being made now.
    asked: Option<Want>,
    /// What the window shows, or showed last: the answer that landed, or
    /// the failure that did.
    shown: Option<Want>,
}

impl Refresh {
    /// How long the typing has to be still before the page is made again.
    pub const QUIET: std::time::Duration = std::time::Duration::from_millis(300);

    /// Note the text as it is now; a new `signature` restarts the wait.
    pub fn edited(&mut self, signature: u64, now: std::time::Instant) {
        if self.edited.map(|(was, _)| was) != Some(signature) {
            self.edited = Some((signature, now));
        }
    }

    fn quiet_since(&self, now: std::time::Instant) -> Option<std::time::Duration> {
        let (_, at) = self.edited?;
        Self::QUIET.checked_sub(now.saturating_duration_since(at)).filter(|left| !left.is_zero())
    }

    /// **Whether to ask for `want` now.**
    pub fn due(&self, want: &Want, now: std::time::Instant) -> bool {
        if self.asked.is_some() {
            return false;
        }
        match &self.shown {
            None => true,
            Some(shown) if shown == want => false,
            Some(shown) if shown.text != want.text => self.quiet_since(now).is_none(),
            Some(_) => true,
        }
    }

    /// `want` was asked for.
    pub fn ask(&mut self, want: Want) {
        self.asked = Some(want);
    }

    /// The answer came — a page, or a failure, which is not asked for again
    /// until something changes.
    pub fn answered(&mut self) {
        if let Some(asked) = self.asked.take() {
            self.shown = Some(asked);
        }
    }

    /// The question will never be answered (the server went): ask again.
    pub fn lost(&mut self) {
        self.asked = None;
    }

    /// Whether a question is out.
    pub fn asking(&self) -> bool {
        self.asked.is_some()
    }

    /// Forget what was shown, so the next look asks again.
    pub fn forget(&mut self) {
        self.shown = None;
    }

    /// **How long the loop may sleep** before [`Refresh::due`] can turn true
    /// for `want` without anything else happening: the rest of the quiet
    /// wait, or `None` when nothing is waiting on the clock.
    pub fn wake_in(&self, want: &Want, now: std::time::Instant) -> Option<std::time::Duration> {
        if self.asked.is_some() {
            return None;
        }
        match &self.shown {
            Some(shown) if shown.text != want.text => self.quiet_since(now),
            _ => None,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// **Too old is the same as unknown** (2026-11-07): `write: false` came in
    /// 0.13.30, and the tinymist before it sent no `serverInfo` at all.
    #[test]
    fn only_a_tinymist_that_says_a_new_enough_version_draws_the_window() {
        assert_eq!(version_of("0.15.12"), Some((0, 15, 12)));
        assert_eq!(version_of("v0.13.30-rc1"), Some((0, 13, 30)));
        assert_eq!(version_of("1.0"), Some((1, 0, 0)));
        assert_eq!(version_of("nightly"), None);
        assert_eq!(version_of(""), None);
        assert!(new_enough(Some("0.15.12")));
        assert!(new_enough(Some("0.13.30")));
        assert!(new_enough(Some("1.0.0")));
        assert!(!new_enough(Some("0.13.29")));
        assert!(!new_enough(Some("0.13.10")));
        assert!(!new_enough(Some("what")));
        assert!(!new_enough(None), "no version is too old");
    }

    /// The commands' arguments, as the research sent them to 0.15.12.
    #[test]
    fn the_page_window_asks_in_the_words_tinymist_reads() {
        let main = Path::new("/書/main.typ");
        assert_eq!(
            export_png_arguments(main, 21, 144),
            r#"["/書/main.typ",{"pages":["21"],"ppi":144},{"write":false}]"#
        );
        let task = task_id(7);
        assert_eq!(task, "yumete-7");
        assert_eq!(
            start_preview_arguments(&task, main),
            r#"[["--task-id","yumete-7","--data-plane-host","127.0.0.1:0","--no-open","--not-primary","/書/main.typ"]]"#
        );
        assert_eq!(
            scroll_preview_arguments(&task, Path::new("/書/第一章.typ"), 100, 5),
            r#"["yumete-7",{"event":"panelScrollTo","filepath":"/書/第一章.typ","line":100,"character":5}]"#
        );
        assert_eq!(kill_preview_arguments(&task), r#"["yumete-7"]"#);
        // **Two previews, two tasks** (2026-11-08): each tab names its own.
        assert_ne!(start_preview_arguments(&task_id(7), main), start_preview_arguments(&task_id(8), main));
        for made in [
            export_png_arguments(main, 1, 72),
            start_preview_arguments(&task, main),
            scroll_preview_arguments(&task, main, 0, 0),
            kill_preview_arguments(&task),
        ] {
            assert!(serde_json::from_str::<serde_json::Value>(&made).is_ok(), "{made}");
        }
    }

    #[test]
    fn a_jump_says_the_page_and_the_point() {
        assert_eq!(jump("jump,16 49.94 476.19"), Some(Jump { page: 16, x: 49.94, y: 476.19 }));
        // Text that lands twice: the first place.
        assert_eq!(jump("jump,3 1 2,4 5 6"), Some(Jump { page: 3, x: 1.0, y: 2.0 }));
        assert_eq!(jump("viewport,3 1 2"), None);
        assert_eq!(jump("jump,"), None);
        assert_eq!(jump("jump,0 1 2"), None, "pages count from 1");
        assert_eq!(jump("jump,x 1 2"), None);
        assert_eq!(jump("diff-v1,..."), None);
    }

    #[test]
    fn base64_comes_back_to_the_bytes_it_was() {
        for bytes in [&b""[..], b"f", b"fo", b"foo", b"foobar", &[0xff, 0xfe, 0xfd, 0, 1]] {
            assert_eq!(decode_base64(&base64(bytes)).as_deref(), Some(bytes), "{bytes:?}");
        }
        assert_eq!(decode_base64("Zm9v"), Some(b"foo".to_vec()));
        assert_eq!(decode_base64("Zg"), Some(b"f".to_vec()), "unpadded");
        assert_eq!(decode_base64("Zm9v!"), None);
        assert_eq!(decode_base64("Z"), None, "one character is no byte");
    }

    /// A server frame: unmasked, three length forms, split across reads and
    /// across frames, with a ping in between.
    #[test]
    fn a_server_s_frames_come_back_as_whole_messages() {
        fn server(fin: bool, opcode: u8, body: &[u8]) -> Vec<u8> {
            let mut out = vec![(u8::from(fin) << 7) | opcode];
            match body.len() {
                n @ 0..=125 => out.push(n as u8),
                n @ 126..=0xFFFF => {
                    out.push(126);
                    out.extend((n as u16).to_be_bytes());
                }
                n => {
                    out.push(127);
                    out.extend((n as u64).to_be_bytes());
                }
            }
            out.extend_from_slice(body);
            out
        }
        let mut incoming = Incoming::default();
        let jump = server(true, 2, b"jump,1 2 3");
        incoming.push(&jump[..4]);
        assert_eq!(incoming.next_message(), None, "half a frame");
        incoming.push(&jump[4..]);
        assert_eq!(incoming.next_message(), Some((2, b"jump,1 2 3".to_vec())));
        // The document, big, in two frames with a ping between them.
        let big = vec![7u8; 70_000];
        incoming.push(&server(false, 2, &big[..300]));
        incoming.push(&server(true, 9, b""));
        incoming.push(&server(true, 0, &big[300..]));
        assert_eq!(incoming.next_message(), Some((9, Vec::new())));
        assert_eq!(incoming.next_message(), Some((2, big)));
        assert_eq!(incoming.next_message(), None);
        // A masked one (a server should not, but a reader should not choke).
        let mask = [1, 2, 3, 4];
        let mut masked = frame("hi", mask);
        masked[0] = 0x81;
        incoming.push(&masked);
        assert_eq!(incoming.next_message(), Some((1, b"hi".to_vec())));
        incoming.push(&server(true, CLOSE, b""));
        assert_eq!(incoming.next_message(), Some((CLOSE, Vec::new())));
    }

    fn want(page: usize, ppi: u32, text: u64) -> Want {
        Want { main: "/a.typ".into(), page, ppi, text }
    }

    /// **The refresh**: one question out at a time, a new page at once, new
    /// text only after the typing has been still for 300 ms.
    #[test]
    fn the_page_is_asked_for_once_at_a_time_and_after_the_typing_stops() {
        let t0 = std::time::Instant::now();
        let ms = |n: u64| t0 + std::time::Duration::from_millis(n);
        let mut refresh = Refresh::default();
        // Nothing shown yet: ask.
        assert!(refresh.due(&want(1, 144, 0), t0));
        refresh.ask(want(1, 144, 0));
        assert!(!refresh.due(&want(1, 144, 0), t0), "one at a time");
        assert!(!refresh.due(&want(2, 144, 0), t0), "even for another page");
        assert_eq!(refresh.wake_in(&want(2, 144, 0), t0), None, "the answer rings");
        refresh.answered();
        assert!(!refresh.due(&want(1, 144, 0), ms(1)), "it is showing");
        // Another page, or another size: at once.
        assert!(refresh.due(&want(2, 144, 0), ms(1)));
        assert!(refresh.due(&want(1, 200, 0), ms(1)));
        // Typing: the server has new text, and the hand is still moving.
        refresh.edited(1, ms(10));
        assert!(!refresh.due(&want(1, 144, 1), ms(100)));
        assert_eq!(refresh.wake_in(&want(1, 144, 1), ms(100)), Some(std::time::Duration::from_millis(210)));
        refresh.edited(2, ms(200));
        assert!(!refresh.due(&want(1, 144, 2), ms(400)), "every key restarts the wait");
        refresh.edited(2, ms(450));
        assert!(refresh.due(&want(1, 144, 2), ms(500)), "the same text is no new edit");
        refresh.ask(want(1, 144, 2));
        // A failure is shown too, and not asked for again until something
        // changes.
        refresh.answered();
        assert!(!refresh.due(&want(1, 144, 2), ms(900)));
        // A server that went: the question is asked again.
        refresh.ask(want(3, 144, 2));
        refresh.lost();
        assert!(refresh.due(&want(3, 144, 2), ms(901)));
        refresh.forget();
        assert!(refresh.due(&want(1, 144, 2), ms(902)), "forgotten: ask again");
    }

    /// The three length forms, and that the mask really is applied.
    #[test]
    fn a_frame_is_masked_and_carries_its_own_length() {
        let mask = [0x11, 0x22, 0x33, 0x44];

        // Short: length lives in the second byte.
        let short = frame("hi", mask);
        assert_eq!(short[0], 0x81, "FIN ＋ text");
        assert_eq!(short[1], 0x80 | 2, "masked, two bytes");
        assert_eq!(&short[2..6], &mask, "the mask goes before the body");
        assert_eq!(short[6] ^ mask[0], b'h');
        assert_eq!(short[7] ^ mask[1], b'i');

        // 126 bytes: two more bytes of length.
        let medium = frame(&"x".repeat(200), mask);
        assert_eq!(medium[1], 0x80 | 126);
        assert_eq!(u16::from_be_bytes([medium[2], medium[3]]), 200);
        assert_eq!(medium.len(), 4 + 4 + 200);

        // Warning: **A manuscript lands here**, so this arm is the feature.
        let long = frame(&"字".repeat(30_000), mask);
        assert_eq!(long[1], 0x80 | 127);
        assert_eq!(u64::from_be_bytes(long[2..10].try_into().unwrap()), 90_000);
        assert_eq!(long.len(), 10 + 4 + 90_000);
    }

    /// The body has to come back out the way it went in.
    #[test]
    fn unmasking_a_frame_gives_the_text_back() {
        let text = "第一章\n那年冬天，天很冷。\n";
        let mask = [0xDE, 0xAD, 0xBE, 0xEF];
        let sent = frame(text, mask);
        let body: Vec<u8> = sent[6..].iter().enumerate().map(|(i, b)| b ^ mask[i % 4]).collect();
        assert_eq!(String::from_utf8(body).unwrap(), text);
    }

    #[test]
    fn the_handshake_says_everything_the_server_asks_for() {
        let asked = handshake("127.0.0.1:23626", "AQIDBAUGBwgJCgsMDQ4PEA==");
        for line in [
            "GET / HTTP/1.1",
            "Host: 127.0.0.1:23626",
            // Warning: 服務器自己在日誌裏說少了它「will be a hard error in the future」。
            "Origin: http://127.0.0.1:23626",
            "Upgrade: websocket",
            "Connection: Upgrade",
            "Sec-WebSocket-Key: AQIDBAUGBwgJCgsMDQ4PEA==",
            "Sec-WebSocket-Version: 13",
        ] {
            assert!(asked.contains(line), "少了 {line:?}：{asked:?}");
        }
        assert!(asked.ends_with("\r\n\r\n"), "頭要自己收尾");
    }

    #[test]
    fn only_a_hundred_and_one_counts_as_yes() {
        assert!(accepted("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n"));
        assert!(!accepted("HTTP/1.1 200 OK\r\n"));
        assert!(!accepted("HTTP/1.1 400 Bad Request\r\n"));
        assert!(!accepted(""));
    }

    /// 16 bytes in, 24 base64 characters out — what the header wants.
    #[test]
    fn the_key_is_sixteen_bytes_of_base64() {
        let made = key(0x0102_0304_0506_0708_090a_0b0c_0d0e_0f10);
        assert_eq!(made, "AQIDBAUGBwgJCgsMDQ4PEA==");
        assert_eq!(made.len(), 24);
        // Two different seeds must not make the same key, or a reconnect looks
        // to a proxy like a replay of the last one.
        assert_ne!(key(1), key(2));
    }

    #[test]
    fn base64_pads_the_way_the_standard_does() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    /// The message the whole feature is for, and that the text is escaped.
    #[test]
    fn the_text_of_a_buffer_goes_over_as_one_json_string() {
        let said = update_memory_files(
            Path::new("/書/第一章.typ"),
            "= 一\n他說「好」。\t\\n",
        );
        assert_eq!(
            said,
            r#"{"event":"updateMemoryFiles","files":{"/書/第一章.typ":"= 一\n他說「好」。\t\\n"}}"#
        );
        // Warning: A stray control character must not be able to break the message:
        // a manuscript with one in it is a manuscript the preview never sees.
        let odd = update_memory_files(Path::new("/a.typ"), "a\u{1}b");
        assert!(odd.contains("a\\u0001b"), "{odd}");
    }

    #[test]
    fn the_cursor_message_scrolls_the_panel_and_is_zero_based() {
        assert_eq!(
            panel_scroll_to(Path::new("/a.typ"), 0, 0),
            r#"{"event":"panelScrollTo","filepath":"/a.typ","line":0,"character":0}"#
        );
    }
}
