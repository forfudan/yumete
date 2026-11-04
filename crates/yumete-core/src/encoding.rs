//! **Legacy encodings** (2026-11-04): GB18030, Big5 and Shift-JIS, read and
//! written back as they came.
//!
//! A file that is valid UTF-8 is UTF-8, always. Otherwise the detector
//! (chardetng, Firefox's) guesses, and the guess is taken only when it is one
//! of the three and **the whole file** decodes in it without a single bad
//! byte. Anything else is refused as before: a Latin-1 file, a binary, UTF-8
//! with one stray byte.
//!
//! Measured on samples of our own (research of 2026-11-04): from 16
//! characters up the guess is right every time; below that a GB file may read
//! as Big5 and the detector may not decide at all (41% of two-character GB
//! files). So a file with fewer than [`SURE_FROM`] characters outside ASCII is
//! not guessed: the writer is asked, with the best guess first.

use std::fmt;
use std::path::PathBuf;

pub use encoding_rs::{Encoding, BIG5, GB18030, SHIFT_JIS, UTF_8};

/// The three, in the order a picker offers them when nothing else decides.
pub const LEGACY: [&Encoding; 3] = [GB18030, BIG5, SHIFT_JIS];

/// How many characters outside ASCII a file needs before its guess is taken
/// without asking.
pub const SURE_FROM: usize = 16;

/// How much of a file the detector reads.
const SNIFF: usize = 64 * 1024;

/// An encoding by any name people write it with; `None` for one yumete does
/// not open.
pub fn by_name(word: &str) -> Option<&'static Encoding> {
    match word.trim().to_ascii_lowercase().replace('_', "-").as_str() {
        "utf-8" | "utf8" => Some(UTF_8),
        "gb18030" | "gbk" | "gb2312" | "cp936" | "euc-cn" => Some(GB18030),
        "big5" | "big5-hkscs" | "cp950" => Some(BIG5),
        "shift-jis" | "sjis" | "cp932" | "ms932" | "windows-31j" => Some(SHIFT_JIS),
        _ => None,
    }
}

/// The name an encoding is shown under, and the word `:encoding` takes.
pub fn name(encoding: &'static Encoding) -> &'static str {
    if encoding == GB18030 {
        "GB18030"
    } else if encoding == BIG5 {
        "Big5"
    } else if encoding == SHIFT_JIS {
        "Shift-JIS"
    } else {
        "UTF-8"
    }
}

/// The words `:encoding` completes.
pub const NAMES: [&str; 4] = ["utf-8", "gb18030", "big5", "shift-jis"];

/// The whole of `bytes` in `encoding`, or `None` at the first byte that is not.
pub fn decode_strict(encoding: &'static Encoding, bytes: &[u8]) -> Option<String> {
    encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|text| text.into_owned())
}

/// What the bytes of a file that is not UTF-8 are.
#[derive(Debug, PartialEq)]
pub enum Detected {
    /// One of the three, sure enough to open without asking.
    Sure(&'static Encoding, String),
    /// One or more of the three decode it, and the writer has to say which;
    /// the likeliest first.
    Ask(Vec<&'static Encoding>),
    /// None of them: not a file yumete opens.
    No,
}

/// Decide what `bytes` (which are not UTF-8) are.
pub fn detect(bytes: &[u8]) -> Detected {
    // A NUL near the start is a binary, and no encoding list is offered for a
    // picture.
    if bytes[..bytes.len().min(1024)].contains(&0) {
        return Detected::No;
    }
    let mut sniff = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
    // **Not finished unless it is the whole file**: a prefix cut in the middle
    // of a character, fed as finished, turned Big5 into windows-1252.
    let last = bytes.len() <= SNIFF;
    sniff.feed(&bytes[..bytes.len().min(SNIFF)], last);
    let guess = sniff.guess(None, chardetng::Utf8Detection::Deny);
    let guess = if guess == encoding_rs::GBK { GB18030 } else { guess };
    let mut decodable: Vec<&'static Encoding> = Vec::new();
    if LEGACY.contains(&guess) {
        decodable.push(guess);
    }
    for one in LEGACY {
        if !decodable.contains(&one) {
            decodable.push(one);
        }
    }
    let mut texts: Vec<(&'static Encoding, String)> = decodable
        .into_iter()
        .filter_map(|e| decode_strict(e, bytes).map(|t| (e, t)))
        .collect();
    if texts.is_empty() {
        return Detected::No;
    }
    let first = texts[0].0;
    let sure = first == guess
        && texts[0].1.chars().filter(|c| !c.is_ascii()).count() >= SURE_FROM;
    if sure {
        let (encoding, text) = texts.swap_remove(0);
        return Detected::Sure(encoding, text);
    }
    Detected::Ask(texts.into_iter().map(|(e, _)| e).collect())
}

/// A file that is not UTF-8 and could be one of several encodings: the
/// writer has to say which. Carried inside an `io::Error`, so every path that
/// opens a file passes it along unchanged; the front end looks for it.
#[derive(Debug, Clone)]
pub struct Undecided {
    pub path: PathBuf,
    /// The encodings that decode it, the likeliest first.
    pub candidates: Vec<&'static Encoding>,
}

impl fmt::Display for Undecided {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.candidates.iter().map(|e| name(e)).collect();
        write!(f, "{}", crate::say!("encoding.undecided", self.path.display(), names.join(" / ")))
    }
}

impl std::error::Error for Undecided {}

/// Characters the file's encoding cannot hold, found before a save touches
/// the disk. Carried inside an `io::Error`, as [`Undecided`] is.
#[derive(Debug, Clone, PartialEq)]
pub struct Unencodable {
    pub encoding: &'static Encoding,
    /// Where the first one is: line and column from 1, and the character.
    pub first: (usize, usize, char),
    /// How many there are in all.
    pub count: usize,
    /// The distinct ones, in order, at most eight.
    pub sample: Vec<char>,
}

impl fmt::Display for Unencodable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (line, col, c) = self.first;
        write!(
            f,
            "{}",
            crate::say!("encoding.cannot-hold", name(self.encoding), c, line, col, self.count)
        )
    }
}

impl std::error::Error for Unencodable {}

/// Whether `c` is one `encoding` can hold.
fn holds(encoding: &'static Encoding, c: char) -> bool {
    if c.is_ascii() {
        return true;
    }
    let mut buf = [0u8; 4];
    let (_, _, bad) = encoding.encode(c.encode_utf8(&mut buf));
    !bad
}

/// The characters of `text` that `encoding` cannot hold, or `None` when it
/// holds them all.
pub fn unencodable(encoding: &'static Encoding, text: &str) -> Option<Unencodable> {
    if encoding == UTF_8 {
        return None;
    }
    // The whole text first: one pass, and the answer nearly every time.
    let (_, _, bad) = encoding.encode(text);
    if !bad {
        return None;
    }
    let mut first = None;
    let mut count = 0;
    let mut sample = Vec::new();
    for (n, line) in text.split('\n').enumerate() {
        for (col, c) in line.chars().enumerate() {
            if holds(encoding, c) {
                continue;
            }
            count += 1;
            first.get_or_insert((n + 1, col + 1, c));
            if sample.len() < 8 && !sample.contains(&c) {
                sample.push(c);
            }
        }
    }
    first.map(|first| Unencodable { encoding, first, count, sample })
}

/// `text` in `encoding`; a character it cannot hold becomes `&#NNNN;`, as
/// helix writes it. Only reached after the writer chose to write anyway.
pub fn encode(encoding: &'static Encoding, text: &str) -> Vec<u8> {
    encoding.encode(text).0.into_owned()
}

/// `text` with every character `encoding` cannot hold written as `&#NNNN;`,
/// so the buffer says what the file will (the 「write anyway」 answer).
pub fn with_references(encoding: &'static Encoding, text: &str) -> String {
    text.chars()
        .map(|c| match holds(encoding, c) {
            true => c.to_string(),
            false => format!("&#{};", c as u32),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(encoding: &'static Encoding, text: &str) -> Vec<u8> {
        let (out, _, bad) = encoding.encode(text);
        assert!(!bad, "{text} fits {}", name(encoding));
        out.into_owned()
    }

    /// Our own sample text, long enough to be sure of.
    const ZHS: &str = "清晨的雾气还没有散去，小镇的石板路上已经有了行人，卖豆浆的摊子冒着热气。";
    const ZHT: &str = "清晨的霧氣還沒有散去，小鎮的石板路上已經有了行人，賣豆漿的攤子冒着熱氣。";
    const JA: &str = "朝の霧がまだ晴れないうちに、小さな町の石畳の道にはもう人が歩いていた。";

    #[test]
    fn each_of_the_three_is_told_apart_when_there_is_enough_of_it() {
        for (encoding, text) in [(GB18030, ZHS), (BIG5, ZHT), (SHIFT_JIS, JA)] {
            match detect(&bytes(encoding, text)) {
                Detected::Sure(got, decoded) => {
                    assert_eq!(name(got), name(encoding));
                    assert_eq!(decoded, text);
                }
                other => panic!("{}: {other:?}", name(encoding)),
            }
        }
    }

    #[test]
    fn a_short_file_is_asked_about_with_the_guess_first() {
        match detect(&bytes(BIG5, "霧氣")) {
            Detected::Ask(candidates) => assert!(candidates.contains(&BIG5), "{candidates:?}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn what_is_none_of_the_three_is_refused() {
        assert_eq!(detect(b"caf\xe9 au lait, s'il vous pla\xeet"), Detected::No, "Latin-1");
        assert_eq!(detect(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR"), Detected::No, "a picture");
    }

    #[test]
    fn what_an_encoding_cannot_hold_is_found_with_its_place() {
        let text = "霧氣\n小鎮简这字";
        let found = unencodable(BIG5, text).expect("简 has no Big5 code");
        assert_eq!(found.first, (2, 3, '简'));
        assert_eq!(found.count, 2, "简 and 这");
        assert_eq!(found.sample, ['简', '这']);
        assert_eq!(unencodable(GB18030, text), None, "GB18030 holds every character");
        assert_eq!(with_references(BIG5, "a简"), "a&#31616;");
        assert_eq!(encode(BIG5, "a简"), b"a&#31616;");
    }

    #[test]
    fn the_names_people_write() {
        assert_eq!(by_name("GBK"), Some(GB18030));
        assert_eq!(by_name("sjis"), Some(SHIFT_JIS));
        assert_eq!(by_name("Shift_JIS"), Some(SHIFT_JIS));
        assert_eq!(by_name("latin1"), None);
    }
}
