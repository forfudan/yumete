//! **同一個字的各種字形**，給搜索用（2026-09-25 提的）。
//!
//! 原話：「在中文状态下，模糊查询其实可以应用到繁简体……比如「書齋」可以搜到
//! 「书斋」」。
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
//! 辦法是定下的：以 **opencc 的繁體字形**為鍵分行，把各標準的字形並進來，再把
//! 鍵自己也並進去；**然後每個字取它出現過的所有行的並集**。
//!
//! Warning: **那個並集是不對稱的，而這個不對稱正是對的**：
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
//! Warning: **編進二進制而不是讀數據目錄**：碼表沒裝的機器上搜索照樣得能用。

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
/// 「書齋」→ `[書书][齋斋]`。沒有別的寫法的字原樣轉義過去，所以這一支對純西文的
/// 查詢什麼都不做。
///
/// Warning: **只給「照字面」那一條路用。** 正則開着的時候整串改寫會把 `.`、`*`、`[`
/// 一起吃掉——那是毀掉使用者寫的式子。正則那一路走 [`widen_pattern`]，它先把式子
/// 解析一遍，只動「原樣打出來的那些字」。
pub fn widen(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for ch in text.chars() {
        match shapes(ch) {
            "" => out.push_str(&regex::escape(&ch.to_string())),
            also => {
                out.push('[');
                // Warning: 表裏全是漢字，可 `regex::escape` 還是要走一趟：字符類裏
                // `]`、`\`、`^`、`-` 有意思，而「這張表永遠不會有它們」是一句
                // 靠生成腳本兌現的話，不是靠這裏。
                out.push_str(&regex::escape(also));
                out.push(']');
            }
        }
    }
    out
}


/// **把一個正則裏「原樣打出來的那些字」換成它們的字形集**，別的一個字節不動
/// （2026-10-01 定，問的是：「正则情况下能否也能兼容繁简体？」）。
///
/// `書.*齋` → `[書书].*[齋斋]`；`\d書` → `\d[書书]`；`a.b` 原封不動。
///
/// 做法是先用 `regex-syntax`（`regex` 自己的解析器）把式子解析成語法樹，樹上每
/// 個字面字都帶着它在原文裏的位置，只替換那幾段。於是 `.`、`*`、`\d`、`^$`、
/// 括號、`(?i)` 一律不碰。
///
/// Warning: **只認 `Verbatim`**——`\x{66F8}`、`\u66F8` 這種寫法不折。使用者特意用碼位
/// 寫出來的那一個字，說的就是那一個字。
///
/// Warning: **類裏的字面字照折。** `[書x]` → `[[書书]x]`，`regex` 認得嵌套的字符類（並
/// 集），所以這一步是安全的。
///
/// `None` ＝ 這個式子解析不了（多半是打了一半），呼叫方原樣用它就好——壞式子
/// 本來就有自己那條路（面板上畫灰）。
pub fn widen_pattern(pattern: &str) -> Option<String> {
    use regex_syntax::ast::{self, visit, Ast, Visitor};

    #[derive(Default)]
    struct Spots(Vec<(usize, usize, char)>);

    impl Spots {
        fn take(&mut self, lit: &ast::Literal) {
            if matches!(lit.kind, ast::LiteralKind::Verbatim) && !shapes(lit.c).is_empty() {
                self.0.push((lit.span.start.offset, lit.span.end.offset, lit.c));
            }
        }
    }

    impl Visitor for Spots {
        type Output = Vec<(usize, usize, char)>;
        type Err = ();
        fn finish(self) -> Result<Self::Output, ()> {
            Ok(self.0)
        }
        fn visit_post(&mut self, ast: &Ast) -> Result<(), ()> {
            if let Ast::Literal(lit) = ast {
                self.take(lit);
            }
            Ok(())
        }
        fn visit_class_set_item_post(&mut self, item: &ast::ClassSetItem) -> Result<(), ()> {
            if let ast::ClassSetItem::Literal(lit) = item {
                self.take(lit);
            }
            Ok(())
        }
    }

    let tree = ast::parse::Parser::new().parse(pattern).ok()?;
    let mut spots = visit(&tree, Spots::default()).ok()?;
    spots.sort_by_key(|spot| spot.0);
    let mut out = String::with_capacity(pattern.len() * 2);
    let mut at = 0usize;
    for (from, to, ch) in spots {
        // Warning: **這一道其實從來沒攔下過什麼**（2026-10-02 審出來的）。類裏的字面
        // 字不是 `Ast` 節點，`regex-syntax` 的訪問器只從
        // `visit_class_set_item_*` 那一條路交出它們，所以兩個鉤子收不到同一個
        // 字——十六種刁鑽寫法逐個instrument過，`dupe=false overlap=false`。
        // 留着是因為它一毛錢不值，而拆掉就等於賭那個訪問器將來不變。
        if from < at {
            continue;
        }
        out.push_str(&pattern[at..from]);
        out.push('[');
        out.push_str(&regex::escape(shapes(ch)));
        out.push(']');
        at = to;
    }
    out.push_str(&pattern[at..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **正則底下也折字形，而使用者寫的符號一個都不許動**（2026-10-01 定）。
    ///
    /// 從前 正則 和 簡繁異體 互斥，理由是整串改寫會把 `.`、`*`、`[` 吃掉。
    /// [`widen_pattern`] 先解析再只動字面字，於是兩個開關可以同時開。
    #[test]
    fn a_pattern_folds_only_the_characters_the_reader_typed() {
        let hay = "我在书斋里发呆，第2书斋。";
        for (pattern, want) in [
            ("書齋", true),
            ("書.*齋", true),
            ("^書齋$", false),
            ("書+", true),
            // 類裏的字面字也折，嵌套的字符類 `regex` 認得（並集）。
            ("[書x]", true),
            ("書|齋", true),
            ("(書)齋", true),
            (r"\d書", true),
            (r"\p{Han}書", true),
            ("發呆", true),
            // 沒有漢字就一個字節都不改。
            ("a.b", false),
        ] {
            let grown = widen_pattern(pattern).expect("解析得了");
            let re = regex::Regex::new(&grown).expect("編譯得過：{grown}");
            assert_eq!(re.is_match(hay), want, "{pattern} → {grown}");
        }
        assert_eq!(widen_pattern("a.b").as_deref(), Some("a.b"), "沒有漢字就原樣");
        assert_eq!(widen_pattern("書").as_deref(), Some("[書书]"));
        // Warning: **類裏也折，否定的類裏也折**——於是 `[^書]` **配得更少**：`书`
        // 從前配得上，開了中文匹配反而配不上（2026-10-02 定：照舊）。理由在
        // `find.rs` 的 `Field::Chinese`，一句話是「關掉那個開關是現成的出路，而
        // 類裏不折會讓 `[書]` 和 `書` 兩種寫法給兩個答案」。
        assert_eq!(widen_pattern("[^書]").as_deref(), Some("[^[書书]]"));
        assert_eq!(widen_pattern("[書]").as_deref(), Some("[[書书]]"));
        // Warning: **碼位寫法不折**：特意用 `\x{...}` 寫出來的那一個字，說的就是那一個。
        assert_eq!(widen_pattern(r"\x{66F8}").as_deref(), Some(r"\x{66F8}"));
        // 打了一半的式子解析不了，呼叫方原樣用它。
        assert_eq!(widen_pattern("[書"), None);
    }

    /// **當初舉的那個例子**：「書齋」搜得到「书斋」。
    #[test]
    fn a_traditional_query_finds_the_simplified_writing() {
        let re = regex::Regex::new(&widen("書齋")).unwrap();
        assert!(re.is_match("书斋"), "{}", widen("書齋"));
        assert!(re.is_match("書齋"));
        // 反過來也要成。
        let re = regex::Regex::new(&widen("书斋")).unwrap();
        assert!(re.is_match("書齋"));
    }

    /// Warning: **不對稱是有意的**：含混的那個放寬，精確的那個保持精確。
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
