//! **用拼音找漢字**，給搜索用（2026-09-25 提的）。
//!
//! 原話：「在中文状态下，模糊查询其实可以应用到繁简体、拼音的范畴。比如「書齋」
//! 可以搜到「书斋」，shuzhai也可以搜到「书斋」「書齋」」。
//!
//! ## 只認全拼（2026-09-25 定）
//!
//! 每一個字要吃掉**一整個音節**。`shuzhai` 找得到「書齋」，`sz` 和 `shuzh` 找不
//! 到。首字母那一路噪音太大——`sz` 在一本書裏是幾百處，而讀者要的是那一處。
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

/// **查詢切成的一段一段**（2026-10-03 作者定）。
///
/// 從前這一支只收「整條全是字母」的查詢，於是**字母和漢字混不起來**：`zhongguo`
/// 找得到「中國」，而 `zhongguo很大`、`zhong国`、`di120`、`juan03` 一個都找不着
/// ——`as_query` 看見一個非字母就整條回 `None`。可真實的查詢幾乎都是混的：找第
/// 一百二十章打的是 `di120`，找那一卷打的是 `juan03`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Atom {
    /// 一串 ASCII 字母。**兩種配法**：照字面，或者當成一串字的讀音。
    ///
    /// 存兩份：`said` 是小寫的（讀音表是小寫的），`typed` 是打進來的原樣——
    /// 照字面那一路在「大小寫要緊」的時候比的是後者（2026-10-03 修，從前只存
    /// 小寫，於是 `--case-sensitive` 對混着寫的查詢是**死的**）。
    Said { said: Vec<char>, typed: Vec<char> },
    /// 別的任何一個字——漢字、數字、標點。照字面配。
    Just(char),
}

/// **查詢切成 [`Atom`]**，`None` ＝ 這一條沒有拼音可問。
///
/// 兩種情形回 `None`：**一個字母都沒有**（那就全是字面，交給字面那一路就夠了，
/// 問讀音是白跑一趟），以及**某一串字母長過 [`LONGEST`]**（那是關遞歸的門）。
///
/// Warning: **大小寫在這裏收掉**，回的是小寫那一份——表裏是小寫，而讀者打 `ShuZhai`
/// 的時候心裏想的不是「這是另一個查詢」。
pub fn atoms(text: &str) -> Option<Vec<Atom>> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let mut out: Vec<Atom> = Vec::new();
    let mut letters: Vec<char> = Vec::new();
    let close = |letters: &mut Vec<char>, out: &mut Vec<Atom>| -> bool {
        if letters.is_empty() {
            return true;
        }
        if letters.len() > LONGEST {
            return false;
        }
        let typed = std::mem::take(letters);
        let said = typed.iter().map(|c| c.to_ascii_lowercase()).collect();
        out.push(Atom::Said { said, typed });
        true
    };
    for c in text.chars() {
        match c.is_ascii_alphabetic() {
            true => letters.push(c),
            false => {
                if !close(&mut letters, &mut out) {
                    return None;
                }
                out.push(Atom::Just(c));
            }
        }
    }
    if !close(&mut letters, &mut out) {
        return None;
    }
    out.iter().any(|a| matches!(a, Atom::Said { .. })).then_some(out)
}

/// **查詢能不能當拼音用**：非空、不太長、全是 ASCII 字母。
///
/// Warning: 只剩測試在用。活的那兩處（高級搜索面板、挑選器）走的是 [`atoms`]。
#[cfg(test)]
fn as_query(text: &str) -> Option<Vec<char>> {
    let text = text.trim();
    let ok = !text.is_empty()
        && text.len() <= LONGEST
        && text.chars().all(|c| c.is_ascii_alphabetic());
    ok.then(|| text.to_ascii_lowercase().chars().collect())
}

/// **這一條查詢配得上的那些段**，按**字**計，互不重疊。
///
/// 和正則的 `find_iter` 一個規矩：從左往右，配上了就從它的末尾接着找。
///
/// `fold` 說簡繁異體算不算同一個字——面板那一扇有一個開關管它，挑選器一律算。
pub fn spans_of(text: &str, atoms: &[Atom], fold: bool) -> Vec<(usize, usize)> {
    spans_cased(text, atoms, fold, true)
}

/// 同上，但說得出大小寫要不要緊——`fold_case` 為假時照字面那一路分大小寫。
pub fn spans_cased(text: &str, atoms: &[Atom], fold: bool, fold_case: bool) -> Vec<(usize, usize)> {
    if atoms.is_empty() {
        return Vec::new();
    }
    let hay: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut at = 0usize;
    while at < hay.len() {
        match eat(&hay, at, atoms, 0, fold, fold_case) {
            // 配上了零個字（查詢是空的）不算一段，否則這個迴圈不往前走。
            Some(end) if end > at => {
                out.push((at, end));
                at = end;
            }
            _ => at += 1,
        }
    }
    out
}

/// 同一個字嗎——`fold` 開着的時候簡繁異體算同一個。
fn same(want: char, have: char, fold: bool) -> bool {
    want == have || (fold && crate::glyphs::shapes(want).contains(have))
}

/// 從第 `i` 個字、第 `a` 段起，能不能把查詢吃完；能就回末尾那一個字的後面一格。
fn eat(
    hay: &[char],
    i: usize,
    atoms: &[Atom],
    a: usize,
    fold: bool,
    fold_case: bool,
) -> Option<usize> {
    let Some(atom) = atoms.get(a) else {
        return Some(i);
    };
    match atom {
        Atom::Just(c) => match same(*c, *hay.get(i)?, fold) {
            true => eat(hay, i + 1, atoms, a + 1, fold, fold_case),
            false => None,
        },
        Atom::Said { said, typed } => {
            // **只有混着寫的查詢纔許照字面配。**
            //
            // 一整條都是字母的時候（`shuzhai`），這一路要的就是「念作這幾個字母
            // 的那幾個漢字」——字面那一路本來就在跑，它會配上稿子裏真的那七個
            // 字母。兩路分工，所以 `spans("shuzhai", …)` 必須是空的，那是一條舊
            // 不變式（`letters_in_the_prose_are_not_read_as_readings`）。
            //
            // 混着寫就不一樣了：`ch第3` 裏那個 `ch` 沒有讀音可問，它就是兩個字母。
            // 不給它照字面配的路，整條查詢就斷在第一段上。
            let literal_ok = atoms.len() > 1;
            // **大小寫要緊的時候比打進來的那一份。** 從前一律比小寫，於是
            // `--case-sensitive` 對混着寫的查詢是死的（`Alpha中` 中了 `alpha中`）。
            let want: &[char] = match fold_case {
                true => said,
                false => typed,
            };
            let fits = literal_ok
                && hay.len() >= i + want.len()
                && hay[i..i + want.len()].iter().zip(want).all(|(h, s)| match fold_case {
                    true => h.to_ascii_lowercase() == *s,
                    false => h == s,
                });
            if fits {
                if let Some(end) = eat(hay, i + want.len(), atoms, a + 1, fold, fold_case) {
                    return Some(end);
                }
            }
            say(hay, i, said, 0, atoms, a, fold, fold_case)
        }
    }
}

/// 一串字母當讀音吃：從第 `i` 個字、第 `q` 個字母起。
///
/// Warning: **讀音按表裏的次序試，先到先得**：表裏常用的在前，所以歧義處取的是常用那
/// 一讀。找的是「有沒有」，不是「哪一種最好」——搜索交出一段就夠了。
#[allow(clippy::too_many_arguments)]
fn say(
    hay: &[char],
    i: usize,
    said: &[char],
    q: usize,
    atoms: &[Atom],
    a: usize,
    fold: bool,
    fold_case: bool,
) -> Option<usize> {
    if q == said.len() {
        return eat(hay, i, atoms, a + 1, fold, fold_case);
    }
    let ch = *hay.get(i)?;
    for syllable in readings(ch) {
        let n = syllable.len();
        if q + n > said.len() {
            continue;
        }
        if !syllable.bytes().zip(&said[q..q + n]).all(|(x, &y)| x as char == y) {
            continue;
        }
        if let Some(end) = say(hay, i + 1, said, q + n, atoms, a, fold, fold_case) {
            return Some(end);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 舊那幾條測試寫的是「整條全是字母」那一種，留着它們——那一種照舊要對。
    fn spans(text: &str, said: &[char]) -> Vec<(usize, usize)> {
        let atoms = vec![Atom::Said { said: said.to_vec(), typed: said.to_vec() }];
        spans_of(text, &atoms, true)
    }

    /// **字母和漢字混得起來了**（2026-10-03 作者定）。
    ///
    /// 從前查詢要麼整條是拼音、要麼整條是字面：`zhongguo` 找得到「中國」，而
    /// `zhongguo很大`、`zhong国`、`di120`、`juan03` 一個都找不着。可真實的查詢
    /// 幾乎都是混的——找第一百二十章打的是 `di120`。
    #[test]
    fn letters_and_漢字_mix_in_one_query() {
        let look = |text: &str, query: &str| spans_of(text, &atoms(query).unwrap_or_default(), true);

        // 讀音在前、漢字在後。
        assert_eq!(look("中國很大。", "zhongguo很大"), [(0, 4)]);
        assert_eq!(look("中國很大。", "zhong国"), [(0, 2)], "簡繁照折");
        // 漢字在前、讀音在後。
        assert_eq!(look("中國很大。", "中guo"), [(0, 2)]);
        // 讀音加數字——真實的檔名就長這樣。
        assert_eq!(look("卷03/第120章.md", "juan03"), [(0, 3)]);
        assert_eq!(look("卷03/第120章.md", "di120"), [(4, 8)]);
        assert_eq!(look("卷03/第120章.md", "juan03/di120"), [(0, 8)]);

        // **一個字母都沒有就不勞駕這一路**：那是字面那一路的事。
        assert!(atoms("很大").is_none());
        assert!(atoms("").is_none());

        // 混着寫的時候，不是讀音的那一段照字面配。
        assert_eq!(look("卷03/第120章.md", "di120章.md"), [(4, 12)]);
        // **整條都是字母的那一種照舊只問讀音**，字面歸字面那一路——見
        // `letters_in_the_prose_are_not_read_as_readings`。
        assert!(look("卷03/第120章.md", "md").is_empty());
    }

    /// 簡繁那一折跟着開關走——關掉就不折。
    #[test]
    fn the_variants_switch_reaches_the_pinyin_pass_too() {
        let atoms = atoms("zhong国").unwrap();
        assert_eq!(spans_of("中國很大。", &atoms, true), [(0, 2)], "折：國算国");
        assert!(spans_of("中國很大。", &atoms, false).is_empty(), "不折：不算");
        assert_eq!(spans_of("中国很大。", &atoms, false), [(0, 2)], "本來就是它，不必折");
    }

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
        let text = "那年書齋下起了大雪";
        assert_eq!(spans(text, &as_query("shuzhai").unwrap()), [(2, 4)]);
        // 只認全拼：首字母和半個音節都不算（2026-09-25 定）。
        assert!(spans(text, &as_query("sz").unwrap()).is_empty());
        assert!(spans(text, &as_query("shuzh").unwrap()).is_empty());
    }

    #[test]
    fn simplified_and_traditional_both_answer_to_the_same_letters() {
        // 拼音這一路自己就跨簡繁——兩邊念的是同一個音。
        assert_eq!(spans("书斋", &as_query("shuzhai").unwrap()), [(0, 2)]);
        assert_eq!(spans("書齋", &as_query("shuzhai").unwrap()), [(0, 2)]);
    }

    #[test]
    fn a_second_reading_is_tried_when_the_first_does_not_fit() {
        // 長 的頭一讀是 zhang，而 changjiang 要的是第二讀。
        assert_eq!(spans("長江", &as_query("changjiang").unwrap()), [(0, 2)]);
        assert_eq!(spans("長大", &as_query("zhangda").unwrap()), [(0, 2)]);
    }

    #[test]
    fn found_twice_is_listed_twice_and_they_do_not_overlap() {
        let text = "書齋，書齋";
        assert_eq!(spans(text, &as_query("shuzhai").unwrap()), [(0, 2), (3, 5)]);
    }

    #[test]
    fn letters_in_the_prose_are_not_read_as_readings() {
        // 「shuzhai」這七個字母自己不是漢字，拼音這一路配不上它——字面那一路會。
        assert!(spans("shuzhai", &as_query("shuzhai").unwrap()).is_empty());
    }

    #[test]
    fn a_query_that_is_not_letters_is_not_a_pinyin_query() {
        assert!(as_query("").is_none());
        assert!(as_query("  ").is_none());
        assert!(as_query("書齋").is_none());
        assert!(as_query("shu zhai").is_none());
        assert!(as_query(&"a".repeat(LONGEST + 1)).is_none());
        assert_eq!(as_query(" ShuZhai ").unwrap().iter().collect::<String>(), "shuzhai");
    }
}
