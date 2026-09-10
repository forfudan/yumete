//! Readings over Yume's own tables — Feature #234.
//!
//! `:ruby-auto` needs an answer to 「how is this read」, and yumete has none of
//! its own: the readings live in 宇浩's 全息拆分表 (`chaifen.ydiv`, one 讀音
//! column per character, 帶調, 多音 separated by `_`) and the disambiguation
//! lives in the 讀音表 (`pinyin.yflb`, 2.17M 讀音-詞 pairs with the weight the
//! language model ranks by). Both are already loaded and reference-counted by
//! [`ImeSession`](crate::ImeSession), exactly as the segmenter's two tables are.
//!
//! ## The whole point is the word
//!
//! A per-character table says 了 is `le` **or** `liǎo` and stops there — which
//! is why Word's 拼音指南 writes `liǎo` in 為了 and nobody can stop it. The
//! 讀音表 knows 為了 as a word, spelled `wei le`, so the question 「which
//! combination of these characters' readings is a *word*」 has an answer:
//!
//! 1. take each character's readings from the 拆分表, 常用 first;
//! 2. build the combinations, cheapest first, as `wei le` / `wei liao`;
//! 3. ask the 讀音表 which of them it has actually seen for **this text**, and
//!    take the likeliest ([`FluencyTable::spell_logprob`] is the same 編碼層
//!    term the 整句 lattice scores with);
//! 4. and when it has seen none of them — a name, a 生僻詞, anything the table
//!    never counted — fall back to each character's 常用 reading, which is what
//!    a per-character tool would have said all along.
//!
//! Step 4 is not a failure: it is the per-character answer, arrived at after
//! asking a better question, and it is still right for every character that has
//! only one reading — which is the overwhelming majority of them.

use std::sync::Arc;

use yume_core::annotation::AnnotationTable;
use yume_core::pinyin_tone;
use yume_core::FluencyTable;
use yumete_cjk::{is_han, Reader};

/// How many readings of one character are worth combining.
///
/// The 拆分表 lists every reading a character has, 古音 and 姓氏音 included; the
/// ones that change a word are the first two (重 chóng/zhòng, 行 xíng/háng,
/// 長 cháng/zhǎng). Taking a third multiplies the combinations for a reading
/// nobody writes — the same 2 [`yume_core::reading_synth::TOP_READINGS`] settled
/// on when it built these very pairs at compile time.
const TOP_READINGS: usize = 2;

/// The most combinations to score for one word.
///
/// 16 is four 多音字 in a row, which in a four-character 成語 is the worst case
/// that occurs. Past it the word falls back to 常用 readings rather than paying
/// for a combinatorial walk in a command that runs over a whole manuscript.
const MAX_COMBOS: usize = 16;

/// 宇浩's tables, asked about readings instead of about candidates.
///
/// Cloning the [`AnnotationTable`] is cheap — the 全息拆分表 behind it is an
/// [`Arc`] — and the 讀音表 is shared outright, so a reader costs a few pointers
/// rather than a second copy of either.
pub struct YumeReader {
    /// The 字料層: 讀音 and 字集 per character.
    annotations: AnnotationTable,
    /// The 讀音表: which spelling of a word the language actually uses.
    readings: Arc<FluencyTable>,
}

impl YumeReader {
    pub(crate) fn new(annotations: AnnotationTable, readings: Arc<FluencyTable>) -> Self {
        Self {
            annotations,
            readings,
        }
    }

    /// One character's readings as `(帶調, 音節)`, 常用 first, capped at
    /// [`TOP_READINGS`].
    ///
    /// Both spellings are kept because they answer different questions: the
    /// toned one is what goes above the character, the plain one is what the
    /// 讀音表's keys are written in (`nǚ` is `nv` there).
    fn readings_of(&self, ch: char) -> Vec<(String, String)> {
        let field = self.annotations.lookup(&ch.to_string()).reading;
        let mut out = Vec::new();
        for part in field.split(pinyin_tone::READING_SEP) {
            let toned = part.trim();
            if toned.is_empty() {
                continue;
            }
            let Some((plain, _tone)) = pinyin_tone::split_tone(toned) else {
                continue;
            };
            if out.iter().any(|(t, _): &(String, String)| t == toned) {
                continue;
            }
            out.push((toned.to_string(), plain));
            if out.len() == TOP_READINGS {
                break;
            }
        }
        out
    }
}

impl Reader for YumeReader {
    fn read(&self, word: &str) -> Option<Vec<String>> {
        let chars: Vec<char> = word.chars().collect();
        if chars.is_empty() || !chars.iter().copied().all(is_han) {
            return None;
        }
        let per_char: Vec<Vec<(String, String)>> =
            chars.iter().map(|&c| self.readings_of(c)).collect();
        // A word is read or it is not: one character the 拆分表 has never heard
        // of makes the whole reading a guess, and a guess written into the
        // manuscript is worse than no annotation at all.
        if per_char.iter().any(|r| r.is_empty()) {
            return None;
        }
        let common = || -> Vec<String> { per_char.iter().map(|r| r[0].0.clone()).collect() };
        let combos: usize = per_char.iter().map(|r| r.len()).product();
        if chars.len() == 1 || combos > MAX_COMBOS {
            return Some(common());
        }
        // Walk the product by counting in mixed radix — the readings are 常用
        // first, so index 0 across the board is the 常用 spelling and the walk
        // never allocates a table of its own.
        let mut best: Option<(f64, Vec<String>)> = None;
        for n in 0..combos {
            let mut rest = n;
            let mut toned = Vec::with_capacity(chars.len());
            let mut plain = Vec::with_capacity(chars.len());
            for r in &per_char {
                let (t, p) = &r[rest % r.len()];
                rest /= r.len();
                toned.push(t.clone());
                plain.push(p.clone());
            }
            let key = plain.join(" ");
            // `contains_pair` and not `spell_logprob` alone: the latter answers
            // `0.0` both for 「this spelling is certain」 and for 「no such
            // row」, and those must not be allowed to tie.
            if !self.readings.contains_pair(&key, word) {
                continue;
            }
            let score = self.readings.spell_logprob(&key, word);
            if best.as_ref().is_none_or(|(prev, _)| score > *prev) {
                best = Some((score, toned));
            }
        }
        Some(best.map(|(_, toned)| toned).unwrap_or_else(common))
    }

    fn is_rare(&self, ch: char) -> Option<bool> {
        if self.annotations.is_empty() {
            return None;
        }
        if !is_han(ch) {
            return Some(false);
        }
        // The 字集 column reads `簡繁古臺港-CJK`: the 字集 marks, then the
        // Unicode block.
        //
        // **Every standard in daily use counts, not 簡 alone.** 簡 is
        // 通用規範漢字表 — a list of *simplified* standard forms — so asking it
        // and nothing else answered 「生僻」 for 說, 為, 這, 裏, 學 and 國, which
        // is every second character of a 繁體 manuscript: the reading went over
        // half the page and `rare` became the mode it exists to avoid. A
        // character is ordinary if any of the four current standards carries
        // it. 古 is deliberately not among them — a character only 古籍 has is
        // exactly the one a reader stumbles on.
        const EVERYDAY: [char; 4] = ['簡', '繁', '臺', '港'];
        let field = self.charset(ch)?;
        let tags = field.split('-').next().unwrap_or("");
        Some(!EVERYDAY.iter().any(|t| tags.contains(*t)))
    }

    fn charset(&self, ch: char) -> Option<String> {
        if self.annotations.is_empty() {
            return None;
        }
        Some(self.annotations.lookup(&ch.to_string()).charset)
    }

    fn available(&self) -> bool {
        !self.annotations.is_empty()
    }
}
