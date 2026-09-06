//! 口頭禪 — the words this manuscript leans on (Feature #242).
//!
//! English writing calls these **crutch words**: the ones a writer leans on
//! when the sentence will not come. `:word habit` is the command.
//!
//! **Not a word count.** Counting words and sorting by the count says 的, 了,
//! 是, 我, and every manuscript in the language gives the same answer; a writer
//! learns nothing from being told that Chinese has particles. The question
//! worth asking is the other one: *which words does this chapter say far more
//! often than prose does* — 然後 forty-seven times, 突然 thirty-one, 有些
//! twenty-two. Those are the writer's own tics, and they are invisible from
//! inside the draft precisely because they read as natural.
//!
//! So a word is scored by **surprisal against a 詞頻表**, not by its count.
//! With `c` occurrences out of `n` words here, and `P(w)` the word's rate in
//! ordinary prose, the rate here is `c/n` and the word is
//!
//! **`n` counts only the words the table holds.** `P(w)` is normalised over
//! the table's own mass, so a `c/n` taken over *every* 漢字 token measures the
//! two rates in different universes and deflates every 倍 by the table's
//! coverage — the bundled 680-word list covers a quarter of a manuscript's
//! tokens, which was a factor of four on every line of the answer.
//!
//! ```text
//! 多 = (c/n) / P(w)          倍
//! 分 = c · ln(多)            nats, the whole document's surplus
//! ```
//!
//! The list is ranked by 分 rather than by 多, and the difference is what makes
//! it readable: a word said three times where prose would say it once is 3×
//! surprising and contributes almost nothing, while 然後 at 4× over forty-seven
//! occurrences is the thing the writer needs to see. This is the same
//! weighted-log-odds quantity a corpus linguist would call a keyness score, and
//! it is only sound in one direction — it finds what is *over*used, and says
//! nothing about what is missing.
//!
//! **Three deliberate silences.**
//!
//! - **A word the table does not hold is skipped**, never reported as
//!   infinitely surprising. 阿甯 is not a crutch word, it is a character's
//!   name; mining a book's own vocabulary is Feature #239 and a different
//!   question.
//! - **Single characters are skipped.** With a dictionary segmenter every
//!   uncounted 漢字 stands alone, so the one-character "words" are mostly the
//!   segmenter's leftovers rather than the writer's choices.
//! - **A word said once or twice is skipped** ([`MIN_COUNT`]). Three sightings
//!   is the least that can be called a habit.

use std::collections::HashMap;

use yumete_cjk::is_han;

/// How often a word must appear before it can be called a habit.
pub const MIN_COUNT: usize = 3;

/// How much more often than prose, before it is worth saying out loud.
///
/// Twice. Below that the number is inside the noise of any two texts — a
/// chapter of dialogue and a chapter of description differ by that much on
/// words neither writer chose.
pub const MIN_RATIO: f64 = 2.0;

/// One word the manuscript leans on.
#[derive(Debug, Clone, PartialEq)]
pub struct Habit {
    /// The word itself.
    pub word: String,
    /// Where it is first said — what `gf` on the listing row goes to.
    pub line: usize,
    /// How many times it is said here.
    pub count: usize,
    /// How many times more often than ordinary prose says it.
    pub ratio: f64,
    /// `count · ln(ratio)`, the whole document's surplus — the ranking.
    pub score: f64,
}

/// The words `text` leans on, heaviest first.
///
/// `segment` splits one line into character-index ranges (the editor's own
/// segmenter, handed a line at a time — the raw one, not the overlay's cached
/// `segment_line`: every line here is asked for exactly once, so a cache would
/// only grow a copy of the whole manuscript), and `log_prob` is the
/// background: `ln P(word)` in ordinary prose, or `None` for a word the table
/// does not hold.
pub fn habits(
    text: &str,
    segment: &dyn Fn(&str) -> Vec<(usize, usize)>,
    log_prob: &dyn Fn(&str) -> Option<f64>,
) -> Vec<Habit> {
    // (count, first line). The denominator counts the words **the table
    // holds**, one-character ones included: `P(w)` is a rate within that
    // vocabulary, so the rate it is compared against has to be one too. A
    // manuscript's names, its coined words and everything else the table has
    // never heard of are not part of either side of the ratio.
    let mut seen: HashMap<String, (usize, usize)> = HashMap::new();
    let mut total = 0usize;
    for (n, line) in text.lines().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        for (start, end) in segment(line) {
            let word: String = chars[start..end].iter().collect();
            if word.is_empty() || !word.chars().all(is_han) {
                continue;
            }
            if log_prob(&word).is_none() {
                continue;
            }
            total += 1;
            let entry = seen.entry(word).or_insert((0, n));
            entry.0 += 1;
        }
    }
    if total == 0 {
        return Vec::new();
    }
    let total = total as f64;
    let mut found: Vec<Habit> = seen
        .into_iter()
        .filter(|(word, (count, _))| *count >= MIN_COUNT && word.chars().count() > 1)
        .filter_map(|(word, (count, line))| {
            let background = log_prob(&word)?;
            let ln_ratio = (count as f64 / total).ln() - background;
            let ratio = ln_ratio.exp();
            (ratio >= MIN_RATIO).then_some(Habit {
                word,
                line,
                count,
                ratio,
                score: count as f64 * ln_ratio,
            })
        })
        .collect();
    // Score first; then the count, so two words with the same surplus are
    // listed with the one said more often on top; then the word itself, so the
    // listing is the same twice running.
    found.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.count.cmp(&a.count))
            .then(a.word.cmp(&b.word))
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every 漢字 its own word, which is what the fallback segmenter does —
    /// enough to test the arithmetic without a dictionary.
    fn by_pairs(line: &str) -> Vec<(usize, usize)> {
        // Split on anything that is not 漢字, and keep runs of two whole.
        let chars: Vec<char> = line.chars().collect();
        let mut out = Vec::new();
        let mut i = 0;
        while i < chars.len() {
            if !is_han(chars[i]) {
                i += 1;
                continue;
            }
            let start = i;
            while i < chars.len() && is_han(chars[i]) {
                i += 1;
            }
            let mut a = start;
            while a < i {
                let b = (a + 2).min(i);
                out.push((a, b));
                a = b;
            }
        }
        out
    }

    /// 然後 at one word in ten, against prose that says it one in a hundred.
    fn common(word: &str) -> Option<f64> {
        match word {
            "然後" => Some((0.01f64).ln()),
            "的話" => Some((0.10f64).ln()),
            _ => None,
        }
    }

    #[test]
    fn a_word_said_far_more_often_than_prose_says_it_is_reported() {
        let text = "然後好的然後好的然後好的然後好的";
        let found = habits(text, &by_pairs, &common);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].word, "然後");
        assert_eq!(found[0].count, 4);
        // Four of the four words the table holds — 好的 is not one of them —
        // against one in a hundred in prose: a hundred times.
        assert!((found[0].ratio - 100.0).abs() < 0.001, "{:?}", found[0].ratio);
    }

    #[test]
    fn what_the_table_never_heard_of_is_not_in_the_denominator() {
        // The same 然後, and then a page of a name the table does not hold.
        // `P(w)` is a rate within the table's vocabulary, so the answer must
        // not move — counting every 漢字 token halved it.
        let plain = habits("然後好的然後好的然後好的然後好的", &by_pairs, &common);
        let named = habits(
            "然後好的然後好的然後好的然後好的\n阿甯阿甯阿甯阿甯阿甯阿甯",
            &by_pairs,
            &common,
        );
        assert_eq!(plain.len(), 1, "{plain:?}");
        assert_eq!(named.len(), 1, "{named:?}");
        assert!((plain[0].ratio - named[0].ratio).abs() < 0.001, "{:?}", named[0].ratio);
    }

    #[test]
    fn a_word_prose_says_just_as_often_is_not_a_crutch() {
        // 的話 four times in eight words is 0.5, against a background of 0.1 —
        // five times, so it *is* reported; 然後 the same count is fifty times
        // and sorts above it.
        let text = "然後的話然後的話然後的話然後的話";
        let found = habits(text, &by_pairs, &common);
        assert_eq!(found.len(), 2, "{found:?}");
        assert_eq!(found[0].word, "然後");
        assert_eq!(found[1].word, "的話");
        assert!(found[0].score > found[1].score);
    }

    #[test]
    fn a_word_the_table_never_heard_of_is_left_alone() {
        // 阿甯 is a name, said as often as anything in the chapter, and has no
        // background: it is #239's question, not this one's.
        let text = "阿甯阿甯阿甯阿甯";
        assert!(habits(text, &by_pairs, &common).is_empty());
    }

    #[test]
    fn twice_is_not_yet_a_habit() {
        let text = "然後好的然後好的";
        assert!(habits(text, &by_pairs, &common).is_empty());
    }

    #[test]
    fn the_line_is_where_the_word_is_first_said() {
        let text = "好的好的\n好的好的\n然後然後然後";
        let found = habits(text, &by_pairs, &common);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].line, 2);
    }

    #[test]
    fn a_text_with_no_han_in_it_says_nothing() {
        assert!(habits("hello there\n", &by_pairs, &common).is_empty());
    }
}
