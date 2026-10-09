//! The sidebar, the pickers, and the clipboard (#94).
//!
//! One column down the side showing files, buffers, the outline or the
//! dictionary; the pickers that open over it; and the copy/paste that the
//! front end has to be asked for, because a terminal cannot read the
//! clipboard by itself.

use super::*;

impl Editor {
    // ---- The side panels (Feature #94, #293) -------------------------------

    /// **Which slot a panel lives in — the one place that decides it** (#293).
    ///
    /// One answer per panel, because a reader may want the outline across from
    /// the tree, or the 字典 stacked under it. Warning: **`Tab` then walks only the
    /// views that share a slot** ([`Editor::cycle_view`]): the motion belongs
    /// to the column, not to the list of views.
    pub fn side_of(&self, panel: crate::sidebar::Panel) -> crate::sidebar::Side {
        self.sides[panel as usize]
    }

    /// Put that panel on that side.
    ///
    /// Whatever is already open moves with it, because a panel that stayed
    /// where the old setting put it would make the setting a lie until the
    /// next restart. A slot that was busy hands what it held back to the slot
    /// this one just left, so nothing is silently closed.
    pub fn set_side(&mut self, panel: crate::sidebar::Panel, side: crate::sidebar::Side) {
        use crate::sidebar::View;
        let was = self.side_of(panel);
        self.sides[panel as usize] = side;
        if was == side {
            return;
        }
        // A panel that is showing goes across; what is *in* the 信息 slot has
        // nothing to carry, since it is worked out afresh every frame (#426).
        match View::ALL.into_iter().find(|&v| crate::sidebar::Panel::from(v) == panel) {
            Some(view) if self.showing(view) == Some(was) => {
                let moving = self.panels[was as usize].take();
                let focused = self.panel_focus == Some(was);
                let displaced = self.panels[side as usize].take();
                self.panels[side as usize] = moving;
                self.panels[was as usize] = displaced;
                if focused {
                    self.panel_focus = Some(side);
                }
            }
            Some(_) => {}
            // Warning: **只有這一格自己正拿着鍵的時候，鍵纔跟過去。** 從前的條件是
            // 「焦點在 `was` 那一側」——`Layer` 在的時候那等於「在下層」，沒了
            // 之後它也可能是**那一側的常駐面板**。於是
            // `:panel-dictionary left` 會把鍵從右邊的百科裏拽走
            // （2026-09-23 審出來的）。
            None if self.panel_focus == Some(was) && Self::view_of(panel).is_some() => {
                self.panel_focus = Some(side)
            }
            None => {}
        }
        self.refresh_sidebar();
    }

    /// Which slot a view opens in.
    pub(super) fn side_for(&self, view: crate::sidebar::View) -> crate::sidebar::Side {
        self.side_of(view.into())
    }

    /// `Tab` in a slot: the next view **that lives in this slot**, wrapping.
    ///
    /// A slot with one view in it has nowhere to go, and says so rather than
    /// looking broken.
    pub(super) fn cycle_view(&mut self, side: crate::sidebar::Side, back: bool) {
        let Some(here) = self.panel(side).map(|p| p.view()) else {
            return;
        };
        let mine: Vec<crate::sidebar::View> = crate::sidebar::View::ALL
            .into_iter()
            .filter(|&v| self.side_for(v) == side)
            .collect();
        if mine.len() < 2 {
            self.status = say!("sidebar.only-view-on-this-side");
            return;
        }
        let at = mine.iter().position(|&v| v == here).unwrap_or(0);
        let n = mine.len();
        let next = mine[match back {
            true => (at + n - 1) % n,
            false => (at + 1) % n,
        }];
        if let Some(panel) = self.panel_mut(side) {
            panel.show(next);
        }
        self.refresh_sidebar();
    }

    /// The panel in that slot, for the front end to draw.
    pub fn panel(&self, side: crate::sidebar::Side) -> Option<&crate::sidebar::Sidebar> {
        self.panels[side as usize].as_ref()
    }

    /// The same, to be moved about in.
    pub(super) fn panel_mut(
        &mut self,
        side: crate::sidebar::Side,
    ) -> Option<&mut crate::sidebar::Sidebar> {
        self.panels[side as usize].as_mut()
    }

    /// **這一側的邊欄此刻擺着哪一種信息**，`None` ＝ 那一格不是信息、或者它空着。
    ///
    /// Warning: **它只問，不存**（#426）。從前這裏答的是「光標頂上來的那一層」，
    /// 而常駐那一層另有一份記錄——兩份記錄對不上就是那四次 bug。現在信息是一
    /// 扇面板，裏面擺哪一種由 [`Editor::info_now`] 算，這一支只是把「那一格是
    /// 信息嗎」和「該擺哪一種」兜在一起。
    pub fn info_in_this_sidebar(&self, side: crate::sidebar::Side) -> Option<crate::sidebar::Info> {
        (self.panel(side).map(|p| p.view()) == Some(crate::sidebar::View::Info))
            .then(|| self.info_now())
            .flatten()
    }

    /// **光標走開了、鍵也不在它身上，那一份字典就不再作數**——當場丟掉。
    ///
    /// 原話：「走出之后回到这个字母，它是不是不应该出现了？」對的：那一則是問
    /// 出來的，問題已經過去了。同
    /// [`Editor::forget_a_hover_nobody_is_looking_at`]，在派鍵**之前**掃。
    pub(super) fn forget_a_dictionary_nobody_is_reading(&mut self) {
        if self.dictionary.is_none() || self.dictionary_live() {
            return;
        }
        self.dictionary = None;
        self.dictionary_query = None;
        self.dictionary_anchor = None;
    }

    /// **字典那一問還算不算數**（#215）。
    ///
    /// 光標還在問的那個字上就算，鍵在那一格裏也算——那時光標本來就不動，而一份
    /// 二十八欄的拆分表要讀得到底。
    fn dictionary_live(&self) -> bool {
        if self.dictionary.is_none() {
            return false;
        }
        self.reading_the_info() || self.dictionary_anchor == Some(self.sel.head())
    }

    /// **信息那一格畫成一列一列的時候，那些列**（#426）。
    ///
    /// 五種裏有三種是成對的字段（字典、記錄）或成行的句子（診斷、文檔），畫法
    /// 同一個；百科走自己那一支（它是排過版的散文）。
    pub fn info_rows(&self, side: crate::sidebar::Side) -> Vec<crate::sidebar::Row> {
        match self.info_in_this_sidebar(side) {
            Some(crate::sidebar::Info::Dictionary) => self.dictionary_rows(),
            Some(crate::sidebar::Info::Docs) => self.hover_rows(),
            Some(crate::sidebar::Info::Problems) => self.problem_rows(),
            _ => Vec::new(),
        }
    }

    /// Whether the detail is the sort that wants a **panel** rather than the
    /// floating note (#294).
    ///
    /// A row has twenty-eight fields and is a tall thing wherever it is
    /// written; a footnote is one short paragraph, and taking a slot off the
    /// page for it would be paying the wrong price — it floats over the
    /// writing instead, near the cursor.
    pub(super) fn detail_is_a_panel(&self) -> bool {
        self.detail_visible()
            && (self.detail_shows_a_row()
                || self.table.as_ref().is_some_and(|view| view.takes_the_pane()))
    }

    /// Whether that layer of that slot has anything in it to look at.
    /// **這個邊欄裏此刻有没有東西**（2026-09-22：一個邊欄一個面板）。
    pub fn slot_showing(&self, side: crate::sidebar::Side) -> bool {
        self.panel(side).is_some()
    }

    /// Which slot and layer the keys are in, if any.
    ///
    /// `None` when they are in the text — and also when what the focus names
    /// has since gone away, so a stale focus can never be reported as a live
    /// one. That second half is what lets the bottom layer be derived: it
    /// vanishes when the cursor moves off, and the keys fall back to the text
    /// without anybody having to put them there.
    pub fn panel_focus(&self) -> Option<crate::sidebar::Side> {
        let side = self.panel_focus?;
        self.slot_showing(side).then_some(side)
    }


    /// Which slot is showing that view, if either is.
    pub(super) fn showing(&self, view: crate::sidebar::View) -> Option<crate::sidebar::Side> {
        crate::sidebar::Side::BOTH
            .into_iter()
            .find(|&side| self.panel(side).is_some_and(|p| p.view() == view))
    }

    /// What `Space f` and `Space o` do — one rule for both, so neither is the
    /// odd one out.
    ///
    /// A key that names a view answers three different intentions depending on
    /// what is already showing, and all three are what a reader means by
    /// pressing it:
    ///
    /// - nowhere → open it in its own slot, with the keys.
    /// - showing, without the keys → take the keys back.
    /// - showing, with the keys → put it away. Pressing the same key twice
    ///   undoes it, which is the one thing every toggle must do.
    /// - its slot busy with **another** view → switch that slot to this view
    ///   and take the keys. The key means "show me the outline", not "toggle
    ///   the panel".
    pub(super) fn show_sidebar(&mut self, view: crate::sidebar::View) {
        if let Some(side) = self.showing(view) {
            match self.panel_focus() == Some(side) {
                true => self.close_panel(side),
                false => self.focus_slot(side),
            }
            return;
        }
        let side = self.side_for(view);
        match self.panel_mut(side) {
            Some(panel) => {
                panel.show(view);
                self.panel_focus = Some(side);
                self.refresh_sidebar();
            }
            None => {
                let root = self.root();
                self.open_sidebar_showing(&root, view);
            }
        }
    }

    /// **開一扇面板，鍵不交過去**（2026-09-29）。
    ///
    /// Warning: 別的入口開面板都順手把焦點給它，因為那幾個是「我要去那裏看」。文檔那
    /// 一扇不是：`空格 K` 說的是「一邊寫一邊讓它跟着」，人還在正文裏
    /// （報上來的原話：「它直接把焦点给到了侧栏，但用户希望焦点留在正文」）。
    /// 要走進去就按 `空格 4`，和別的邊欄一個樣。
    pub(super) fn open_panel_without_the_keys(
        &mut self,
        side: crate::sidebar::Side,
        view: crate::sidebar::View,
    ) {
        let was = self.panel_focus;
        self.open_side_showing(side, view);
        self.panel_focus = was;
    }

    /// **Open `side` showing `view`** — `show_sidebar`'s second half, without
    /// the toggle:    /// **Open `side` showing `view`** — `show_sidebar`'s second half, without
    /// the toggle: `:sidebar-left outline` says which side, so the side is not
    /// the configured one and 「already showing」 is not a reason to close it.
    pub(super) fn open_side_showing(
        &mut self,
        side: crate::sidebar::Side,
        view: crate::sidebar::View,
    ) {
        match self.panel_mut(side) {
            Some(panel) => {
                panel.show(view);
                self.panel_focus = Some(side);
                self.refresh_sidebar();
            }
            None => {
                let root = self.root();
                let mut sidebar = crate::sidebar::Sidebar::new(&root);
                sidebar.show(view);
                if let Some(path) = self.current_buffer().path() {
                    if let Ok(full) = std::fs::canonicalize(path) {
                        sidebar.reveal(&full);
                    }
                }
                self.panels[side as usize] = Some(sidebar);
                self.panel_focus = Some(side);
                self.refresh_sidebar();
            }
        }
    }

    /// **`Tab` 在這一欄裏走的那幾個視圖，按它走的次序**（2026-09-25 原話：
    /// 「底部……可以在上面写上 tab 的循环顺序，比如『Tab 文件 > 缓冲区 > 大纲 >
    /// 搜索』」）。
    ///
    /// Warning: **算出來的，不是寫死的**：哪個視圖歸哪一欄是使用者配得動的
    /// （`side_for`），底邊上那一行要說的就是他這一台此刻的真話。少於兩個的時候
    /// 回空——`Tab` 那時什麼都不做，寫一行「Tab 文件」是在許一個不存在的諾。
    pub fn views_on(&self, side: crate::sidebar::Side) -> Vec<crate::sidebar::View> {
        let mine: Vec<crate::sidebar::View> = crate::sidebar::View::ALL
            .into_iter()
            .filter(|&view| self.side_for(view) == side)
            .collect();
        match mine.len() < 2 {
            true => Vec::new(),
            false => mine,
        }
    }

    /// Which view a bare `:sidebar-left` opens: the first one that side owns,
    /// and the file tree when it owns none — the same answer the key gives.
    pub(super) fn side_view(&self, side: crate::sidebar::Side) -> crate::sidebar::View {
        crate::sidebar::View::ALL
            .into_iter()
            .find(|&view| self.side_for(view) == side)
            .unwrap_or(crate::sidebar::View::Explorer)
    }

    /// The view a panel's name stands for. Every panel is one now (#426), so
    /// this never answers `None` — it stays an `Option` because callers读它
    /// 的時候還不知道那一點。
    pub(super) fn view_of(panel: crate::sidebar::Panel) -> Option<crate::sidebar::View> {
        crate::sidebar::View::ALL
            .into_iter()
            .find(|&view| crate::sidebar::Panel::from(view) == panel)
    }

    /// Put that slot's resident panel away, and the keys back in the text if
    /// they were in it.
    pub(super) fn close_panel(&mut self, side: crate::sidebar::Side) {
        self.panels[side as usize] = None;
        if self.panel_focus == Some(side) && !self.slot_showing(side) {
            self.panel_focus = None;
        }
    }


    /// **The keys every panel answers, wherever it sits.**
    ///
    /// Warning: 左欄、右欄、光標放上去的那幾種是**同一個組件擺在不同位置**，所以這一組鍵
    /// 必須是同一份。分成三份各自維護的代價已經付過一次：常駐面板和搜索面板都
    /// 有 `q`，臨時層漏了，於是 `空格 d` 打開的字典**關不掉**——`q`、`Esc`、`j`
    /// 全被那一句 `_ => {}` 吃掉，唯一的出路是 `C-w` 再移動光標，兩步，而且提示
    /// 行一個字都沒說。
    ///
    /// Tried **last**, so a panel's own meaning for a key still wins: the
    /// search panel spends `Space` on a switch when the keys are on one.
    /// Answers whether it took the key.
    ///
    /// `Esc` is deliberately **not** here — see `on_sidebar_key`: a panel with
    /// a field in it spends `Esc` on leaving Insert, and one press too many
    /// would then put the panel away. `q` is the door, and the hint row says
    /// so in every panel.
    pub(super) fn panel_key_in_common(&mut self, key: Key, side: crate::sidebar::Side) -> bool {
        match key {
            // **邊欄裏 `C-w` 也是那一組的門**（2026-09-30）。從前它在這裏直接
            // 走下一區，而在正文裏是前綴——同一個和弦兩個意思，正是這一輪在
            // 拆的毛病。走下一區照舊是 `C-w w`，兩處一樣。
            Key::Ctrl('w') => self.pending = Pending::Region,
            // Warning: **信息那一格的 `q` 連內容一起丟**，見 `close_the_info`。
            Key::Char('q') => match self.info_in_this_sidebar(side).is_some() {
                true => self.close_the_info(side),
                false => self.close_panel(side),
            },
            // **`:` opens the command line from in here too.** It used to be
            // swallowed, so a reader with the keys in a panel had no way to
            // run a command at all — and `:panel-left` is a command
            // *about* the panel you are standing in, which nobody could have
            // reached. The focus stays where it is while the line is typed, so
            // 「this one」 still means this one.
            Key::Char(':') => {
                self.mode = Mode::Command;
                self.command_line.clear();
                self.command_caret = 0;
            }
            // Space still opens the menu, so the key that opened the sidebar
            // closes it again from inside it.
            Key::Char(' ') => self.pending = Pending::Space,
            _ => return false,
        }
        true
    }

    /// **`q` 在信息那一格上：把它收起來**（#426）。
    ///
    /// Warning: **連「誰叫的」一起丟，不只關容器。** 「畫在哪」是算出來的，所以
    /// 光關掉邊欄那一格，手動叫出來的那一則下一幀就浮到光標旁邊去了——讀者說的
    /// 是「我不要看這個」，不是「換個地方給我看」（2026-09-30 審出來的）。
    ///
    /// Warning: **即時的那一種照樣會浮回來，那是對的。** 模型是「右侧栏就
    /// 是固定的『浮窗』，它开着，浮窗就不用开了」——反過來說，關掉邊欄，浮窗就
    /// 回來。即時的那一種關不掉（`:info` 纔換得動它），能收起來的只有手動叫的。
    pub(super) fn close_the_info(&mut self, side: crate::sidebar::Side) {
        if let Some(one) = self.info_in_this_sidebar(side) {
            self.stop_showing_this_info(one);
        }
        self.info_asked = None;
        self.close_panel(side);
    }

    /// Give that sidebar the keys, if it is a place they can be.
    pub(super) fn focus_slot(&mut self, side: crate::sidebar::Side) {
        if !self.slot_showing(side) {
            return;
        }
        // 常駐的單子記得自己讀到哪；光標放上去的那幾種每次都是新的，從頭讀。
        if self.panel_focus != Some(side) && self.info_in_this_sidebar(side).is_some() {
            self.panel_scroll = 0;
        }
        self.panel_focus = Some(side);
        self.refresh_sidebar();
    }

    /// How far the 信息 panel has been scrolled (#293, #426).
    pub fn panel_scroll(&self) -> usize {
        self.panel_scroll
    }

    /// One key in the 信息 panel: it is read, not walked into.
    ///
    /// No `l`/`Enter` — a reading is not a place to go. What is left is moving
    /// the eye down a long answer, spelled the way the text and the lists
    /// already spell it, and `q` to put it away.
    fn on_info_key(&mut self, key: Key, side: crate::sidebar::Side) {
        let last = self.info_len(side).saturating_sub(1);
        let step = |at: usize, by: usize, down: bool| match down {
            true => at.saturating_add(by).min(last),
            false => at.saturating_sub(by),
        };
        match key {
            Key::Char('j') | Key::Down => self.panel_scroll = step(self.panel_scroll, 1, true),
            Key::Char('k') | Key::Up => self.panel_scroll = step(self.panel_scroll, 1, false),
            Key::Char('J') | Key::PageDown => {
                self.panel_scroll = step(self.panel_scroll, Self::PAGE_IN_A_LIST, true)
            }
            Key::Char('K') | Key::PageUp => {
                self.panel_scroll = step(self.panel_scroll, Self::PAGE_IN_A_LIST, false)
            }
            Key::Char('g') | Key::Home => self.panel_scroll = 0,
            Key::Char('G') | Key::End => self.panel_scroll = last,
            // Warning: **落到 `on_sidebar_key_after_the_list`，不是直接落到
            // `panel_key_in_common`**（2026-09-30 修）。那一支纔認 `w`（寬窄）、
            // `R`（重讀）、`Tab`、`Esc`——只落到後者的話，信息那一格裏 `w` 又
            // 一次悄悄失效，而那正是 2026-09-29 報的那一條。
            other => self.on_sidebar_key_after_the_list(other),
        }
    }


    /// 大綱最多縮幾級。第四級起和第三級對齊——見 `outline_rows`。
    const INDENT_STOPS: usize = 2;

    /// **這一側此刻是哪一檔寬度**，給前端算版面用。
    pub fn width_of(&self, side: crate::sidebar::Side) -> crate::sidebar::Width {
        self.width[side as usize]
    }

    /// **一扇側面板要這麼多欄纔擺得下**（2026-10-03 定）。
    ///
    /// 版面算術是 `正文保底 = max(窗口/3, 24)`，面板分的是剩下的（見 `yumete-tui`
    /// 的 `sidebar_columns`）。要讓面板分到十二欄——六個漢字，比那更窄的一列名字
    /// 只剩一個字加省略號——窗口就得有 24 + 12 欄。
    pub const PANEL_NEEDS: u16 = 36;

    /// **窗口擺得下一扇側面板嗎。**
    pub fn room_for_a_panel(&self) -> bool {
        self.window.0 >= Self::PANEL_NEEDS
    }

    /// **前端每一幀報一次窗口有多大**（欄、行）。
    ///
    /// 不是視口：編輯器不照着它滾動也不照着它折行，它只答「這扇面板擺得下嗎」。
    /// 離屏那一支在按鍵**之前**報，否則 `--shot` 看不見窄窗口下的行為。
    pub fn note_window(&mut self, columns: u16, rows: u16) {
        self.window = (columns, rows);
    }

    /// **這一刻站在第幾區。** 編號就是 `空格 1`–`4` 那四個號。
    pub(super) fn which_region(&self) -> u32 {
        use crate::sidebar::Side;
        match self.panel_focus() {
            Some(Side::Left) => 3,
            Some(Side::Right) => 4,
            None => 1 + self.live_pane().min(1) as u32,
        }
    }

    /// **那一區此刻在不在屏幕上。** 第一工作區永遠在。
    pub(super) fn region_open(&self, nth: u32) -> bool {
        use crate::sidebar::Side;
        match nth {
            1 => true,
            2 => self.other_pane().is_some(),
            3 => self.slot_showing(Side::Left),
            4 => self.slot_showing(Side::Right),
            _ => false,
        }
    }

    /// **`空格 w`/`C-w`：去下一個開着的區**（2026-09-26 定的）。
    ///
    /// 原話：「可不可以把工作区和侧边栏统一成一个概念「区域」以简化思维模型……保留
    /// space+w（切换到下个**可视**区域，按照第一工作区、第二工作区、左边栏、右边栏
    /// 这样的顺序）」。
    ///
    /// Warning: **走的次序就是 `空格 1`–`4` 那四個號**，不是屏幕上從左到右的次序。這樣
    /// 兩個鍵只有一套坐標：號碼是地址，`w` 是走一步，學會一個就學會另一個。從前
    /// 這一支按屏幕排（左欄 → 正文 → 右欄），和號碼各說各的。
    ///
    /// Warning: **只走開着的**：不在的區不會被順手開出來——那是 `空格 1`–`4` 和 `空格 W`
    /// 的事。「下一個」說的是眼前這幾個裏的下一個。
    /// **區域那一組的一個鍵**（`C-w` 或 `空格 w` 之後，2026-09-30 定）。
    ///
    /// 照 helix 的 `C-w`（`keymap/default.rs:193`）：`w` 走下一個，`hjkl` 按方
    /// 向走過去，`s` 切一刀，`q` 關掉，`o` 只留這一個。
    ///
    /// Warning: **`e`/`i` 是這一頭自己加的**：開關左右邊欄而**鍵不過去**。
    /// helix 沒有邊欄，所以沒有這一對；原話：「_we / _wi for toggling left
    /// and right sidebars……will not move focus to the sidebar」。大寫那一對
    /// （`E`/`I`）是開了就走進去，而且**只開不關**——「it does not close the
    /// sidebar as _we/_wi will do this」。
    pub(super) fn on_region_key(&mut self, key: Key) {
        use crate::sidebar::Side;
        // **`C-w C-w` 和 `C-w w` 是同一件事**（2026-10-06，對齊 helix）。helix 的
        // 區域組每一格都綁着兩個拼法——`C-w C-w`、`C-w C-s`、`C-w C-o`、`C-w C-h`…
        // （`default.rs` 的 window map）——因為按着 Ctrl 不放是這一組最自然的
        // 按法。從前這裏只認 `Key::Char`，於是那半邊**整組靜默掉地**。
        let key = match key {
            Key::Ctrl(c) => Key::Char(c),
            other => other,
        };
        match key {
            Key::Char('w') => self.next_region(),
            // 按方向走。四個區域：左欄、正文、副編輯區、右欄。
            //
            // Warning: **走，不開**（2026-10-02 報的）。這四個從前和 `E`/`I`/
            // `s` 共用 `go_to_region`，而那一支「沒有就開一個」——於是 `空格 w j`
            // 在只有一個編輯區的時候**切出一個新的**，`空格 w h` 在沒開左欄的時候
            // 把左欄開出來。原話：「_w + h/j/k/l 不是在可见的窗口里导航，而是会打
            // 开新的窗口。这个是不对的。」選單自己早就分清楚了：`h j k l` 是「按
            // 方向走」，`E I` 是「進左右欄」，`s` 是「切成兩個編輯區」。
            //
            // 那個方向沒東西就**什麼都不做，也不說話**。原話：「你 j 到最下面一行
            // 继续按 j 需要提示『没有更多行』吗？」——走到頭不是拒絕。
            Key::Char('h') | Key::Left => self.walk_to_region(3),
            Key::Char('l') | Key::Right => self.walk_to_region(4),
            Key::Char('k') | Key::Up => self.walk_to_region(1),
            Key::Char('j') | Key::Down => self.walk_to_region(2),
            // 開關那一欄，鍵留在原地。
            Key::Char('e') => self.toggle_region(Side::Left),
            Key::Char('i') => self.toggle_region(Side::Right),
            // 開出來並且走進去（已經開着就只是走進去）。
            Key::Char('E') => self.go_to_region(3),
            Key::Char('I') => self.go_to_region(4),
            // 切一刀：眼下只有上下兩個編輯區，所以「切出來」和「走到第二個」
            // 是同一件事（`go_to_region` 沒有就開一個）。Warning: 往後真有第三
            // 個編輯區的時候這兩件事要分家——`s` 是「再切一刀」，`j` 是「走下
            // 去」。原話「We will expand it to more possibilities in
            // future」指的就是這裏。
            // `v` 是 helix 的豎切（`vsplit`）。眼下只有上下兩個編輯區，所以它
            // 和 `s` 落在同一處；真切得了豎的那一天這兩個分家。
            Key::Char('s') | Key::Char('v') => self.go_to_region(2),
            Key::Char('q') => self.close_this_region(),
            Key::Char('o') => self.close_other_regions(),
            _ => {}
        }
    }

    /// **開關那一側的邊欄，鍵不過去**（`C-w e`/`C-w i`，2026-09-30）。
    pub(super) fn toggle_region(&mut self, side: crate::sidebar::Side) {
        match self.panel(side).is_some() {
            true => self.close_panel(side),
            false => {
                let view = self.side_view(side);
                self.open_panel_without_the_keys(side, view);
            }
        }
    }

    pub(super) fn next_region(&mut self) {
        let open: Vec<u32> = (1..=4).filter(|&n| self.region_open(n)).collect();
        if open.len() < 2 {
            return;
        }
        let here = self.which_region();
        let at = open.iter().position(|&n| n == here).unwrap_or(0);
        self.go_to_region(open[(at + 1) % open.len()]);
    }


    /// **`空格 q`：關掉站着的這一區**（2026-09-26 定的）。
    ///
    /// 邊欄就是關掉它自己——和在邊欄裏按 `q` 同一件事（定的，原話：「对于侧栏来说，q
    /// 和 space+q 是一个意思」）。工作區是關掉這一半、鍵跟到另一半去。
    ///
    /// Warning: **只剩一個區就出聲，別靜悄悄**（定的，原話：「什么都不做，出一声」）。關掉
    /// 最後一個工作區等於退出編輯器，而 `空格 q` 比 `:q` 好按得多——誤觸的代價是
    /// 丟稿子。
    pub(super) fn close_this_region(&mut self) {
        if let Some(side) = self.panel_focus() {
            return self.close_panel(side);
        }
        if self.other_pane().is_none() {
            self.status = say!("region.only-one-left");
            return;
        }
        if self.switch_pane() {
            self.close_split();
            self.status = say!("pane.closed");
        }
    }

    /// **`空格 Q`：只留一個工作區**（2026-09-26 定的）。
    ///
    /// 原話：「只保留一个工作区，优先保留光标所在的，回退到第一工作区」。所以站在
    /// 第二工作區上按它，留下的是第二個；站在邊欄裏按它，留下第一個。
    pub(super) fn close_other_regions(&mut self) {
        // 站在邊欄裏就退回工作區——留哪一個由「此刻在哪一個」決定，而邊欄不是。
        if self.panel_focus().is_some() {
            self.go_to_region(1);
        }
        for side in crate::sidebar::Side::BOTH {
            self.panels[side as usize] = None;
        }
        self.dictionary = None;
        self.dictionary_anchor = None;
        self.hovered = None;
        self.show_detail = Some(false);
        self.panel_focus = None;
        self.close_split();
        self.refresh_sidebar();
        self.status = say!("region.only-this-one");
    }


    /// **按方向走到第 `nth` 個區域——只走到開着的那些。**
    ///
    /// 和 [`Self::go_to_region`] 的分別就是這一句：那一支沒有就開一個（`E`、
    /// `I`、`s` 要的正是這個），這一支沒有就不動。
    fn walk_to_region(&mut self, nth: u32) {
        if self.region_open(nth) {
            self.go_to_region(nth);
        }
    }

    pub(super) fn go_to_region(&mut self, nth: u32) {
        use crate::sidebar::Side;
        match nth {
            1 | 2 => {
                let want = usize::from(nth == 2);
                // 沒有第二個工作區就開一個，開在站着的地方（`open_split` 的話）。
                if want == 1 && self.other_pane().is_none() {
                    let at = self.sel.head();
                    let caption = self.current_buffer().display_name().to_string();
                    self.open_split(at, None, caption);
                }
                self.panel_focus = None;
                // 和 `which_region` 裏那一句同一個算法，別各算各的。
                if self.other_pane().is_some() && self.live_pane().min(1) != want {
                    self.switch_pane();
                }
            }
            3 | 4 => {
                let side = match nth {
                    3 => Side::Left,
                    _ => Side::Right,
                };
                if !self.slot_showing(side) {
                    // **開那一側該開的那一扇**——問 `side_view`，別各算各的。
                    //
                    // Warning: **從前這裏自己又找了一遍**（`View::ALL` 裏第一個歸這
                    // 一側的），於是它繞過了兩件 `side_view` 知道的事：這一份稿子
                    // 容不容得下那一扇（散文沒有文檔），以及右邊那一格此刻「應該」
                    // 擺文檔還是診斷。報上來的就是這個：在一行有警告的地方按
                    // `空格 4`，開出來的是**空的文檔面板**。
                    if self.views_on(side).is_empty() && self.side_view(side) == crate::sidebar::View::Explorer
                        && self.side_of(crate::sidebar::Panel::Files) != side
                    {
                        self.status = say!("region.nothing-lives-there");
                        return;
                    }
                    let view = self.side_view(side);
                    self.show_sidebar(view);
                }
                self.focus_slot(side);
            }
            _ => {}
        }
    }

    /// Show the file tree rooted at `root` and give it the keys.
    pub fn open_sidebar_at(&mut self, root: &Path) {
        self.open_sidebar_showing(root, crate::sidebar::View::Explorer);
    }

    /// Show a panel rooted at `root`, opened on `view`, in that view's slot.
    pub fn open_sidebar_showing(&mut self, root: &Path, view: crate::sidebar::View) {
        let mut sidebar = crate::sidebar::Sidebar::new(root);
        sidebar.show(view);
        // Open on the file being written, so the tree says where you are rather
        // than making you find yourself in it.
        if let Some(path) = self.current_buffer().path() {
            if let Ok(full) = std::fs::canonicalize(path) {
                sidebar.reveal(&full);
            }
        }
        let side = self.side_for(view);
        self.panels[side as usize] = Some(sidebar);
        self.panel_focus = Some(side);
        self.refresh_sidebar();
    }

    /// Fill the sidebar with whatever its current view shows.
    ///
    /// The tree builds its own rows from the file system; the other two are the
    /// editor's own knowledge, so they are pushed in from here.
    /// Refreshed when it is asked for, not on every keystroke.
    ///
    /// Building the outline walks the document, and doing that per key is the
    /// trap this editor has fallen into three times. Headings do not change
    /// while a sentence is being typed, so the views are rebuilt when the
    /// sidebar is opened, focused, switched, or the file under it changes —
    /// every moment a reader is about to look at it.
    pub(super) fn refresh_sidebar(&mut self) {
        for side in crate::sidebar::Side::BOTH {
            self.refresh_panel(side);
        }
    }

    /// Fill one slot with whatever the view in it shows.
    fn refresh_panel(&mut self, side: crate::sidebar::Side) {
        use crate::sidebar::{Row, View};
        let Some(view) = self.panel(side).map(|p| p.view()) else {
            return;
        };
        let rows = match view {
            View::Explorer => {
                if let Some(panel) = self.panel_mut(side) {
                    panel.rebuild();
                }
                return;
            }
            // `depth` carries the index the row stands for — the buffer's, or
            // the line's — since a flat list has no depth to spend.
            View::Buffers => self
                .buffers
                .iter()
                .enumerate()
                .map(|(i, b)| Row {
                    path: b.path().map(Path::to_path_buf).unwrap_or_default(),
                    name: format!(
                        "{}{}",
                        b.display_name(),
                        if b.is_modified() { " +" } else { "" }
                    ),
                    depth: i,
                    is_dir: false,
                    expanded: i == self.current,
                })
                .collect(),
            View::Outline => self.outline_rows(),
            // Its own store, its own shape: a form and a list of hits, not
            // rows of a tree (#419).
            View::Search => return,
            // Drawn from the cursor every frame, not from rows (#287, #426):
            // 五種內容各有各的來源，`info_rows` 現算，這裏沒有一份行要存。
            View::Info => return,
        };
        if let Some(panel) = self.panel_mut(side) {
            panel.set_rows(rows);
        }
    }

    /// **百科那一頁的滾動**（2026-09-22）。
    ///
    /// 與別的視圖同一套鍵：`j`/`k` 一行，`J`/`K` 半頁，`g`/`G` 兩頭；`q` 與
    /// `C-w` 照舊由 [`Self::panel_key_in_common`] 接。
    fn scroll_wiki(&mut self, key: Key) {
        let page = Self::PAGE_IN_A_LIST;
        let (at, _) = self.wiki_scroll_now();
        let moved = match key {
            Key::Char('j') | Key::Down => at.saturating_add(1),
            Key::Char('k') | Key::Up => at.saturating_sub(1),
            Key::Char('J') | Key::PageDown => at.saturating_add(page),
            Key::Char('K') | Key::PageUp => at.saturating_sub(page),
            Key::Char('g') | Key::Home => 0,
            // Warning: **`G` 不在這裏算底在哪**——這一頭數不出來（見
            // [`Editor::wiki_scroll`]）。存一個到不了的數，畫的那一趟走到底、
            // 算出總數、把真正的那個數夾回來。
            Key::Char('G') | Key::End => usize::MAX,
            other => return self.on_sidebar_key_after_the_list(other),
        };
        self.wiki_scroll.set((moved, self.sel.head()));
    }

    /// 百科那一條此刻讀到第幾行——光標換了詞條就從頭算。
    pub fn wiki_scroll_now(&self) -> (usize, usize) {
        let held = self.wiki_scroll.get();
        match held.1 == self.sel.head() {
            true => held,
            false => (0, self.sel.head()),
        }
    }

    /// **前端畫完那一趟，把夾好的那個數寫回來**——它是唯一折得出屏幕行的一頭。
    pub fn set_wiki_scroll(&self, at: usize) {
        self.wiki_scroll.set((at, self.sel.head()));
    }

    /// **不管這一頁是單子還是文章，這幾個鍵都算數**（2026-09-23 審出來的：提示
    /// 行在百科那一頁上照樣寫着 `Tab 換視圖`/`w 寬窄`，而那兩個鍵在那裏什麽都
    /// 不做——「拿走鍵的那一半有義務」）。
    pub(super) fn on_sidebar_key_after_the_list(&mut self, key: Key) {
        let Some(side) = self.panel_focus() else { return };
        match key {
            Key::Char('R') => self.refresh_sidebar(),
            // **`w` 走一格：窄 → 中 → 寬 → 窄**（2026-09-26 定的）。
            //
            // Warning: **不問這一格裏裝的是什麼**：寬度歸側欄，面板只是借它。所以字典、
            // 懸停、表格詳情按 `w` 一樣管用——從前它們根本不認這個鍵。
            Key::Char('w') => {
                let step = self.width[side as usize].next();
                self.width[side as usize] = step;
                let (num, den) = step.fraction();
                // 一句回聲，三秒之後自己下去——按的人眼睛已經看見邊欄寬了一檔，
                // 而這一行底下正是按鍵提示的位子（2026-09-27）。
                self.murmur(say!("sidebar.width", num, den));
            }
            Key::Tab => {
                self.cycle_view(side, false);
            }
            Key::BackTab => {
                self.cycle_view(side, true);
            }
            // **`Esc` 在邊欄裏也是「把輸入法的挂起再說一遍」**（2026-09-27 報的：
            // 「光标在侧栏中的时候，我在别的 app 中使用了系统的宇夢，回到
            // terminal，无法通过 esc 把它挂起。它会在工作区的光标处形成一个输入
            // 面板」）。
            //
            // 正文那一頭 2026-09-22 就有這條退路（`keys.rs` 的 `say_it_again`）：
            // 「已經挂起了」是一個**信念**，而輸入法在別的程序裏自己恢復之後，這
            // 一頭還記着舊的，於是再也不發第二次信號。抹掉信念，下一輪自己重發。
            // 邊欄這一支從前根本收不到那一下 `Esc`——它在這裏什麼都不做。
            Key::Esc => self.say_it_again = true,
            other => {
                self.panel_key_in_common(other, side);
            }
        }
    }

    /// The 大綱's headings, whichever way this document spells them.
    ///
    /// A typst master file is a table of contents and nothing else, so its
    /// outline reaches into the files it includes; everything else reads its
    /// own headings.
    fn outline_headings(&self) -> Vec<crate::sidebar::Heading> {
        if self.current_buffer().syntax() == crate::syntax::Syntax::Typst {
            return self.included_headings();
        }
        self.outline()
            .into_iter()
            .map(|(line, level, title)| crate::sidebar::Heading {
                path: PathBuf::new(),
                line,
                level,
                title,
            })
            .collect()
    }

    /// The 大綱's rows: its headings, with whatever is folded away left out
    /// (#37).
    ///
    /// `is_dir` says the heading has something under it and `expanded` whether
    /// that something is showing — the two fields the tree already spends on
    /// the same question, so the front end draws one mark for both views.
    fn outline_rows(&self) -> Vec<crate::sidebar::Row> {
        let headings = self.outline_headings();
        let folded = |key: &(PathBuf, usize)| {
            self.outline_panel()
                .is_some_and(|panel| panel.is_folded(key))
        };
        let mut rows = Vec::new();
        // The level of the shallowest fold currently hiding rows. Anything
        // deeper than it is inside that fold; the first row that is not ends
        // it, and *that* row is the one asked whether it folds in turn.
        let mut hidden_under: Option<usize> = None;
        for (i, heading) in headings.iter().enumerate() {
            if hidden_under.is_some_and(|level| heading.level > level) {
                continue;
            }
            hidden_under = None;
            let has_children = headings
                .get(i + 1)
                .is_some_and(|next| next.level > heading.level);
            let shut = has_children && folded(&heading.key());
            if shut {
                hidden_under = Some(heading.level);
            }
            rows.push(crate::sidebar::Row {
                path: heading.path.clone(),
                // **每級兩格，縮到第三級封頂**（2026-09-26 定的）。
                //
                // Warning: **深處的層級靠編號自己說**：`5.8.11` 一看就比 `5.8` 深一層，
                // 不必再花欄位重說一遍。development.md 有到五級，不封頂的話那一條
                // 縮 8 欄——24 欄的邊欄只剩 14 欄給標題，而邊欄存在就是為了給標題。
                name: format!(
                    "{}{}",
                    "  ".repeat(heading.level.saturating_sub(1).min(Self::INDENT_STOPS)),
                    heading.title
                ),
                depth: heading.line,
                is_dir: has_children,
                expanded: has_children && !shut,
            });
        }
        rows
    }

    /// `h` in the 大綱: fold what the highlight is on, or the heading holding
    /// it (#37).
    ///
    /// **One key for both, as in the tree**: pressing it again and again walks
    /// out of the branch rather than stopping at the first heading that has
    /// nothing to fold. A heading with nothing under it, or one already
    /// folded, has no fold of its own to close, so the one above it closes and
    /// takes the highlight.
    pub(super) fn fold_outline(&mut self) {
        let Some(here) = self.outline_row_key() else {
            return;
        };
        let headings = self.outline_headings();
        let Some(i) = headings.iter().position(|h| h.key() == here) else {
            return;
        };
        let has_children = headings
            .get(i + 1)
            .is_some_and(|next| next.level > headings[i].level);
        let shut = self
            .outline_panel()
            .is_some_and(|panel| panel.is_folded(&here));
        let target = match has_children && !shut {
            true => here,
            // The nearest heading above it that is shallower than it is.
            false => match headings[..i]
                .iter()
                .rposition(|h| h.level < headings[i].level)
            {
                Some(parent) => headings[parent].key(),
                None => return,
            },
        };
        self.set_outline_fold(target, true);
    }

    /// `l` in the 大綱: open a folded heading. Whether it was folded — if it
    /// was not, the key goes on to mean 「take me there」, as `Enter` does.
    pub(super) fn unfold_outline(&mut self) -> bool {
        let Some(here) = self.outline_row_key() else {
            return false;
        };
        if !self
            .outline_panel()
            .is_some_and(|panel| panel.is_folded(&here))
        {
            return false;
        }
        self.set_outline_fold(here, false);
        true
    }

    /// What the highlighted 大綱 row stands for: the file it is in and the
    /// line it is on, which is what the fold set remembers.
    fn outline_row_key(&self) -> Option<(PathBuf, usize)> {
        let panel = self.outline_panel()?;
        let row = panel.rows().get(panel.selected())?;
        Some((row.path.clone(), row.depth))
    }

    /// The panel showing the 大綱, whichever slot it is in.
    fn outline_panel(&self) -> Option<&crate::sidebar::Sidebar> {
        self.panel(self.showing(crate::sidebar::View::Outline)?)
    }

    /// Fold or open one heading, rebuild the rows, and keep the highlight on
    /// it — folding takes rows away, and a highlight that slid onto whatever
    /// filled the gap would be reading the wrong chapter.
    fn set_outline_fold(&mut self, key: (PathBuf, usize), folded: bool) {
        let Some(side) = self.showing(crate::sidebar::View::Outline) else {
            return;
        };
        if !self.panel_mut(side).is_some_and(|p| p.set_folded(key.clone(), folded)) {
            return;
        }
        self.refresh_sidebar();
        if let Some(panel) = self.panel_mut(side) {
            let at = panel
                .rows()
                .iter()
                .position(|r| (r.path.clone(), r.depth) == key);
            if let Some(at) = at {
                panel.select(at);
            }
        }
    }

    /// The 字典 panel's rows — Feature #215.
    ///
    /// The names are padded to the widest of them so the values line up down a
    /// column, and the padding is counted in **columns** rather than characters
    /// (`拆分` is two characters and four columns wide).
    ///
    /// A row with no value is a heading — the character itself at the top, and
    /// the 陸/臺/港 label above each block when the 拆分表 has more than one
    /// answer. `is_dir` is what the sidebar draws headings with; the flat views
    /// already spend the tree's fields on what they have instead of what a tree
    /// has, and this is that.
    /// 服務器說的那段話，一行一行擺進邊欄（#53 ③）。
    ///
    /// Warning: **原樣的 Markdown**，和浮窗裏那一份一個字不差——邊欄畫它的時候走的是
    /// 同一支行內標記渲染。
    /// **這一行上服務器說的那幾句**，給診斷那一扇（2026-09-29）。
    ///
    /// 診斷不必問——它是服務器自己推過來的，早就在內存裏，所以這一支和百科一樣
    /// 是現算的純函數。
    pub(super) fn problem_rows(&self) -> Vec<crate::sidebar::Row> {
        let Some((loud, said)) = self.problem_here() else { return Vec::new() };
        let head = match loud {
            crate::problem::Severity::Error => say!("problem.error"),
            crate::problem::Severity::Warn => say!("problem.warn"),
            crate::problem::Severity::Note => say!("problem.note"),
            crate::problem::Severity::Hint => say!("problem.hint"),
        };
        said.into_iter()
            .map(|line| crate::sidebar::Row {
                path: PathBuf::new(),
                name: format!("{head}　{line}"),
                depth: 0,
                is_dir: false,
                expanded: false,
            })
            .collect()
    }

    pub(super) fn hover_rows(&self) -> Vec<crate::sidebar::Row> {
        let Some(told) = self.hover_here() else {
            return Vec::new();
        };
        told.lines()
            .map(|line| crate::sidebar::Row {
                path: PathBuf::new(),
                name: line.to_string(),
                depth: 0,
                is_dir: false,
                expanded: false,
            })
            .collect()
    }

    fn dictionary_rows(&self) -> Vec<crate::sidebar::Row> {
        use crate::sidebar::Row;
        let heading = |name: String| Row {
            path: PathBuf::new(),
            name,
            depth: 0,
            is_dir: true,
            expanded: false,
        };
        let Some((ch, answer)) = self.dictionary.as_ref() else {
            return Vec::new();
        };
        let mut rows = vec![heading(ch.to_string())];
        let Some(fields) = answer else {
            return rows;
        };
        if fields.is_empty() {
            rows.push(Row {
                path: PathBuf::new(),
                name: match self.ime_available() {
                    true => say!("ui.not-in-the-table"),
                    false => say!("ui.no-table-yet"),
                },
                depth: 0,
                is_dir: false,
                expanded: false,
            });
            return rows;
        }
        let width = fields
            .iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, _)| yumete_cjk::str_width(name))
            .max()
            .unwrap_or(0);
        for (name, value) in fields {
            if value.is_empty() {
                rows.push(heading(name.clone()));
                continue;
            }
            let pad = " ".repeat(width.saturating_sub(yumete_cjk::str_width(name)));
            rows.push(Row {
                path: PathBuf::new(),
                name: format!("{name}{pad}  {value}"),
                depth: 0,
                is_dir: false,
                expanded: false,
            });
        }
        rows
    }

    /// Look this character up in the 拆分表 — `Space d`, and `Tab` on a
    /// candidate (#215).
    ///
    /// The editor does not hold the table: yume does, and only the front end
    /// has it. So the character is parked here and the panel is opened empty;
    /// the answer arrives on the next pass through the loop, one frame later,
    /// which is not long enough for a reader to see the gap.
    pub fn look_up(&mut self, ch: char, focus: bool) -> bool {
        self.look_up_here(ch, false, focus)
    }

    /// **`空格 d`：只浮一個窗**（2026-09-22 定）。
    ///
    /// 同一個問題、同一份答案，畫在光標旁邊而不是邊欄裏——而且**一點都不碰邊
    /// 欄**：邊欄是容器，只由人開由人關，看一眼字不該讓工作區變樣。
    ///
    /// 回 `false` 表示這一下是**關掉**（再按一次收起來）。
    pub fn look_up_afloat(&mut self, ch: char) -> bool {
        self.look_up_here(ch, true, false)
    }

    /// 兩個問法共用的那一半（#426：它們差的只是容器）。
    fn look_up_here(&mut self, ch: char, afloat: bool, focus: bool) -> bool {
        // Warning: **同一個字再問一次纔算「收起來」。** `Tab` 選候選也走這一支，
        // 問的是另一個字——那是換一份答案，不是關窗，所以它走不收起的那一支。
        let same = self.dictionary.as_ref().is_some_and(|(at, _)| *at == ch);
        match same {
            true if !self.ask_for_info(crate::sidebar::Info::Dictionary, afloat) => {
                self.panel_focus = None;
                return false;
            }
            true => {}
            false => self.put_this_info_here(crate::sidebar::Info::Dictionary, afloat),
        }
        self.dictionary_query = Some(ch);
        // Asked, unanswered: what is showing until the answer arrives is the
        // character alone, which is not the same panel as 「查不到」.
        self.dictionary = Some((ch, None));
        // **Where the question was asked from.** The answer stays up while the
        // cursor is still there and goes when it leaves — nothing has to close
        // it, which is the whole of why this slot holds no state (#293).
        self.dictionary_anchor = Some(self.sel.head());
        self.panel_scroll = 0;
        // Asked from the page, the keys go with the question. Asked while a
        // word is being typed, they must not — the reader is mid-word, and the
        // panel is only there to be glanced at.
        // Warning: **問的時候不交鍵，就把鍵留在原地**——別清成 `None`。從前寫的是
        // `focus.then(..)`，於是從檔案樹裏按 `空格 d`（那條路是通的：面板裏的
        // 空格照樣開選單）會把鍵從樹裏悄悄拿走，而 `空格 d` 的說明寫着「一點都
        // 不碰邊欄」（2026-09-23 審出來的）。
        if focus {
            if let Some(side) = self.info_in_the_sidebar() {
                self.panel_focus = Some(side);
            }
        }
        self.refresh_sidebar();
        true
    }

    /// The character `Space d` or `Tab` asked about, for the front end to
    /// answer once (#215).
    pub fn take_dictionary_query(&mut self) -> Option<char> {
        self.dictionary_query.take()
    }

    /// The answer to [`Editor::take_dictionary_query`].
    ///
    /// Dropped if the reader has since asked about a different character —
    /// the answer to last frame's question must not overwrite this frame's.
    pub fn set_dictionary(&mut self, ch: char, fields: Vec<Gloss>) {
        if self.dictionary.as_ref().is_some_and(|(at, _)| *at != ch) {
            return;
        }
        self.dictionary = Some((ch, Some(fields)));
        self.refresh_sidebar();
    }

    /// **這一則字典該不該浮在光標旁邊**——那一格沒開，而此刻擺的正是字典。
    pub fn dictionary_afloat(&self) -> Option<(char, Option<&[Gloss]>)> {
        (self.info_afloat() == Some(crate::sidebar::Info::Dictionary))
            .then(|| self.dictionary())
            .flatten()
    }

    pub fn dictionary(&self) -> Option<(char, Option<&[Gloss]>)> {
        self.dictionary
            .as_ref()
            .map(|(ch, answer)| (*ch, answer.as_deref()))
    }

    /// Whether the keys are in either panel.
    pub fn sidebar_focused(&self) -> bool {
        self.panel_focus().is_some()
    }

    /// What the sidebar's keys are, for the status line to say while it has
    /// them.
    ///
    /// A pane that takes the keys has to say how to give them back, in the
    /// place a reader already looks for what is going on.
    ///
    /// Warning: **百科那一頁是一段文章**，沒有行可以 `l` 進去、也沒有行可以 `h` 收
    /// 起，`R` 重讀的是一張單子而不是一條詞條——這一行從前照樣寫着那三個鍵
    /// （2026-09-23 審出來的：「拿走鍵的那一半有義務」說清楚）。
    pub fn sidebar_keys(&self) -> String {
        let article = self.info_now() == Some(crate::sidebar::Info::Wiki)
            && self
                .panel_focus()
                .and_then(|side| self.panel(side))
                .is_some_and(|p| p.view() == crate::sidebar::View::Info);
        match article {
            true => say!("hint.sidebar.keys-wiki"),
            false => say!("hint.sidebar.keys"),
        }
    }

    /// How many rows `J`/`K` move in a list — a screenful of a sidebar, near
    /// enough. The sidebar does not know how tall it is drawn (the front end
    /// does), and a list moves by a *fixed* amount for the same reason `J`
    /// moves by half a page in the text: the eye keeps its place.
    const PAGE_IN_A_LIST: usize = 12;

    /// Run one key while the sidebar has the keys.
    ///
    /// The same letters that move in the text move here — `j`/`k` down and up,
    /// `l` into, `h` out of — so there is nothing new to learn; only what they
    /// move through is different.
    pub(super) fn on_sidebar_key(&mut self, key: Key) {
        // The 大綱's fold keys are answered before the borrow below, because
        // they need the whole editor: only it knows how deep each heading sits
        // (#37). `outline_row_key` is `Some` only in that view, with a row
        // under the highlight.
        if self.outline_row_key().is_some() {
            match key {
                // Out of a branch, as `h` is in the tree.
                Key::Char('h') | Key::Left => return self.fold_outline(),
                // Into one. On a folded heading `l` opens it rather than
                // jumping — 「more of this」 is what it already means in the
                // tree. It falls through on any other row, and `Enter` never
                // folds at all: that is the key that goes, and a heading is a
                // place whether or not it is holding others.
                Key::Char('l') | Key::Right if self.unfold_outline() => return,
                _ => {}
            }
        }
        let Some(side) = self.panel_focus() else {
            self.panel_focus = None;
            return;
        };
        // 光標放上去的那一個在眼前，鍵就歸它——常駐那一個在底下等着，不收鍵。
        if self.panel(side).map(|p| p.view()) == Some(crate::sidebar::View::Info) {
            // **百科是一段文章，要滾不要走**（2026-09-22）：它軟折行，只有畫的
            // 那一頭數得出屏幕行，所以它的捲軸自己存一份（`wiki_scroll`）。
            return match self.info_now() {
                Some(crate::sidebar::Info::Wiki) => self.scroll_wiki(key),
                _ => self.on_info_key(key, side),
            };
        }
        if self.panel(side).map(|p| p.view()) == Some(crate::sidebar::View::Search) {
            return self.on_search_panel_key(key, side);
        }
        let Some(sidebar) = self.panel_mut(side) else {
            return;
        };
        match key {
            Key::Char('j') | Key::Down => sidebar.step(true),
            Key::Char('k') | Key::Up => sidebar.step(false),
            // **A list pages by the same keys the page does.** `J`/`K` are
            // half a page in the text; a 700-chapter outline is the one list
            // where walking it by `j` is not walking, and `PageDown` is not on
            // every keyboard a novelist owns.
            Key::Char('J') | Key::PageDown => {
                for _ in 0..Self::PAGE_IN_A_LIST {
                    sidebar.step(true);
                }
            }
            Key::Char('K') | Key::PageUp => {
                for _ in 0..Self::PAGE_IN_A_LIST {
                    sidebar.step(false);
                }
            }
            // …and the ends, spelled as they are in the text.
            Key::Char('g') | Key::Home => sidebar.go_to_end(false),
            Key::Char('G') | Key::End => sidebar.go_to_end(true),
            Key::Char('h') | Key::Left => sidebar.collapse(),
            Key::Char('l') | Key::Right | Key::Enter => {
                let chosen = sidebar.activate();
                match chosen {
                    Some(crate::sidebar::Chosen::File(path)) => {
                        if let Err(err) = self.open_file(&path) {
                            self.status = say!("buffer.cannot-open", path.display(), err);
                        }
                        // Entering a file means going to write in it.
                        self.panel_focus = None;
                        self.refresh_sidebar();
                    }
                    Some(crate::sidebar::Chosen::Buffer(i)) => {
                        self.show_buffer(i);
                        self.panel_focus = None;
                        self.refresh_sidebar();
                    }
                    Some(crate::sidebar::Chosen::Line(line)) => {
                        self.goto_line(line + 1);
                        self.panel_focus = None;
                    }
                    Some(crate::sidebar::Chosen::FileLine(path, line)) => {
                        match self.open_included_file(&path) {
                            Ok(()) => self.goto_line(line + 1),
                            Err(err) => {
                                self.status = say!("buffer.cannot-open", path.display(), err)
                            }
                        }
                        self.panel_focus = None;
                        self.refresh_sidebar();
                    }
                    None => {}
                }
            }
            // **`Esc` does nothing here** (#293). It is everyone's 「get me
            // out」 key, so it is tempting — but a panel with a field in it
            // spends `Esc` on leaving Insert, and one press too many would
            // then put the panel away. Two doors instead, and both say so in
            // the hint row: `q` closes this slot, `C-w` walks on to the next
            // region and leaves it up. Both live in `panel_key_in_common`,
            // with `:` and `Space`, because every panel owes the reader the
            // same ones.
            // …and everything that is the same whether this page is a list or
            // an article: `R`, `w`, `Tab`/`S-Tab`, then `q`/`C-w`/`:`/空格.
            // **Tab walks the views that live in *this* slot.** Which ones
            // those are is a setting, so the question belongs to the editor
            // rather than to the panel — with the outline moved across, this
            // slot walks two and the other one walks one (#293). A chapter's
            // whole name does not fit in a column narrow enough to be worth
            // keeping open, so `w` trades the columns for the name and back;
            // the views are built when they are opened, not on every key, so
            // `R` is how a writer who has just added a chapter says to look
            // again.
            other => self.on_sidebar_key_after_the_list(other),
        }
    }

    /// **How many fields the bottom layer holds** — what its scrolling is
    /// clamped to (#293).
    ///
    /// Warning: **Fields, not drawn lines.** A value too long for the column wraps,
    /// and only the front end knows how wide the column is — so scrolling by
    /// line would have to be clamped by a number the editor cannot work out.
    /// A field is also the better step: it is the thing a reader is looking
    /// for, and one press moves to the next one whether it took one row or
    /// four.
    /// **信息那一格有幾行可讀**——`j`/`k` 走到這裏為止。
    ///
    /// Warning: **和畫出來的那幾行是同一個數。** 兩邊各算一次是「滾到底之後還
    /// 能再按三下」那一族 bug 的來源，所以行是列表的一律問 [`Editor::info_rows`]。
    pub fn info_len(&self, side: crate::sidebar::Side) -> usize {
        match self.info_in_this_sidebar(side) {
            Some(crate::sidebar::Info::Record) => {
                self.detail().map_or(0, |detail| detail.rows.len())
            }
            Some(crate::sidebar::Info::Wiki) => {
                self.wiki_here().map_or(0, |view| view.as_prose().lines().count())
            }
            Some(_) => self.info_rows(side).len(),
            None => 0,
        }
    }

    /// **What the picker is standing on, to be shown beside the list** — the
    /// head of the file, the head of the buffer, or the lines around a row
    /// (2026-09-17: 「左側是文件窗口，右側是預覽」).
    ///
    /// `rows` lines at most, and the answer is kept until the highlight moves
    /// off it: without that, holding `j` down would read a file per keystroke.
    pub fn picker_preview(&self, rows: usize) -> Option<(String, Vec<String>)> {
        let item = self.picker.as_ref()?.chosen()?;
        let head = |text: &str| -> Vec<String> {
            text.lines().take(rows).map(|l| l.replace('\t', "    ")).collect()
        };
        match item {
            crate::picker::Item::File(path) => {
                let full = match self.picker.as_ref().and_then(|p| p.root.as_ref()) {
                    Some(root) => root.join(&path),
                    None => PathBuf::from(&path),
                };
                // Already open? Then the buffer is the truer answer: it holds
                // what has been typed and not saved.
                if let Some(open) = self.buffers.iter().find(|b| b.path() == Some(full.as_path())) {
                    return Some((path, head(&open.text())));
                }
                let mut cached = self.preview.borrow_mut();
                if cached.as_ref().is_none_or(|(at, _)| *at != full) {
                    // A directory, a 14 MB 碼表, a binary: read the head and
                    // nothing more, and say nothing rather than guess.
                    //
                    // Warning: **真的只讀頭上那幾行。** 這裏從前是
                    // `fs::read_to_string`——整個文件進內存，然後纔切掉
                    // 99.99%。2026-09-22 報的就是這個：「picker 如果用 jk 快速
                    // 過文件，會出現到某個文件的時候突然卡死十幾二十秒」，而走
                    // 過的那一個是十幾兆的碼表。上面那句註釋一直寫着「read the
                    // head and nothing more」，代碼卻没做到。
                    *cached = Some((full.clone(), head_of_file(&full, rows)?));
                }
                cached.as_ref().map(|(_, lines)| (path, lines.clone()))
            }
            crate::picker::Item::Buffer(i, name) => {
                Some((name, head(&self.buffers.get(i)?.text())))
            }
            crate::picker::Item::Row(line, name) => {
                let rope = self.current_buffer().rope();
                let lines = (line..(line + rows).min(rope.len_lines()))
                    .map(|n| rope.line(n).to_string().trim_end().to_string())
                    .collect();
                Some((name, lines))
            }
            // A clipboard entry is already the whole of what it is.
            crate::picker::Item::Paste(_, _) => None,
            // **The entry itself, beside the list of names** (2026-09-25).
            // The preview a picker already has is exactly the 「選中的條目浮窗
            // 顯示」 the request asked for — no second mechanism.
            crate::picker::Item::Wiki(name, _) => {
                let view = self.wiki_view_of(&name)?;
                let lines = view
                    .parts
                    .iter()
                    .flat_map(|part| part.lines.iter().map(|l| l.text().to_string()))
                    .filter(|l| !l.trim().is_empty())
                    .take(rows)
                    .collect();
                Some((view.name.clone(), lines))
            }
        }
    }

    /// Open whatever the picker is standing on — from either layer.
    fn choose_from_picker(&mut self) {
        let chosen = self.picker.as_ref().and_then(crate::picker::Picker::chosen);
        // Warning: **篩不出東西的時候 `Enter` 不關窗**（2026-10-02 一輪黑盒審查報來
        // 的）。從前它照樣關掉，於是打錯一個字母按了 `Enter`，面板沒了、打過的
        // 那幾個字也沒了，人回到正文裏看着一句「沒有符合的」——要重開一次、重打
        // 一遍。helix 的挑選器在這一步什麼都不做。那句話照說，窗留着，退一格就
        // 改得動。
        if chosen.is_none() {
            self.status = say!("picker.nothing-matched");
            return;
        }
        // **根要在關掉挑選器之前取**：關掉就連根一起沒了。
        let root = self.picker.as_ref().and_then(|p| p.root.clone());
        self.close_picker();
        match chosen {
            Some(crate::picker::Item::File(path)) => {
                let full = match &root {
                    Some(root) => root.join(&path),
                    None => PathBuf::from(&path),
                };
                if let Err(err) = self.open_file(&full) {
                    self.status = say!("buffer.cannot-open", path, err);
                }
            }
            Some(crate::picker::Item::Buffer(i, _)) => self.show_buffer(i),
            // The picker belongs to whichever key opened it, so choosing from
            // it lands the way that key lands.
            Some(crate::picker::Item::Row(line, _)) => {
                let preview = self.definition_preview;
                self.land_on_row(line, preview)
            }
            Some(crate::picker::Item::Paste(Some(which), _)) => self.paste_from_menu(which),
            // The system clipboard is the front end's to read.
            Some(crate::picker::Item::Paste(None, _)) => self.clipboard_paste(true),
            // **釘住它**，直到光標一動（2026-09-25）。面板關掉，鍵回正文——
            // 「直到光標移動」本來就要求鍵已經不在面板裏了。
            Some(crate::picker::Item::Wiki(name, _)) => self.pin_wiki_entry(name),
            None => self.status = say!("picker.nothing-matched"),
        }
    }

    /// **`:wiki <詞條名>` —— 一扇挑詞條的面板**（2026-09-25）。
    ///
    /// 原話：「先彈出類似「找命令」面板一樣的面板，詞條+部分內容（用 ... 省略）
    /// 然後tab和shift + Tab 上下移動。選中的條目浮窗顯示或者右邊欄顯示（如果右
    /// 邊欄開着），按下enter 之后固定浮窗和面板直到光標移動。」
    ///
    /// 四件事這扇面板本來就有：`Tab`/`S-Tab` 走單子、右邊那半是預覽、打字就篩、
    /// `Enter` 挑中。要新做的只有「釘住」。
    ///
    /// Warning: **從前這裏還要撥一下「鍵落在查詢裏」**（`type_here(true)`）——命令行上
    /// 打過名字的人還在打字那個心境裏。2026-10-08 挑選器只剩打字這一層，每一扇
    /// 開門就是這樣，那一句隨之刪了。
    pub(super) fn open_wiki_picker(&mut self, name: &str) {
        if self.wiki.by_name.is_empty() {
            self.status = say!("wiki.none", self.book_wiki_path().display());
            return;
        }
        // 一個名字一行——同名的幾條（這本書的、全局的）是同一頁的幾節，不是
        // 幾行（`wiki_view_of`）。`BTreeMap` 進來的時候就是排好的。
        let items = self
            .wiki
            .by_name
            .iter()
            .map(|(name, found)| {
                let blurb = found
                    .first()
                    .and_then(|&i| self.wiki.entries.get(i))
                    .and_then(|e| e.body.iter().find(|l| !l.trim().is_empty()))
                    .map(|l| blurb_of(l.trim()))
                    .unwrap_or_default();
                crate::picker::Item::Wiki(name.clone(), blurb)
            })
            .collect();
        let mut picker = crate::picker::Picker::new(&say!("picker.wiki"), items);
        for c in name.chars() {
            picker.push(c);
        }
        self.picker = Some(picker);
        self.mode = Mode::Picker;
    }

    /// Open a picker over the files of the project (`Space f`).
    //
    // (`blurb_of` 在檔尾，和別的自由函數放在一起。)
    ///
    /// **Two orders, one list** (2026-09-18). What is gathered is
    /// 「写的东西在前面」: the prose — `.md`, `.txt`, `.typ` and the rest of
    /// [`PROSE`] — comes first in path order, which for a novel is chapter
    /// order, and everything else (the build files, the code, the
    /// `Cargo.lock`) follows. What is *offered* is then that list with the
    /// files this session has been in lifted to the top
    /// ([`Editor::visited`]), because the one being written is what 「open a
    /// file」 most often means. Both are tie-breakers: once anything is typed,
    /// the match decides.
    pub(super) fn open_file_picker(&mut self) {
        self.open_file_picker_in(self.root(), crate::editor::Sieve::default());
    }

    /// `空格 F` —— 同上，但搜的是[工作路徑][`Editor::working_dir`]（2026-10-01，
    /// 照 helix 的 `file_picker_in_current_directory`）。
    pub(super) fn open_file_picker_here(&mut self) {
        self.open_file_picker_in(self.working_dir(), crate::editor::Sieve::default());
    }

    /// **`ye --files`**：照挑選器那一支的規矩，列出名字配得上 `query` 的檔。
    ///
    /// Warning: **不另寫一個匹配器。** 它建的就是挑選器那個 [`crate::picker::Picker`]、
    /// 問的就是它的 `matches()`——所以 `ye --files jia` 和編輯器裏 `空格 f` 打
    /// `jia` 永遠是同一份答案。管道那一邊另寫一份一定會分岔。
    ///
    /// Warning: **不封頂。** 挑選器封在 `PICKER_LIMIT`（四千）條，因為那是給人翻的；
    /// 管道印給別的程序看，少印一條就是錯一條。
    pub fn files_matching(&self, root: &Path, query: &str, sieve: &crate::editor::Sieve) -> Vec<String> {
        let mut names = Vec::new();
        // Warning: **篩子要傳進來**（2026-10-03 一輪審查報來的）。從前這裏寫死
        // `Sieve::default()`，於是 `ye --files md --hidden`、`--glob=`、`--exclude=`
        // 三個開關**全是死的**，而 `--help` 說「下面每一個都還管用」。
        crate::editor::walk_with(root, sieve, &mut |path| {
            names.push(path.strip_prefix(root).unwrap_or(path).display().to_string());
        });
        let items = names.into_iter().map(crate::picker::Item::File).collect();
        let mut picker = crate::picker::Picker::new("", items);
        // **管道那一邊收緊**（2026-10-04 定）：跳着配和亂序都不收，因為它把全部印
        // 出來，長尾就是噪音。`空格 f` 不撥它——那是一張排過序的單子，只看前十條。
        picker.tighten();
        for c in query.chars() {
            picker.push(c);
        }
        picker.matches().iter().map(|item| item.label().to_string()).collect()
    }

    /// **`ye --files … --open`**：挑選器開着、字已經打好、單子已經篩過。
    ///
    /// Warning: **打進去，不是塞進去**：一個字一個字走 `Picker::push`，和人敲鍵盤走的
    /// 是同一支，所以篩選、評分、第一條預覽都和手打出來的一模一樣。
    ///
    /// Warning: **篩子要傳進來**（2026-10-08 修）。從前這一支走的是寫死的
    /// `Sieve::default()`，於是 `ye --files --hidden --open` 的 `--hidden`、
    /// `--no-ignore`、`--glob=`、`--exclude=` 四個開關**印出來那條路管用、進編輯
    /// 器那條路全是死的**，而 `--help` 說它們都管用。它也是 `A-h` 轉一格之後重走
    /// 那一趟走的同一支。
    pub fn open_file_picker_with(
        &mut self,
        query: &str,
        root: PathBuf,
        sieve: crate::editor::Sieve,
    ) {
        self.open_file_picker_in(root, sieve);
        let Some(picker) = self.picker.as_mut() else { return };
        for c in query.chars() {
            picker.push(c);
        }
    }

    /// **`A-h` 轉一格：跳過哪些，然後照新的篩子重走一趟**（2026-10-08 定）。
    ///
    /// 四態的次序照搜索面板那個 `7` 鍵（`find.rs` 的 `Field::Hidden`）：
    /// `(隱藏, 忽略)` 走 `(f,f) → (f,t) → (t,f) → (t,t) → (f,f)`。那邊記下的理由
    /// 是「第一步放開的是忽略那一半——想找回來的多半是 `.gitignore` 擋掉的目錄，
    /// 不是點文件」，兩扇一個答案。
    ///
    /// Warning: **鍵是 `A-h`，不是 `C-h`**：`C-h` 在很多終端裏就是退格，而 helix 的挑選
    /// 器已經把 `C-t` 用在切預覽上了（它那一扇只用 Alt 做 `A-Enter`）。
    ///
    /// 打過的字留着：重走完一趟，那幾個字一個一個再 `push` 回去（`open_file_picker_with`）。
    /// 不走磁碟的那幾扇（緩衝區、百科…）沒有篩子，這一下什麼都不做。
    fn cycle_what_the_picker_skips(&mut self) {
        let Some(picker) = self.picker.as_ref() else { return };
        let (Some(root), Some(sieve)) = (picker.root.clone(), picker.sieve.clone()) else {
            return;
        };
        let query = picker.query().to_string();
        let mut sieve = sieve;
        (sieve.hidden, sieve.ignored) = match (sieve.hidden, sieve.ignored) {
            (false, false) => (false, true),
            (false, true) => (true, false),
            (true, false) => (true, true),
            (true, true) => (false, false),
        };
        self.open_file_picker_with(&query, root, sieve);
    }

    fn open_file_picker_in(&mut self, root: PathBuf, sieve: crate::editor::Sieve) {
        let mut prose = Vec::new();
        let mut rest = Vec::new();
        // Warning: **這張單子可能是半截的，而此刻一個字都不說**（2026-10-09 審出來的）。
        //
        // 兩種半截：走查自己停了（`Walked.cut` ——`WALK_CEILING`／`WALK_DEADLINE`），
        // 和 `PICKER_LIMIT` 四千條裝滿了。`Walked` 自己的註釋寫着「呼叫方有義務說出
        // 來」，搜索那一頭說得出（`search.hits-more`），這一頭兩種都沒說。
        //
        // 沒有順手補：要寫的是一則**新的界面文案**（而且得把兩種成因說清楚），那歸
        // 作者定——`PICKER_LIMIT` 那一則本來就在等他的那張單子上。接上文案之後，這
        // 裏要收的是 `walk_with` 的回值與 `prose.len() + rest.len() >= PICKER_LIMIT`
        // 這一格。
        let _half = crate::editor::walk_with(&root, &sieve, &mut |path| {
            if prose.len() + rest.len() >= PICKER_LIMIT {
                return;
            }
            let shown = path.strip_prefix(&root).unwrap_or(path).display().to_string();
            let is_prose = path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| PROSE.contains(&&*e.to_lowercase()));
            match is_prose {
                true => prose.push((path.to_path_buf(), shown)),
                false => rest.push((path.to_path_buf(), shown)),
            }
        });
        prose.append(&mut rest);
        if prose.is_empty() {
            self.status = say!("picker.no-files-here");
            return;
        }
        // Newest first, so the head of `visited` is worth the most. The step is
        // small beside a match's own score (a run of two adjacent letters is
        // worth 800), which is what keeps this a tie-breaker.
        let bonus = prose
            .iter()
            .map(|(full, _)| {
                self.visited()
                    .iter()
                    .position(|seen| seen == full)
                    .map_or(0, |n| (VISITED_BONUS - n as i64 * 20).max(20))
            })
            .collect();
        let items = prose
            .into_iter()
            .map(|(_, shown)| crate::picker::Item::File(shown))
            .collect();
        let mut picker = crate::picker::Picker::new(&say!("picker.files"), items);
        // 挑選器的根存在挑選器身上，不借 `listing_root` 那個槽——見 `Picker::root`。
        picker.root = Some(root);
        // 篩子也存在它身上：`A-h` 轉一格要知道此刻跳過的是哪些，提示行要寫出來。
        picker.sieve = Some(sieve);
        picker.prefer(bonus);
        self.picker = Some(picker);
        self.mode = Mode::Picker;
    }

    /// Open a picker over the buffers already open (`Space b`).
    pub(super) fn open_buffer_picker(&mut self) {
        let items = self
            .buffers
            .iter()
            .enumerate()
            .map(|(i, b)| crate::picker::Item::Buffer(i, b.display_name()))
            .collect();
        self.picker = Some(crate::picker::Picker::new(&say!("picker.buffers"), items));
        self.mode = Mode::Picker;
    }

    /// Whether `Space` is waiting for the key that says what to do — which is
    /// when the which-key menu is drawn.
    pub fn space_pending(&self) -> bool {
        matches!(self.pending, Pending::Space)
    }

    /// The open picker, for the front end to draw.
    pub fn picker(&self) -> Option<&crate::picker::Picker> {
        self.picker.as_ref()
    }

    /// **告訴挑選器名單畫了幾行**，`PageUp`/`PageDown` 翻的就是這個數。
    ///
    /// 和 [`Editor::set_page`] 同一個理由：視口住在 TUI 那一側，這裏只收它量出
    /// 來的數。没開挑選器就没人要知道。
    pub fn note_picker_rows(&mut self, rows: usize) {
        if let Some(picker) = self.picker.as_mut() {
            picker.note_rows(rows);
        }
    }

    /// Run one key while a picker is open.
    ///
    /// **一扇挑選器只有一個狀態**（2026-10-08 定）：開門就在打字，`Esc` 一下關掉。
    ///
    /// Warning: **從前有兩層**（2026-09-17 到 2026-10-08）：開門在「列表」那一層，`jk`
    /// 走單子、`i` 或 `/` 進打字、`Esc` 回列表、再一下纔出門。四個編輯器對過一遍
    /// ——helix、VS Code、Zed、nvim **一個都沒有挑選器裏的模式**，四個都是開門就
    /// 打字、一下 `Esc` 關掉（helix 的 `helix-term/src/ui/picker.rs:1090-1108` 把
    /// `Tab`/`Down`/`ctrl-n`、`shift-Tab`/`Up`/`ctrl-p` 和
    /// `key!(Esc) | ctrl!('c') => return close_fn(self)` 寫在同一個 match 裏，配不
    /// 上的一律落到那一行輸入框上，`:1160`）。
    ///
    /// Warning: **2026-10-01 試過「開門就打字」，當天撤回**，理由是關窗變成要按兩次
    /// `Esc`。那一條是對的，而答案是把另一半也做掉：`Esc` 現在一下就關。
    pub(super) fn on_picker_key(&mut self, key: Key) {
        let Some(picker) = self.picker.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        match key {
            // **一下就關。** 沒有層可退回去了。
            Key::Esc => self.close_picker(),
            // Warning: **框空着的時候退格什麼都不做**。從前它換層（`i` 是怎麼進來的，
            // 退到頭就該退回去）；只剩一層之後那個去處不存在，而「退格到頭就關
            // 窗」是另一回事——打空了接着打纔是人要的。
            Key::Backspace => picker.backspace(),
            // **往前三個鍵，往後三個鍵**，照 helix 那一行。`Tab`/`S-Tab` 先說，
            // 原話：「这两个其实更加顺手」。
            Key::Tab | Key::Ctrl('n') | Key::Down => picker.step(true),
            Key::BackTab | Key::Ctrl('p') | Key::Up => picker.step(false),
            Key::PageDown => picker.page(true),
            Key::PageUp => picker.page(false),
            Key::Enter => self.choose_from_picker(),
            // 跳過哪些，轉一格——見 [`Editor::cycle_what_the_picker_skips`]。
            Key::Alt('h') => self.cycle_what_the_picker_skips(),
            Key::Char(c) => picker.push(c),
            // A query is typed text, and typed text is edited in the middle.
            Key::Delete => picker.delete(),
            Key::Left => picker.move_caret(crate::picker::Caret::Left),
            Key::Right => picker.move_caret(crate::picker::Caret::Right),
            Key::Home | Key::Ctrl('a') => picker.move_caret(crate::picker::Caret::Start),
            Key::End | Key::Ctrl('e') => picker.move_caret(crate::picker::Caret::End),
            Key::Ctrl('u') => picker.clear_before_caret(),
            _ => {}
        }
    }

    /// Shut the picker and go back to Normal.
    fn close_picker(&mut self) {
        self.picker = None;
        self.mode = Mode::Normal;
    }

    /// Insert text that arrived from outside — the system clipboard, by way of
    /// the terminal's bracketed paste (Feature #108).
    ///
    /// It is *writing*, whatever mode the editor is in. Without this a paste is
    /// a stream of keystrokes, and in Normal mode every character of the pasted
    /// paragraph runs as a command: that is not a paste going wrong so much as
    /// the editor running a macro nobody wrote.
    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // **A spreadsheet's clipboard becomes rows** (Feature #226). Excel,
        // Numbers, LibreOffice and a browser table all put tab-separated lines
        // on the clipboard, and until now the cell refused every one of them
        // for holding a tab — the writer got 「格子裏不能有 Tab」 for the one
        // paste a table editor exists to accept. `t p` had already learned to
        // read a block out of the register; this is the same block arriving by
        // the other door, and it lands the same way.
        if (self.mode == Mode::Insert || self.mode == Mode::Normal) && self.table_here() {
            if let Some(grid) = sniff_grid(text.trim_end_matches(['\n', '\r'])) {
                self.paste_grid(grid);
                return;
            }
        }
        // **Judged before anything happens**, in Insert as well as in Normal:
        // the Insert branch used to hand the text to `insert_str`, which
        // silently drops what a cell refuses, and then say 「貼了 8 個字」 about
        // a paste that had not happened. It also spent an undo point on it.
        if self.mode == Mode::Insert || self.mode == Mode::Normal {
            if let Some(why) = self.cell_refuses_text(text) {
                self.status = why;
                return;
            }
        }
        self.snapshot();
        match self.mode {
            // In Insert it lands where the caret is, like anything typed.
            Mode::Insert => self.insert_str(text),
            // In Normal it replaces the selection, which is what `p` over a
            // selection does — and what a writer means by pasting over
            // something they have just picked out.
            Mode::Normal => {
                // **Both halves judged before either runs**, the way `r` and
                // `R` already do it: pasting a comma into a cell used to
                // delete what was selected and *then* refuse the paste, so the
                // cell came back short and the message only talked about the
                // refusal.
                self.delete_selection();
                let at = self.sel.head();
                if !self.edit_insert(at, text) {
                    return;
                }
                let rope = self.current_buffer().rope();
                let end = at + text.chars().count();
                let head = motion::prev_grapheme(rope, end).max(at);
                self.sel.set_anchor(at);
                self.sel.set_head(head);
                self.refresh_goal_column();
            }
            // A prompt takes it as typing, minus the line breaks that would
            // submit it — asked of the mode rather than listed here (#351).
            // The picker is a prompt too, but its query has a store of its
            // own and nothing here reaches it.
            // A panel's field has a store of its own, like the picker's.
            Mode::Field => {
                let text: String = text.chars().filter(|c| !c.is_control()).collect();
                self.type_into_field(&text);
            }
            mode => {
                if mode.types_into_command_line() {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        self.command_line.push(c);
                    }
                    self.completion = None;
                }
            }
        }
        self.status = say!("edit.pasted-characters", text.chars().count());
    }

    /// Put the selection on the system clipboard (`Space y`).
    ///
    /// Through OSC 52, the terminal's own copy escape: it needs no library, and
    /// it is the only way that works over ssh and inside tmux, which is where a
    /// terminal editor is often run from. The terminal may refuse — many do by
    /// default — so this says what it asked for rather than claiming success.
    pub(super) fn copy_to_clipboard(&mut self) {
        let (start, end) = self.selection();
        let text = self.current_buffer().rope().slice(start..end).to_string();
        if text.is_empty() {
            self.status = say!("edit.nothing-selected");
            return;
        }
        // Into the editor's own register too: having copied something, `p` is
        // the next thing a hand reaches for.
        self.store(text.clone());
        let n = text.chars().count();
        self.clipboard_request = Some(text);
        self.status = say!("edit.copied-to-clipboard", n);
    }

    /// Put the cursor at char index `pos`, starting a selection there
    /// (Feature #109).
    pub fn point_at(&mut self, pos: usize) {
        let pos = pos.min(self.current_buffer().char_count());
        self.extend = false;
        self.sel.set_anchor(pos);
        self.sel.set_head(pos);
        self.refresh_goal_column();
        // **The mouse leaves a guessed block too** (#275). Walking out of one
        // with `j` drops it at the end of `on_key`; clicking out of one never
        // went through `on_key`, so the mode stayed on until the next
        // keystroke — the cursor was in the paragraph and `hjkl` were still
        // walking cells.
        self.forget_a_guessed_table();
        self.find_the_table_here();
    }

    /// Drag the selection's head to char index `pos`, keeping its anchor.
    pub fn drag_to(&mut self, pos: usize) {
        self.sel.set_head(pos.min(self.current_buffer().char_count()));
        self.refresh_goal_column();
    }

    /// Take a pending clipboard copy, for the front end to send to the terminal.
    pub fn take_clipboard_request(&mut self) -> Option<String> {
        self.clipboard_request.take()
    }

    /// Ask for the system clipboard, to be pasted after (or before) the
    /// selection once the front end has fetched it.
    pub(super) fn clipboard_paste(&mut self, after: bool) {
        self.clipboard_read = Some(crate::editor::Pasting::AsIs { after });
    }

    /// `:paste-table <格式>` — 剪貼板裏那張表，轉成這一種再貼（2026-10-02 定）。
    pub(super) fn paste_table(
        &mut self,
        to: crate::table::Shape,
        from: Option<crate::table::Shape>,
    ) {
        self.clipboard_read = Some(crate::editor::Pasting::AsTable { to, from });
    }

    /// Take a pending clipboard read, and what to do with what comes back.
    pub fn take_clipboard_read(&mut self) -> Option<crate::editor::Pasting> {
        self.clipboard_read.take()
    }

    /// Hand over what the system clipboard held, and paste it.
    pub fn provide_clipboard(&mut self, text: &str, how: crate::editor::Pasting) {
        use crate::editor::Pasting;
        if text.is_empty() {
            self.status = say!("edit.clipboard-empty");
            return;
        }
        let after = match how {
            Pasting::AsIs { after } => after,
            Pasting::AsTable { to, from } => {
                let Some(text) = self.as_a_table(text, to, from) else {
                    return;
                };
                self.snapshot();
                self.store(text);
                self.paste(true);
                return;
            }
        };
        self.snapshot();
        // Whole lines go back as whole lines, and the selection is replaced
        // when there is one — the same rules `p` follows, because this is `p`
        // with the text coming from somewhere else.
        self.store(text.to_string());
        self.paste(after);
    }

    /// 剪貼板那幾行寫成 `to` 那一種——看不出它是張表就說一聲，什麼都不貼。
    fn as_a_table(
        &mut self,
        text: &str,
        to: crate::table::Shape,
        from: Option<crate::table::Shape>,
    ) -> Option<String> {
        let lines: Vec<String> = text
            .trim_end_matches(['\n', '\r'])
            .lines()
            .map(str::to_string)
            .filter(|l| !l.trim().is_empty())
            .collect();
        let Some(from) = from.or_else(|| crate::table::shape_of(&lines)) else {
            self.status = say!("table.no-delimiter-in-sight");
            return None;
        };
        Some(format!("{}\n", crate::table::recast(&lines, from, to).join("\n")))
    }

    /// Move to the first non-blank character of line `n`, counting from 1 and
    /// clamped to the end of the buffer (`10gg`, `:10`, `:goto 10`).
    pub(super) fn goto_line(&mut self, n: usize) {
        self.remember_jump();
        self.move_to_line(n);
    }

    /// The same, without noting a jump.
    ///
    /// For the callers that have already noted one — a mark, `:table-jump` — where a
    /// second note would be of the place *after* the file switch, and `C-o`
    /// would then take you to the file you had just arrived in.
    pub(super) fn move_to_line(&mut self, n: usize) {
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        let line = n.saturating_sub(1).min(last);
        let at = rope.line_to_char(line);
        let pos = motion::line_first_non_blank(rope, at);
        self.move_head(pos);
    }
}

/// **The first `rows` lines of a file, and not one byte more** (2026-09-22).
///
/// The picker walks files as fast as `j` repeats, and it previews whatever it
/// is standing on. Reading the whole file to show twenty lines of it made one
/// keystroke cost as much as opening the file — which on a 14 MB 碼表 is the
/// 「突然卡死十幾二十秒」 that was reported.
///
/// Warning: **`lines()` stops where it is told**, so a huge file costs the head and
/// the buffering, not its length. `None` for what cannot be read as text at
/// all — a directory, a binary — which is what the caller shows nothing for.
///
/// Warning: **Invalid UTF-8 ends the preview, it does not fail it.** A `.ytab` that
/// is text for the first megabyte and binary after is still worth showing the
/// head of, and `read_to_string` would have refused the whole file.
fn head_of_file(path: &Path, rows: usize) -> Option<Vec<String>> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    let mut out = Vec::with_capacity(rows.min(64));
    for line in std::io::BufReader::new(file).lines().take(rows) {
        match line {
            Ok(line) => out.push(line.replace('\t', "    ")),
            // 讀到這裏爲止——前面那些照樣是正文。
            Err(_) => break,
        }
    }
    Some(out)
}

/// **一條詞條說的頭一句，裁到一行放得下**——挑詞條那扇面板每一行的後半截。
///
/// 原話：「詞條+部分內容（用 ... 省略）」。裁的是**字**不是字節，而且只在這裏
/// 裁一次：名字有多長、窗口有多寬是前端的事，這裏給的是「一句話的量」。
fn blurb_of(line: &str) -> String {
    const BLURB: usize = 40;
    let mut out: String = line.chars().take(BLURB).collect();
    if line.chars().count() > BLURB {
        out.push('…');
    }
    out
}
