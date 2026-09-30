//! **「信息」——光標底下這個東西是什麽**（#426，2026-09-30 定）。
//!
//! 五種內容（字典、百科、數據、文檔、診斷），**一個槽**，兩個容器（浮窗、邊欄）。
//! 作者的原話：
//!
//! > 右侧栏和浮窗可以完全做成这样的特征：它永远只有一个信息可以即时显示，其他的
//! > 都必须手动触发，包括：表格、百科、文档、诊断……换句话说，右侧栏就是固定的
//! > 「浮窗」，对于这几类信息，它开着，浮窗就不用开了。他如果是空的，就显示
//! > 「信息」标题。如果不是空的，就显示对应的标题。
//!
//! Warning: **這一整個模組只回答問題，不存答案。** 從前這五樣是三扇常駐面板加兩扇
//! 臨時面板，於是「擺哪一種」存在 `spot_chosen`、「畫在哪」存在 `docs_follow`／
//! `problems_follow`／`dictionary_afloat`／`hover_afloat`——同一件事有兩三份記錄，
//! 而 bug 全是那幾份對不上。四次同一族的錯報之後作者說：「有没有可能是模型（逻辑）
//! 的问题，如果代码正确，不应该会这样……你不要只解决特例，这样永远都无法从根本上
//! 解决问题。你需要看看是不是解耦。」對的——所以現在只剩兩格狀態，別的一律算：
//!
//! | 問題 | 誰答 | 存嗎 |
//! | --- | --- | --- |
//! | 擺哪一種 | [`Editor::info_now`] | 不存 |
//! | 即時的是哪一種 | [`Editor::info_live`] | 只存 `:info` 的覆蓋 |
//! | 手動叫的是哪一種 | `info_asked` | **存**，光標一走就作廢 |
//! | 畫在哪 | [`Editor::info_afloat`] ／ [`Editor::info_in_the_sidebar`] | 不存 |
//! | 有沒有東西可畫 | [`Editor::info_has_body`] | 不存 |

use super::*;
use crate::sidebar::{Info, Side, View};

impl Editor {
    /// **這一份稿子即時顯示哪一種**（#426）。
    ///
    /// 原話：「散文里是百科，代码里是诊断或者文档（我不确定哪个实用），表格是
    /// 『数据』。」診斷是代碼那一格的出廠值（作者選的：「診斷（推荐）」）——它
    /// 是內存裏現成的，不花什麽；文檔要問服務器，一來一回，所以要 `:info docs`
    /// 明說纔即時顯示。
    ///
    /// Warning: **永遠有一種。** 作者定的模型是「它永远只有一个信息可以即时显示」
    /// ——沒有「都關掉」這一檔，所以這一支不回 `Option`。
    pub fn info_live(&self) -> Info {
        if let Some(one) = self.info_live {
            return one;
        }
        if self.detail_is_a_panel() {
            return Info::Data;
        }
        match self.writes_code() {
            true => Info::Problems,
            false => Info::Wiki,
        }
    }

    /// **此刻那一格擺的是哪一種**，`None` ＝ 什麽都沒有（標題寫「信息」）。
    ///
    /// 手動叫的那一種蓋過即時的那一種，而它光標一走就作廢——那一則是問出來的，
    /// 問題已經過去了。即時的那一種要有東西纔畫。
    pub fn info_now(&self) -> Option<Info> {
        if let Some(one) = self.info_asked_now() {
            return Some(one);
        }
        let one = self.info_live();
        self.info_has_body(one).then_some(one)
    }

    /// 手動叫的那一種，還算不算數。
    ///
    /// Warning: **鍵在那一格裏的時候光標本來就不動**，所以「光標還在原處」這一條
    /// 對它不成立——長的一則讀到一半，判準要換成「你正在讀它」。
    fn info_asked_now(&self) -> Option<Info> {
        let (one, at) = self.info_asked?;
        (at == self.sel.head() || self.reading_the_info()).then_some(one)
    }

    /// 鍵此刻是不是就在那一格裏。
    pub(super) fn reading_the_info(&self) -> bool {
        self.info_in_the_sidebar().is_some_and(|side| self.panel_focus == Some(side))
    }

    /// **那一種此刻有沒有東西可畫**（#426）。
    ///
    /// 五個內容各有各的來源，這裏只問它們一句同樣的話。Warning: 這一支**不問光標在
    /// 不在原處**——那是 [`Editor::info_asked_now`] 的事，兩件事分開問纔不會又
    /// 絞成一團。
    pub fn info_has_body(&self, one: Info) -> bool {
        match one {
            Info::Dictionary => self.dictionary.is_some(),
            Info::Wiki => self.wiki_here().is_some(),
            Info::Data => self.detail_is_a_panel(),
            Info::Docs => self.hover_here().is_some(),
            Info::Problems => self.problem_here().is_some(),
        }
    }

    /// **那一格開在哪一側**，`None` ＝ 沒開（那就浮）。
    ///
    /// 這一支就是「容器」那一半：邊欄開着，浮窗就不用開了。
    pub fn info_in_the_sidebar(&self) -> Option<Side> {
        self.showing(View::Info)
    }

    /// **此刻該浮哪一種**，`None` ＝ 不浮。
    ///
    /// Warning: **邊欄開着就一個字都不浮。** 兩個地方畫同一句話是作者報過的
    /// （「浮窗不应该和边栏同时出现」），而這一支和 [`Editor::info_in_the_sidebar`]
    /// 問的是同一件事的兩面，所以它們不可能對不上。
    pub fn info_afloat(&self) -> Option<Info> {
        self.info_in_the_sidebar().is_none().then(|| self.info_now()).flatten()
    }

    /// **手動叫一種出來**——`空格 k`／`空格 i`／`空格 d`／`t i` 都走這一句。
    ///
    /// `afloat` ＝ 小寫那一下：只在那一格沒開的時候纔浮。大寫那一下先把那一格開
    /// 出來（鍵不交過去），開了之後「畫在哪」自己就答對了，不必再記一個字段。
    ///
    /// 回 `false` ＝ 這一下是**收起來**（再按一次同一個鍵）。
    pub(super) fn ask_for_info(&mut self, one: Info, afloat: bool) -> bool {
        // 再按一次同一個鍵就收起來——此刻擺的就是它，而且畫在這一鍵要的那個
        // 地方。Warning: 判準要連容器一起看：浮着的時候按 `空格 K`，說的是「搬進邊
        // 欄」，不是「關掉」。
        let here = self.info_in_the_sidebar().is_none();
        if self.info_asked_now() == Some(one) && here == afloat {
            self.info_asked = None;
            self.stop_showing_this_info(one);
            self.refresh_sidebar();
            return false;
        }
        self.put_this_info_here(one, afloat);
        true
    }

    /// **叫一種出來，不收起**（#426）。
    ///
    /// [`Editor::ask_for_info`] 減去「再按一次就收起來」那一半。換一個**新的**
    /// 題目走這一支：問的是另一個字、另一個名字，那是換一份答案，不是關窗——
    /// 走帶 toggle 的那一支會把它當成「又按了一次」，當場關掉
    /// （2026-09-30 測出來的）。
    pub(super) fn put_this_info_here(&mut self, one: Info, afloat: bool) {
        if !afloat && self.info_in_the_sidebar().is_none() {
            let side = self.side_for(View::Info);
            self.open_panel_without_the_keys(side, View::Info);
        }
        self.info_asked = Some((one, self.sel.head()));
    }

    /// 收起來的時候，那一種自己的那份內容也丟掉。
    ///
    /// Warning: **不丟的話它還在**，於是即時那一輪又把它畫回來——「按一次關掉」
    /// 就成了「閃一下」。
    pub(super) fn stop_showing_this_info(&mut self, one: Info) {
        match one {
            Info::Dictionary => {
                self.dictionary = None;
                self.dictionary_query = None;
                self.dictionary_anchor = None;
            }
            Info::Docs => self.hovered = None,
            Info::Data => self.show_detail = Some(false),
            // 百科與診斷是稿子自己的事實，沒有一份「答案」可丟——收起來就是不
            // 再叫它，即時那一輪要不要畫是另一回事。
            Info::Wiki | Info::Problems => {}
        }
    }

    /// **`PageUp`／`PageDown`：在五種之間翻頁**（2026-09-30 定）。
    ///
    /// 原話：「既然这几个面板要么在浮窗要么在右边栏，我们就可以用 page up /
    /// page down 来对这五类进行翻页了」「这样，光标就在编辑区，也可以对五类信息
    /// 进行翻页。」Warning: **光標留在正文**——這是這兩個鍵存在的全部理由，所以它
    /// 們既不開邊欄也不搶鍵。
    ///
    /// Warning: **只停在有東西的那幾種上。** 五種裏能同時成立的通常只有一兩種
    /// （散文裏沒有診斷，代碼裏沒有百科），逐格翻過空的等於按五下什麽都不變。
    /// 一種都沒有就什麽都不做，回 `false`——那一鍵讓給正文。
    pub(super) fn page_the_info(&mut self, forward: bool) -> bool {
        let all = Info::ALL;
        let here = self.info_now();
        // 從此刻這一種的下一格起走一圈；沒擺着東西就從頭找。
        let from = here.and_then(|one| all.iter().position(|&x| x == one)).map_or(0, |at| {
            match forward {
                true => at + 1,
                false => at + all.len() - 1,
            }
        });
        let step = |n: usize| match forward {
            true => (from + n) % all.len(),
            false => (from + all.len() * all.len() - n) % all.len(),
        };
        let Some(want) = (0..all.len()).map(|n| all[step(n)]).find(|&one| {
            Some(one) != here && self.info_has_body(one)
        }) else {
            return false;
        };
        self.info_asked = Some((want, self.sel.head()));
        self.info_scroll = 0;
        self.status = say!("info.now-showing", crate::messages::say(want.tag(), &[]));
        self.refresh_sidebar();
        true
    }

    /// **`:info <名>`：換即時顯示的那一種**（#426）。
    ///
    /// `None` ＝ `:info` 光打了名字以外的什麽都沒有，回到按稿子算。
    pub fn set_info_live(&mut self, one: Option<Info>) {
        self.info_live = one;
        // 撥一次開關是一次新的意思，蓋過上一次按鍵叫出來的那一種。
        self.info_asked = None;
        self.info_scroll = 0;
        // 文檔那一種要問服務器，開的那一刻就起錶——不然第一問要等到光標動過一次
        // 纔算數。
        self.docs_asked_at = None;
        self.docs_moved = Some(std::time::Instant::now());
        self.status = match one {
            Some(one) => say!("info.live-is", crate::messages::say(one.tag(), &[])),
            None => say!("info.live-by-the-file", crate::messages::say(self.info_live().tag(), &[])),
        };
        self.refresh_sidebar();
    }

    /// 即時顯示的那一種是不是 `:info` 指定的（狀態欄與 `:info` 自己要問）。
    pub fn info_live_chosen(&self) -> Option<Info> {
        self.info_live
    }
}
