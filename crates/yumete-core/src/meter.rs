//! 平仄 — the shape of a line of verse (Feature #247).
//!
//! Classical Chinese verse is built out of two tone classes rather than out of
//! stress: 平 (the level tone) and 仄 (everything else), alternating to a
//! pattern the poet is writing *against*. A poet writing 律詩 or 填詞 keeps that
//! pattern in their head and checks it by ear, and the check is exactly the sort
//! of thing an editor can do without being asked twice — the 拆分表 already
//! carries a 帶調 reading for every character on the page.
//!
//! So `:meter` puts the 詞譜's own notation in the margin the readings and the
//! 着重號 share: `○` 平, `●` 仄, and at the end of a 句 the hollow and solid
//! **triangles** that mark a 韻腳.
//!
//! ## ⚠️ This is 今音平仄, and 入聲 is where it lies
//!
//! The tone class is read off 現代漢語 拼音, because that is the reading the
//! data holds. 入聲 — the fourth class of 中古音, and 仄 in every classical
//! rule — was distributed across all four modern tones (入派三聲), so a
//! character like 竹 (zhú), 白 (bái) or 石 (shí) is **仄 in the rule and 平 in
//! this margin**. There is no 中古音 column in the 拆分表 to consult, and
//! inventing one from the 拼音 is not possible: the information is gone.
//!
//! For 新韻 (中華新韻, 十四韻) this margin is simply correct. For 平水韻 it is a
//! first pass that catches the 三四聲 half of the mistakes and stays silent
//! about the other half — which is worth having, and worth knowing the shape
//! of.
//!
//! ## 輕聲 says nothing
//!
//! A syllable with no tone at all is neither 平 nor 仄; it gets no mark. In
//! classical verse the question does not arise — 輕聲 is a feature of the modern
//! spoken language — and in modern verse a 的 or a 了 is not where the pattern
//! is.

use yumete_cjk::is_han;

/// Which of the two classes a character's tone puts it in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// 平: 陰平 and 陽平, the first and second tones.
    Ping,
    /// 仄: 上聲 and 去聲 — and, in the rule though not in this data, 入聲.
    Ze,
}

/// One character's 平仄, and whether it closes a 句.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    /// Which character of the line, counted in `char`s.
    pub column: usize,
    /// 平 or 仄.
    pub level: Level,
    /// A 韻腳: the last 漢字 of a 句, which is where a rhyme falls.
    pub rhyme: bool,
}

/// The punctuation that closes a 句.
///
/// The comma is in here and the 頓號 is not, which is the distinction 詞譜 makes:
/// a 逗 ends a 句 that rhymes in half the 詞牌 there are, while 、 separates
/// items inside one. A 韻腳 mark is a statement about *where a rhyme could
/// fall*, not a claim that this line rhymes there.
const STOPS: [char; 10] = ['。', '，', '！', '？', '；', '：', '.', ',', '!', '?'];

/// Which tone a 帶調 pinyin syllable is written in: 1–4, or [`None`] for 輕聲.
///
/// Both spellings of a diacritic are read — `ā` as one character and `a` plus a
/// combining macron — because a reading that came out of a table somebody
/// typed may be in either, and telling a writer their 平聲 line has a 輕聲 in it
/// because of a normalisation form would be a lie about their poem.
pub fn tone_of(syllable: &str) -> Option<u8> {
    for ch in syllable.chars() {
        let tone = match ch {
            'ā' | 'ē' | 'ī' | 'ō' | 'ū' | 'ǖ' | '\u{304}' => 1,
            'á' | 'é' | 'í' | 'ó' | 'ú' | 'ǘ' | 'ń' | 'ḿ' | '\u{301}' => 2,
            'ǎ' | 'ě' | 'ǐ' | 'ǒ' | 'ǔ' | 'ǚ' | 'ň' | '\u{30c}' => 3,
            'à' | 'è' | 'ì' | 'ò' | 'ù' | 'ǜ' | 'ǹ' | '\u{300}' => 4,
            _ => continue,
        };
        return Some(tone);
    }
    None
}

/// Which class that tone falls in, or [`None`] for 輕聲.
pub fn level_of(syllable: &str) -> Option<Level> {
    match tone_of(syllable)? {
        1 | 2 => Some(Level::Ping),
        _ => Some(Level::Ze),
    }
}

/// Read the 平仄 off one line.
///
/// `words` are the line's own word boundaries in `char`s — the segmenter's, so
/// that `read` is asked the question it can answer (了 is `le` in 為了 and
/// `liǎo` in 了解, and only a word-level table knows which) — and `read` is the
/// installed [`yumete_cjk::Reader`], which gives back one syllable per
/// character or nothing at all.
///
/// A word the reader has never heard of is skipped in silence: a mark under
/// a character whose reading was guessed is worse than a gap, because the poet
/// would trust it.
pub fn marks(
    chars: &[char],
    words: &[(usize, usize)],
    read: &dyn Fn(&str) -> Option<Vec<String>>,
) -> Vec<Mark> {
    let mut out: Vec<Mark> = Vec::new();
    for &(start, end) in words {
        if end > chars.len() || start >= end {
            continue;
        }
        let word: String = chars[start..end].iter().collect();
        if !word.chars().all(is_han) {
            continue;
        }
        let Some(readings) = read(&word) else { continue };
        // A reading that does not cover the word would put the tones under the
        // wrong characters — the same rule `:ruby auto` keeps.
        if readings.len() != end - start {
            continue;
        }
        for (n, syllable) in readings.iter().enumerate() {
            if let Some(level) = level_of(syllable) {
                out.push(Mark {
                    column: start + n,
                    level,
                    rhyme: closes_a_sentence(chars, start + n),
                });
            }
        }
    }
    out.sort_by_key(|m| m.column);
    out
}

/// Whether the 漢字 at `column` is the last one of its 句.
///
/// Which is asked of what comes *after* it: a 句讀 mark before the next 漢字, or
/// nothing at all before the end of the line. Anything else in between — a
/// closing 「」, a footnote marker, a space — is transparent, because a rhyme
/// falls on the character and not on the quotation mark that follows it.
fn closes_a_sentence(chars: &[char], column: usize) -> bool {
    for &ch in &chars[column + 1..] {
        if is_han(ch) {
            return false;
        }
        if STOPS.contains(&ch) {
            return true;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reader for the four tones and nothing else.
    fn toy(word: &str) -> Option<Vec<String>> {
        let mut out = Vec::new();
        for ch in word.chars() {
            let syllable = match ch {
                '春' => "chūn",
                '眠' => "mián",
                '不' => "bù",
                '覺' => "jué",
                '曉' => "xiǎo",
                '的' => "de",
                _ => return None,
            };
            out.push(syllable.to_string());
        }
        Some(out)
    }

    fn per_char(chars: &[char]) -> Vec<(usize, usize)> {
        (0..chars.len()).map(|i| (i, i + 1)).collect()
    }

    fn read(line: &str) -> Vec<Mark> {
        let chars: Vec<char> = line.chars().collect();
        marks(&chars, &per_char(&chars), &toy)
    }

    #[test]
    fn the_first_two_tones_are_ping_and_the_other_two_are_ze() {
        let found = read("春眠不覺曉");
        let levels: Vec<Level> = found.iter().map(|m| m.level).collect();
        assert_eq!(
            levels,
            vec![Level::Ping, Level::Ping, Level::Ze, Level::Ping, Level::Ze]
        );
        // 覺 is jué — 入聲 in the rule, 陽平 in this data. The doc comment says
        // so out loud and this test is where that promise is written down.
        assert_eq!(found[3].level, Level::Ping);
    }

    #[test]
    fn a_syllable_with_no_tone_gets_no_mark() {
        let found = read("春的眠");
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].column, 0);
        assert_eq!(found[1].column, 2);
    }

    #[test]
    fn the_last_character_of_a_sentence_is_where_a_rhyme_falls() {
        let found = read("春眠，不覺曉。");
        let rhymes: Vec<usize> = found.iter().filter(|m| m.rhyme).map(|m| m.column).collect();
        assert_eq!(rhymes, vec![1, 5]);
    }

    #[test]
    fn a_closing_quotation_mark_does_not_move_the_rhyme() {
        let found = read("「春眠」。");
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[1].rhyme, "{found:?}");
    }

    #[test]
    fn the_end_of_a_line_closes_a_sentence_whether_or_not_it_is_punctuated() {
        let found = read("春眠");
        assert!(!found[0].rhyme);
        assert!(found[1].rhyme);
    }

    #[test]
    fn a_word_the_reader_has_never_heard_of_is_left_unmarked() {
        let chars: Vec<char> = "春阿眠".chars().collect();
        let found = marks(&chars, &per_char(&chars), &toy);
        assert_eq!(found.len(), 2, "{found:?}");
    }

    #[test]
    fn a_reading_that_does_not_cover_the_word_is_refused() {
        let chars: Vec<char> = "春眠".chars().collect();
        let short = |_: &str| Some(vec!["chūn".to_string()]);
        assert!(marks(&chars, &[(0, 2)], &short).is_empty());
    }

    #[test]
    fn a_combining_diacritic_is_read_as_the_tone_it_is() {
        assert_eq!(tone_of("chu\u{304}n"), Some(1));
        assert_eq!(tone_of("de"), None);
        assert_eq!(level_of("ni\u{30c}"), Some(Level::Ze));
    }

    #[test]
    fn what_is_not_han_carries_no_meter() {
        let chars: Vec<char> = "abc".chars().collect();
        assert!(marks(&chars, &[(0, 3)], &toy).is_empty());
    }
}
