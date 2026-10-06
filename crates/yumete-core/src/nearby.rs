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
//! Warning: **It is counted in characters, not bytes** — 漢字 are the whole point —
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

/// **兩個字算不算同一個** —— 大小寫與繁簡那兩條折疊規矩，一份，兩個工具都調它。
///
/// Warning: **參數有方向**：`query` 是打進來的那一邊，`text` 是稿子/檔名那一邊。
/// 繁簡**放寬的是查詢那一邊**——`shapes(发) = 发發髮`，所以打「头发」找得到「頭髮」；
/// `shapes(發) = 發发`，所以打「發」不會誤中「髮」。對調就把這個性質毀了。
///
/// Warning: **從前這條規矩在兩個檔裏各寫了一遍**（`nearby` 與 `picker`，2026-10-04 合
/// 的）。兩份逐字等價，可「同一條規矩兩處實現」本身就是下一個分歧的種子。
pub fn alike(query: char, text: char, fold_case: bool, glyphs: bool) -> bool {
    let plain = match fold_case {
        true => query.to_lowercase().eq(text.to_lowercase()),
        false => query == text,
    };
    plain || (glyphs && crate::glyphs::shapes(query).contains(text))
}

/// **從 `from` 起找一處命中，交出每一個字落在哪** —— 兩個工具共用的那個核。
///
/// 兩趟：前向那一趟只找「命中能在哪裏收尾」，再從那裏反向收緊。單趟貪心會取**最
/// 早**配得上的位置，`ch6` 在 `chapters/ch6.md` 上會標中「chapters」的 `ch` 再跳老遠
/// 去夠那個 `6`——散開的一串，卻按一處算分，還高亮在錯的地方。fzf 的 v1 算法為同
/// 一個理由做同一件事。
///
/// `reach` 是**整處命中最多跨多少個字**，`None` ＝ 不限。面板限（見 [`window`]：
/// 「他説」要找得到「他輕輕地説」，但不許跨句），挑選器從前不限。
pub fn one(
    hay: &[char],
    needle: &[char],
    from: usize,
    reach: Option<usize>,
    fold_case: bool,
    glyphs: bool,
) -> Option<Vec<usize>> {
    if needle.is_empty() || from >= hay.len() {
        return None;
    }
    let same = |query: char, text: char| alike(query, text, fold_case, glyphs);
    let mut at = from;
    loop {
        at = (at..hay.len()).find(|&i| same(needle[0], hay[i]))?;
        let mut want = 1usize;
        let mut end = at + 1;
        while end < hay.len()
            && want < needle.len()
            && reach.is_none_or(|reach| end - at < reach)
        {
            if same(needle[want], hay[end]) {
                want += 1;
            }
            end += 1;
        }
        if want == needle.len() {
            // 反向收緊：從終點往回，每一個字取**最晚**配得上的那一格。
            let mut upto = end;
            let mut positions = vec![0usize; needle.len()];
            for (k, &want) in needle.iter().enumerate().rev() {
                let found = (0..upto).rev().find(|&i| same(want, hay[i])).unwrap_or(at);
                positions[k] = found;
                upto = found;
            }
            return Some(positions);
        }
        at += 1;
    }
}

/// Every place `needle` is found in `hay` within [`window`], as `(start, end)`
/// character ranges — **non-overlapping**, earliest and tightest first.
///
/// `fold` lower-cases both sides; the caller decides that from 大小寫.
/// `glyphs` counts 書 and 书 as one character, from 中文匹配.
///
/// 兩趟那一套在 [`one`] 身上，和挑選器共用（2026-10-04 合的）。這一支只多做一件
/// 事：接着上一處的後面再找一次，所以一句話裏重複的詞是兩行，不是一串疊着的。
pub fn spans(hay: &[char], needle: &[char], fold: bool, glyphs: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    if needle.is_empty() || hay.is_empty() {
        return out;
    }
    let reach = Some(window(needle.len()));
    let mut from = 0usize;
    while from < hay.len() {
        let Some(positions) = one(hay, needle, from, reach, fold, glyphs) else { break };
        let (start, last) = (positions[0], positions[positions.len() - 1]);
        out.push((start, last + 1));
        // 不重疊：下一趟從這一處的後面接着找。
        from = last + 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(hay: &str, needle: &str) -> Vec<String> {
        let hay: Vec<char> = hay.chars().collect();
        let needle: Vec<char> = needle.chars().collect();
        spans(&hay, &needle, false, false)
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
        assert!(spans(&hay, &needle, false, false).is_empty());
        assert_eq!(spans(&hay, &needle, true, false), [(0, 5)]);
    }
}
