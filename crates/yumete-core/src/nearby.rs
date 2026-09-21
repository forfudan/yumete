//! **Nearly these characters, nearly in a row** — the search panel's 模糊
//! switch (2026-09-19: 「寫小説的人記得『差不多是這幾個字』卻記不得原
//! 句」).
//!
//! The panel's other setting is a regular expression, which answers a
//! different question: a regex is for a *shape* (行首的「他説」, 兩個以上的驚
//! 嘆號). This one is for a half-remembered line — the characters are right and
//! the order is right, and something in the middle is not.
//!
//! ## Why not the picker's scoring
//!
//! The picker ([`crate::picker`]) matches a subsequence over a whole label and
//! scores it. That works because a label is a file name: short, and a scatter
//! across it is still nearby. **A line of prose is not short.** 「返亭」 as a
//! plain subsequence matches 「返回的時候他在亭子裏」 and a hundred lines like
//! it, and a list where nine rows in ten are noise is worse than no list.
//!
//! So the rule here has a **window**: every character of the query, in order,
//! inside a run of at most [`window`] characters. 「差不多是這幾個字」 is a
//! statement about a *phrase*, not about a line.
//!
//! ⚠️ **It is counted in characters, not bytes** — 漢字 are the whole point —
//! and 「consecutive」 matters more here than in a Latin fuzzy finder: two
//! 漢字 carry as much as eight letters, so a gap of two is already a lot to
//! forgive.

/// How far the match may reach for a query of `len` characters.
///
/// Query length ＋ a little: enough to forgive a 的, a 了, a name in the
/// middle — 「他説」 finding 「他輕輕地説」 — and not enough to reach across a
/// sentence. Twice the query plus four, so that a two-character query still
/// has room to breathe (8) while a long one does not turn into a paragraph.
pub fn window(len: usize) -> usize {
    len * 2 + 4
}

/// Every place `needle` is found in `hay` within [`window`], as `(start, end)`
/// character ranges — **non-overlapping**, earliest and tightest first.
///
/// `fold` lower-cases both sides; the caller decides that from 大小寫.
///
/// ⚠️ **Two passes, and the second is backwards** — the same shape as
/// [`crate::picker`], for the same reason. A forward walk alone takes the
/// *first* place each character fits: 「他説」 in 「他。他説」 would be marked
/// from the first 他, and the reader would see a range with a full stop in the
/// middle of it when the phrase is right there. So the forward pass only finds
/// **where a match can end**, and a backward pass from there takes the last
/// place each character fits — the tightest range ending at that point.
///
/// The next search resumes at the end of the one before, so a phrase repeated
/// in a sentence is two rows rather than a cascade of overlapping ones.
pub fn spans(hay: &[char], needle: &[char], fold: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() || hay.is_empty() {
        return out;
    }
    let same = |a: char, b: char| match fold {
        true => a.to_lowercase().eq(b.to_lowercase()),
        false => a == b,
    };
    let reach = window(needle.len());
    let mut from = 0usize;
    while from < hay.len() {
        let Some(at) = (from..hay.len()).find(|&i| same(hay[i], needle[0])) else {
            break;
        };
        // Forward from here, taking each character of the query the first time
        // it turns up — the tightest match that starts at `at`.
        let mut want = 1usize;
        let mut end = at + 1;
        while end < hay.len() && want < needle.len() && end - at < reach {
            if same(hay[end], needle[want]) {
                want += 1;
            }
            end += 1;
        }
        match want == needle.len() {
            true => {
                // Backwards from the end, taking the last place each character
                // fits: the tightest range that ends here.
                let mut upto = end;
                let mut start = end;
                for &want in needle.iter().rev() {
                    let found = (0..upto).rev().find(|&i| same(hay[i], want)).unwrap_or(at);
                    start = found;
                    upto = found;
                }
                out.push((start, end));
                from = end;
            }
            // No match from here; the next candidate start is the next place
            // the first character turns up.
            false => from = at + 1,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(hay: &str, needle: &str) -> Vec<String> {
        let hay: Vec<char> = hay.chars().collect();
        let needle: Vec<char> = needle.chars().collect();
        spans(&hay, &needle, false)
            .into_iter()
            .map(|(a, b)| hay[a..b].iter().collect())
            .collect()
    }

    #[test]
    fn the_characters_in_order_with_something_in_between() {
        assert_eq!(found("他輕輕地説了一句", "他説"), ["他輕輕地説"]);
        // Exact is a match too — the loose rule contains the tight one.
        assert_eq!(found("他説了一句", "他説"), ["他説"]);
        // Out of order is not a match.
        assert!(found("説他", "他説").is_empty());
    }

    /// The window is the whole point: without it this is 「these characters
    /// somewhere in the line」, which a line of prose almost always satisfies.
    #[test]
    fn it_does_not_reach_across_a_sentence() {
        assert!(found("他走了很久很久，天亮的時候纔聽見有人説話", "他説").is_empty());
        assert_eq!(window(2), 8);
        assert_eq!(found("他輕輕地説", "他説"), ["他輕輕地説"]);
        // 8 is the window for a two-character query: eight characters fit…
        assert_eq!(found("他輕輕地輕輕地説", "他説"), ["他輕輕地輕輕地説"]);
        // …and nine do not.
        assert!(found("他輕輕地輕輕地地説", "他説").is_empty(), "too far");
    }

    /// Found three times, not a cascade of overlapping ranges.
    #[test]
    fn the_same_phrase_twice_is_two_answers() {
        assert_eq!(found("他説。他輕聲説。", "他説"), ["他説", "他輕聲説"]);
    }

    /// A start that cannot be completed does not swallow the one after it.
    #[test]
    fn a_false_start_is_stepped_over() {
        assert_eq!(found("他。他説", "他説"), ["他説"]);
    }

    #[test]
    fn case_is_the_callers_business() {
        let hay: Vec<char> = "The Plan".chars().collect();
        let needle: Vec<char> = "tp".chars().collect();
        assert!(spans(&hay, &needle, false).is_empty());
        assert_eq!(spans(&hay, &needle, true), [(0, 5)]);
    }
}
