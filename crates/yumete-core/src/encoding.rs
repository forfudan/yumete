//! **Legacy encodings**: GB18030, Big5, Shift-JIS, EUC-JP, EUC-KR and
//! UTF-16, read and written back as they came.
//!
//! A file that is valid UTF-8 is UTF-8, always. A file that opens with a
//! UTF-16 byte-order mark is UTF-16 (Notepad's 「Unicode」, Excel's
//! 「Unicode text」), and is written back with the mark. Otherwise the
//! detector (chardetng, Firefox's) guesses, and the guess is taken only when
//! it is one of the five and **the whole file** decodes in it without a single
//! bad byte. Anything else is refused: a Latin-1 file, a binary, UTF-8 with
//! one stray byte.
//!
//! Measured on samples of our own from 16
//! characters up the guess is right every time; below that a GB file may read
//! as Big5 and the detector may not decide at all (41% of two-character GB
//! files). So a file with fewer than [`SURE_FROM`] characters outside ASCII is
//! not guessed: the writer is asked, with the best guess first.
//!
//! EUC-JP and EUC-KR are never the first offer for a short file: two Chinese
//! characters in GB18030 are very often two Korean ones in EUC-KR as well,
//! and a picker that led with Korean would be wrong for nearly every file
//! here. A short file is asked about among the three, and an EUC guess comes
//! after them.

use std::fmt;
use std::path::PathBuf;

pub use encoding_rs::{Encoding, BIG5, EUC_JP, EUC_KR, GB18030, SHIFT_JIS, UTF_16BE, UTF_16LE, UTF_8};

/// The three, in the order a picker offers them when nothing else decides.
pub const LEGACY: [&Encoding; 3] = [GB18030, BIG5, SHIFT_JIS];

/// Opened without asking only when the detector names them and the file is
/// long enough to be sure; otherwise offered after the three.
const SURE_ONLY: [&Encoding; 2] = [EUC_JP, EUC_KR];

/// Whether `encoding` is UTF-16: read only behind its byte-order mark, and
/// written here, since encoding_rs, following the WHATWG standard, writes
/// UTF-8 when it is asked for UTF-16.
fn is_utf16(encoding: &'static Encoding) -> bool {
    encoding == UTF_16LE || encoding == UTF_16BE
}

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
        "euc-jp" | "eucjp" | "ujis" => Some(EUC_JP),
        "euc-kr" | "euckr" | "cp949" | "uhc" => Some(EUC_KR),
        // Only the spellings that say the byte order: vim's bare `utf-16`
        // and `ucs-2` are big-endian, the WHATWG's (helix's) little-endian.
        "utf-16le" | "utf16le" | "ucs-2le" => Some(UTF_16LE),
        "utf-16be" | "utf16be" | "ucs-2be" => Some(UTF_16BE),
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
    } else if encoding == EUC_JP {
        "EUC-JP"
    } else if encoding == EUC_KR {
        "EUC-KR"
    } else if encoding == UTF_16LE {
        "UTF-16LE"
    } else if encoding == UTF_16BE {
        "UTF-16BE"
    } else {
        "UTF-8"
    }
}

/// The words `:encoding` completes.
pub const NAMES: [&str; 8] =
    ["utf-8", "gb18030", "big5", "shift-jis", "euc-jp", "euc-kr", "utf-16le", "utf-16be"];

/// The whole of `bytes` in `encoding`, or `None` at the first byte that is not.
///
/// UTF-16 must open with its own byte-order mark, which is not part of the
/// text: almost any even run of bytes is valid UTF-16, so a UTF-8 or GB file
/// read as UTF-16 without the mark would come out as a page of rare
/// characters instead of being refused.
pub fn decode_strict(encoding: &'static Encoding, bytes: &[u8]) -> Option<String> {
    if is_utf16(encoding) {
        let (marked, len) = Encoding::for_bom(bytes)?;
        if marked != encoding {
            return None;
        }
        return encoding
            .decode_without_bom_handling_and_without_replacement(&bytes[len..])
            .map(|text| text.into_owned());
    }
    encoding
        .decode_without_bom_handling_and_without_replacement(bytes)
        .map(|text| text.into_owned())
}

/// **Another file's bytes as text** (`:read`, `:diff` against another file):
/// valid UTF-8 is UTF-8, as on open; then `encoding`, the buffer's; then any
/// other encoding [`detect`] is sure of.
pub fn read_other(bytes: &[u8], encoding: &'static Encoding) -> Option<String> {
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some(text.strip_prefix('\u{feff}').unwrap_or(text).to_string());
    }
    read_bytes(bytes, encoding)
}

/// **This file's own bytes, from elsewhere** (the copy on disk, the
/// committed copy behind the change bar): in `encoding` first, the one the
/// file is in; then as UTF-8; then in any other encoding [`detect`] is sure
/// of. `None` when none of those reads it — a change bar then draws
/// nothing rather than every line changed.
pub fn read_bytes(bytes: &[u8], encoding: &'static Encoding) -> Option<String> {
    if let Some(text) = decode_strict(encoding, bytes) {
        return Some(text);
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return Some(text.to_string());
    }
    match detect(bytes) {
        Detected::Sure(_, text) => Some(text),
        _ => None,
    }
}

/// [`read_bytes`] of a file, or [`read_other`] when it is `other`.
pub fn read_file(path: &std::path::Path, encoding: &'static Encoding, other: bool) -> std::io::Result<String> {
    let bytes = std::fs::read(path)?;
    let text = match other {
        true => read_other(&bytes, encoding),
        false => read_bytes(&bytes, encoding),
    };
    text.ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            crate::say!("encoding.refused", path.display()),
        )
    })
}

/// What the bytes of a file that is not UTF-8 are.
#[derive(Debug, PartialEq)]
pub enum Detected {
    /// One of the encodings yumete reads, sure enough to open without asking.
    Sure(&'static Encoding, String),
    /// One or more of the three (and an EUC guess) decode it, and the writer
    /// has to say which; the likeliest first.
    Ask(Vec<&'static Encoding>),
    /// None of them: not a file yumete opens.
    No,
}

/// Decide what `bytes` (which are not UTF-8) are.
pub fn detect(bytes: &[u8]) -> Detected {
    // **A UTF-16 mark decides**, before the NUL test: every ASCII character
    // in UTF-16 carries a zero byte.
    if let Some((marked, _)) = Encoding::for_bom(bytes).filter(|(e, _)| is_utf16(e)) {
        return match decode_strict(marked, bytes) {
            Some(text) => Detected::Sure(marked, text),
            None => Detected::No,
        };
    }
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
    let non_ascii = |text: &str| text.chars().filter(|c| !c.is_ascii()).count();
    let euc = match SURE_ONLY.contains(&guess) {
        true => decode_strict(guess, bytes),
        false => None,
    };
    if let Some(text) = euc.as_deref().filter(|text| non_ascii(text) >= SURE_FROM) {
        return Detected::Sure(guess, text.to_string());
    }
    let mut decodable: Vec<&'static Encoding> = Vec::new();
    if LEGACY.contains(&guess) {
        decodable.push(guess);
    }
    for one in LEGACY {
        if !decodable.contains(&one) {
            decodable.push(one);
        }
    }
    if euc.is_some() {
        decodable.push(guess);
    }
    let mut texts: Vec<(&'static Encoding, String)> = decodable
        .into_iter()
        .filter_map(|e| decode_strict(e, bytes).map(|t| (e, t)))
        .collect();
    if texts.is_empty() {
        return Detected::No;
    }
    let first = texts[0].0;
    let enough = texts[0].1.chars().filter(|c| !c.is_ascii()).count() >= SURE_FROM;
    // **Long enough to be sure, and the detector says it is none of the
    // three** (Latin-1 with its umlauts, say): refused, as before, rather
    // than offered as Chinese read wrong.
    if enough && !LEGACY.contains(&guess) {
        return Detected::No;
    }
    let sure = first == guess && enough;
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

/// **The characters a file can hold that the standard encoder cannot write**:
/// Big5's Hong Kong (HKSCS) range, Shift-JIS's user-defined area and
/// EUC-JP's JIS X 0212 (its three-byte codes). encoding_rs reads them, as the
/// WHATWG standard says, and writes none of them, so a Cantonese file opened
/// as Big5 could not be saved back untouched. The table is the decoder run
/// backwards over every code, built the first time it is needed.
fn written_back(encoding: &'static Encoding) -> &'static Back {
    use std::sync::OnceLock;
    static BIG5_BACK: OnceLock<Back> = OnceLock::new();
    static SJIS_BACK: OnceLock<Back> = OnceLock::new();
    static EUCJP_BACK: OnceLock<Back> = OnceLock::new();
    static NONE: OnceLock<Back> = OnceLock::new();
    let cell = if encoding == BIG5 {
        &BIG5_BACK
    } else if encoding == SHIFT_JIS {
        &SJIS_BACK
    } else if encoding == EUC_JP {
        &EUCJP_BACK
    } else {
        return NONE.get_or_init(Back::default);
    };
    cell.get_or_init(|| {
        let codes = if encoding == BIG5 {
            two_byte(0x81..=0xFE, &[0x40..=0x7E, 0xA1..=0xFE])
        } else if encoding == SHIFT_JIS {
            two_byte(0x81..=0xFC, &[0x40..=0x7E, 0x80..=0xFC])
        } else {
            let pairs = two_byte(0xA1..=0xFE, std::slice::from_ref(&(0xA1..=0xFE)));
            let threes: Vec<Vec<u8>> = pairs.iter().map(|code| [&[0x8F][..], code].concat()).collect();
            [pairs, threes].concat()
        };
        let mut back = Back::default();
        for code in codes {
            let Some(text) = decode_strict(encoding, &code) else { continue };
            let mut chars = text.chars();
            match (chars.next(), chars.next(), chars.next()) {
                (Some(c), None, _) => {
                    let mut buf = [0u8; 4];
                    let (_, _, bad) = encoding.encode(c.encode_utf8(&mut buf));
                    if bad {
                        back.one.entry(c).or_insert(code);
                    }
                }
                // Four Big5-HKSCS codes are a letter and a combining mark
                // (Ê̄ Ê̌ ê̄ ê̌), written back only as the pair.
                (Some(a), Some(b), None) => back.two.push(((a, b), code)),
                _ => {}
            }
        }
        back
    })
}

/// [`written_back`]'s table: one character to its code, and the few codes
/// that are two characters.
#[derive(Default)]
struct Back {
    one: std::collections::HashMap<char, Vec<u8>>,
    two: Vec<((char, char), Vec<u8>)>,
}

impl Back {
    /// The code `a` followed by `b` is written as together, if any.
    fn pair(&self, a: char, b: Option<char>) -> Option<&[u8]> {
        let b = b?;
        self.two.iter().find(|(p, _)| *p == (a, b)).map(|(_, code)| &code[..])
    }
}

/// Every two-byte code from `leads` and `trails`.
fn two_byte(leads: std::ops::RangeInclusive<u8>, trails: &[std::ops::RangeInclusive<u8>]) -> Vec<Vec<u8>> {
    leads
        .flat_map(|lead| trails.iter().flat_map(|r| r.clone()).map(move |trail| vec![lead, trail]))
        .collect()
}

/// Whether `c` is one `encoding` can hold on its own.
fn holds(encoding: &'static Encoding, c: char) -> bool {
    if c.is_ascii() || is_utf16(encoding) {
        return true;
    }
    let mut buf = [0u8; 4];
    let (_, _, bad) = encoding.encode(c.encode_utf8(&mut buf));
    !bad || written_back(encoding).one.contains_key(&c)
}

/// Each character of `text` with whether `encoding` holds it; the mark of a
/// pair that has a code of its own counts as held.
fn each_held<'a>(encoding: &'static Encoding, text: &'a str) -> impl Iterator<Item = (char, bool)> + 'a {
    let back = written_back(encoding);
    let mut chars = text.chars().peekable();
    let mut paired = false;
    std::iter::from_fn(move || {
        let c = chars.next()?;
        if std::mem::take(&mut paired) {
            return Some((c, true));
        }
        paired = back.pair(c, chars.peek().copied()).is_some();
        Some((c, paired || holds(encoding, c)))
    })
}

/// The characters of `text` that `encoding` cannot hold, or `None` when it
/// holds them all.
pub fn unencodable(encoding: &'static Encoding, text: &str) -> Option<Unencodable> {
    if encoding == UTF_8 || is_utf16(encoding) {
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
        for (col, (c, held)) in each_held(encoding, line).enumerate() {
            if held {
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
/// helix writes it, which is reached only after the writer chose to write
/// anyway. The Hong Kong, user-defined and JIS X 0212 codes go back as they
/// came. UTF-16 is written with its byte-order mark, the only way yumete
/// reads it.
pub fn encode(encoding: &'static Encoding, text: &str) -> Vec<u8> {
    if is_utf16(encoding) {
        let mut out = Vec::with_capacity(2 + 2 * text.len());
        for unit in std::iter::once(0xFEFF).chain(text.encode_utf16()) {
            out.extend_from_slice(&match encoding == UTF_16LE {
                true => unit.to_le_bytes(),
                false => unit.to_be_bytes(),
            });
        }
        return out;
    }
    let (bytes, _, bad) = encoding.encode(text);
    if !bad {
        return bytes.into_owned();
    }
    let back = written_back(encoding);
    let mut out = Vec::with_capacity(text.len());
    let mut run = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        let code = match back.pair(c, chars.peek().copied()) {
            Some(code) => {
                chars.next();
                Some(code)
            }
            None => back.one.get(&c).map(|code| &code[..]),
        };
        match code {
            Some(code) => {
                out.extend_from_slice(&encoding.encode(&run).0);
                run.clear();
                out.extend_from_slice(code);
            }
            None => run.push(c),
        }
    }
    out.extend_from_slice(&encoding.encode(&run).0);
    out
}

/// `text` with every character `encoding` cannot hold written as `&#NNNN;`,
/// so the buffer says what the file will (the 「write anyway」 answer).
pub fn with_references(encoding: &'static Encoding, text: &str) -> String {
    each_held(encoding, text)
        .map(|(c, held)| match held {
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

    /// **Hong Kong characters go back as they came**:
    /// encoding_rs reads Big5-HKSCS and writes none of it.
    #[test]
    fn a_big5_hkscs_file_saves_back_as_it_came() {
        // 咗 is 0x8C 0xBB in Big5-HKSCS: read, and not written by the standard.
        let bytes = [0x8Cu8, 0xBB];
        let text = decode_strict(BIG5, &bytes).expect("HKSCS is read");
        assert_eq!(unencodable(BIG5, &text), None, "and it can be written back");
        assert_eq!(encode(BIG5, &text), bytes);
    }

    #[test]
    fn a_long_latin1_file_is_refused_not_offered_as_chinese() {
        let text = "Grüße aus München, schöne Grüße an die Familie, über die Brücke".repeat(3);
        let latin1: Vec<u8> = text.chars().map(|c| c as u32 as u8).collect();
        assert_eq!(detect(&latin1), Detected::No);
    }

    #[test]
    fn another_file_in_utf8_is_read_as_utf8_whatever_this_one_is() {
        let note = "清晨的雾气还没有散去".as_bytes();
        assert_eq!(read_other(note, GB18030).as_deref(), Some("清晨的雾气还没有散去"));
    }

    #[test]
    fn the_names_people_write() {
        assert_eq!(by_name("GBK"), Some(GB18030));
        assert_eq!(by_name("sjis"), Some(SHIFT_JIS));
        assert_eq!(by_name("Shift_JIS"), Some(SHIFT_JIS));
        assert_eq!(by_name("latin1"), None);
        assert_eq!(by_name("EUC-KR"), Some(EUC_KR));
        assert_eq!(by_name("utf-16le"), Some(UTF_16LE));
        assert_eq!(by_name("utf-16"), None, "vim reads it as big-endian, helix as little-endian");
        for word in NAMES {
            let encoding = by_name(word).unwrap_or_else(|| panic!("{word}"));
            assert_eq!(by_name(name(encoding)), Some(encoding), "the shown name reads back");
        }
    }

    const KO: &str = "아침 안개가 아직 걷히지 않았는데 작은 마을의 돌길에는 벌써 사람들이 걷고 있었다.";

    /// **Notepad's 「Unicode」**: UTF-16 behind its mark, either byte order,
    /// read whole and written back to the same bytes.
    #[test]
    fn a_utf16_file_with_its_mark_reads_and_writes_back_unchanged() {
        let text = "清晨的雾气还没有散去。\r\nA line with 𠀀 outside the BMP.\r\n";
        for encoding in [UTF_16LE, UTF_16BE] {
            let file = encode(encoding, text);
            let mark: &[u8] = if encoding == UTF_16LE { &[0xFF, 0xFE] } else { &[0xFE, 0xFF] };
            assert!(file.starts_with(mark), "{}", name(encoding));
            match detect(&file) {
                Detected::Sure(got, read) => {
                    assert_eq!(got, encoding);
                    assert_eq!(read, text, "the mark is not part of the text");
                }
                other => panic!("{}: {other:?}", name(encoding)),
            }
            assert_eq!(read_bytes(&file, encoding).as_deref(), Some(text));
            assert_eq!(unencodable(encoding, text), None, "UTF-16 holds every character");
        }
        // A short ASCII file is UTF-16 too: its zero bytes are not a binary's.
        assert_eq!(detect(&encode(UTF_16LE, "ab")), Detected::Sure(UTF_16LE, "ab".into()));
        // Without the mark it is not read as UTF-16: almost any bytes would be.
        assert_eq!(decode_strict(UTF_16LE, "ab".as_bytes()), None);
        // A lone surrogate is not UTF-16.
        assert_eq!(detect(&[0xFF, 0xFE, 0x00, 0xD8, 0x61, 0x00]), Detected::No);
    }

    /// EUC-KR and EUC-JP open when the detector is sure, and go back to the
    /// same bytes, JIS X 0212's three-byte codes included.
    #[test]
    fn euc_files_long_enough_open_and_write_back_unchanged() {
        for (encoding, text) in [(EUC_KR, KO), (EUC_JP, JA)] {
            let file = bytes(encoding, text);
            match detect(&file) {
                Detected::Sure(got, read) => {
                    assert_eq!(name(got), name(encoding));
                    assert_eq!(read, text);
                }
                other => panic!("{}: {other:?}", name(encoding)),
            }
            assert_eq!(encode(encoding, text), file);
        }
        // 丂 is JIS X 0212 only: 0x8F 0xB0 0xA1, which encoding_rs reads and
        // does not write.
        let file = [b"a".as_slice(), &[0x8F, 0xB0, 0xA1], b"b"].concat();
        let text = decode_strict(EUC_JP, &file).expect("JIS X 0212 is read");
        assert_eq!(text, "a丂b");
        assert_eq!(unencodable(EUC_JP, &text), None);
        assert_eq!(encode(EUC_JP, &text), file);
    }

    /// A short Korean file is not offered as Korean first: two GB characters
    /// are often two EUC-KR ones as well.
    #[test]
    fn a_short_euc_file_is_offered_after_the_three() {
        if let Detected::Ask(candidates) = detect(&bytes(EUC_KR, "안개")) {
            if let Some(at) = candidates.iter().position(|e| *e == EUC_KR) {
                assert!(candidates[..at].iter().all(|e| LEGACY.contains(e)), "{candidates:?}");
                assert_eq!(at, candidates.len() - 1);
            }
        }
    }

    /// **Every Big5-HKSCS code goes back as it came**, the four that are a
    /// letter and a combining mark included; a code that duplicates a
    /// standard Big5 character is written in the standard form, and the text
    /// is the same.
    #[test]
    fn every_big5_code_saves_back_to_a_code_for_the_same_text() {
        for code in two_byte(0x81..=0xFE, &[0x40..=0x7E, 0xA1..=0xFE]) {
            let Some(text) = decode_strict(BIG5, &code) else { continue };
            assert_eq!(unencodable(BIG5, &text), None, "{code:x?} {text}");
            let back = encode(BIG5, &text);
            assert_eq!(decode_strict(BIG5, &back).as_deref(), Some(&text[..]), "{code:x?}");
        }
        // 0x88 0x62 is Ê̄, two characters.
        let text = decode_strict(BIG5, &[0x88, 0x62]).unwrap();
        assert_eq!(text.chars().count(), 2);
        assert_eq!(encode(BIG5, &format!("a{text}b")), [b"a".as_slice(), &[0x88, 0x62], b"b"].concat());
        assert_eq!(with_references(BIG5, &text), text);
    }
}
