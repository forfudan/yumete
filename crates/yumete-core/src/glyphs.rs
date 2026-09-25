//! **同一個字的各種字形**，給搜索用（2026-09-25 作者提）。
//!
//! 原話：「在中文状态下，模糊查询其实可以应用到繁简体……比如「天門」可以搜到
//! 「天门」」。
//!
//! ## 為什麼這裏可以用一張字表，而 `:convert` 不可以
//!
//! [`crate::convert`] 開頭那一段說得很明白：簡繁**轉換**要在 發／髮 之間挑一個，
//! 那需要詞典和分詞，所以它喊 opencc。
//!
//! **搜索問的是另一個問題**：「這兩個字有沒有可能是同一個字？」——不需要上下文。
//! 搜「头发」順帶命中一條「頭發」在單子上只是多一行，而 `:convert` 把稿子裏的
//! 「头发」轉成「頭發」是毀稿。**代價差着好幾個數量級**，所以這裏一張字表就夠。
//!
//! ## 這張表怎麼來的，以及那個不對稱
//!
//! `scripts/make_glyph_sets.py` 從 opencc 的 `TSCharacters`／`TWVariants`／
//! `HKVariants` 加上倉裏那兩張 GujiCC 表（`glyphs_c.txt`／`glyphs_g.txt`）生成。
//! 辦法是作者定的：以 **opencc 的繁體字形**為鍵分行，把各標準的字形並進來，再把
//! 鍵自己也並進去；**然後每個字取它出現過的所有行的並集**。
//!
//! ⚠️ **那個並集是不對稱的，而這個不對稱正是對的**：
//!
//! | 查詢 | 展開成 | 於是 |
//! | --- | --- | --- |
//! | 发 | 发發髮 | 搜「头发」找得到「頭髮」 |
//! | 發 | 發发 | 搜「發」**不會**誤中「髮」 |
//!
//! 含混的那個字放寬，精確的那個字保持精確。誰要是改成傳遞閉包就毀了它。
//!
//! ## 為什麼熱路徑上沒有東西要優化
//!
//! **折疊發生在查詢上，不在書上。** 每敲一個鍵的工作量是「查詢有幾個字」次查表，
//! 掃描仍舊交給正則引擎。查詢就幾個字，和書多大無關。唯一的開銷是把這張表讀進
//! 內存一次——`include_str!` 編進二進制，首次用到時解析。
//!
//! ⚠️ **編進二進制而不是讀數據目錄**：碼表沒裝的機器上搜索照樣得能用。

use std::collections::HashMap;
use std::sync::OnceLock;

/// 生成物，編進二進制。見 `scripts/make_glyph_sets.py`。
const TABLE: &str = include_str!("glyph_sets.txt");

fn table() -> &'static HashMap<char, &'static str> {
    static ONCE: OnceLock<HashMap<char, &'static str>> = OnceLock::new();
    ONCE.get_or_init(|| {
        let mut out = HashMap::new();
        for line in TABLE.lines() {
            if line.starts_with('#') {
                continue;
            }
            let Some((head, also)) = line.split_once('\t') else {
                continue;
            };
            let Some(ch) = head.chars().next() else { continue };
            out.insert(ch, also);
        }
        out
    })
}

/// 這個字可以是的所有字形，**自己排在最前**。沒有別的寫法就只有它自己。
pub fn shapes(ch: char) -> &'static str {
    table().get(&ch).copied().unwrap_or("")
}

/// **把一串要照字面找的字改寫成正則**，每個字換成它的字形集。
///
/// 「天門」→ `[天][門门]`。沒有別的寫法的字原樣轉義過去，所以這一支對純西文的
/// 查詢什麼都不做。
///
/// ⚠️ **只給「照字面」那一條路用。** 正則開着的時候，把每個字改寫成 `[...]` 會把
/// `.`、`*`、`[` 一起吃掉——那是毀掉使用者寫的式子。兩個開關因此互斥，面板上
/// 正則開着時這一個畫灰。
pub fn widen(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        match shapes(ch) {
            "" => out.push_str(&regex::escape(&ch.to_string())),
            also => {
                out.push('[');
                // ⚠️ 表裏全是漢字，可 `regex::escape` 還是要走一趟：字符類裏
                // `]`、`\`、`^`、`-` 有意思，而「這張表永遠不會有它們」是一句
                // 靠生成腳本兌現的話，不是靠這裏。
                out.push_str(&regex::escape(also));
                out.push(']');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **作者舉的那個例子**：「天門」搜得到「天门」。
    #[test]
    fn a_traditional_query_finds_the_simplified_writing() {
        let re = regex::Regex::new(&widen("天門")).unwrap();
        assert!(re.is_match("天门"), "{}", widen("天門"));
        assert!(re.is_match("天門"));
        // 反過來也要成。
        let re = regex::Regex::new(&widen("天门")).unwrap();
        assert!(re.is_match("天門"));
    }

    /// ⚠️ **不對稱是有意的**：含混的那個放寬，精確的那個保持精確。
    #[test]
    fn the_ambiguous_one_widens_and_the_precise_one_does_not() {
        let loose = regex::Regex::new(&widen("头发")).unwrap();
        assert!(loose.is_match("頭髮"), "{}", widen("头发"));
        let tight = regex::Regex::new(&widen("發")).unwrap();
        assert!(tight.is_match("發"));
        assert!(!tight.is_match("髮"), "「發」不該誤中「髮」：{}", widen("發"));
    }

    /// 西文和標點原樣過去——這一支對它們什麼都不做，而且不許把正則語法漏出來。
    #[test]
    fn latin_and_punctuation_come_through_as_themselves() {
        let re = regex::Regex::new(&widen("a.b")).unwrap();
        assert!(re.is_match("a.b"));
        assert!(!re.is_match("axb"), "`.` 要照字面：{}", widen("a.b"));
    }
}
