//! **這一屏上，哪些地方寫着這兩個字母** —— 按「目標處寫的什麼」跳的那一支的核心。
//!
//! vim 那邊這一族叫 leap/flash：不給屏幕上的位置發號碼（那是 easymotion，也就是
//! 這個倉的 [`gw`](crate::editor)），而是**打你要去的那個地方寫的字**。
//!
//! # 一條規矩管兩種文字
//!
//! 「打頭兩個字母」在西文裏就是字面那兩個字母；在中文裏是**讀音**的頭兩個字母。
//! 所以 `do` 一次問出兩件事：屏幕上的 `do…`，和屏幕上讀作 `do…` 的那些漢字
//! （冬、東、都、動…）。一條規矩，不是兩套機制拼起來——這是這個設計最值錢的地方。
//!
//! Warning: **讀音是現成的**（`crate::pinyin::readings`，一萬多條，編進二進制）。先前想
//! 的是問輸入法「這個字的編碼是什麼」，那要給 `yume-core` 加一支反查、跨倉跨機器；
//! 而且使用者換一個形碼方案，落腳點就全變了。讀音是字的性質，和方案無關。
//!
//! # 漢字按**字**分組交出去
//!
//! 不是交一串位置，是交「哪幾個字，各在哪幾處」。因為界面那一層要先讓人挑**是哪
//! 一個字**（候選面板，按數字選），挑完纔給那一個字的位置貼標籤。
//!
//! 這樣漢字本身**從頭到尾不會被標籤蓋住**——而那正是中文下非做不可的一件事：
//! 西文裏 `wi` 就是你打的那兩個字母，蓋掉也知道是什麼；中文裏 `do` 命中的是一堆
//! **不同的字**，那個字本身就是你要讀的信息（2026-10-04 指出的）。
//!
//! 順帶把標籤數壓下去了：從「一屏二十個不同的字」變成「冬 出現的那兩三處」。
//!
//! # 數字和字母是兩個不相交的集合
//!
//! 界面那一層靠這個分辨使用者要的是哪一類：打完兩個字母，西文那些當場貼字母標籤，
//! 漢字那些列進候選面板；**下一鍵是數字就是在挑漢字，是字母就是在按西文的標籤**。
//! leap 為「標籤字母會不會被當成第三個查詢字符」專門設計過一套「安全標籤」，這裏
//! 不需要——兩個集合本來就不重疊（2026-10-04 定）。

/// 這一屏上配得上的那些位置。
///
/// 位置是**字符下標**，相對於餵進來的那一段文字。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Targets {
    /// 西文那一類：那裏**字面**就寫着這幾個字母。
    ///
    /// 照屏幕順序。詞中間也算——leap 就是這個規矩，而「我看見那兩個字母在那裏」
    /// 說的本來就不是詞首。
    pub latin: Vec<usize>,
    /// 漢字那一類：**哪個字，在哪幾處**，照那個字頭一次出現的先後排。
    ///
    /// Warning: **按出現先後，不按出現次數。** 次數排序讀者預測不了——他正看着屏幕上
    /// 某一個字，面板的次序要跟他的眼睛走。
    pub han: Vec<(char, Vec<usize>)>,
}

impl Targets {
    /// 一處都沒有。
    pub fn is_empty(&self) -> bool {
        self.latin.is_empty() && self.han.is_empty()
    }

    /// 一共多少處（漢字那一邊按位置數，不按字數）。
    pub fn count(&self) -> usize {
        self.latin.len() + self.han.iter().map(|(_, at)| at.len()).sum::<usize>()
    }
}

/// **這一段文字裏，哪些地方寫着 `query`。**
///
/// `query` 是打進來的那幾個字母（通常兩個，可是一個也答得了——界面那一層每敲一鍵
/// 問一次，好讓命中隨着打字收窄）。大小寫不分，走的是
/// [`crate::nearby::alike`]，和搜索面板、挑選器同一支。
///
/// Warning: **一個位置只算一類。** 漢字不可能是 ASCII，兩邊天然不重疊。
///
/// Warning: **查詢裏有一個不是 ASCII 字母的，漢字那一路就不問了。** 讀音是字母串，
/// 拿「冬」去問讀音沒有意義；西文那一路照舊按字面配，所以 `ye --files` 那種
/// 混寫在這裏不適用——這一支要的就是「兩個字母」。
pub fn targets(text: &str, query: &str) -> Targets {
    let mut found = Targets::default();
    if query.is_empty() {
        return found;
    }
    let hay: Vec<char> = text.chars().collect();
    let needle: Vec<char> = query.chars().collect();
    // 讀音是小寫字母串，所以查詢不全是 ASCII 字母的時候那一路直接不走。
    let says = query.chars().all(|c| c.is_ascii_alphabetic());
    let said: String = query.to_ascii_lowercase();
    // 同一個字只記一次，位置攢在一起；`Vec` 而不是 `HashMap`，因為要的就是**出現
    // 先後**那個次序，而且一屏上不同的字至多幾十個。
    let mut seen: Vec<char> = Vec::new();
    for (at, &ch) in hay.iter().enumerate() {
        if says && crate::pinyin::readings(ch).any(|r| r.starts_with(&said)) {
            // **多音字全收**（長 ＝ cháng/zhǎng）。收窄是使用者下一鍵的事，
            // 這裏少收一個就是一處按不到的地方。
            match seen.iter().position(|&c| c == ch) {
                Some(k) => found.han[k].1.push(at),
                None => {
                    seen.push(ch);
                    found.han.push((ch, vec![at]));
                }
            }
            continue;
        }
        if hay.len() - at >= needle.len()
            && needle
                .iter()
                .enumerate()
                .all(|(k, &want)| crate::nearby::alike(want, hay[at + k], true, false))
        {
            found.latin.push(at);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn han(text: &str, query: &str) -> Vec<(char, Vec<usize>)> {
        targets(text, query).han
    }

    /// **一條規矩管兩種文字**：西文配字面，漢字配讀音。
    #[test]
    fn two_letters_ask_the_latin_and_the_han_at_once() {
        let found = targets("在 dock 旁邊的冬天", "do");
        // `dock` 的 `do` 在第 2 個字符。
        assert_eq!(found.latin, [2]);
        // 「冬」讀 dōng。「東」不在這一句裏。
        assert_eq!(found.han, [('冬', vec![10])]);
    }

    /// 漢字按**字**分組，照頭一次出現的先後——界面要拿它畫候選面板。
    #[test]
    fn the_han_come_grouped_by_character_in_reading_order() {
        // 東(dong) 冬(dong) 東(dong)：兩個字，三處。
        assert_eq!(
            han("東邊冬天東風", "do"),
            [('東', vec![0, 4]), ('冬', vec![2])]
        );
        assert_eq!(targets("東邊冬天東風", "do").count(), 3);
    }

    /// **多音字全收。** 長 讀 cháng 也讀 zhǎng，兩個都該找得到它。
    #[test]
    fn a_character_answers_to_every_reading_it_has() {
        assert_eq!(han("長江", "ch"), [('長', vec![0])]);
        assert_eq!(han("長大", "zh"), [('長', vec![0])]);
    }

    /// 大小寫不分，和搜索面板、挑選器同一條規矩（`nearby::alike`）。
    #[test]
    fn case_does_not_matter_on_the_latin_side() {
        assert_eq!(targets("Dock and dock", "do").latin, [0, 9]);
        assert_eq!(targets("Dock and dock", "DO").latin, [0, 9]);
    }

    /// 詞中間也算 —— leap 的規矩。「我看見那兩個字母在那裏」說的不是詞首。
    #[test]
    fn a_match_inside_a_word_counts() {
        assert_eq!(targets("window", "nd").latin, [2]);
    }

    /// 打一個字母也答得了：界面每敲一鍵問一次，命中跟着收窄。
    #[test]
    fn one_letter_is_a_question_too() {
        assert_eq!(targets("冬天東風", "d").han, [('冬', vec![0]), ('東', vec![2])]);
        assert_eq!(targets("冬天東風", "do").han, [('冬', vec![0]), ('東', vec![2])]);
        assert_eq!(targets("冬天東風", "don").han, [('冬', vec![0]), ('東', vec![2])]);
        // `dongt` 不是任何一個字的讀音開頭。
        assert!(targets("冬天東風", "dongt").is_empty());
    }

    /// Warning: **查詢不全是 ASCII 字母，就不問讀音了。** 讀音是字母串。
    #[test]
    fn a_query_that_is_not_letters_asks_nothing_of_the_readings() {
        assert!(han("冬天", "d1").is_empty());
        assert!(han("冬天", "冬").is_empty());
        // 西文那一路照舊按字面配。
        assert_eq!(targets("a1b a1c", "a1").latin, [0, 4]);
    }

    /// **空格也是一個字母** —— 「a bus」那個孤零零的 `a` 怎麼去（2026-10-04 問的）。
    ///
    /// 定長兩個字母，而那個 `a` 後面就是空格，所以第二鍵只能是空格。leap 也是這樣
    /// ——它要的是「目標處寫的那兩個字符」，空格是字符。
    ///
    /// Warning: **查詢裏有空格，讀音那一路就不問了**（`says` 為假）：讀音是字母串，
    /// 沒有一個字讀作「a 」。
    #[test]
    fn a_space_is_one_of_the_two_letters() {
        let found = targets("a bus and a cat", "a ");
        assert_eq!(found.latin, [0, 10], "兩個孤零零的 a");
        assert!(found.han.is_empty(), "帶空格的查詢不問讀音");
        // 「and」裏那個 a 後面是 n，不中。
        assert_eq!(targets("and", "a ").latin, Vec::<usize>::new());
        // 行尾那一個：後面沒有空格了，不中——它本來就沒有第二個字符可打。
        assert_eq!(targets("a", "a ").latin, Vec::<usize>::new());
    }

    #[test]
    fn nothing_asked_is_nothing_found() {
        assert!(targets("冬天", "").is_empty());
        assert!(targets("", "do").is_empty());
    }
}

#[cfg(test)]
mod measure {
    /// **量一量：一屏中文正文，幾個字母命中幾個不同的字。**
    ///
    /// 2026-10-04 跑出來的數推翻了「定長兩個字母」那個前提：
    ///
    /// ```text
    /// do    →  4 個不同的字、 10 處
    /// zh    → 21 個不同的字、 43 處      ← 候選面板要翻三頁
    /// sh    → 21 個不同的字、 81 處
    /// ji    → 23 個不同的字、 72 處
    /// zho   →  4 個不同的字、  9 處      ← 第三個字母一加就塌下來
    /// ```
    ///
    /// `zh`/`sh`/`ji` 是最常見的那幾個聲母組合，兩個字母根本收不住。所以中文
    /// 這一邊要**邊打邊收窄**，像輸入法那樣——而那本來就是打中文的人熟的節奏
    /// （搜索裏打的也是 `zhongguo` 不是 `zh`）。
    ///
    /// 留着是為了**重跑得了**：換一張讀音表、換一段語料，數都會變。
    #[test]
    #[ignore = "量數用的，不是斷言"]
    fn how_crowded_is_a_screen() {
        // 從倉根跑也好、從 crate 跑也好，找不到就不量——它不是一條斷言。
        let Some(all) = ["../../docs/manual_tc.md", "docs/manual_tc.md"]
            .iter()
            .find_map(|p| std::fs::read_to_string(p).ok())
        else {
            println!("找不到 docs/manual_tc.md，跳過");
            return;
        };
        // 一屏約四十行。
        let screen: String = all.lines().skip(600).take(40).collect::<Vec<_>>().join("\n");
        for q in ["do", "zh", "sh", "ji", "yi", "w", "zho"] {
            let t = super::targets(&screen, q);
            println!(
                "{q:5} → {:2} 個不同的字、{:3} 處漢字；西文 {:2} 處",
                t.han.len(),
                t.han.iter().map(|(_, a)| a.len()).sum::<usize>(),
                t.latin.len()
            );
        }
    }
}
