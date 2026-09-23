//! **The wire to a preview server's control plane** (2026-09-23).
//!
//! `tinymist preview` opens two ports. One of them is the page the browser
//! reads (`crate::…`／`yumete_tui::server_address` picks it). The other is a
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
//! ⚠️ **It is not 「save, then let it notice」.** Measured against the tinymist
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
//! ⚠️ **No websocket crate.** A client that only ever *sends* needs the
//! handshake, the masking, and nothing else: it may skip `Sec-WebSocket-Accept`
//! (a 101 is the answer), which is the only part that would want sha1. That is
//! the whole of why this file is 150 lines and adds no dependency — the same
//! bargain #53 took (「新依賴只有 serde_json」).

use std::path::Path;

/// **The opening request**, ready to write to a freshly connected socket.
///
/// ⚠️ **`Origin` is not optional here.** The server logs 「websocket connection
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
/// ⚠️ **The status line, not the accept key.** Verifying `Sec-WebSocket-Accept`
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
/// ⚠️ **A client frame must be masked** (RFC 6455 §5.3): a server is required
/// to close the connection on an unmasked one. The mask is four bytes the
/// caller supplies; nothing here is a secret, so anything that varies will do.
///
/// ⚠️ **Three length forms, and a manuscript needs the third.** Under 126 the
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
/// ⚠️ **`line` and `character` are 0-based**, and tinymist's own comment says
/// 「fixme: character is 0-based, UTF-16 code unit. We treat it as UTF-8 now」 —
/// so this passes characters, not UTF-16 units, deliberately matching what it
/// does rather than what it says it means to do one day.
pub fn change_cursor(path: &Path, line: usize, character: usize) -> String {
    format!(
        r#"{{"event":"changeCursorPosition","filepath":{path},"line":{line},"character":{character}}}"#,
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
fn base64(bytes: &[u8]) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

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

        // ⚠️ **A manuscript lands here**, so this arm is the feature.
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
            // ⚠️ 服務器自己在日誌裏說少了它「will be a hard error in the future」。
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
        // ⚠️ A stray control character must not be able to break the message:
        // a manuscript with one in it is a manuscript the preview never sees.
        let odd = update_memory_files(Path::new("/a.typ"), "a\u{1}b");
        assert!(odd.contains("a\\u0001b"), "{odd}");
    }

    #[test]
    fn the_cursor_message_is_zero_based() {
        assert_eq!(
            change_cursor(Path::new("/a.typ"), 0, 0),
            r#"{"event":"changeCursorPosition","filepath":"/a.typ","line":0,"character":0}"#
        );
    }
}
