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
}

impl Seeking {
    /// 還收不收查詢字母。
    ///
    /// `go` 定長兩個——打滿就不收了，往後那幾鍵是標籤和數字的事。
    pub fn takes_more(&self) -> bool {
        self.only_han || self.typed.chars().count() < 2
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
        let takes = c.is_ascii_alphabetic() || c == ' ';
        if !takes || !seeking.takes_more() {
            self.cancel_seek();
            return true;
        }
        if let Some(seeking) = self.seeking.as_mut() {
            seeking.typed.push(c.to_ascii_lowercase());
        }
        self.look_again_for_targets();
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
