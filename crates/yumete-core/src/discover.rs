//! 本書自己的詞 — the words no dictionary has (Feature #239).
//!
//! **The one word a manuscript needs most is the one no table holds.** 阿寧 is
//! on every page and in no 詞頻表, so the segmenter splits it as `[阿][寧]`: `w`
//! steps through the name a character at a time, the tint draws it as two
//! words, `:word-habit` cannot weigh it, and the IME never offers it. `.yumete/
//! words.txt` fixes all of that — [`yumete_cjk::WithWords`] merges whatever is
//! listed there — but only for the names somebody remembered to type in, and a
//! novel with forty characters, three sects and a province of invented
//! place-names is exactly the case where nobody remembers.
//!
//! So the book is asked instead. This is 新詞發現, the same three signals
//! OpenCC's `PhraseExtract` uses, computed over one manuscript rather than a
//! corpus:
//!
//! ```text
//! logP(w)    = ln 次數(w) − ln 總字數
//! 內聚度(w)  = min over 每一種二分切法 of  logP(w) − logP(左) − logP(右)
//! 左熵 / 右熵 = −Σ p ln p  over 緊挨着它的那一個字
//! ```
//!
//! 內聚度 asks 「do these characters occur together far more often than two
//! independent characters would?」 — that is what makes 阿寧 a word and 的時 not.
//! The two entropies ask the other half: 「does it appear in *varied* company?」
//! A real word turns up after anything and before anything; a fragment such as
//! 阿寧說 has one habitual neighbour and little else. Both are needed, and
//! neither alone is worth much.
//!
//! **Only what the segmenter cannot already do.** Every candidate is offered to
//! the segmenter first, and one it already joins is dropped without a word.
//! That single test does most of the filtering, and it is why the thresholds
//! here can be gentle: 說道 and 突然 never reach them.
//!
//! **Two prunes for nested candidates**, because cohesion alone cannot tell a
//! name from a name with a verb stuck to it:
//!
//! - 阿寧說 is dropped because 阿寧 occurs [`CROWDED_OUT`]× more often — the
//!   shorter string is the word, and the longer one is that word plus context.
//! - 王語 is dropped because 王語嫣 occurs nearly as often ([`ABSORBED`]) — the
//!   shorter string barely exists outside the longer one, so it is a fragment.
//!
//! **What this deliberately is not.** It does not read frequencies from
//! anywhere, so it cannot say whether a word is *common in the language* — that
//! is [`crate::words`], and it needs a table. And it mines only 漢字 runs:
//! a coined English word or a romanised name is somebody else's problem.

use std::collections::HashMap;

use yumete_cjk::is_han;

/// How often a string must occur before it can be argued about.
///
/// Five. The three signals are all ratios of counts, and a count of two makes
/// every one of them meaningless — two occurrences of anything look perfectly
/// cohesive and perfectly varied.
pub const MIN_COUNT: usize = 5;

/// The longest candidate, in characters.
///
/// Four. Chinese words longer than that are 成語 and phrases, which a word list
/// does not need: 突然之間 segments correctly as 突然 ＋ 之間 already.
pub const MAX_LEN: usize = 4;

/// How much more often than chance, in nats, before it looks like one word.
///
/// `e³ ≈ 20`: the two halves must occur together twenty times more often than
/// two unrelated strings of those frequencies would.
pub const MIN_COHESION: f64 = 3.0;

/// How varied its company must be, in nats, on **both** sides.
///
/// `0.6` is roughly two neighbours used evenly. Below that the string has one
/// habitual context, which is what a fragment looks like.
pub const MIN_ENTROPY: f64 = 0.6;

/// A longer candidate is dropped when a substring occurs this many times more.
pub const CROWDED_OUT: usize = 2;

/// A shorter candidate is dropped when a longer one holds this share of it.
pub const ABSORBED: f64 = 0.9;

/// One word the book uses and no dictionary has.
#[derive(Debug, Clone, PartialEq)]
pub struct Found {
    /// The word itself.
    pub word: String,
    /// How many times the manuscript says it.
    pub count: usize,
    /// 內聚度, in nats — how much more than chance its parts stick together.
    pub cohesion: f64,
    /// The smaller of 左熵 and 右熵, in nats — how varied its company is.
    pub entropy: f64,
}

/// The end of a 漢字 run, counted as a neighbour in its own right.
///
/// A name that always opens a line of dialogue has nothing to its left, and
/// treating that as 「no evidence」 would give it 左熵 = 0 and throw it away.
/// The edge *is* a context, so it is counted as one.
const EDGE: char = '\0';

/// How many 漢字 a text holds — the denominator [`cap`] is measured against.
///
/// Only 漢字: a repository of Markdown is mostly punctuation, code fences and
/// ASCII, and none of that can ever become a word here.
pub fn han_count(text: &str) -> usize {
    text.chars().filter(|&c| is_han(c)).count()
}

/// How many of the words found are worth keeping, for a text of `han` 漢字.
///
/// **A share, not a number** (#455). Two hundred is right for a chapter and
/// wrong for a book: 「一个一百万字的小说可以取到 1000 个词（比如说名字）」,
/// and a cast of four hundred does not fit in two hundred slots. One in a
/// thousand characters, with two hundred as the floor so a short file is not
/// punished for being short:
///
/// | 讀了多少 | 留幾個 |
/// | --- | --- |
/// | 一章（2 萬字） | 200 |
/// | 一部中篇（12 萬） | 200 |
/// | 一部長篇（100 萬） | 1,000 |
/// | 笑傲江湖（145 萬） | 1,450 |
///
/// The tail is cheap — the list is a `HashSet` the segmenter consults — and
/// everything in it has already passed 內聚度, 左右熵 and [`MIN_COUNT`]. This
/// bound is a safety valve on the *length of the list*, not a judgement about
/// the words in it.
pub fn cap(han: usize) -> usize {
    (han / 1000).max(200)
}

/// Words `text` uses that `joins` does not already treat as one word, commonest
/// first.
///
/// `joins(w)` is the segmenter: true when handing it `w` alone comes back as a
/// single range. Anything it says yes to is not news, and is never returned.
pub fn words(text: &str, joins: &dyn Fn(&str) -> bool) -> Vec<Found> {
    let chars: Vec<char> = text.chars().collect();
    // Only 漢字, and only unbroken runs of it: a word never spans the comma or
    // the line break that ends a run, so counting across one would invent
    // neighbours that are not there.
    let mut runs: Vec<(usize, usize)> = Vec::new();
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
        runs.push((start, i));
    }
    let total: usize = runs.iter().map(|(a, b)| b - a).sum();
    if total < MIN_COUNT {
        return Vec::new();
    }

    // Apriori by length, not by suffix array: `counts[n - 1]` holds the n-grams
    // worth keeping, and length n + 1 is only counted where its n-character
    // prefix survived. Every substring of a frequent string is at least as
    // frequent, so nothing that matters is lost — and the pass that would
    // otherwise hold every 4-gram in the book never happens.
    let mut counts: Vec<HashMap<&[char], usize>> = Vec::new();
    for n in 1..=MAX_LEN {
        let mut pass: HashMap<&[char], usize> = HashMap::new();
        for &(a, b) in &runs {
            if b - a < n {
                continue;
            }
            for start in a..=(b - n) {
                if n > 1 && !counts[n - 2].contains_key(&&chars[start..start + n - 1]) {
                    continue;
                }
                *pass.entry(&chars[start..start + n]).or_insert(0) += 1;
            }
        }
        pass.retain(|_, c| *c >= MIN_COUNT);
        let empty = pass.is_empty();
        counts.push(pass);
        if empty {
            break;
        }
    }
    let count_of = |k: &[char]| counts.get(k.len() - 1).and_then(|m| m.get(&k)).copied();
    let total = total as f64;
    let lp = |k: &[char]| (count_of(k).unwrap_or(1) as f64 / total).ln();

    // 左右的鄰字 for the survivors, in one more pass over the runs. This is the
    // part that has to be done on the text rather than on the counts: what a
    // string's neighbours are is not derivable from how often it occurs.
    let mut sides: HashMap<&[char], (HashMap<char, usize>, HashMap<char, usize>)> = HashMap::new();
    for n in 2..=counts.len() {
        if counts[n - 1].is_empty() {
            continue;
        }
        for &(a, b) in &runs {
            if b - a < n {
                continue;
            }
            for start in a..=(b - n) {
                let word = &chars[start..start + n];
                if !counts[n - 1].contains_key(&word) {
                    continue;
                }
                let left = if start > a { chars[start - 1] } else { EDGE };
                let right = if start + n < b { chars[start + n] } else { EDGE };
                let seen = sides.entry(word).or_default();
                *seen.0.entry(left).or_insert(0) += 1;
                *seen.1.entry(right).or_insert(0) += 1;
            }
        }
    }

    let mut kept: Vec<(&[char], usize, f64, f64)> = Vec::new();
    for n in 2..=counts.len() {
        for (&word, &count) in &counts[n - 1] {
            let cohesion = (1..n)
                .map(|k| lp(word) - lp(&word[..k]) - lp(&word[k..]))
                .fold(f64::INFINITY, f64::min);
            if cohesion < MIN_COHESION {
                continue;
            }
            let Some((left, right)) = sides.get(&word) else {
                continue;
            };
            let entropy = spread(left).min(spread(right));
            if entropy < MIN_ENTROPY {
                continue;
            }
            // Last, because it is the only test that costs a segmentation.
            let spelled: String = word.iter().collect();
            if joins(&spelled) {
                continue;
            }
            kept.push((word, count, cohesion, entropy));
        }
    }

    // 阿寧說 out: the shorter string is the word, the rest is context.
    kept.retain(|&(word, count, _, _)| {
        !inner(word).any(|part| count_of(part).is_some_and(|c| c >= CROWDED_OUT * count))
    });
    // 王語 out: it hardly ever occurs outside 王語嫣, so it is half a name.
    let longer: Vec<(&[char], usize)> = kept.iter().map(|&(w, c, _, _)| (w, c)).collect();
    kept.retain(|&(word, count, _, _)| {
        !longer.iter().any(|&(other, c)| {
            other.len() > word.len()
                && holds(other, word)
                && c as f64 >= ABSORBED * count as f64
        })
    });

    // Commonest first: what a writer wants at the top of the list is the name
    // that is on every page, not the tightest-binding rare compound.
    kept.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(b.2.total_cmp(&a.2))
            .then_with(|| a.0.cmp(b.0))
    });
    kept.into_iter()
        .map(|(word, count, cohesion, entropy)| Found {
            word: word.iter().collect(),
            count,
            cohesion,
            entropy,
        })
        .collect()
}

/// −Σ p ln p over the neighbours seen on one side.
fn spread(seen: &HashMap<char, usize>) -> f64 {
    let n: usize = seen.values().sum();
    if n == 0 {
        return 0.0;
    }
    let n = n as f64;
    -seen
        .values()
        .map(|&c| {
            let p = c as f64 / n;
            p * p.ln()
        })
        .sum::<f64>()
}

/// Every proper substring of `word` two characters or longer.
fn inner(word: &[char]) -> impl Iterator<Item = &[char]> {
    let n = word.len();
    (2..n).flat_map(move |len| (0..=n - len).map(move |at| &word[at..at + len]))
}

/// Whether `word` occurs inside `whole`.
fn holds(whole: &[char], word: &[char]) -> bool {
    whole.windows(word.len()).any(|w| w == word)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing joins anything: the segmenter of a session with no dictionary.
    fn alone(_: &str) -> bool {
        false
    }

    /// A page of prose that repeats nothing, for the candidates to stand out
    /// against.
    ///
    /// **The thresholds are ratios, so a toy text cannot exercise them.** In
    /// four sentences every character occurs once and 內聚度 comes out at ln 4
    /// for a real word and for a coincidence alike; it takes a body of text for
    /// 「far more often than chance」 to mean anything. Sixty characters drawn
    /// at random from 千字文 give a bigram an expected count below one.
    fn prose() -> String {
        let pool: Vec<char> = "天地玄黃宇宙洪荒日月盈昃辰宿列張寒來暑往秋收冬藏閏餘成歲律呂調陽雲騰致雨露結爲霜金生麗水玉出崑岡劍號巨闕珠稱夜光果珍李柰菜重芥薑"
            .chars()
            .collect();
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut roll = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        let mut text = String::new();
        for _ in 0..400 {
            for _ in 0..8 {
                text.push(pool[roll() % pool.len()]);
            }
            text.push_str("。\n");
        }
        text
    }

    /// `word` in varied company, once for each character of `where_at`.
    fn among(word: &str, where_at: &str) -> String {
        let mut text = String::new();
        for c in where_at.chars() {
            text.push(c);
            text.push_str(word);
            text.push(c);
            text.push_str("。\n");
        }
        text
    }

    const TWELVE: &str = "甲乙丙丁戊己庚辛壬癸子丑";
    const THIRTY: &str = "甲乙丙丁戊己庚辛壬癸子丑寅卯辰巳午未申酉戌亥虎兔龍蛇馬羊猴雞";

    #[test]
    fn a_name_in_varied_company_is_found() {
        let found = words(&(prose() + &among("阿寧", TWELVE)), &alone);
        let name = found.iter().find(|f| f.word == "阿寧");
        let name = name.unwrap_or_else(|| panic!("{found:?}"));
        assert_eq!(name.count, 12);
        assert!(name.cohesion > MIN_COHESION, "{name:?}");
        assert!(name.entropy > MIN_ENTROPY, "{name:?}");
    }

    #[test]
    fn a_string_with_one_habitual_neighbour_is_not_a_word() {
        // 阿寧 always after 說 and before 道: the company never varies, so on
        // this evidence 說阿寧道 is the unit and 阿寧 is a piece of it.
        let found = words(&(prose() + &"說阿寧道。\n".repeat(12)), &alone);
        assert!(!found.iter().any(|f| f.word == "阿寧"), "{found:?}");
    }

    #[test]
    fn what_the_segmenter_already_joins_is_never_offered() {
        let text = prose() + &among("阿寧", TWELVE);
        let found = words(&text, &|w| w == "阿寧");
        assert!(!found.iter().any(|f| f.word == "阿寧"), "{found:?}");
    }

    #[test]
    fn a_name_with_a_verb_stuck_to_it_is_crowded_out() {
        let text = prose() + &among("阿寧", THIRTY) + &among("阿寧說", TWELVE);
        let found = words(&text, &alone);
        assert!(found.iter().any(|f| f.word == "阿寧"), "{found:?}");
        assert!(!found.iter().any(|f| f.word == "阿寧說"), "{found:?}");
    }

    #[test]
    fn half_a_name_is_absorbed_by_the_whole_one() {
        let found = words(&(prose() + &among("王語嫣", TWELVE)), &alone);
        assert!(found.iter().any(|f| f.word == "王語嫣"), "{found:?}");
        assert!(!found.iter().any(|f| f.word == "王語"), "{found:?}");
        assert!(!found.iter().any(|f| f.word == "語嫣"), "{found:?}");
    }

    #[test]
    fn four_sightings_are_not_enough() {
        let found = words(&(prose() + &among("阿寧", "甲乙丙丁")), &alone);
        assert!(!found.iter().any(|f| f.word == "阿寧"), "{found:?}");
    }

    #[test]
    fn a_word_never_spans_a_break() {
        // 寧甲 straddles the full stop twelve times and is never a word.
        let found = words(&(prose() + &"阿寧。甲乙\n".repeat(12)), &alone);
        assert!(!found.iter().any(|f| f.word == "寧甲"), "{found:?}");
    }

    #[test]
    fn prose_with_no_repetition_yields_nothing() {
        assert!(words("今天天氣很好，我們出去走走。", &alone).is_empty());
    }

    #[test]
    fn the_commonest_comes_first() {
        let text = prose() + &among("阿寧", THIRTY) + &among("蕭峯", TWELVE);
        let found = words(&text, &alone);
        assert_eq!(found[0].word, "阿寧", "{found:?}");
        assert!(found.iter().any(|f| f.word == "蕭峯"), "{found:?}");
    }
}
