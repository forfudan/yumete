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

use crate::word::word_ranges;

/// The bundled word list, put here at build time by `build.rs` when this
/// machine has one — **empty when it does not**.
///
/// It is a generated table, not source, so it is not in the repository: see
/// `build.rs` for where it comes from and why. An empty one is a working
/// build with no bundled dictionary, and [`DictionarySegmenter::builtin`]
/// then cuts one 漢字 at a time, exactly as it did before the list existed.
mod bundled {
    include!(concat!(env!("OUT_DIR"), "/words.rs"));
}
use bundled::BUILTIN_DICTIONARY;

/// Splits a string into word ranges as character-index `(start, end)` pairs,
/// with whitespace skipped. `end` is exclusive.
pub trait Segmenter {
    /// Segment `s` into word ranges (character indices, whitespace skipped).
    fn segment(&self, s: &str) -> Vec<(usize, usize)>;

    /// How readily this segmenter joins characters into words (`:word-level`).
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
    /// reads — the background `:word-habit` measures a manuscript against (#242).
    ///
    /// **`None` means「I have no table」, not「that word is rare」.** A
    /// segmenter with no dictionary cannot tell 然後 from 阿甯, and `:word-habit`
    /// says so rather than reporting every proper noun in the chapter as a
    /// crutch. A word the table simply does not hold is also `None`: an
    /// unseen word has no background to be surprising against, and that
    /// question is #239's, not this one's.
    fn log_prob(&self, _word: &str) -> Option<f64> {
        None
    }
}

/// How the overlay says where one word ended and the next began (#278).
///
/// **The same fact, drawn two ways.** A tint under the writing is the loudest
/// of the two and reads at a glance across a whole page; alternating the *ink*
/// instead leaves the paper alone, which matters to a writer who has the page
/// tinted for something else already — a `==highlight==`, a container, a
/// selection — and to anyone reading on a screen where a background band is
/// heavier than the characters standing on it.
///
/// Both alternate strictly per word, so an unmarked word is always between two
/// marked ones and says exactly as much as they do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WordMark {
    /// A hair of colour under every other word. The default.
    #[default]
    Tint,
    /// Every other word's characters in a second ink, the paper untouched.
    Ink,
}

impl WordMark {
    /// Parse a `:word-show` argument or a `word_mark` config value.
    pub fn parse(value: &str) -> Option<WordMark> {
        match value.trim().to_ascii_lowercase().as_str() {
            "tint" | "bg" | "background" | "底色" | "背景" => Some(WordMark::Tint),
            "ink" | "fg" | "colour" | "color" | "字色" | "文字" => Some(WordMark::Ink),
            _ => None,
        }
    }

    /// Its name, as the config writes it.
    pub fn name(self) -> &'static str {
        match self {
            WordMark::Tint => "tint",
            WordMark::Ink => "ink",
        }
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
    /// **No dictionary at all: a 漢字 is a letter.** 「我們都是apple」 is one
    /// word, the way helix reads it — and the way `e` always reads it, whatever
    /// this is set to (#304). For somebody who would rather `w` behaved the
    /// same in Chinese as it does in English, and for a file where the
    /// dictionary is guessing badly.
    Off,
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
    /// Parse a `:word-level` argument or a `word_level` config value.
    pub fn parse(value: &str) -> Option<WordLevel> {
        match value.trim().to_ascii_lowercase().as_str() {
            "strict" | "few" | "少" | "嚴" => Some(WordLevel::Strict),
            "off" | "none" | "關" | "关" => Some(WordLevel::Off),
            "balanced" | "normal" | "平衡" => Some(WordLevel::Balanced),
            "full" | "all" | "全" => Some(WordLevel::Full),
            _ => None,
        }
    }

    /// Its name, as the config writes it and the status line says it.
    pub fn name(self) -> &'static str {
        match self {
            WordLevel::Off => "off",
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
            // Unreachable in practice — `Off` never reaches a dictionary at
            // all — but a threshold nothing meets is the honest answer for a
            // level whose whole point is that no word ever joins.
            WordLevel::Off => i64::MAX,
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
            WordLevel::Off => 0.0,
            WordLevel::Strict => 2.0,
            WordLevel::Balanced => 0.0,
            WordLevel::Full => -1.5,
        }
    }
}

/// **No dictionary and no single-character rule: a 漢字 is a letter** (#304).
///
/// What [`WordLevel::Off`] gives `w`, and what `e` reads by whatever the level
/// says. Distinct from [`CategorySegmenter`], which is also dictionary-free but
/// puts every 漢字 in a word of its own — that is a third behaviour, and the
/// one nobody asked for.
#[derive(Debug, Default, Clone, Copy)]
pub struct CoarseSegmenter;

impl Segmenter for CoarseSegmenter {
    fn segment(&self, s: &str) -> Vec<(usize, usize)> {
        crate::word_ranges_coarse(s)
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

/// **The maximum-probability split of one run of 漢字** — the one
/// implementation (#349).
///
/// Two of them had been written: this one, over a `word → weight` table, and
/// [`YumeSegmenter`](../../yumete_ime/segment/struct.YumeSegmenter.html)'s,
/// over 宇浩's 1.25M-entry 詞頻表. The same dynamic program both times — solve
/// from the right, so every position only reads answers it already has — and
/// only the scoring differed, which is the one thing a caller supplies.
///
/// * `score(word, len)` is `ln P(word)` plus whatever bias the caller wants,
///   or `None` when `word` is not a word here. **It must answer `Some` for a
///   single character**: that is what guarantees a path exists at all. A
///   caller that does not gets the character taken alone, charged nothing.
/// * `extends(word)` says whether any longer word starts with this prefix, so
///   a table that can answer it cheaply need not probe up to `max_len`. A
///   table that cannot says `true`.
pub fn best_path(
    chars: &[char],
    max_len: usize,
    mut score: impl FnMut(&str, usize) -> Option<f64>,
    mut extends: impl FnMut(&str) -> bool,
) -> Vec<(usize, usize)> {
    let n = chars.len();
    if n == 0 {
        return Vec::new();
    }
    // `route[i]` = (the best total score for `chars[i..]`, where its first word
    // ends). `route[n]` is the empty tail, worth nothing.
    let mut route = vec![(0.0f64, 0usize); n + 1];
    let mut word = String::with_capacity(max_len * 4);
    for i in (0..n).rev() {
        let mut best = f64::NEG_INFINITY;
        let mut best_end = i + 1;
        word.clear();
        for len in 1..=max_len.min(n - i) {
            word.push(chars[i + len - 1]);
            if let Some(here) = score(&word, len) {
                let total = here + route[i + len].0;
                if total > best {
                    best = total;
                    best_end = i + len;
                }
            }
            if !extends(&word) {
                break;
            }
        }
        if best == f64::NEG_INFINITY {
            best = route[i + 1].0;
            best_end = i + 1;
        }
        route[i] = (best, best_end);
    }
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < n {
        let end = route[i].1.max(i + 1);
        ranges.push((i, end));
        i = end;
    }
    ranges
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

    /// Build a segmenter from the word list bundled with this binary, so word
    /// motions and the overlay work without any setup.
    ///
    /// ⚠️ **The list may be empty** — it is a build-time input, not a tracked
    /// file (see `build.rs`). Then every word is one 漢字 wide, which is
    /// honest and is what this did before the list existed. Ask
    /// [`Self::word_count`] before writing a test that needs real words.
    pub fn builtin(threshold: i64) -> Self {
        DictionarySegmenter::from_text(BUILTIN_DICTIONARY, threshold)
    }

    /// Whether this build carries a bundled word list at all.
    pub fn has_builtin() -> bool {
        !BUILTIN_DICTIONARY.trim().is_empty()
    }

    /// The number of distinct words in the dictionary.
    pub fn word_count(&self) -> usize {
        self.dict.len()
    }

    /// Segment one run of CJK characters, returning local `(start, end)` char
    /// ranges via the maximum-probability path through the word graph.
    ///
    /// The path is [`best_path`]'s; what belongs here is the scoring — a
    /// stored word's weight against the table's total, single characters
    /// always eligible (floor weight one, so a path always exists) and longer
    /// words only when common enough to be joined.
    fn segment_cjk_run(&self, chars: &[char]) -> Vec<(usize, usize)> {
        let log_total = self.total.ln();
        best_path(
            chars,
            self.max_len,
            |word, len| {
                let weight = match self.dict.get(word).copied() {
                    Some(w) if len == 1 || w >= self.threshold => w as f64,
                    _ if len == 1 => 1.0,
                    // Unknown or too rare to join — but a longer word may still
                    // start here, so this is not the end of the extending.
                    _ => return None,
                };
                Some(weight.ln() - log_total)
            },
            // The table is a plain `word → weight` map with no prefix index, so
            // it cannot say; probing up to `max_len` is the price of that.
            |_| true,
        )
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
        crate::ranges_around_cjk(s, |run| self.segment_cjk_run(run))
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
        // The list is a build input, not a tracked file (`build.rs`), so a
        // machine that has never installed 宇浩 has none — and「every word is
        // one 漢字」is the right answer there, not a failure.
        if !DictionarySegmenter::has_builtin() {
            return;
        }
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
        // The list is a build input, not a tracked file (`build.rs`), so a
        // machine that has never installed 宇浩 has none — and「every word is
        // one 漢字」is the right answer there, not a failure.
        if !DictionarySegmenter::has_builtin() {
            return;
        }
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

    /// #349. A dictionary decides where a *word* ends inside a run of 漢字 and
    /// nothing else: everything outside such a run is cut the same way by every
    /// segmenter here, because they all walk the line through one function.
    #[test]
    fn every_segmenter_cuts_the_same_way_outside_a_run_of_han() {
        let line = "冬天 abc123，山路。「好」\t二〇二五年";
        let chars: Vec<char> = line.chars().collect();
        let outside = |ranges: Vec<(usize, usize)>| -> Vec<(usize, usize)> {
            ranges
                .into_iter()
                .filter(|&(a, _)| !crate::is_han(chars[a]))
                .collect()
        };
        let plain = CategorySegmenter.segment(line);
        for other in [
            DictionarySegmenter::builtin(0).segment(line),
            DictionarySegmenter::new([("山路".to_string(), 900)], 1).segment(line),
        ] {
            assert_eq!(outside(other), outside(plain.clone()));
        }
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
        // The list is a build input, not a tracked file (`build.rs`), so a
        // machine that has never installed 宇浩 has none — and「every word is
        // one 漢字」is the right answer there, not a failure.
        if !DictionarySegmenter::has_builtin() {
            return;
        }
        let seg = DictionarySegmenter::builtin(0);
        assert!(seg.word_count() > 100);
        // 你好 and 世界 are both in the bundled list.
        assert_eq!(seg.segment("你好世界"), vec![(0, 2), (2, 4)]);
        // Two words in a row cut apart. ⚠️ Pick a pair the corpus does *not*
        // also list as one four-character entry — 「我们今天」 is such an entry
        // (7,395), so the maximum-probability path joins it, correctly.
        assert_eq!(seg.segment("冬天早晨"), vec![(0, 2), (2, 4)]);
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
            list.add(line.split('#').next().unwrap_or("").trim());
        }
        list
    }

    /// Add one word to the list in force, as `from_text` would read it.
    ///
    /// This is how a word starts working **before** it is on disk: `:word
    /// discover` writes its candidates into an unsaved buffer for the writer
    /// to weed, and adds them here at the same time, so `w` walks 落霞鎮 in
    /// one step while that review is going on. Saving the buffer re-reads the
    /// file, and whatever was struck out of it stops counting then.
    pub fn add(&mut self, word: &str) {
        // A one-character "word" is what every segmenter already produces, so
        // listing one says nothing and cannot join anything.
        if word.chars().count() < 2 {
            return;
        }
        self.longest = self.longest.max(word.chars().count());
        self.words.insert(word.to_string());
    }

    /// How many words are in force.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Take in everything `other` holds.
    ///
    /// The segmenter is handed **one** list, and there are two sources for it:
    /// the writer's own `.yumete/words.txt` and what autodetect found in the
    /// manuscript (#448). They are kept apart at the editor — one is a file the
    /// writer owns, the other is a cache — and joined here.
    pub fn merge(&mut self, other: &WordList) {
        self.longest = self.longest.max(other.longest);
        self.words.extend(other.words.iter().cloned());
    }

    /// The words, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        self.words.iter().map(String::as_str)
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

/// How many lines' answers a [`Memo`] holds before it starts again.
const MEMO_LINES: usize = 512;

/// How much a [`Memo`] holds before it starts again, in bytes.
///
/// A count of lines is not a bound on a Chinese novel: one paragraph can be a
/// whole chapter, and 512 of those are not 512 short lines.
const MEMO_BYTES: usize = 1 << 20;

/// The answers a [`Memo`] is holding, shared with whoever installed it.
#[derive(Debug, Default)]
pub struct SegmentMemo {
    answers: HashMap<String, Vec<(usize, usize)>>,
    /// What they take, so [`MEMO_BYTES`] can be enforced without a walk.
    bytes: usize,
}

impl SegmentMemo {
    /// Forget every answer — the dictionary, the level or the book's own word
    /// list has changed, and none of that changed the text they are kept
    /// against.
    pub fn clear(&mut self) {
        self.answers.clear();
        self.bytes = 0;
    }

    /// How many lines' answers are being held.
    pub fn len(&self) -> usize {
        self.answers.len()
    }

    /// Whether none are.
    pub fn is_empty(&self) -> bool {
        self.answers.is_empty()
    }
}

/// A segmenter that remembers the lines it has already cut — #321.
///
/// `w` asks for the word boundaries of the line it stands on, and used to get
/// them by cutting that line from scratch every time: a copy of the text, a
/// Viterbi pass over each run of 漢字 in it, then a linear scan for the first
/// boundary past the cursor. Held down, `w` paid for all of it again on every
/// repeat — fifty presses along one two-thousand-segment line of mixed 中英
/// cost 11.9 ms — and not one of those answers had changed since the first.
///
/// The answers are kept against **the text that was cut**, not against a line
/// number or a buffer revision. Ranges are relative to the string, so a key
/// that matches is a correct answer however much of the document moved, and
/// the paragraph being typed into is the only one whose answer is thrown away.
/// The key is that text itself rather than a hash of it: a collision here
/// would hand `w` another paragraph's boundaries, and holding a few hundred
/// short strings is cheaper than that.
///
/// ⚠️ **What a line cuts into depends on more than the line.** The dictionary
/// in force, the [`WordLevel`], and the book's own word list all change the
/// answer without changing a character of the text.
/// [`set_level`](Memo::set_level) throws away what it holds; the other two are
/// the reason [`new`](Memo::new) takes the store by handle rather than making
/// one — whoever swaps the dictionary or reloads `words.txt` clears it.
pub struct Memo {
    inner: Box<dyn Segmenter>,
    kept: std::rc::Rc<std::cell::RefCell<SegmentMemo>>,
}

impl Memo {
    /// Remember what `inner` cuts, in `kept`. The store is shared so that a
    /// change nothing in the text shows can reach it.
    pub fn new(
        inner: Box<dyn Segmenter>,
        kept: std::rc::Rc<std::cell::RefCell<SegmentMemo>>,
    ) -> Memo {
        Memo { inner, kept }
    }
}

impl Segmenter for Memo {
    fn segment(&self, s: &str) -> Vec<(usize, usize)> {
        if let Some(ranges) = self.kept.borrow().answers.get(s).cloned() {
            return ranges;
        }
        let ranges = self.inner.segment(s);
        let mut kept = self.kept.borrow_mut();
        // Cleared rather than evicted oldest-first: keeping an order costs
        // something on every hit, and what a clear costs is one Viterbi pass
        // per line still on screen, once.
        if kept.len() >= MEMO_LINES || kept.bytes >= MEMO_BYTES {
            kept.clear();
        }
        kept.bytes += s.len() + ranges.len() * std::mem::size_of::<(usize, usize)>();
        kept.answers.insert(s.to_string(), ranges.clone());
        ranges
    }

    /// The level is one of the things the text does not show, so the answers
    /// held against it go.
    fn set_level(&mut self, level: WordLevel) {
        self.inner.set_level(level);
        self.kept.borrow_mut().clear();
    }

    fn log_prob(&self, word: &str) -> Option<f64> {
        self.inner.log_prob(word)
    }

    fn source(&self) -> String {
        self.inner.source()
    }
}
