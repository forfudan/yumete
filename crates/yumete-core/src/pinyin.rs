//! **用拼音找漢字**，給搜索用（2026-09-25 作者提）。
//!
//! 原話：「在中文状态下，模糊查询其实可以应用到繁简体、拼音的范畴。比如「天門」
//! 可以搜到「天门」，tianmen也可以搜到「天门」「天門」」。
//!
//! ## 只認全拼（作者 2026-09-25 定）
//!
//! 每一個字要吃掉**一整個音節**。`tianmen` 找得到「天門」，`tm` 和 `tianm` 找不
//! 到。首字母那一路噪音太大——`tm` 在一本書裏是幾百處，而讀者要的是那一處。
//!
//! ## 為什麼這一路不能是正則
//!
//! 字形那一路（[`crate::glyphs`]）是把查詢裏的每個字換成 `[…]`，掃描仍舊交給正則
//! 引擎。**拼音不行**：查詢是拉丁字母，要配的是漢字，而「幾個漢字的讀音連起來正好
//! 是這一串字母」沒有正則寫法——音節邊界要邊配邊定。所以它自己走一趟，命中與字面
//! 命中**合並**（同一條線上的兩種問法，見 `find.rs` 的 `Look`）。
//!
//! 同一個道理，它和 模糊 一樣**只在高級搜索面板裏管用**：正文的 `n`／`N` 走的是
//! `last_search` 那個正則。
//!
//! ## 熱路徑
//!
//! 每一個起點做一次深度優先，深度不超過查詢的字母數。**分支幾乎不存在**：一個字
//! 的幾個讀音互不為前綴（`長` 是 `zhang chang`），所以每一步至多一條路走得通。
//! 表用 `include_str!` 編進二進制，首次用到時解析——碼表沒裝的機器上照樣能搜。

use std::collections::HashMap;
use std::sync::OnceLock;

/// 生成物，編進二進制。見 `scripts/make_readings.py`。
const TABLE: &str = include_str!("readings.txt");

/// 一串查詢最多幾個字母。**不是為了省時間，是為了關上遞歸的門**：深度就是它。
const LONGEST: usize = 64;

fn table() -> &'static HashMap<char, &'static str> {
    static ONCE: OnceLock<HashMap<char, &'static str>> = OnceLock::new();
    ONCE.get_or_init(|| {
        let mut out = HashMap::new();
        for line in TABLE.lines() {
            if line.starts_with('#') {
                continue;
            }
            let Some((head, said)) = line.split_once('\t') else {
                continue;
            };
            let Some(ch) = head.chars().next() else { continue };
            out.insert(ch, said);
        }
        out
    })
}

/// 一個字的讀音，去調，常用的在前。不認得就是空的。
pub fn readings(ch: char) -> impl Iterator<Item = &'static str> {
    table().get(&ch).copied().unwrap_or("").split_ascii_whitespace()
}

/// **查詢能不能當拼音用**：非空、不太長、全是 ASCII 字母。
///
/// ⚠️ **大小寫在這裏收掉**，回的是小寫那一份——表裏是小寫，而讀者打 `TianMen`
/// 的時候心裏想的不是「這是另一個查詢」。
pub fn as_query(text: &str) -> Option<Vec<char>> {
    let text = text.trim();
    let ok = !text.is_empty()
        && text.len() <= LONGEST
        && text.chars().all(|c| c.is_ascii_alphabetic());
    ok.then(|| text.to_ascii_lowercase().chars().collect())
}

/// **`said` 這串字母念得出來的那些段**，按**字**計，互不重疊。
///
/// 和正則的 `find_iter` 一個規矩：從左往右，配上了就從它的末尾接着找。
pub fn spans(text: &str, said: &[char]) -> Vec<(usize, usize)> {
    if said.is_empty() {
        return Vec::new();
    }
    let hay: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < hay.len() {
        match eat(&hay, at, said, 0) {
            Some(end) => {
                out.push((at, end));
                at = end;
            }
            None => at += 1,
        }
    }
    out
}

/// 從第 `i` 個字、查詢的第 `q` 個字母起，能不能把查詢吃完；能就回末尾那一個字的
/// 後面一格。
///
/// ⚠️ **讀音按表裏的次序試，先到先得**：表裏常用的在前，所以歧義處取的是常用那
/// 一讀。找的是「有沒有」，不是「哪一種最好」——搜索交出一段就夠了。
fn eat(hay: &[char], i: usize, said: &[char], q: usize) -> Option<usize> {
    if q == said.len() {
        return Some(i);
    }
    let ch = *hay.get(i)?;
    for syllable in readings(ch) {
        let n = syllable.len();
        if q + n > said.len() {
            continue;
        }
        if !syllable.bytes().zip(&said[q..q + n]).all(|(a, &b)| a as char == b) {
            continue;
        }
        if let Some(end) = eat(hay, i + 1, said, q + n) {
            return Some(end);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_character_says_what_it_says() {
        assert_eq!(readings('天').collect::<Vec<_>>(), ["tian"]);
        // 多音字全收，不按頻次砍。
        assert!(readings('長').any(|s| s == "zhang"));
        assert!(readings('長').any(|s| s == "chang"));
        // ü 兩種寫法都在。
        assert!(readings('女').any(|s| s == "nv"));
        assert!(readings('女').any(|s| s == "nu"));
        // 不是漢字的，安靜地沒有讀音。
        assert_eq!(readings('a').count(), 0);
    }

    #[test]
    fn a_whole_syllable_each_and_nothing_less() {
        let text = "那年天門下起了大雪";
        assert_eq!(spans(text, &as_query("tianmen").unwrap()), [(2, 4)]);
        // 只認全拼：首字母和半個音節都不算（作者 2026-09-25 定）。
        assert!(spans(text, &as_query("tm").unwrap()).is_empty());
        assert!(spans(text, &as_query("tianm").unwrap()).is_empty());
    }

    #[test]
    fn simplified_and_traditional_both_answer_to_the_same_letters() {
        // 拼音這一路自己就跨簡繁——兩邊念的是同一個音。
        assert_eq!(spans("天门", &as_query("tianmen").unwrap()), [(0, 2)]);
        assert_eq!(spans("天門", &as_query("tianmen").unwrap()), [(0, 2)]);
    }

    #[test]
    fn a_second_reading_is_tried_when_the_first_does_not_fit() {
        // 長 的頭一讀是 zhang，而 changjiang 要的是第二讀。
        assert_eq!(spans("長江", &as_query("changjiang").unwrap()), [(0, 2)]);
        assert_eq!(spans("長大", &as_query("zhangda").unwrap()), [(0, 2)]);
    }

    #[test]
    fn found_twice_is_listed_twice_and_they_do_not_overlap() {
        let text = "天門，天門";
        assert_eq!(spans(text, &as_query("tianmen").unwrap()), [(0, 2), (3, 5)]);
    }

    #[test]
    fn letters_in_the_prose_are_not_read_as_readings() {
        // 「tian」這四個字母自己不是一個字，拼音這一路配不上它——字面那一路會。
        assert!(spans("tianmen", &as_query("tianmen").unwrap()).is_empty());
    }

    #[test]
    fn a_query_that_is_not_letters_is_not_a_pinyin_query() {
        assert!(as_query("").is_none());
        assert!(as_query("  ").is_none());
        assert!(as_query("天門").is_none());
        assert!(as_query("tian men").is_none());
        assert!(as_query(&"a".repeat(LONGEST + 1)).is_none());
        assert_eq!(as_query(" TianMen ").unwrap().iter().collect::<String>(), "tianmen");
    }
}
