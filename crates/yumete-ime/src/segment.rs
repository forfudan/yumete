//! Word segmentation over Yume's language model — Feature #63.
//!
//! `w`, `b` and `e` step by *word*, and in Chinese that needs a dictionary: the
//! text carries no spaces, so where one word ends is a judgement about the
//! language, not something the characters say. yumete shipped a 214-entry list
//! for this, which covers almost no real prose — with it, `w` walks one 漢字 at
//! a time.
//!
//! Yume already owns the answer. Its 詞頻表 (`lang.ywtb`, 1.25M weighted
//! entries) and 詞彙表 (`lang.ywl`, the words it knows but has never counted)
//! are the language layer its 整句 composition ranks with; the same two tables
//! segment existing text just as well, and are already loaded and shared by
//! [`ImeSession`](crate::ImeSession).
//!
//! The method is the standard maximum-probability path: build every candidate
//! word starting at every position, score each by `ln P(word)`, and take the
//! split whose total is highest. It is the same shape as the jieba-style
//! segmenter in `yumete-cjk` — what changes is that the dictionary is now a real
//! one.

use std::sync::Arc;

use yume_core::lexicon::Lexicon;
use yume_core::UnigramTable;
use yumete_cjk::Segmenter;

/// The longest word the segmenter will consider, in characters.
///
/// Chinese words past four characters are overwhelmingly idioms and names, and
/// every extra length multiplies the table probes per position. Eight leaves
/// room for 成語 and four-character titles without paying for the tail.
const MAX_WORD: usize = 8;

/// The score for text neither table has counted: `ln P` as if it had been seen
/// **once**, i.e. `ln 1 − ln Σw`.
///
/// Deriving it from the table rather than picking a constant is what keeps the
/// scale honest — `ln P` depends entirely on how big the corpus is, so any fixed
/// number would be right for one table and nonsense for the next. This is
/// jieba's rule, and it gives an uncounted *word* a large edge over the same
/// span of uncounted characters for free: the word pays the floor once, the
/// characters pay it each.
fn floor_score(unigram: &UnigramTable) -> f64 {
    -unigram.ln_total()
}

/// A flat bonus added to every word, biasing the split toward *more, shorter*
/// words.
///
/// Yume's 詞頻表 counts phrases as well as words — 「坐在窗前」 and 「我們的」
/// are in it with real frequencies — so an unbiased maximum-probability split
/// happily swallows a whole phrase, and `w` then jumps further than a writer
/// means by "next word". Rewarding each boundary pulls those apart again
/// without discarding the phrase entries: a phrase still wins when it is much
/// likelier than its parts.
///
/// 3 nats is where 「我們的」 splits into 我們 ／ 的 and 「正在改變」 into
/// 正在 ／ 改變, while 那年冬天, 人工智能 and 生活方式 stay whole. Below about 2
/// the particles stay glued on; above about 4 real words start coming apart
/// (那 ／ 年, 三 ／ 朵).
const WORD_BONUS: f64 = 3.0;

/// A [`Segmenter`] backed by Yume's language model.
pub struct YumeSegmenter {
    unigram: Arc<UnigramTable>,
    lexicon: Arc<Lexicon>,
    /// `:word-level`, as an adjustment to [`WORD_BONUS`] — see
    /// [`yumete_cjk::WordLevel::split_bias`].
    ///
    /// **The tables are shared and never touched.** This lives on the
    /// segmenter, which only the editor's word motions and the segmentation
    /// overlay use; typing, candidates and 整句 go through the engine and
    /// cannot see it.
    bias: f64,
}

impl YumeSegmenter {
    /// Build a segmenter over the two language-layer tables.
    pub fn new(unigram: Arc<UnigramTable>, lexicon: Arc<Lexicon>) -> Self {
        YumeSegmenter {
            unigram,
            lexicon,
            bias: 0.0,
        }
    }

    /// A probe for tuning the three levels; not part of the editor's path.
    #[doc(hidden)]
    pub fn set_bias_for_probe(&mut self, bias: f64) {
        self.bias = bias;
    }

    /// Whether either table holds enough to segment with.
    pub fn is_available(&self) -> bool {
        !self.unigram.is_empty() || !self.lexicon.is_empty()
    }

    /// `ln P(word)`, or `None` when neither table knows it.
    fn score(&self, word: &str, chars: usize, floor: f64) -> Option<f64> {
        if let Some(p) = self.unigram.log_prob(word) {
            return Some(p);
        }
        // Known to exist but never counted: charge it the once-seen floor.
        if chars > 1 && self.lexicon.contains(word) {
            return Some(floor);
        }
        None
    }

    /// Segment one run of CJK characters into local `(start, end)` char ranges
    /// along the maximum-probability path.
    ///
    /// The path is [`yumete_cjk::best_path`]'s — the same dynamic program the
    /// bundled dictionary walks, written once (#349). What belongs here is the
    /// scoring: two tables, a floor for what neither has counted, and the
    /// bonus that keeps a phrase from swallowing its own words.
    fn segment_run(&self, chars: &[char]) -> Vec<(usize, usize)> {
        let floor = floor_score(&self.unigram);
        yumete_cjk::best_path(
            chars,
            MAX_WORD,
            |word, len| {
                let score = match self.score(word, len, floor) {
                    Some(score) => score,
                    // Not a word; but a longer one may still start here, so
                    // this is not the end of the extending.
                    None if len > 1 => return None,
                    None => floor,
                };
                Some(score + WORD_BONUS + self.bias)
            },
            // Both tables carry a prefix index, so「nothing continues this」is
            // cheap to ask and saves the probes up to [`MAX_WORD`].
            |word| self.unigram.has_extension(word) || self.lexicon.has_extension(word),
        )
    }
}

impl Segmenter for YumeSegmenter {
    fn set_level(&mut self, level: yumete_cjk::WordLevel) {
        self.bias = level.split_bias();
    }

    /// The 詞頻表's own answer. The lexicon is deliberately not consulted: a
    /// word 宇浩 knows but never counted has no rate to be measured against,
    /// and charging it the once-seen floor the way [`Self::score`] does would
    /// make every uncounted word look like the most surprising thing in the
    /// chapter.
    fn log_prob(&self, word: &str) -> Option<f64> {
        self.unigram.log_prob(word)
    }

    fn source(&self) -> yumete_cjk::WordSource {
        yumete_cjk::WordSource {
            entries: self.unigram.count().max(self.lexicon.count()),
            book: 0,
            yume: true,
        }
    }

    /// Non-CJK stretches keep the category rules; only the runs of 漢字 and
    /// kana need the dictionary. Which is which is
    /// [`yumete_cjk::ranges_around_cjk`]'s to say, not this file's (#349):
    /// the copy that used to live here dropped every 標點 instead of giving it
    /// a range, so `w` stopped before 「 with the bundled dictionary loaded
    /// and stepped straight over it with 宇浩's.
    fn segment(&self, s: &str) -> Vec<(usize, usize)> {
        yumete_cjk::ranges_around_cjk(s, |run| self.segment_run(run))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segmenter(weights: &[(&str, u32)], lexicon: &[&str]) -> YumeSegmenter {
        let mut unigram = UnigramTable::new();
        unigram.load_text(
            &weights
                .iter()
                .map(|(w, n)| format!("{w}\t{n}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let mut lex = Lexicon::new();
        lex.build(lexicon.iter().map(|w| w.to_string()));
        YumeSegmenter::new(Arc::new(unigram), Arc::new(lex))
    }

    fn words(seg: &YumeSegmenter, text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        seg.segment(text)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect())
            .collect()
    }

    #[test]
    fn joins_the_words_the_table_knows() {
        let seg = segmenter(
            &[
                ("冬天", 5000),
                ("往常", 3000),
                ("那年", 2000),
                ("天", 90000),
            ],
            &[],
        );
        assert_eq!(
            words(&seg, "那年冬天"),
            ["那年", "冬天"],
            "a counted two-character word must beat two single characters"
        );
    }

    #[test]
    fn prefers_the_higher_probability_split() {
        // 「日文」 and 「文法」 overlap; the likelier pair wins the whole run.
        let seg = segmenter(&[("日文", 100), ("文法", 9000), ("日", 500)], &[]);
        assert_eq!(words(&seg, "日文法"), ["日", "文法"]);
    }

    #[test]
    fn a_lexicon_word_beats_splitting_into_uncounted_characters() {
        // 「老梅」 is known to be a word but has never been counted, and neither
        // character has either: the word pays the once-seen floor once, the two
        // characters would pay it twice.
        let seg = segmenter(&[("冬天", 5000)], &["老梅"]);
        assert_eq!(words(&seg, "老梅"), ["老梅"]);
    }

    /// Against *common* characters an uncounted word can still lose, and that is
    /// the model being honest rather than a bug — a real 詞頻表 counts the words
    /// that matter, so this only decides the tail.
    #[test]
    fn a_counted_word_outranks_an_uncounted_one() {
        let seg = segmenter(&[("老梅樹", 800), ("老", 50), ("梅樹", 900)], &["老梅"]);
        assert_eq!(words(&seg, "老梅樹"), ["老梅樹"]);
    }

    #[test]
    fn unknown_characters_stand_alone() {
        let seg = segmenter(&[("冬天", 5000)], &[]);
        assert_eq!(words(&seg, "㸚㹴冬天"), ["㸚", "㹴", "冬天"]);
    }

    /// The 詞頻表 counts phrases too, so without a boundary bias a trailing
    /// particle gets swallowed into the word before it.
    #[test]
    fn a_trailing_particle_is_split_off() {
        let seg = segmenter(&[("我們的", 900), ("我們", 4000), ("的", 90000)], &[]);
        assert_eq!(words(&seg, "我們的"), ["我們", "的"]);
    }

    /// #349. Which dictionary is loaded decides where a *word* ends and
    /// nothing else — 標點 is a range under both, or `w` would stop before 「
    /// with the bundled list and step over it with 宇浩's.
    #[test]
    fn latin_and_punctuation_keep_the_rules_every_segmenter_keeps() {
        let line = "冬天 abc123，冬天。「好」";
        let seg = segmenter(&[("冬天", 5000)], &[]);
        assert_eq!(
            words(&seg, line),
            ["冬天", "abc123", "，", "冬天", "。「", "好", "」"]
        );
        // The same ranges, boundaries inside a run of 漢字 aside.
        let plain = yumete_cjk::DictionarySegmenter::builtin(0);
        let outside = |ranges: Vec<(usize, usize)>| -> Vec<(usize, usize)> {
            let chars: Vec<char> = line.chars().collect();
            ranges
                .into_iter()
                .filter(|&(a, _)| !yumete_cjk::is_han(chars[a]))
                .collect()
        };
        assert_eq!(
            outside(yumete_cjk::Segmenter::segment(&seg, line)),
            outside(yumete_cjk::Segmenter::segment(&plain, line))
        );
    }

    #[test]
    fn an_empty_model_still_segments() {
        let seg = segmenter(&[], &[]);
        assert!(!seg.is_available());
        assert_eq!(words(&seg, "冬天"), ["冬", "天"]);
    }
}
