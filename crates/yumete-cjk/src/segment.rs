//! Word segmentation strategies (Feature #24).
//!
//! Word motions (`w` / `b` / `e`) and the optional segmentation overlay ask a
//! [`Segmenter`] to split a string into word ranges. Two strategies ship:
//!
//! - [`CategorySegmenter`] — the dependency-free default: alphanumeric runs,
//!   punctuation runs, and **each CJK ideograph or kana as its own word**. This
//!   is what `w` did before a dictionary existed.
//! - [`DictionarySegmenter`] — a jieba-style segmenter that groups runs of CJK
//!   characters into dictionary words along the maximum-probability path, so
//!   `w` steps over 中文 *words* rather than single characters.
//!
//! ## Why a hand-rolled jieba-style segmenter, not the `jieba-rs` crate
//!
//! `jieba-rs` is a solid, MIT-licensed, actively maintained crate and a good
//! reference. It is, however, not the right fit here: its value is an embedded
//! Simplified-Chinese dictionary plus an HMM model for out-of-vocabulary words,
//! which is exactly what yumete does *not* want — segmentation is meant to be
//! driven by Yume's own weight table with a configurable weight threshold (only
//! sufficiently common words are joined). With the default dictionary disabled
//! we would keep only its DAG + Viterbi core — a few dozen lines — while still
//! paying for heavy transitive dependencies (`regex`, `cedarwood`, a proc-macro
//! crate, `phf`) in a crate that otherwise depends on nothing. So we implement
//! the same DAG + maximum-probability algorithm directly over a plain
//! `word → weight` dictionary, keeping `yumete-cjk` dependency-light.
//!
//! The dictionary here is fed as plain `word → weight` entries; wiring Yume's
//! compiled weight table into it belongs to the IME milestone (Phase 2). Until
//! then the default [`CategorySegmenter`] is used, and a user may supply a
//! dictionary file to enable [`DictionarySegmenter`].

use std::collections::HashMap;

use crate::word::{category, is_cjk, word_ranges};

/// A compact everyday-prose Chinese word list, embedded so segmentation works
/// out of the box. See [`DictionarySegmenter::builtin`].
const BUILTIN_DICTIONARY: &str = include_str!("common_words.txt");

/// Splits a string into word ranges as character-index `(start, end)` pairs,
/// with whitespace skipped. `end` is exclusive.
pub trait Segmenter {
    /// Segment `s` into word ranges (character indices, whitespace skipped).
    fn segment(&self, s: &str) -> Vec<(usize, usize)>;

    /// How readily this segmenter joins characters into words (`:word level`).
    ///
    /// A default that does nothing, because a segmenter with no dictionary has
    /// no scale to be strict about: [`CategorySegmenter`] gives every 漢字 a
    /// word of its own whatever anyone asks for.
    fn set_level(&mut self, _level: WordLevel) {}

    /// Where these words come from, for the status line to name.
    fn source(&self) -> String {
        String::new()
    }

    /// `ln P(word)` in ordinary prose, from whatever 詞頻表 this segmenter
    /// reads — the background `:word habit` measures a manuscript against (#242).
    ///
    /// **`None` means「I have no table」, not「that word is rare」.** A
    /// segmenter with no dictionary cannot tell 然後 from 阿甯, and `:word habit`
    /// says so rather than reporting every proper noun in the chapter as a
    /// crutch. A word the table simply does not hold is also `None`: an
    /// unseen word has no background to be surprising against, and that
    /// question is #239's, not this one's.
    fn log_prob(&self, _word: &str) -> Option<f64> {
        None
    }
}

/// How readily a segmenter joins characters into words.
///
/// **One question asked of two different dictionaries.** The bundled list
/// answers it with a weight threshold (a rare word simply does not join); Yume's
/// language model answers it with a bias on the score of every multi-character
/// word, because there is no threshold in a maximum-probability path — the two
/// mechanisms differ, the reader's question does not.
///
/// It is about the **word motions and the overlay only**. Typing, candidates and
/// 整句 go through the IME engine, which never sees this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordLevel {
    /// Only words common enough to be beyond argument. 「山路」 stays two
    /// characters, which is what a proofreader stepping character by character
    /// actually wants.
    Strict,
    /// What the dictionary says, with no thumb on the scale.
    #[default]
    Balanced,
    /// Every word in the table joins, long and odd ones included — 「不由得」,
    /// 「一時之間」. Useful on 古文, where the long ones are real.
    Full,
}

impl WordLevel {
    /// Parse a `:word level` argument or a `word_level` config value.
    pub fn parse(value: &str) -> Option<WordLevel> {
        match value.trim().to_ascii_lowercase().as_str() {
            "strict" | "few" | "少" | "嚴" => Some(WordLevel::Strict),
            "balanced" | "normal" | "平衡" => Some(WordLevel::Balanced),
            "full" | "all" | "全" => Some(WordLevel::Full),
            _ => None,
        }
    }

    /// Its name, as the config writes it and the status line says it.
    pub fn name(self) -> &'static str {
        match self {
            WordLevel::Strict => "strict",
            WordLevel::Balanced => "balanced",
            WordLevel::Full => "full",
        }
    }

    /// The weight a word must reach to join, on the bundled list's own scale
    /// (where 我 is 60,000, the median entry 6,000, and a word a novel uses
    /// once a chapter about 1,000).
    ///
    /// `strict` keeps the top quarter — 時候, 什麼, 知道 — and leaves the rest
    /// as single characters. `balanced` and `full` differ only for the model
    /// (see [`WordLevel::split_bias`]): a threshold below the smallest weight
    /// admits everything the list holds, and the list holds nothing absurd.
    pub fn threshold(self) -> i64 {
        match self {
            WordLevel::Strict => 12_000,
            WordLevel::Balanced | WordLevel::Full => 0,
        }
    }

    /// What is added to the **per-word bonus** in a maximum-probability path,
    /// in log-probability units (nats).
    ///
    /// A bonus paid once per word favours *more* words, so a bigger one splits
    /// harder. The model's own bonus is 3 nats; these are the adjustment.
    ///
    /// **Measured, not guessed** — on the installed tables, over 「那年冬天他抬
    /// 頭看了看那片天，山路已經看不見了。」 (the probe is
    /// `yumete-ime/tests/real_data.rs::probe_bias_sensitivity`):
    ///
    /// ```text
    /// -1.5   8  那年冬天 他 抬頭 看了看 那片天 山路 已經 看不見了
    ///  0.0   9  那年冬天 他 抬頭 看了看 那片天 山路 已經 看不見 了
    /// +2.0  12  那 年 冬天 他 抬頭 看了看 那片 天 山路 已經 看不見 了
    /// +3.0  16  那 年 冬天 他 抬頭 看 了 看 那 片 天 山 路 已經 看不見 了
    /// ```
    ///
    /// +2.0 for `strict`: 那年冬天 comes apart and 山路 does not, which is 「more
    /// single characters」 without making the motion useless. +3 takes real
    /// words apart. −1.5 for `full`: the particles stay glued (看不見了 as one),
    /// and further down changes nothing — the plateau starts at about −0.5.
    pub fn split_bias(self) -> f64 {
        match self {
            WordLevel::Strict => 2.0,
            WordLevel::Balanced => 0.0,
            WordLevel::Full => -1.5,
        }
    }
}

/// The default, dictionary-free segmenter: alphanumeric runs, punctuation runs,
/// and each CJK ideograph/kana as its own word.
#[derive(Debug, Default, Clone, Copy)]
pub struct CategorySegmenter;

impl Segmenter for CategorySegmenter {
    fn segment(&self, s: &str) -> Vec<(usize, usize)> {
        word_ranges(s)
    }
}

/// A jieba-style dictionary segmenter over a `word → weight` table.
///
/// Runs of CJK characters are segmented along the maximum-probability path
/// through a word graph built from the dictionary; non-CJK text falls back to
/// the same category rules as [`CategorySegmenter`]. Only words whose weight is
/// at least `threshold` are eligible to be joined, so uncommon words stay split
/// into single characters — this is the "only very common words are segmented"
/// behaviour, tunable per the weight table.
#[derive(Debug, Clone)]
pub struct DictionarySegmenter {
    /// Word → weight (frequency-like). Single-character out-of-vocabulary words
    /// are always available with a floor weight of one.
    dict: HashMap<String, i64>,
    /// Sum of all dictionary weights, for log-probability normalization.
    total: f64,
    /// The longest dictionary word, in characters (bounds the DAG scan).
    max_len: usize,
    /// Minimum weight for a multi-character word to be joined.
    threshold: i64,
}

impl DictionarySegmenter {
    /// Build a segmenter from `word → weight` entries. Words below `threshold`
    /// are still stored but only joined when their weight qualifies; a single
    /// character is always eligible.
    pub fn new<I>(entries: I, threshold: i64) -> Self
    where
        I: IntoIterator<Item = (String, i64)>,
    {
        let mut dict: HashMap<String, i64> = HashMap::new();
        let mut max_len = 1;
        for (word, weight) in entries {
            let len = word.chars().count();
            if len == 0 {
                continue;
            }
            max_len = max_len.max(len);
            dict.insert(word, weight.max(1));
        }
        let total = dict.values().map(|&w| w as f64).sum::<f64>().max(1.0);
        DictionarySegmenter {
            dict,
            total,
            max_len,
            threshold,
        }
    }

    /// Build a segmenter from `word<TAB>weight` (or `word weight`) lines. Blank
    /// lines and lines beginning with `#` are skipped; a missing weight defaults
    /// to one.
    pub fn from_text(text: &str, threshold: i64) -> Self {
        let entries = text.lines().filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut parts = line.split(['\t', ' ']).filter(|s| !s.is_empty());
            let word = parts.next()?.to_string();
            let weight = parts
                .next()
                .and_then(|w| w.parse::<i64>().ok())
                .unwrap_or(1);
            Some((word, weight))
        });
        DictionarySegmenter::new(entries, threshold)
    }

    /// Build a segmenter from the compact common-word dictionary bundled with
    /// yumete, so word motions and the overlay work without any setup.
    pub fn builtin(threshold: i64) -> Self {
        DictionarySegmenter::from_text(BUILTIN_DICTIONARY, threshold)
    }

    /// The number of distinct words in the dictionary.
    pub fn word_count(&self) -> usize {
        self.dict.len()
    }

    /// Segment one run of CJK characters, returning local `(start, end)` char
    /// ranges via the maximum-probability path through the word graph.
    fn segment_cjk_run(&self, chars: &[char]) -> Vec<(usize, usize)> {
        let n = chars.len();
        if n == 0 {
            return Vec::new();
        }
        let log_total = self.total.ln();

        // route[i] = (best total log-probability from i to the end, chosen end).
        // A single character is always a candidate (floor weight one), so a path
        // always exists; multi-character words join only when common enough.
        let mut route = vec![(0.0f64, 0usize); n + 1];
        for i in (0..n).rev() {
            let mut best = f64::NEG_INFINITY;
            let mut best_end = i + 1;
            let mut frag = String::new();
            let max_j = (i + self.max_len).min(n);
            for (j, &ch) in chars.iter().enumerate().take(max_j).skip(i) {
                frag.push(ch);
                let end = j + 1;
                let is_single = end == i + 1;
                let weight = match self.dict.get(&frag).copied() {
                    // A stored word: single characters always qualify; longer
                    // words only when common enough to be joined.
                    Some(w) if is_single || w >= self.threshold => w as f64,
                    // An out-of-vocabulary single character: floor weight one.
                    _ if is_single => 1.0,
                    // A longer word that is unknown or too rare: not joinable.
                    _ => continue,
                };
                let score = weight.ln() - log_total + route[end].0;
                if score > best {
                    best = score;
                    best_end = end;
                }
            }
            route[i] = (best, best_end);
        }

        let mut ranges = Vec::new();
        let mut i = 0;
        while i < n {
            let end = route[i].1;
            ranges.push((i, end));
            i = end;
        }
        ranges
    }
}

impl Segmenter for DictionarySegmenter {
    fn set_level(&mut self, level: WordLevel) {
        self.threshold = level.threshold();
    }

    fn log_prob(&self, word: &str) -> Option<f64> {
        self.dict
            .get(word)
            .map(|&weight| (weight as f64 / self.total).ln())
    }

    fn source(&self) -> String {
        format!("{} 條（內置）", self.word_count())
    }

    fn segment(&self, s: &str) -> Vec<(usize, usize)> {
        let chars: Vec<char> = s.chars().collect();
        let mut ranges = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if c.is_whitespace() {
                i += 1;
                continue;
            }
            if is_cjk(c) {
                // Group a maximal run of CJK characters and segment it.
                let start = i;
                let mut j = i;
                while j < chars.len() && is_cjk(chars[j]) {
                    j += 1;
                }
                for (a, b) in self.segment_cjk_run(&chars[start..j]) {
                    ranges.push((start + a, start + b));
                }
                i = j;
                continue;
            }
            // Non-CJK: an alphanumeric or punctuation run (category rules).
            let cat = category(c);
            let start = i;
            i += 1;
            while i < chars.len()
                && !chars[i].is_whitespace()
                && !is_cjk(chars[i])
                && category(chars[i]) == cat
            {
                i += 1;
            }
            ranges.push((start, i));
        }
        ranges
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The bundled dictionary reads the script this editor is written in.**
    ///
    /// It was simplified-only, so `w` stepped one character at a time through
    /// every 繁體 manuscript — including the editor's own lesson, whose example
    /// sentence exists to show the feature working. The sentence below is that
    /// line, verbatim from `tutor.rs`.
    #[test]
    fn word_motions_find_words_in_traditional_prose() {
        let seg = DictionarySegmenter::builtin(0);
        let line = "他抬頭看了看那片天，雪還在下，山路已經看不見了。";
        // The ranges are in characters, the way every motion in the editor
        // counts them.
        let chars: Vec<char> = line.chars().collect();
        let words: Vec<String> = seg
            .segment(line)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect())
            .collect();
        for word in ["抬頭", "已經"] {
            assert!(words.contains(&word.to_string()), "{words:?} never joins {word}");
        }
    }

    /// The traditional forms are an addition, not a replacement: a writer with a
    /// simplified manuscript keeps every word they had.
    #[test]
    fn the_simplified_words_are_still_there() {
        let seg = DictionarySegmenter::builtin(0);
        let line = "他抬头看了看那片天，雪还在下，山路已经看不见了。";
        // The ranges are in characters, the way every motion in the editor
        // counts them.
        let chars: Vec<char> = line.chars().collect();
        let words: Vec<String> = seg
            .segment(line)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect())
            .collect();
        for word in ["抬头", "已经"] {
            assert!(words.contains(&word.to_string()), "{words:?} never joins {word}");
        }
    }


    #[test]
    fn category_segmenter_matches_word_ranges() {
        let seg = CategorySegmenter;
        // Same behaviour as word_ranges: latin run, punctuation, CJK singles.
        assert_eq!(
            seg.segment("hello, 世界 rust"),
            vec![(0, 5), (5, 6), (7, 8), (8, 9), (10, 14)]
        );
    }

    #[test]
    fn dictionary_joins_known_words() {
        // 世界 and 中文 are words; 界中 is not, so it splits between them.
        let seg =
            DictionarySegmenter::new([("世界".to_string(), 100), ("中文".to_string(), 100)], 1);
        // "世界中文" → 世界 | 中文
        assert_eq!(seg.segment("世界中文"), vec![(0, 2), (2, 4)]);
    }

    #[test]
    fn dictionary_respects_weights_at_a_fork() {
        // Overlapping words AB and BC compete over "ABC"; the heavier pairing
        // plus the leftover single character should win.
        // 甲乙 heavy → 甲乙 | 丙 ; 乙丙 heavy → 甲 | 乙丙.
        let heavy_left =
            DictionarySegmenter::new([("甲乙".to_string(), 1000), ("乙丙".to_string(), 1)], 1);
        assert_eq!(heavy_left.segment("甲乙丙"), vec![(0, 2), (2, 3)]);

        let heavy_right =
            DictionarySegmenter::new([("甲乙".to_string(), 1), ("乙丙".to_string(), 1000)], 1);
        assert_eq!(heavy_right.segment("甲乙丙"), vec![(0, 1), (1, 3)]);
    }

    #[test]
    fn threshold_keeps_rare_words_split() {
        // 世界 exists but is below the threshold, so it stays split into singles.
        let seg = DictionarySegmenter::new([("世界".to_string(), 5)], 100);
        assert_eq!(seg.segment("世界"), vec![(0, 1), (1, 2)]);
    }

    #[test]
    fn mixes_cjk_words_with_latin_and_punctuation() {
        let seg = DictionarySegmenter::new([("世界".to_string(), 100)], 1);
        // "hi 世界!" → "hi", 世界, "!"
        assert_eq!(seg.segment("hi 世界!"), vec![(0, 2), (3, 5), (5, 6)]);
    }

    #[test]
    fn from_text_parses_word_weight_lines() {
        let seg = DictionarySegmenter::from_text("# comment\n世界\t100\n中文 50\n", 1);
        assert_eq!(seg.word_count(), 2);
        assert_eq!(seg.segment("世界中文"), vec![(0, 2), (2, 4)]);
    }

    #[test]
    fn builtin_dictionary_segments_common_prose() {
        let seg = DictionarySegmenter::builtin(0);
        assert!(seg.word_count() > 100);
        // 你好 and 世界 are both in the bundled list.
        assert_eq!(seg.segment("你好世界"), vec![(0, 2), (2, 4)]);
        // A common sentence groups into words, not single characters.
        assert_eq!(seg.segment("我们今天"), vec![(0, 2), (2, 4)]);
    }
}

/// A segmenter with a project's own words layered over it — Feature #144.
///
/// The name on every page of a novel is the one word no dictionary has. 阿寧
/// segments as `[阿][寧]`, so `w` steps through it a character at a time, the
/// overlay tints it as two words, and every count of 字 that treats a word as a
/// unit is wrong about the character the book is *about*.
///
/// It works by **merging what the segmenter under it already decided** rather
/// than by joining another dictionary: whatever chose those boundaries — yume's
/// 1.25M-entry model, a `segmentation.txt`, the bundled list — keeps choosing
/// them, and a run of adjacent ranges that spells a project word becomes one.
/// Longest match wins, so 阿寧 beats 阿.
pub struct WithWords {
    inner: Box<dyn Segmenter>,
    words: std::rc::Rc<std::cell::RefCell<WordList>>,
}

/// The words a project has told the editor about.
#[derive(Debug, Default, Clone)]
pub struct WordList {
    words: std::collections::HashSet<String>,
    /// The longest word, in characters, so a merge never looks further.
    longest: usize,
}

impl WordList {
    /// Read one word per line; `#` opens a comment and blanks are skipped.
    pub fn from_text(text: &str) -> WordList {
        let mut list = WordList::default();
        for line in text.lines() {
            let word = line.split('#').next().unwrap_or("").trim();
            // A one-character "word" is what every segmenter already produces,
            // so listing one says nothing and cannot join anything.
            if word.chars().count() < 2 {
                continue;
            }
            list.longest = list.longest.max(word.chars().count());
            list.words.insert(word.to_string());
        }
        list
    }

    /// How many words are in force.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

impl WithWords {
    /// Layer `words` over `inner`. The list is shared, so reloading it reaches
    /// a segmenter that has already been handed out.
    pub fn new(
        inner: Box<dyn Segmenter>,
        words: std::rc::Rc<std::cell::RefCell<WordList>>,
    ) -> WithWords {
        WithWords { inner, words }
    }
}

impl Segmenter for WithWords {
    fn set_level(&mut self, level: WordLevel) {
        self.inner.set_level(level);
    }

    /// The dictionary's, never the book's own list: 阿甯 appearing four hundred
    /// times is what this book is *about*, and a name is not a crutch word.
    fn log_prob(&self, word: &str) -> Option<f64> {
        self.inner.log_prob(word)
    }

    /// The dictionary underneath, and this book's own words on top of it.
    fn source(&self) -> String {
        let inner = self.inner.source();
        match self.words.borrow().len() {
            0 => inner,
            n => format!("{inner} ＋ 本書 {n} 個詞"),
        }
    }

    fn segment(&self, s: &str) -> Vec<(usize, usize)> {
        let ranges = self.inner.segment(s);
        let list = self.words.borrow();
        if list.is_empty() || ranges.len() < 2 {
            return ranges;
        }
        let chars: Vec<char> = s.chars().collect();
        let mut out: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
        let mut i = 0;
        while i < ranges.len() {
            let (start, mut end) = ranges[i];
            let mut took = 1;
            // The longest run of *adjacent* ranges that spells a known word.
            // Adjacent, because a merge may not swallow the whitespace the
            // segmenter deliberately skipped.
            let mut j = i + 1;
            let mut reach = ranges[i].1;
            while j < ranges.len() && ranges[j].0 == reach {
                reach = ranges[j].1;
                if reach.saturating_sub(start) > list.longest {
                    break;
                }
                // Clamped at both ends: this trusts another segmenter's ranges,
                // and a slice whose start is past its end is a panic rather
                // than a wrong answer.
                let from = start.min(chars.len());
                let to = reach.clamp(from, chars.len());
                let word: String = chars[from..to].iter().collect();
                if list.words.contains(&word) {
                    end = reach;
                    took = j + 1 - i;
                }
                j += 1;
            }
            out.push((start, end));
            i += took;
        }
        out
    }
}
