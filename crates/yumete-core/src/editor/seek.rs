//! **`go`／`gu` — 按「那裏寫的什麼」跳**（§5.73）。
//!
//! `gw` 給屏幕上的位置**發號碼**（easymotion／helix 那一路）；這一支反過來，**打你
//! 要去的那個地方寫的字**（leap／flash 那一路）。兩個不衝突，§5.12.63 當時就留了口子。
//!
//! 一條規矩管兩種文字：西文打**字面**那幾個字母，中文打**讀音**那幾個。判誰配得上
//! 在 [`crate::written`]，那一層是純文字進、位置出，兩種一起答；這裏管的是「走到哪
//! 一步了」，並且**一個鍵只取其中一半**。
//!
//! | | 打幾個字母 | 收什麼 | 怎麼挑 |
//! | --- | --- | --- | --- |
//! | **`go`** | **定長兩個** | 只有西文 | 字母標籤，同 `gw` |
//! | **`gu`** | **不定長**，打到候選夠短 | 只有中文 | 數字挑字、`-`／`=` 翻頁，再貼標籤 |
//!
//! Warning: **一個鍵一種文字**（2026-10-04 定）。當天早先那一版是 `go` 兩種一起
//! 收——字母標籤和漢字候選面板同時畫出來，數字挑字、字母走標籤，兩個集合不重疊所以
//! 不含糊。可**面板落在光標處，正好蓋住光標附近那幾處匹配**，而那恰恰是人要跳去的
//! 地方。分開之後 `go` 根本不開面板，遮擋從源頭上沒有了；中文那一路也沒損失——`gu`
//! 不限長，想要兩個字母的那種速度，打兩個字母照樣按得到數字。
//!
//! `zh`／`sh`／`ji` 那幾個擁擠的聲母（量出來一屏 21–23 個字，量在 [`crate::written`]）
//! 歸 `gu`——它不定長，編碼一長候選就塌下來。
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
    /// `gu` ＝ 按**讀音**找漢字、不定長；`go` ＝ 按**字面**找西文、定長兩個。
    ///
    /// Warning: **一個鍵一種文字**（2026-10-04 定，改掉了當天早先那一版）。
    /// 先前 `go` 是「兩個字母同時問西文和中文」，候選面板和西文標籤一起畫在屏幕
    /// 上——而面板落在光標處，正好蓋住光標附近那幾處匹配。分開之後 `go` 根本不開
    /// 面板，遮擋這件事從源頭上沒有了；中文那一路也沒損失，`gu` 不限長，打兩個
    /// 字母照樣按得到數字。
    pub reading: bool,
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
    /// **已經按數字挑定了一個漢字。** 挑定之後候選面板就收掉，數字也不再有意思。
    ///
    /// Warning: **這個不等於「標籤亮着」。** `go` 打滿兩個字母的那一刻，西文的標籤
    /// 和漢字的候選面板**同時**在屏幕上——下一鍵是字母就去標籤，是數字就是在挑字
    /// （2026-10-04 定的那個關竅）。拿 `labels.is_empty()` 當「面板收了沒有」
    /// 會把這兩樣攪成一樣，`go` 的中文那一半就整個沒了。
    pub picked: bool,
}

/// 候選面板一頁幾個 —— 數字鍵就那麼多（`1`–`9`）。
pub const PER_PAGE: usize = 9;

impl Seeking {
    /// 還收不收查詢字母。
    ///
    /// `go` 定長兩個——打滿就不收了，往後那幾鍵是標籤和數字的事。
    pub fn takes_more(&self) -> bool {
        self.reading || self.typed.chars().count() < 2
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
            rows.push(format!("{}. {}  {}", n + 1, ch, say!("seek.places", at.len())));
        }
        rows
    }
}

impl Editor {
    /// `go`／`gu` 按下去：記一筆，等前端畫完一幀（見本檔開頭）。
    pub(super) fn start_seek(&mut self, reading: bool) {
        self.owed_seek = Some(reading);
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
    pub fn run_owed_seek(&mut self, reading: bool) {
        self.seeking = Some(Seeking { reading, ..Seeking::default() });
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
        // **一個鍵一種文字。** `gu` 扔掉西文，`go` 扔掉漢字——見 `Seeking::reading`
        // 上面那條為什麼。
        match seeking.reading {
            true => found.latin.clear(),
            false => found.han.clear(),
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
        // **出現得多的排前面**（2026-10-04 定）。`sort_by_key` 是穩定的，所以
        // 次數一樣的仍按在屏幕上頭一次出現的先後排——號碼小的那幾個離人的視線近。
        found.han.sort_by_key(|(_, at)| std::cmp::Reverse(at.len()));
        // **候選剩一個也不自動挑定**（2026-10-04 試過之後撤回）。做過一版，
        // 當天就撤了：讀音是前綴匹配，所以「只剩一個」隨時可能發生在人打完整個
        // 拼音之前——原話：「比如我想去『而』這個字，我打了 `e` 他就直接跳轉
        // 了，但是我其實習慣性會打 `er`，這時候 r 其實無效的。」那個多出來的字母
        // 配不上任何標籤，整件事當場收掉。**人打的是一個完整的讀音，不是一個剛好
        // 夠用的前綴**，界面不該在他話說完之前替他截斷。
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
        // **候選面板還開着的時候，數字和翻頁鍵歸面板**——標籤亮沒亮都一樣。
        // `go` 打滿兩個字母的那一刻兩樣東西同時在屏幕上，而數字和字母是兩個不相交
        // 的集合，所以一個鍵同時答兩種文字不會含糊（2026-10-04 定的關竅）。
        let panel = !seeking.picked && !seeking.found.han.is_empty();
        if panel {
            // **數字挑漢字**（`1`–`9`）。
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
            if matches!(c, '-' | '=') {
                let pages = seeking.pages();
                if let Some(seeking) = self.seeking.as_mut() {
                    seeking.page = match c {
                        '-' => (seeking.page + pages - 1) % pages,
                        _ => (seeking.page + 1) % pages,
                    };
                }
                return true;
            }
        }
        // **標籤亮起來之後字母都是標籤**，同 `gw`。
        if !seeking.labels.is_empty() {
            return self.seek_label_key(c);
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
            .filter(|s| !s.picked)
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
            seeking.picked = true;
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
    fn on_screen(text: &str, reading: bool) -> Editor {
        let mut ed = Editor::new();
        ed.current_buffer_mut().replace(0..0, text).unwrap();
        let n = ed.current_buffer().rope().len_chars();
        ed.set_page_span(0, n);
        ed.start_seek(reading);
        let want = ed.take_owed_seek().expect("按了就欠着");
        ed.run_owed_seek(want);
        ed
    }

    fn press(ed: &mut Editor, keys: &str) {
        for c in keys.chars() {
            ed.seek_key(Key::Char(c));
        }
    }

    /// **`go` 定長兩個字母，只問西文。**
    #[test]
    fn go_asks_the_latin_only() {
        let mut ed = on_screen("在 dock 旁邊的冬天", false);
        press(&mut ed, "do");
        let found = &ed.seeking().expect("還開着").found;
        assert_eq!(found.latin, [2], "dock 的 do");
        assert!(found.han.is_empty(), "冬 讀 dong，可那是 gu 的事");
        // 打滿兩個就不收查詢字母了——往後那幾鍵是標籤的事。
        assert!(!ed.seeking().unwrap().takes_more());
    }

    /// **`go` 一個候選面板都不開。**
    ///
    /// Warning: **這一條守的是「面板不許遮住光標附近」**（2026-10-04 定）。
    /// 面板落在光標處，而 `go` 要跳的那幾處正在光標附近——兩樣東西搶同一塊地方。
    /// 分成一鍵一種文字之後，`go` 這一路根本不開面板，遮擋從源頭上沒有了。
    #[test]
    fn go_never_opens_a_panel() {
        let mut ed = on_screen("在 dock 旁邊的冬天東風", false);
        press(&mut ed, "do");
        assert!(ed.seek_rows().is_empty(), "不開面板：{:?}", ed.seek_rows());
        assert_eq!(ed.seek_labels().len(), 1, "dock 那一處有標籤");
    }

    /// 同一屏上按字母走的是西文那一邊，和上面那一條是同一個狀態的另一半。
    #[test]
    fn a_letter_follows_the_latin_label_instead() {
        let mut ed = on_screen("在 dock 旁邊的冬天東風", false);
        press(&mut ed, "do");
        press(&mut ed, "aa");
        assert!(ed.seeking().is_none(), "跳完收攤");
        assert_eq!(ed.sel.head(), 2, "落在 dock 的 d 上");
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

    /// **候選剩一個也要按數字。**
    ///
    /// Warning: **這是回歸測試，守的是一條撤回來的設計**（2026-10-04）。做過一版
    /// 「只剩一個就自己挑定」，當天就撤了：讀音是前綴匹配，「只剩一個」隨時發生在
    /// 人打完整個拼音之前——想去「而」，打 `e` 就挑定了，而人習慣打 `er`，那個 `r`
    /// 配不上標籤，整件事收掉。人打的是一個完整的讀音，不是一個剛好夠用的前綴。
    #[test]
    fn the_last_character_standing_still_waits_for_a_digit() {
        // 這一句裏只有「都」讀 du。
        let mut ed = on_screen("冬天東風都城", true);
        press(&mut ed, "du");
        assert_eq!(ed.seek_rows().len(), 2, "查詢一行，「都」一行");
        assert!(ed.seek_labels().is_empty(), "沒有自己挑定");
        // 人照舊打得完整個讀音。
        press(&mut ed, "1");
        assert_eq!(ed.seek_labels().len(), 1, "按了數字纔貼標籤");
    }

    /// **出現得多的排前面**（2026-10-04 定）。次數一樣的照舊按出現先後。
    #[test]
    fn the_crowded_characters_come_first() {
        // 冬 一處、東 兩處：東 先出現，可冬 排在它前面是錯的。
        let mut ed = on_screen("冬天東風東雨", true);
        press(&mut ed, "do");
        let han: Vec<char> = ed.seeking().unwrap().found.han.iter().map(|(c, _)| *c).collect();
        assert_eq!(han, ['東', '冬'], "兩處的排在一處的前面");
        assert_eq!(ed.seek_rows(), ["do", "1. 東  共2處", "2. 冬  共1處"]);
    }

    /// **打到一半，屏幕上有兩處在說話：HUD 和命令行。**
    ///
    /// Warning: **這是回歸測試**（2026-10-04 報上來的）。按下 `o`／`u` 的那一刻
    /// `pending` 回到 `None`，於是 HUD 空了、`g` 那扇菜單也收了——屏幕上一個字
    /// 都不說話，而編輯器其實正等着人打字母。原話：「我以为我现在在 normal
    /// 模式，但其实 yumete 是在等我打拼音。這其實有些危險的。」
    ///
    /// **HUD 回顯整串按鍵（`gudon`），命令行說在等什麼（請輸入拼音：don）。**
    #[test]
    fn the_hud_and_the_command_row_both_speak_while_it_waits() {
        use crate::editor::Hint;
        let says = |ed: &Editor| match ed.hint() {
            Hint::Says(text) => text,
            _ => String::new(),
        };

        let mut ed = on_screen("冬天東風都城", true);
        assert_eq!(ed.typed_so_far(), "gu", "還沒打字母");
        assert_eq!(says(&ed), "請輸入拼音");
        press(&mut ed, "don");
        assert_eq!(ed.typed_so_far(), "gudon", "整串按鍵");
        assert_eq!(says(&ed), "請輸入拼音：don");

        let mut ed = on_screen("在 dock 旁邊", false);
        assert_eq!(ed.typed_so_far(), "go");
        assert_eq!(says(&ed), "請輸入兩個字母");
        press(&mut ed, "d");
        assert_eq!(ed.typed_so_far(), "god");
        assert_eq!(says(&ed), "請輸入兩個字母：d");
    }

    /// **標籤一亮，兩處都閉嘴**——那時屏幕上全是標籤，自己會說話。
    #[test]
    fn both_go_quiet_once_the_labels_are_up() {
        let mut ed = on_screen("在 dock 旁邊", false);
        press(&mut ed, "do");
        assert!(!ed.seek_labels().is_empty(), "標籤亮了");
        assert_eq!(ed.typed_so_far(), "", "HUD 閉嘴");
        assert!(matches!(ed.hint(), crate::editor::Hint::Quiet), "命令行閉嘴");
    }

    /// Warning: **`g` 那扇菜單不許在這時候畫。** 做過一版是「還在等字母就照
    /// `Pending::Goto` 畫」，當場被否：菜單上那些鍵這時候一個都按不了，而且 `gu`
    /// 的候選框一開，兩扇浮窗就疊在同一屏上（2026-10-04 截圖報的）。
    #[test]
    fn the_goto_menu_is_not_redrawn_while_it_waits() {
        let ed = on_screen("冬天東風都城", true);
        assert!(ed.pending_menu().is_none(), "不畫菜單");
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
        assert_eq!(ed.seek_rows(), ["do", "1. 東  共2處", "2. 冬  共1處"]);
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
        assert_eq!(ed.seek_rows(), ["do", "1. 東  共2處", "2. 冬  共1處"]);
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
