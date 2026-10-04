//! **`go`／`gu` — 按「那裏寫的什麼」跳**（§5.73）。
//!
//! `gw` 給屏幕上的位置**發號碼**（easymotion／helix 那一路）；這一支反過來，**打你
//! 要去的那個地方寫的字**（leap／flash 那一路）。兩個不衝突，§5.12.63 當時就留了口子。
//!
//! 一條規矩管兩種文字：西文打字面那幾個字母，中文打**讀音**那幾個。判誰配得上在
//! [`crate::written`]，那一層是純文字進、位置出；這裏管的是「走到哪一步了」。
//!
//! | | 打幾個字母 | 收什麼 |
//! | --- | --- | --- |
//! | **`go`** | **定長兩個** | 中英混合 |
//! | **`gu`** | **不定長**，打到候選夠短 | 只有中文 |
//!
//! `go` 一格都不影響英文：字母照舊是標籤、按下去就跳；**要到中文那一邊走的是數字
//! 和 `-`／`=`**，兩個集合不重疊（2026-10-04 定）。`zh`／`sh`／`ji` 那幾個擁擠的
//! 聲母（量出來一屏 21–23 個字）歸 `gu`——它不定長，編碼一長候選就塌下來。
//!
//! # 要等一幀
//!
//! 和 `gw` 同病：落腳點只在**這一屏**上，而屏幕畫了哪一段是前端每幀交過來的
//! （`set_page_span`）。所以按鍵只記一筆欠着（[`Editor::start_seek`]），前端畫完一幀
//! 再回頭問。`--shot --keys` 那條路也走同一道門，不然離屏和真路徑又分家。

use super::*;

/// 這一次按「那裏寫的什麼」跳，走到哪一步了。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Seeking {
    /// `gu` ＝ 只問中文、不定長；`go` ＝ 定長兩個、中英都問。
    pub only_han: bool,
    /// 打進去的那幾個字母。
    pub typed: String,
    /// 這一屏上配得上的。
    pub found: crate::written::Targets,
    /// 候選面板翻到第幾頁（從 0 數）。
    pub page: usize,
    /// **落腳點和它們的標籤**，空着 ＝ 還在挑。
    ///
    /// Warning: **標籤就蓋在目標身上**，兩個字母、兩格——一個漢字正好，兩個西文字符
    /// 也正好，同 `gw`。先前擔心的是「蓋住了就看不出是哪個字」，可那隻在**候選面板
    /// 那一步**成立：到貼標籤這一步，`go` 的西文是你自己打的兩個字母，中文那一批
    /// 全是你剛按數字挑定的**同一個字**——都不必再看。
    pub labels: Vec<super::labels::Jump>,
    /// 標籤已經被打進去的那幾個字母。
    pub typed_label: String,
}

/// 候選面板一頁幾個 —— 數字鍵就那麼多（`1`–`9`）。
pub const PER_PAGE: usize = 9;

impl Seeking {
    /// 還收不收查詢字母。
    ///
    /// `go` 定長兩個——打滿就不收了，往後那幾鍵是標籤和數字的事。
    pub fn takes_more(&self) -> bool {
        self.only_han || self.typed.chars().count() < 2
    }

    /// 候選一共幾頁（一個字都沒有時是 1，空面板也是一頁）。
    pub fn pages(&self) -> usize {
        self.found.han.len().div_ceil(PER_PAGE).max(1)
    }

    /// 這一頁上的那幾個字。
    pub fn page_han(&self) -> &[(char, Vec<usize>)] {
        let from = (self.page * PER_PAGE).min(self.found.han.len());
        let to = (from + PER_PAGE).min(self.found.han.len());
        &self.found.han[from..to]
    }

    /// **面板上畫的那幾行。**
    ///
    /// 打進去的那幾個字母一行（帶頁碼，同搜索面板那個 `1/31`），一個字一行：
    /// 號碼、字、它在這一屏上出現幾處。
    ///
    /// Warning: **一行都沒有就交空的**——畫不畫由前端定，可「畫幾行」是這裏的事。
    /// 核心答內容、前端只管擺，和這個倉別處一樣。
    pub fn rows(&self) -> Vec<String> {
        if self.found.han.is_empty() {
            return Vec::new();
        }
        let mut rows = vec![match self.pages() > 1 {
            true => format!("{}   {}/{}", self.typed, self.page + 1, self.pages()),
            false => self.typed.clone(),
        }];
        for (n, (ch, at)) in self.page_han().iter().enumerate() {
            rows.push(format!("{}. {}  {}", n + 1, ch, at.len()));
        }
        rows
    }
}

impl Editor {
    /// `go`／`gu` 按下去：記一筆，等前端畫完一幀（見本檔開頭）。
    pub(super) fn start_seek(&mut self, only_han: bool) {
        self.owed_seek = Some(only_han);
    }

    /// 欠着沒有——問一句，不取走（`--shot` 那條路用它）。
    pub fn owes_a_seek(&self) -> bool {
        self.owed_seek.is_some()
    }

    /// 前端畫完一幀、交了範圍之後，回頭取走。
    pub fn take_owed_seek(&mut self) -> Option<bool> {
        self.owed_seek.take()
    }

    /// 開起來，等着打字。
    pub fn run_owed_seek(&mut self, only_han: bool) {
        self.seeking = Some(Seeking { only_han, ..Seeking::default() });
    }

    /// 正在按「那裏寫的什麼」跳。
    pub fn seeking(&self) -> Option<&Seeking> {
        self.seeking.as_ref()
    }

    /// 收攤。
    pub(super) fn cancel_seek(&mut self) {
        self.seeking = None;
    }

    /// **這一屏畫出來的那一段字**——配得上誰是拿它問的。
    fn page_text(&self) -> (usize, String) {
        let rope = self.current_buffer().rope();
        let (from, to) = self.page_span;
        let to = to.min(rope.len_chars());
        match from >= to {
            true => (0, String::new()),
            false => (from, rope.slice(from..to).to_string()),
        }
    }

    /// 重新問一遍這一屏上誰配得上。
    fn look_again_for_targets(&mut self) {
        let (from, text) = self.page_text();
        let Some(seeking) = self.seeking.as_ref() else { return };
        let mut found = crate::written::targets(&text, &seeking.typed);
        // **`gu` 只問中文。** 西文那一半歸 `go`——這一檔存在的理由就是「中文兩個
        // 字母收不住」，收進西文只會把候選面板攪渾。
        if seeking.only_han {
            found.latin.clear();
        }
        // 位置換成**檔裏**的下標：`written` 答的是相對於餵進去那一段的。
        for at in &mut found.latin {
            *at += from;
        }
        for (_, at) in &mut found.han {
            for one in at {
                *one += from;
            }
        }
        if let Some(seeking) = self.seeking.as_mut() {
            seeking.found = found;
        }
    }

    /// 這一步裏的一鍵。回 `true` ＝ 吃掉了。
    ///
    /// Warning: **不認得的鍵一律收攤並吃掉它自己**，同 `gw`（`labels.rs` 的 `jump_key`）：
    /// 「按錯一個鍵就跳到別處去」比「按錯一個鍵什麼都沒發生」壞得多。
    pub(super) fn seek_key(&mut self, key: Key) -> bool {
        let Some(seeking) = self.seeking.as_ref() else { return false };
        let Key::Char(c) = key else {
            self.cancel_seek();
            return true;
        };
        // **空格也是一個字母**（2026-10-04 問的）：「a bus」那個孤零零的 `a`
        // 後面就是空格，不收的話它永遠去不了。帶空格的查詢自己就不問讀音了
        // （沒有哪個字讀作「a 」），所以不必在這裏分。
        // **標籤亮起來之後整個鍵盤都是標籤**，同 `gw`。
        if !seeking.labels.is_empty() {
            return self.seek_label_key(c);
        }
        // **數字挑漢字**（`1`–`9`）。字母和數字是兩個不相交的集合，所以一個鍵同時
        // 答兩種文字不會含糊——這是整個設計的關竅（2026-10-04 定）。
        if let Some(n) = c.to_digit(10).filter(|&n| n >= 1) {
            if seeking.page_han().get(n as usize - 1).is_some() {
                self.pick_the_han(n as usize - 1);
                return true;
            }
            self.cancel_seek();
            return true;
        }
        // **翻頁用 `-`／`=`，寫死**（2026-10-04 定）。不跟方案走：輸入法的翻頁鍵
        // 由方案決定、各家不同，而跳轉必須可預測。
        if matches!(c, '-' | '=') && !seeking.found.han.is_empty() {
            let pages = seeking.pages();
            if let Some(seeking) = self.seeking.as_mut() {
                seeking.page = match c {
                    '-' => (seeking.page + pages - 1) % pages,
                    _ => (seeking.page + 1) % pages,
                };
            }
            return true;
        }
        let takes = c.is_ascii_alphabetic() || c == ' ';
        if !takes || !seeking.takes_more() {
            self.cancel_seek();
            return true;
        }
        if let Some(seeking) = self.seeking.as_mut() {
            seeking.typed.push(c.to_ascii_lowercase());
            // 名單整個換了，停在第三頁沒有意思。
            seeking.page = 0;
        }
        self.look_again_for_targets();
        // `go` 打滿兩個，西文那一批當場發標籤。
        self.label_the_latin();
        true
    }

    /// 面板上畫的那幾行——沒開着、挑過字了、或者一個漢字都沒配上，就是空的。
    pub fn seek_rows(&self) -> Vec<String> {
        self.seeking
            .as_ref()
            .filter(|s| s.labels.is_empty())
            .map(Seeking::rows)
            .unwrap_or_default()
    }

    /// **還畫得出來的那幾個標籤**，以及每個已經被打掉的字母數——同 `jump_labels`。
    pub fn seek_labels(&self) -> Vec<(usize, &str)> {
        let Some(seeking) = self.seeking.as_ref() else { return Vec::new() };
        seeking
            .labels
            .iter()
            .filter(|j| j.label.starts_with(&seeking.typed_label))
            .map(|j| (j.at, &j.label[seeking.typed_label.len()..]))
            .collect()
    }

    /// 西文那一批當場發標籤——`go` 打滿兩個字母就該看見它們。
    fn label_the_latin(&mut self) {
        let Some(seeking) = self.seeking.as_ref() else { return };
        if seeking.takes_more() || !seeking.labels.is_empty() {
            return;
        }
        let spots = seeking.found.latin.clone();
        if let Some(seeking) = self.seeking.as_mut() {
            seeking.labels = super::labels::label_them(spots);
            seeking.typed_label.clear();
        }
    }

    /// 按數字挑定一個漢字：面板收掉，那一個字的每一處拿一個標籤。
    fn pick_the_han(&mut self, nth: usize) {
        let Some(seeking) = self.seeking.as_ref() else { return };
        let Some((_, at)) = seeking.page_han().get(nth) else { return };
        let spots = at.clone();
        if let Some(seeking) = self.seeking.as_mut() {
            seeking.labels = super::labels::label_them(spots);
            seeking.typed_label.clear();
        }
    }

    /// 標籤亮着的時候這一鍵怎麼算。回 `true` ＝ 吃掉了。
    fn seek_label_key(&mut self, c: char) -> bool {
        if !c.is_ascii_alphabetic() {
            self.cancel_seek();
            return true;
        }
        let Some(seeking) = self.seeking.as_ref() else { return false };
        let mut typed = seeking.typed_label.clone();
        typed.push(c.to_ascii_lowercase());
        match seeking.labels.iter().find(|j| j.label == typed) {
            Some(j) => {
                let at = j.at;
                self.cancel_seek();
                // Warning: **記一筆，可是別挪版面**（2026-10-04）——落腳點本來就在屏幕
                // 上，挪走它這一跳就白跳。同 `gw`，見 `note_where_we_came_from`。
                self.note_where_we_came_from();
                self.move_head(at);
            }
            None if seeking.labels.iter().any(|j| j.label.starts_with(&typed)) => {
                if let Some(seeking) = self.seeking.as_mut() {
                    seeking.typed_label = typed;
                }
            }
            None => self.cancel_seek(),
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一個「已經畫過一幀」的編輯器：`page_span` 是前端交的，測試自己交。
    fn on_screen(text: &str, only_han: bool) -> Editor {
        let mut ed = Editor::new();
        ed.current_buffer_mut().replace(0..0, text).unwrap();
        let n = ed.current_buffer().rope().len_chars();
        ed.set_page_span(0, n);
        ed.start_seek(only_han);
        let want = ed.take_owed_seek().expect("按了就欠着");
        ed.run_owed_seek(want);
        ed
    }

    fn press(ed: &mut Editor, keys: &str) {
        for c in keys.chars() {
            ed.seek_key(Key::Char(c));
        }
    }

    /// **`go` 定長兩個字母，中英一起問。**
    #[test]
    fn go_asks_both_scripts_with_two_letters() {
        let mut ed = on_screen("在 dock 旁邊的冬天", false);
        press(&mut ed, "do");
        let found = &ed.seeking().expect("還開着").found;
        assert_eq!(found.latin, [2], "dock 的 do");
        assert_eq!(found.han, [('冬', vec![10])], "冬 讀 dong");
        // 打滿兩個就不收查詢字母了——往後那幾鍵是標籤和數字的事。
        assert!(!ed.seeking().unwrap().takes_more());
    }

    /// **`gu` 不定長，而且只問中文。**
    ///
    /// `zh`／`sh`／`ji` 一屏命中二十多個字，兩個字母收不住（量在 `written.rs`），
    /// 這一檔就是為它們開的。
    #[test]
    fn gu_keeps_narrowing_and_ignores_the_latin() {
        let mut ed = on_screen("dock 東邊冬天", true);
        press(&mut ed, "do");
        let found = &ed.seeking().unwrap().found;
        assert!(found.latin.is_empty(), "gu 不問西文");
        assert_eq!(found.han.len(), 2, "東 和 冬");
        assert!(ed.seeking().unwrap().takes_more(), "還收");
        // 再打一個字母收窄。
        press(&mut ed, "n");
        assert_eq!(ed.seeking().unwrap().found.han.len(), 2, "dong 兩個都還在");
    }

    /// 位置是**檔裏**的下標，不是這一屏那一段裏的。
    #[test]
    fn the_places_are_counted_from_the_start_of_the_file() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().replace(0..0, "前面一行\n在 dock 旁").unwrap();
        let n = ed.current_buffer().rope().len_chars();
        // 只畫第二行。
        ed.set_page_span(5, n);
        ed.start_seek(false);
        let want = ed.take_owed_seek().unwrap();
        ed.run_owed_seek(want);
        press(&mut ed, "do");
        assert_eq!(ed.seeking().unwrap().found.latin, [7], "檔裏第 7 個字符");
    }

    /// **空格也是一個字母**：「a bus」那個孤零零的 `a`。
    #[test]
    fn a_space_can_be_the_second_letter() {
        let mut ed = on_screen("a bus and a cat", false);
        press(&mut ed, "a ");
        assert_eq!(ed.seeking().unwrap().found.latin, [0, 10]);
    }

    /// Warning: **不認得的鍵收攤，而且吃掉它自己。** 同 `gw`——按錯一鍵跳到別處去，
    /// 比按錯一鍵什麼都不發生壞得多。
    #[test]
    fn a_key_that_means_nothing_here_just_closes_it() {
        let mut ed = on_screen("冬天", false);
        assert!(ed.seek_key(Key::Esc), "吃掉了");
        assert!(ed.seeking().is_none(), "收攤了");

        let mut ed = on_screen("冬天", false);
        assert!(ed.seek_key(Key::Char('!')));
        assert!(ed.seeking().is_none());

        // 打滿兩個之後再來一個字母，也是收攤（那時字母是標籤的事，還沒做）。
        let mut ed = on_screen("冬天", false);
        press(&mut ed, "do");
        assert!(ed.seeking().is_some());
        press(&mut ed, "n");
        assert!(ed.seeking().is_none());
    }

    /// **面板上畫什麼**：打的那幾個字母一行，一個字一行（號碼、字、出現幾處）。
    #[test]
    fn the_panel_lists_one_row_per_character() {
        let mut ed = on_screen("東邊冬天東風", true);
        press(&mut ed, "do");
        assert_eq!(ed.seek_rows(), ["do", "1. 東  2", "2. 冬  1"]);
    }

    /// 一個漢字都沒配上，面板就是空的——畫不畫是前端的事，可行數是這裏答的。
    #[test]
    fn no_character_means_no_panel() {
        let mut ed = on_screen("dock and dog", false);
        press(&mut ed, "do");
        assert!(!ed.seeking().unwrap().found.latin.is_empty(), "西文那一邊有");
        assert!(ed.seek_rows().is_empty(), "可漢字那一邊沒有，面板不畫");
    }

    /// **`-`／`=` 翻頁，寫死的兩個鍵**（不跟方案走——IME 的翻頁鍵各家不同，而跳轉
    /// 必須可預測）。頁碼帶在第一行上，同搜索面板那個 `1/31`。
    #[test]
    fn the_panel_pages_with_minus_and_equals() {
        // 十個讀 d 開頭的字，一頁放九個。
        let mut ed = on_screen("大地東冬都動斷到daodian燈等", true);
        press(&mut ed, "d");
        let seeking = ed.seeking().unwrap();
        assert!(seeking.found.han.len() > PER_PAGE, "湊得出兩頁：{:?}", seeking.found.han);
        assert_eq!(seeking.pages(), 2);
        assert!(ed.seek_rows()[0].ends_with("1/2"), "{:?}", ed.seek_rows()[0]);
        assert_eq!(ed.seek_rows().len(), 1 + PER_PAGE, "第一頁滿九個");

        press(&mut ed, "=");
        assert_eq!(ed.seeking().unwrap().page, 1);
        assert!(ed.seek_rows()[0].ends_with("2/2"));
        // 轉回去。
        press(&mut ed, "=");
        assert_eq!(ed.seeking().unwrap().page, 0);
        press(&mut ed, "-");
        assert_eq!(ed.seeking().unwrap().page, 1, "`-` 往回也轉得動");
    }

    /// 再打一個字母，頁碼歸零——名單整個換了，停在第二頁沒有意思。
    #[test]
    fn narrowing_goes_back_to_the_first_page() {
        let mut ed = on_screen("大地東冬都動斷到燈等", true);
        press(&mut ed, "d");
        press(&mut ed, "=");
        assert_eq!(ed.seeking().unwrap().page, 1);
        press(&mut ed, "o");
        assert_eq!(ed.seeking().unwrap().page, 0);
    }

    /// Warning: **`-`／`=` 不許把面板關掉**，可是一個漢字都沒有的時候它們不是翻頁鍵
    /// ——那時沒有頁可翻，照「不認得的鍵就收攤」辦。
    #[test]
    fn minus_closes_it_when_there_is_nothing_to_page() {
        let mut ed = on_screen("dock", false);
        press(&mut ed, "do");
        press(&mut ed, "-");
        assert!(ed.seeking().is_none());
    }

    /// **`go` 打滿兩個字母，西文那一批當場發標籤。**
    #[test]
    fn the_latin_gets_its_labels_as_soon_as_the_query_is_full() {
        let mut ed = on_screen("dock and a dog", false);
        press(&mut ed, "d");
        assert!(ed.seek_labels().is_empty(), "纔打一個，還在問");
        press(&mut ed, "o");
        assert_eq!(ed.seek_labels(), [(0, "aa"), (11, "ab")], "dock 和 dog");
    }

    /// **按數字挑定一個漢字，面板收掉，那一個字的每一處拿一個標籤。**
    ///
    /// 標籤就蓋在目標身上（兩個字母、兩格）——到這一步屏幕上那一批全是**同一個
    /// 字**，你剛挑的，不必再看。
    #[test]
    fn a_digit_picks_a_character_and_labels_every_place_it_sits() {
        let mut ed = on_screen("東邊冬天東風", true);
        press(&mut ed, "do");
        assert_eq!(ed.seek_rows(), ["do", "1. 東  2", "2. 冬  1"]);
        press(&mut ed, "1");
        assert!(ed.seek_rows().is_empty(), "挑完了，面板收掉");
        assert_eq!(ed.seek_labels(), [(0, "aa"), (4, "ab")], "東 的兩處");
    }

    /// 打標籤就跳過去，而且**版面不許挪**、`C-o` 回得來。
    #[test]
    fn typing_a_label_goes_there_without_moving_the_page() {
        let mut ed = on_screen("東邊冬天東風", true);
        press(&mut ed, "do1");
        press(&mut ed, "ab");
        assert!(ed.seeking().is_none(), "跳完收攤");
        assert_eq!(ed.selection().0, 4, "第二個東");
        assert!(!ed.jumped(), "落腳點本來就在屏幕上，版面不挪");
        ed.on_key(Key::Ctrl('o'));
        assert_eq!(ed.selection().0, 0, "回得來");
    }

    /// 打了一個字母：剩下以它開頭的那些，別的當場滅掉。
    #[test]
    fn one_letter_of_a_label_narrows_what_is_left() {
        let mut ed = on_screen("東邊冬天東風", true);
        press(&mut ed, "do1");
        assert_eq!(ed.seek_labels().len(), 2);
        press(&mut ed, "a");
        assert_eq!(ed.seek_labels(), [(0, "a"), (4, "b")], "剩下第二個字母");
    }

    /// 面板上沒有那一號，按下去就收攤——不是悄悄什麼都不做。
    #[test]
    fn a_digit_with_no_row_behind_it_closes_it() {
        let mut ed = on_screen("東邊冬天", true);
        press(&mut ed, "do");
        assert_eq!(ed.seek_rows().len(), 1 + 2, "兩個字");
        press(&mut ed, "7");
        assert!(ed.seeking().is_none());
    }

    /// 還沒畫過一幀就按——`page_span` 是空的，一處都找不到，可也不許炸。
    #[test]
    fn before_the_first_frame_there_is_simply_nothing() {
        let mut ed = Editor::new();
        ed.current_buffer_mut().replace(0..0, "冬天").unwrap();
        ed.start_seek(false);
        let want = ed.take_owed_seek().unwrap();
        ed.run_owed_seek(want);
        press(&mut ed, "do");
        assert!(ed.seeking().unwrap().found.is_empty());
    }
}
