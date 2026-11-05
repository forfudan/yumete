//! Where a key goes (#296).
//!
//! `on_key` is the one door into the editor, and everything under it is the
//! sorting: which mode, which pane, which of the two grids, and the `空格`
//! menu. It sat under 「word segmentation」 for no reason but the order it was
//! written in.

use super::*;
use yumete_cjk::actions::NormalCmd;

/// The goal column vim's `$` leaves: past any line's end. Compared with,
/// never added to, so it cannot overflow.
pub(super) const LINE_END_GOAL: usize = usize::MAX / 4;

/// 一段是**整行**還是行內的一截——`do_vim_span` 那道「落在換行符上就什麼都不做」
/// 的閘只擋後者（2026-10-13）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shape {
    /// `dd`／`yy`／`cc` 那一路：這一段就是整行，連它的行尾。
    Lines,
    /// 算子 ＋ 動作那一路：`dw`、`d$`、`x`、`D`。
    Chars,
}

/// How a Normal key reaches the cursors (see `Editor::binding_fan`).
enum Fan {
    /// As this key would, with this pending.
    Key(Pending, Key),
    /// Once; a binding's commands reach the cursors themselves.
    Once,
}

impl Editor {
    /// Handle a single key press according to the current mode.
    pub fn on_key(&mut self, key: Key) -> KeyOutcome {
        let was = self.before_a_change();
        let outcome = self.on_key_now(key);
        self.settle_folds();
        self.keep_the_signature_up(was);
        self.settle_the_change(was);
        match std::mem::take(&mut self.quit_from_binding) {
            true => KeyOutcome::Quit,
            false => outcome,
        }
    }

    /// A key that moved the text says where it left the cursors, for helix's
    /// redo to come back to (see [`crate::Buffer::settle_after`]).
    pub(super) fn settle_the_change(&mut self, (_, id, revision, _): (Mode, u64, u64, usize)) {
        let buffer = self.current_buffer();
        if buffer.id() != id || buffer.revision() == revision {
            return;
        }
        let (at, every) = (self.sel.head(), self.sel.clone());
        self.current_buffer_mut().settle_after(at, every);
        self.move_the_problems();
    }

    /// The mode, buffer, revision and caret, for [`Self::keep_the_signature_up`].
    pub(super) fn before_a_change(&self) -> (Mode, u64, u64, usize) {
        let buffer = self.current_buffer();
        (self.mode, buffer.id(), buffer.revision(), self.sel.head())
    }

    fn on_key_now(&mut self, key: Key) -> KeyOutcome {
        // **A question standing takes every key** (#295), before recording and
        // before the sidebar: while one is on the screen there is no keystroke
        // that reaches the manuscript, and none that a macro should capture —
        // the answer is about this file at this moment, not about the sequence
        // being recorded.
        if self.query.is_some() {
            self.answer_query(key);
            return KeyOutcome::Continue;
        }
        // **按「那裏寫的什麼」跳的時候，鍵先歸它**（`gx`/`gz`，§5.73）——和底下
        // `gw` 同一個理由，而且同樣擺在錄製之前：打進去的那兩個字母是**這一屏上**
        // 的事，回放的時候屏幕上不是同一批字。
        if self.seeking().is_some() && self.seek_key(key) {
            return KeyOutcome::Continue;
        }
        // **標籤亮着的時候整個鍵盤都是標籤**（`gw`，#406）——在錄製之前，因為
        // 那一兩下打的是屏幕上臨時發的號碼，回放的時候屏幕上不會是同一批號碼。
        // 錄進去的等於一句永遠對不上的話。
        if self.jumping() {
            self.jump_key(key);
            return KeyOutcome::Continue;
        }
        // Recording happens here rather than in Normal mode's handler, so a
        // macro captures the text typed in Insert and the pattern typed at a
        // prompt too — a macro that can only move is not much of one.
        if self.recording.is_some() && !self.expanding_alias {
            // **Every key goes in, and stopping cuts back to where the keys
            // that stopped it began** (2026-10-31). It used to leave out a
            // `Q` that was a command, so a recording stopped by any other key
            // (`"C-r" = "record_macro"`, or `Space q` bound to it) ended with
            // that key, and replaying the macro started a new recording.
            // Warning: **only the `Q` that is a command stops it**: a `Q` that
            // a half-finished sequence is waiting for is an operand (`fQ` is
            // 「find Q」), and dropping it once left a bare `f` that swallowed
            // whatever came next on replay — one reviewer's macro deleted their
            // buffer that way. A key that begins a sequence is where that
            // sequence's keys begin, so that is the mark.
            if let Some(keys) = self.recording.as_mut() {
                if self.mode == Mode::Normal
                    && self.pending == Pending::None
                    && self.alias_held.is_empty()
                    && self.count.is_none()
                {
                    self.recording_from = keys.len();
                }
                keys.push(key);
            }
        }
        // **一份已經不作數的字典答案，這裏丟掉**（2026-09-23 審出來的）。
        //
        // Warning: **在派鍵之前，不是之後。** `dictionary_live()` 問的是「焦點在不在
        // 字典那一側」——`Layer` 還在的時候那等於「在不在下層」，没了之後同一側
        // 的常駐面板拿到鍵也是這個值。於是：`空格 D` 開字典、`C-w` 出來、光標一
        // 走（字典從畫面上下去了，可 `self.dictionary` 還是 `Some`）、
        // `:wiki panel`、再 `C-w` 進去——焦點一落到右邊，那份**過期的字典就重新
        // 蓋住百科**，而且把鍵接走。掃在派鍵之前，`C-w` 那一下焦點還没挪過去，
        // 掃得掉；掃在之後就正好掃不掉。
        self.forget_a_dictionary_nobody_is_reading();
        // 同上，釘住的那一條詞條也是：光標一動就鬆開（2026-09-25）。
        self.forget_a_pinned_entry();
        // `空格 k` 問來的那一則說明同理：光標走開就作廢，不許走回去又冒出來
        // （2026-09-29 報的）。
        self.forget_a_hover_nobody_is_looking_at();
        // 改動那一條問的是一塊而不是一個字，所以它有自己那一句（2026-10-11）。
        self.forget_a_change_the_cursor_has_left();
        self.unhush_the_float_once_the_cursor_has_left();
        // 跟着光標走的那一問，光標一動就重新等它停穩。
        self.the_cursor_moved_under_the_docs();
        // **浮着的那一則，四個鍵翻得動**（2026-09-29 定，#426 起五種都收）。
        //
        // 原話：「既然这几个面板要么在浮窗要么在右边栏，我们就可以用
        // page up / page down 来对这五类进行翻页了」「这样，光标就在编辑区，也
        // 可以对五类信息进行翻页。」Warning: **翻的是那一則的行，不是換一種**
        // ——五種共用一個容器，那就該共用一套翻頁鍵；而「光標就在編輯區」正是
        // 這四個鍵的全部價值：讀完一條長詞條不必先把光標挪進邊欄。
        //
        // Warning: **從前只有文檔收這四個鍵**，於是散文稿子裏默認浮的百科被切在
        // 「…」上卻翻不動，`C-u`/`C-d` 去翻了正文——等於没有出路。
        //
        // Warning: **和 helix 逐鍵相同**（`ui/popup.rs:289-297`）：浮窗開着的時候
        // 這四個鍵歸浮窗，没開就照舊翻正文，別的鍵一按浮窗就走。所以這裏**不
        // 攔**——翻不動就讓路，正文那一支自己接住。
        if self.mode == Mode::Normal && self.pending == Pending::None {
            // Warning: **翻頁那一族要整族在這裏**（2026-10-01 報的：「你翻页
            // 翻的是 buffer 不是浮窗/边栏」）。從前只攔了四個，而 `C-f`/`C-b`
            // 與當天新加的 `C-n`/`C-p` 没攔住——同一族鍵，開着浮窗按下去有的
            // 翻浮窗有的翻正文，讀者没法預測。
            //
            // 行數跟着「翻多少」走：整頁 8、三分之二 6、半頁 4。
            let by = match key {
                Key::PageDown | Key::Ctrl('f') => Some(8),
                Key::PageUp | Key::Ctrl('b') => Some(-8),
                Key::Ctrl('n') if self.flavor_on(yumete_cjk::FlavorItem::TwoThirdsPage) => Some(6),
                Key::Ctrl('p') if self.flavor_on(yumete_cjk::FlavorItem::TwoThirdsPage) => Some(-6),
                Key::Ctrl('d') => Some(4),
                Key::Ctrl('u') => Some(-4),
                _ => None,
            };
            if let Some(by) = by {
                if self.scroll_the_float(by) {
                    return KeyOutcome::Continue;
                }
            }
        }
        // The sidebar takes Normal-mode keys while it has the focus; every
        // other mode is about the text and goes to the text.
        if self.sidebar_focused() && self.mode == Mode::Normal && self.pending == Pending::None {
            // Warning: **上一句話這裏散掉**，同 `on_normal_key` 開頭那一句（2026-09-27
            // 報的：「邊欄：4/10 的消息一直都在。我就算在边栏中换行他也在，我根本
            // 看不到任何快捷键提示」）。從前這一支在清之前就回去了，於是側欄裏設
            // 的任何一句話都賴着不走——而命令行那一格被它佔着，按鍵提示就没地方
            // 畫。話是說給上一個按鍵的，下一個按鍵一到它就該散。
            self.status.clear();
            self.on_sidebar_key(key);
            return KeyOutcome::Continue;
        }
        // Watch this command, so `.` can play it back. A command begins in
        // Normal mode with nothing pending; it ends when it is back there.
        // A key describes one move, and「was that a jump?」is about *this* one.
        self.jumped = false;
        self.centring = None;
        self.just_switched = false;
        // Warning: **`z` 那一層的覆蓋也是一次性的。** 不清的話 `zt` 之後每一幀都把光標釘在頂
        // 上，`j` 走一行整頁跟着挪一行——那是打字機模式，不是 `zt`。清掉之後視口停在
        // 剛纔擺好的地方，因為下一幀 `page_inset` 回的是「光標舒舒服服在頁面上，別動」。
        self.aim = None;
        // Warning: **An expansion is not a command of its own** (2026-09-19). Every
        // key `play_keys` replays arrives here in Normal mode with nothing
        // pending, so it cleared the record and then wrote itself into it: `.`
        // after the vim preset's `x` pressed `D`, which the alias layer read
        // again as 「cut to end of line」 — `x.` took the whole line. What `.`
        // has to remember is the key the reader pressed, and that one was
        // recorded before the expansion began.
        let watching = !self.repeating_edit && !self.expanding_alias;
        if watching {
            // A count is part of the command it prefixes, not a command: `3>`
            // is one change and `.` has to repeat all three levels of it.
            if self.mode == Mode::Normal && self.pending == Pending::None && self.count.is_none() {
                self.edit_keys.clear();
                self.edit_was_insert = false;
                self.edit_revision = (self.current_buffer().id(), self.current_buffer().revision());
            }
            self.edit_keys.push(key);
        }
        // 插入態的 `C-o` 借走了一個鍵嗎——借的是**這一鍵之前**就借好的，所以要在
        // 這裏記下來（見 [`Self::go_back_after_one_key`]）。
        let borrowed = self.one_normal_key;
        // 這一鍵按下去之前延伸開着没有——`gv` 要的那一段就是它關掉的那一刻留下
        // 的（見 [`Self::keep_the_last_selection`]）。
        let was_extending = self.extend.then(|| self.span());
        // Where the pane's window was pointed before this key — see
        // [`Self::hold_the_pane`].
        let held = self
            .table
            .as_ref()
            .is_some_and(|view| view.takes_the_pane())
            .then(|| (self.current_buffer().id(), self.sel.head()));
        let outcome = match self.mode {
            // Warning: **not while an alias plays** — `w` under the vim keymap
            // is keys played through here, and those are the motion itself.
            Mode::Normal if self.block.is_some() && !self.expanding_alias => {
                self.on_block_key(key);
                KeyOutcome::Continue
            }
            Mode::Normal => {
                self.back_from_the_pair_shown();
                // **Judged by what the key becomes** (2026-10-31, 2026-11-01).
                // A one-key binding to one key (`"C-q" = "@w"`) is swapped
                // inside `on_normal_key`, after this choice, so the swap is
                // looked ahead here: `w` moves every cursor. A key that
                // begins or finishes any other binding goes in once, and the
                // binding's commands each reach the cursors their own keys
                // reach (`run_one_binding`): `"zx" = "delete_selection"` used
                // to run at the first cursor and send a plain `x` to the
                // rest, because `x` alone moves every cursor.
                match self.binding_fan(key) {
                    Fan::Key(waiting, acting) => self.fan_out(waiting, acting, true, &|e| e.on_normal_key(key)),
                    Fan::Once => {
                        let before = self.before_a_primary_edit();
                        self.on_normal_key(key);
                        self.after_a_primary_edit(before);
                    }
                }
                // **An undo point is where the typing begins** (2026-10-27,
                // helix's): `A` announces it and then goes to the line's end,
                // and `u` comes back to that end, with every cursor.
                //
                // Warning: **`i`'s range kept in Normal mode's terms.** Its anchor is
                // the gap after the last character while typing; `u` lands in
                // Normal mode, where an end is the character itself. `a`'s is
                // stored as it stands, head past the end, because helix stores
                // it so: `w` `a` `X` `Esc` `u` gives back `foo b` there, measured.
                if self.mode == Mode::Insert && self.keymap_rules().undo_at_typing_start {
                    let rope = self.current_buffer().rope().clone();
                    let mut every = self.sel.clone();
                    every.map(|one| match one.anchor > one.head {
                        true => crate::selection::Range {
                            anchor: motion::prev_grapheme(&rope, one.anchor).max(one.head),
                            ..one
                        },
                        false => one,
                    });
                    let at = every.primary().head;
                    self.current_buffer_mut().retarget_pending(at, every);
                }
                KeyOutcome::Continue
            }
            Mode::Insert => {
                if !self.insert_binding(key) {
                    self.insert_typed(key);
                }
                KeyOutcome::Continue
            }
            Mode::Command => {
                self.on_command_key(key);
                KeyOutcome::Continue
            }
            Mode::Lookfor => {
                self.on_lookfor_key(key);
                KeyOutcome::Continue
            }
            Mode::Search => {
                self.on_search_key(key);
                KeyOutcome::Continue
            }
            Mode::Ruby => {
                self.on_ruby_key(key);
                KeyOutcome::Continue
            }
            Mode::Picker => {
                self.on_picker_key(key);
                KeyOutcome::Continue
            }
            Mode::Field => {
                self.on_field_key(key);
                KeyOutcome::Continue
            }
        };
        if watching {
            self.finish_watching();
        }
        // A block change is repeated as a block, from wherever `.` is pressed.
        if let Some(keys) = self.block_repeat.take() {
            if !self.repeating_edit {
                if self.edit_was_insert {
                    self.last_insert_keys = keys.clone();
                }
                self.last_edit_keys = keys;
            }
        }
        // Warning: **留人要排在忘掉前面**（2026-10-08 報的：「我在 txt 文件中有個
        // tab-separated 部分，我进入了 _tt 後， gg 會跑到文件最頂端」）。
        //
        // 兩支誰先跑決定了結果：`gg` 把光標丟到表格外面之後，
        // `forget_a_guessed_table` 看見「猜來的表格，而光標不在區域裏」就把視圖扔了；
        // 等 `hold_the_pane` 跑到的時候 `self.table` 已經是 `None`，它第一句就回來了。
        // `|` 表格没這個毛病，因為它不是猜來的（`Reach::File`）——于是同一下 `gg`，
        // markdown 裏停在表格第一行，txt 裏的制表符表格跑到文首。
        //
        // 換個順序就對了：留住之後光標回到格子裏，忘掉那一支自己就不再成立。
        // Warning: 內嵌那一種（不佔整窗）不受影響：`hold_the_pane` 第二句就問
        // `takes_the_pane()`，不佔窗就直接回來，走出猜來的塊照舊把它忘掉。
        self.hold_the_pane(held);
        self.forget_a_guessed_table();
        self.keep_the_last_selection(was_extending);
        self.go_back_after_one_key(borrowed);
        self.keep_off_the_newline();
        self.find_the_table_here();
        outcome
    }

    /// **插入態 `C-o` 借的那一個鍵走完了，把模式放回去**（vim，2026-10-06）。
    ///
    /// `borrowed` 是**這一鍵開始之前**那一格的值：`C-o` 自己那一下會把它設上，
    /// 而那一下不該算成借走的那一鍵。所以問的是進門時的快照，不是現在的。
    ///
    /// Warning: **進了別的模式就不放**：`C-o` 之後按 `:`、`/`、`i` 都是人自己要去的
    /// 地方，把他拽回插入態是搶鍵。只有還站在 Normal 上才回去。
    fn go_back_after_one_key(&mut self, borrowed: Option<crate::input::Mode>) {
        let Some(back) = borrowed else {
            return;
        };
        if self.one_normal_key != Some(back) {
            // 那一鍵自己換了這一格（又一次 `C-o`），聽它的。
            return;
        }
        // Warning: **一個「命令」不一定是一個鍵**（寫這一支的時候當場測出來的）。
        // `C-o` 之後按 `gg`：第一個 `g` 只是把 `Pending::Goto` 架起來，那一下就
        // 放回插入態的話，第二個 `g` 就打進稿子裏了。數字也一樣（`C-o` `3` `j`）。
        // 所以要等這一串**湊完**：没有半截的序列、没有攢着的數字、没有按住的別名。
        let halfway = self.pending != Pending::None
            || self.count.is_some()
            || self.operator_count.is_some()
            || self.alias_count.is_some()
            || !self.alias_held.is_empty();
        if halfway {
            return;
        }
        self.one_normal_key = None;
        if self.mode == Mode::Normal {
            self.mode = back;
        }
    }

    /// **記下延伸關掉那一刻的那一段**，`gv` 拿它重選（vim，2026-10-06）。
    ///
    /// 和 [`Self::hold_the_pane`] 同一個位置、同一條理由：這件事要在**每一鍵之後**
    /// 問一次，而「延伸是從哪幾條路關掉的」不止一條（`Esc`、`v` 再按一次、`d`、
    /// `y`、可視模式那八個鍵…）。寫在出口上，就不會漏。
    ///
    /// Warning: **動完手的那一段也記**，同 vim：`vjd` 之後 `gv` 選的是那一段原來佔的
    /// 位置。文字已經不在了，所以還原的時候要夾一次——vim 在這件事上同樣是「大致
    /// 那一塊」。
    fn keep_the_last_selection(&mut self, was: Option<(usize, usize)>) {
        let Some(span) = was else {
            return;
        };
        if self.extend {
            return;
        }
        self.last_selection = Some(span);
    }

    /// **vim 鍵位下光標不停在換行符上**（十三條的第 9 條，規劃在 §5.85）。
    ///
    /// 他報的是「`x` 在行尾會把下一行接上來」。實測比那細：
    ///
    /// ```text
    /// gl      → 列 3    停在 c 上               ✓ 本來就對
    /// glx     → ab      只吃掉 c                ✓ 本來就對
    /// glxx    → abdef   ⚠ 第二下把下一行接上來
    /// ```
    ///
    /// 漏的只有**挪動/編輯之後夾回來**那一步：第一下 `x` 之後光標落在 `ab` 的換行
    /// 符上，第二下就把它吃了。`l` 自己擋得住（`line_last`），`x` 擋不住——而擋不住
    /// 的不只 `x`。
    ///
    /// **寫在一處，不是四十處。** `clamp_cursor` 全樹叫了四十多次而它只夾到檔尾；
    /// 再往每一處加一道行内的夾，等於把一條規矩交給四十個函數去守。這裏和
    /// [`Self::hold_the_pane`] 並排，理由是它上面那一句：「a rule kept by forty
    /// movement functions is a rule one of them will break」。
    ///
    /// 四條守衛，每一條都是定下來的：
    ///
    /// * **helix 鍵位不夾**——那一端的選區本來就含換行符，兩邊各跟各的。
    /// * **插入態不夾**，不然 `A` 到不了行尾之後。
    /// * **`V` 選的整行不夾**：這個倉把「整行」表示成**含換行**的一段，夾了
    ///   `Vd` 會留下空行。現成的 `vim_lines` 旗標說的正是這件事。
    /// * **空行上停在換行符上**——没別的地方可去，照 vim。
    ///
    /// ⚠ **檔尾那條虛行也一起夾**（10-06 定）。檔案以換行結尾時我們多出一條
    /// 最後那個換行符後面的空行，`j` 下得去而 nvim 眼裏没有它：掃神諭 2256 格裏
    /// 34 格不同，**25 格全是它**（`jjhhhhhhhhhhx` 那一式）。
    fn keep_off_the_newline(&mut self) {
        if !self.keymap_rules().caret_off_newline {
            return;
        }
        // Warning: **`vim_lines && extend`，不是光看 `vim_lines`**（2026-10-06 寫這一支
        // 的時候當場測出來的）。那個旗標只在 `d`/`c`/`y` 幾個 arm 裏花掉，所以
        // `V` 之後按 `Esc`（或者任何一條不動手就走的路）它會**一直留着 true**，
        // 而夾這件事從此整個關掉。問「那一段**現在**還是不是整行可視選區」纔對。
        if self.mode != Mode::Normal || (self.vim_lines && self.extend) {
            return;
        }
        let rope = self.current_buffer().rope().clone();
        let floor = self.last_real_line(&rope);
        self.sel.map(|one| crate::selection::Range {
            anchor: hold_in_line(&rope, one.anchor, floor),
            head: hold_in_line(&rope, one.head, floor),
            ..one
        });
    }

    /// 檔尾那條虛行在 vim 鍵位下**到不了**，所以瞄到那裏的縱向動作是一次失手
    /// （nvim 裏末行按 `j` 原地不動），不是一次「走過去再夾回來」。
    ///
    /// 夾回來也不是沒道理——[`Self::keep_off_the_newline`] 本來就把頭拉回最後一
    /// 條真行——可是它夾的是**位置**，不是**列**：`foo\n` 的第一行按 `j`，光標
    /// 走到虛行行首、被拉回上一行、落在行尾，看着就是往右跳了一格。
    ///
    /// 2026-10-13 定的後一半：「光标進得去嗎？這個随 keymap，vim keymap 的話进
    /// 不去（`~` 表示这里有个结尾换行符而已），helix keymap 進得去。」前一半是
    /// 畫法：那一格照畫，`~` 當行號，同 helix。
    pub(super) fn vim_cannot_reach(&self, rope: &crate::Rope, pos: usize) -> bool {
        self.keymap_rules().caret_off_newline
            && self.mode == Mode::Normal
            && !(self.vim_lines && self.extend)
            && rope.char_to_line(pos) > self.last_real_line(rope)
    }

    /// 檔尾那條虛行之前的最後一條真行。
    ///
    /// Rope 把 `abc\n` 數成兩行——第二條是空的，在那個換行符後面。vim 没有它。
    /// 檔案不以換行結尾（或者整個是空的）時没有虛行，最後一條就是最後一條。
    fn last_real_line(&self, rope: &crate::Rope) -> usize {
        let lines = rope.len_lines();
        let phantom = lines > 1
            && rope.line_to_char(lines - 1) == rope.len_chars()
            && rope.len_chars() > 0;
        match phantom {
            true => lines - 2,
            false => lines.saturating_sub(1),
        }
    }

    /// **全窗表格 is a window onto one table, and nothing walks out of it**
    /// (2026-09-07: 「理論上不能通過鼠標滾動或者 hjkl 前往正文……衹能
    /// 通過 tq/tf/tb 離開回到其他模式，或者 t[ t] 去下一個表格」).
    ///
    /// The pane used to be a leaky mode: `gg`, `G`, `:120` and a search hit
    /// are file-wide, they walked the cursor into the chapter, and the widget
    /// — which draws between the table's first and last row and nowhere else
    /// — then had nothing to draw. The old bargain was to hand the window
    /// back to the prose page, so 全窗表格 could end without anybody asking
    /// for it. Bounding it deletes that whole class of case, and it is what
    /// lets the gutter number the table's **own** rows: inside the window
    /// every number means the same thing.
    ///
    /// One place, at the end of every key, for the reason
    /// [`Self::forget_a_guessed_table`] gives: a rule kept by forty movement
    /// functions is a rule one of them will break.
    ///
    /// It holds only what it is sure of — the same buffer, the pane still
    /// asked for — so `t o`, `t q`, `:e` and a buffer switch are none of its
    /// business.
    fn hold_the_pane(&mut self, held: Option<(u64, usize)>) {
        // `t ]` and `t [` are the way *to* another table, so they are never
        // held — and the flag is this key's, cleared however this ends.
        let crossed = std::mem::take(&mut self.crossed_tables);
        let Some((buffer, was)) = held else {
            return;
        };
        if crossed {
            return;
        }
        if !self.table.as_ref().is_some_and(|view| view.takes_the_pane()) {
            return;
        }
        if self.current_buffer().id() != buffer {
            return;
        }
        // **A table with no rows still has to hold the caret somewhere**
        // (2026-10-23). `ropey` reports a trailing empty line for every file
        // that ends in a newline (#384), and that is where `␣t d` on the last
        // row left the caret: `o` and `␣t r` then opened their row *under* it
        // and wrote the blank line into the file (`名,值,尾␊␊,,␊`). The header
        // line is the only line such a table has, and every key that means
        // 「this cell」 is already turned away there, so it is somewhere the
        // caret can safely wait for `␣t r`.
        //
        // Silently: 「删了一行」 is the sentence the reader is owed for the key
        // they pressed, and 「窗子拉得住你」 would eat it.
        if self.table_is_rowless() {
            let rope = self.current_buffer().rope();
            if rope.char_to_line(self.caret().min(rope.len_chars())) != 0 {
                let at = rope.line_to_char(0);
                self.set_cursor(at);
            }
            return;
        }
        // **Only what this key moved.** A cursor that was already off the rows
        // — parked on a header row, which only an internal call can do — is
        // not something to drag anywhere: holding it there would move the
        // caret under keys that never asked, and `t d` on the header would
        // delete the row below instead of saying 「標題行不能刪」.
        let line = self.cursor_line();
        let rope = self.current_buffer().rope();
        let home = rope.char_to_line(was.min(rope.len_chars()));
        let Some((first, last)) = self.table_row_span_at(home) else {
            return;
        };
        if !(first..=last).contains(&home) {
            return;
        }
        // Still on a row of **this** table — the table the window is showing.
        // A `G` that lands on a row of the *next* table in the document is
        // the window being walked out of just as much as one that lands in
        // the prose between them; `t ]` is the key that means to do that.
        if (first..=last).contains(&line) {
            return;
        }
        self.goto_line(line.clamp(first, last) + 1);
        self.snap_into_the_grid();
        self.status = say!("table.window-holds-you");
    }

    /// Drop a table mode that was **guessed**, once the cursor has left it.
    ///
    /// The rule for the second class of file (#275): 「如果一個文件没
    /// 有確切的表格語法，比如 txt、yaml 用空格制表符隔開…離開表格立刻回到
    /// prose 狀態，如果要再進入表格狀態需要再次按 ti tt。」
    ///
    /// A run of tab-separated lines in a chapter is the editor's inference, not
    /// the file's statement, and an inference should not be left standing on
    /// the screen after the reader has walked away from what suggested it. A
    /// `.csv`, a `.md`, a file a schema claims — those say what they are, and
    /// [`Reach::File`] keeps them on.
    ///
    /// One place, at the end of every key: a mode that has to be undone by
    /// forty movement functions is a mode that will be left on by one of them.
    pub(super) fn forget_a_guessed_table(&mut self) {
        let guessed = self
            .table
            .as_ref()
            .is_some_and(|v| v.reach == Reach::Cursor);
        if guessed && self.prose_region().is_none() {
            self.leave_table_quietly();
        }
    }

    /// Build the view the **level has already promised** (#283).
    ///
    /// `TableLevel::Basic` is the factory value, and its own words are 「the
    /// columns line up *and* the **keys** belong to the grid where a table
    /// is」. Only the first half was ever delivered: the padding asks the
    /// level (`table_padding_on`), while the keys and the no-soft-wrap rule
    /// ask [`Editor::table`] — which nothing but a `t` key ever built. So
    /// opening a `.md` gave the columns squared up with `hjkl` still walking
    /// letters and the rows still wrapping *inside their own padding*, a
    /// state neither `t o` nor `t b` names. `t b` then「did something」from a
    /// level it was already on, because what it actually did was build this.
    ///
    /// Deliberately narrower than [`Self::enter_table_as`], which is a door a
    /// person opened and may therefore write to the file:
    ///
    /// - **Only a `|` table that already parses** — [`Self::with_md_tables`]'s
    ///   answer, the same set the renderer draws, so the two cannot disagree.
    ///   A header with no `| --- |` under it gets one written by `t b`; a key
    ///   may rewrite the buffer, opening a file may not.
    /// - **Never a `.csv`, a file a schema claims, or a guessed block.** The
    ///   first two are a whole-file takeover with a cursor jump ([`Self::
    ///   table_on_open`] has them already), and the third is an inference
    ///   rule says must be asked for: 「離開表格立刻回到 prose 狀
    ///   態，如果要再進入表格狀態需要再次按 ti tt」.
    /// - **Never the pane**, which is a different question with its own key.
    /// - **Nothing is said, nothing moves, nothing is written.** No status, no
    ///   `snap_to_cell`, no `turn_for_table`: this is the level being read,
    ///   not a command being run.
    ///
    /// Gated on [`Self::table_padding_on`] rather than on the level alone, so
    /// the two halves of the promise arrive together — on a 縱書 page nothing
    /// is padded, so nothing takes the keys either, and `t b` there is still
    /// the reader's own decision. [`Self::table_here`] asks the same question
    /// of an `Md` view every time, so the halves also *leave* together: `t o`
    /// takes the keys back with the padding, and a page turned 縱 after the
    /// view was built goes quiet without the view being thrown away.
    ///
    /// The mirror of [`Self::forget_a_guessed_table`] and called from the same
    /// places: a mode that forty movement functions have to switch on is a
    /// mode that one of them will leave off.
    pub(super) fn find_the_table_here(&mut self) {
        if self.table.is_some() || !self.table_view_opens_itself() {
            return;
        }
        // **Not while the table is being typed.** The grid refuses a `|` in a
        // cell (`a_pipe_cannot_be_typed_into_a_cell`) — right, once a person
        // has said 「this is a table」, and intolerable before: a writer typing
        // `| --- | --- |` under a fresh header watches the region start
        // parsing halfway along the line and the rest of their pipes get
        // swallowed. A level read off a file may not change what typing does.
        if self.mode == Mode::Insert {
            return;
        }
        if self.syntax() != crate::syntax::Syntax::Markdown {
            return;
        }
        // **The cursor's own line first, and it is one line.** `with_md_tables`
        // walks the file on every edit, and this runs at the end of every key
        // — so a chapter with no table in it must not pay for a scan to be
        // told so. Standing on a `|` row is when the scan is worth having, and
        // it is also when the renderer is about to run it anyway.
        if !self.md_row_at_cursor() {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        let Some(first) =
            self.with_md_tables(|rows| rows.iter().find(|&&(a, b)| line >= a && line <= b).map(|&(a, _)| a))
        else {
            return;
        };
        let header = self.line_text(first).unwrap_or_default();
        self.table = Some(TableView {
            schema: crate::mdtable::schema(&header),
            from: PathBuf::new(),
            goal: 0,
            layer: Layer::default(),
            separator: Separator::Pipe,
            pane: false,
            bounds: Bounds::Md,
            // **A `|` table says what it is on every one of its own lines**,
            // so the view is the file's the moment it exists — which is what
            // keeps a table three screens down from wrapping while the cursor
            // is up here in the prose, padded and folded at once.
            reach: Reach::File,
            // A `|` cell has no quoting and cannot hold a line break.
            record_starts: None,
        });
    }

    /// If the command that just ended changed the buffer, it is what `.`
    /// repeats.
    fn finish_watching(&mut self) {
        if self.mode != Mode::Normal || self.pending != Pending::None {
            return;
        }
        if (self.current_buffer().id(), self.current_buffer().revision()) == self.edit_revision {
            return;
        }
        // A command that ended in a different buffer changed nothing here.
        if self.current_buffer().id() != self.edit_revision.0 {
            return;
        }
        // Four kinds of key change the buffer and are not *changes* in the
        // sense `.` means. Undo is the obvious one — repeating it would make
        // `.` mean "undo again" the moment you used it. A `:` line and a macro
        // are their own way of being repeated, and `.` repeating itself is not
        // a definition.
        // It is the **command's own key** that says which kind this is: the
        // first key after any count digits, and nothing after it. `3` then `.`
        // is recorded as `['3', '.']`, so looking only at the head let `.`
        // adopt a definition of itself and replay the replay — a stack
        // overflow, which is an *abort*, so nothing unwound and every unsaved
        // buffer went with it. Three keystrokes.
        //
        // Scanning the whole sequence instead was worse in the other
        // direction: `i` `3` `.` `1` `4` Esc — typing 3.14 into a cell — has a
        // `.` in it, so the insertion was refused as a definition and `.`
        // silently replayed some older edit into the document. So do neither:
        // ask what command this was. Repeating it is guarded separately, at
        // `repeat_edit`, which is what actually stops the recursion.
        let excluded = self
            .edit_keys
            .iter()
            .find(|key| !matches!(key, Key::Char(c) if c.is_ascii_digit()))
            .is_some_and(|key| {
                matches!(
                    key,
                    Key::Char(':' | '/' | '?' | 'u' | 'U' | '.' | 'q' | 'Q') | Key::Ctrl('r')
                )
            });
        if excluded || self.edit_keys.is_empty() {
            return;
        }
        if self.edit_was_insert {
            self.last_insert_keys = self.edit_keys.clone();
        }
        self.last_edit_keys = std::mem::take(&mut self.edit_keys);
    }


    /// **vim 的可視模式那八個鍵**，表在呼叫處（2026-10-06）。
    ///
    /// 三條規矩，每一條都是夜審報出來的：
    ///
    /// 一、⚠ **`V` 選的是整行，而「整行」全樹只有一處守着**——`d`/`c`/`y` 那幾個
    /// arm 開頭的 `if self.vim_lines { extend_to_line_bounds() }`。這一族寫在它們
    /// 前面並且 `return`，所以那一句在這裏要自己再做一次，不然 `Vy` 之後 `p` 是按
    /// 字符貼的（實測貼進了 `beta` 中間），而且 `vim_lines` 永遠留着 `true`，下一個
    /// `d` 會拿它當整行。`x`/`s` 例外：它們轉交給老 arm，那邊自己做。
    ///
    /// 二、**計數歸這一段**，不許漏到下一個鍵。`v` 之後 `2u` 轉小寫，接着的 `j` 從前
    /// 走兩行——那個 2 没人花掉。
    ///
    /// 三、**動完就回 Normal，光標落在那一段的開頭**。vim 的可視模式是為一次動作而
    /// 開的，一動完就結束；helix 的 select mode 不是，所以這一條只在這裏。
    /// A vim word motion cut short at the first empty line it crosses
    /// (2026-10-30): from `one two three` over a blank line, `w` lands on the
    /// blank line and `b` from below comes back to it.
    pub(super) fn stop_on_an_empty_line(&self, span: motion::Span, forward: bool) -> motion::Span {
        let motion::Span::Over { anchor, head } = span else { return span };
        let rope = self.current_buffer().rope();
        let empty = |line: usize| {
            let text = rope.line(line);
            text.len_chars() == 0 || text.chars().all(|c| c == '\n' || c == '\r')
        };
        // From where the caret is now: a caret reading's span may start at its
        // own target.
        let here = self.sel.head().min(rope.len_chars());
        let (from, to) = (rope.char_to_line(here), rope.char_to_line(head.min(rope.len_chars())));
        let hit = match forward {
            true => (from + 1..=to).find(|&line| empty(line) && rope.line_to_char(line) < head),
            false => (to..from).rev().find(|&line| empty(line) && rope.line_to_char(line) > head && rope.line_to_char(line) < here),
        };
        match hit {
            Some(line) if line + 1 < rope.len_lines() || rope.line_to_char(line) < rope.len_chars() => {
                motion::Span::Over { anchor, head: rope.line_to_char(line) }
            }
            _ => span,
        }
    }

    /// The character `column` characters into `line`, kept on the line.
    pub(super) fn column_on_line(&self, line: usize, column: usize) -> usize {
        let rope = self.current_buffer().rope();
        let line = line.min(rope.len_lines().saturating_sub(1));
        let start = rope.line_to_char(line);
        (start + column).min(motion::line_last(rope, start)).max(start)
    }

    // **vim's Visual mode acts on the whole selection** (2026-10-06): `o`
    // to its other end, `x` `s` delete and change it, `u` `U` `~` change its
    // case, `p` writes the register over it, `y` yanks it, and each leaves
    // Visual mode. They were taken ahead of the keymap; since 2026-11-01 they
    // are rows of the vim keymap's table (`typed_extending`), so a reader's
    // own binding of one of these keys still wins, and vim's `x`/`s` aliases
    // (`;{n}D`, written for Normal mode) are left out of select mode.
    /// **`J`, `>` and `<` in vim's Visual mode** (`:h v_J`, `:h v_>`): act on
    /// the lines and leave Visual mode, the cursor on the seam for `J` and
    /// back in its column for the others; `.` repeats them as the
    /// Normal-mode command they were. Run by `vim_join_selections`,
    /// `vim_indent` and `vim_unindent` while extending.
    pub(super) fn vim_visual_lines(&mut self, key: Key, c: char) {
        // The cursor's own column, read before a linewise
        // selection is widened to whole lines (nvim keeps it).
        // Under `V` the selection already reaches the line's end, so
        // the column is the one `V` was pressed in. (Not yet followed
        // through `h`/`l` inside `V`, which vim does.)
        let column = match self.vim_lines {
            true => self.vim_line_column,
            false => {
                let rope = self.current_buffer().rope();
                let head = self.sel.head().min(rope.len_chars());
                head - rope.line_to_char(rope.char_to_line(head))
            }
        };
        if self.vim_lines {
            self.extend_to_line_bounds();
            self.vim_lines = false;
        }
        let (line, lines) = {
            let rope = self.current_buffer().rope();
            let (start, end) = self.selection();
            let line = rope.char_to_line(start);
            let last = rope.char_to_line(end.saturating_sub(1).max(start));
            (line, last - line + 1)
        };
        let was = std::mem::replace(&mut self.expanding_alias, true);
        self.on_normal_key(key);
        self.expanding_alias = was;
        // **`.` repeats it as the Normal-mode command it was**
        // (2026-10-30): `3>>` for three lines indented, `3J` for
        // three joined. Recorded as the bare `>` it was a waiting
        // operator that ate the next keys.
        let mut again: Vec<Key> = lines.to_string().chars().map(Key::Char).collect();
        match c {
            'J' => again.push(Key::Char('J')),
            _ => again.extend([Key::Char(c), Key::Char(c)]),
        }
        self.block_repeat = Some(again);
        self.extend = false;
        let to = match c {
            'J' => self.sel.head(),
            _ => self.column_on_line(line, column),
        };
        self.sel.set_anchor(to);
        self.sel.set_head(to);
        self.refresh_goal_column();
        self.clamp_cursor();
    }

    pub(super) fn vim_visual_key(&mut self, c: char) {
        // `x`/`s` 原樣轉交：`d`/`c` 那兩個 arm 自己管整行與計數。
        if matches!(c, 'x' | 's') {
            let as_if = match c {
                'x' => 'd',
                _ => 'c',
            };
            return self.on_normal_key(Key::Char(as_if));
        }
        self.count = None;
        self.alias_count = None;
        // `o` 只換頭，不動手也不收工——選區還是整行，`vim_lines` 要留着。
        if c == 'o' {
            return self.flip_selection();
        }
        if self.vim_lines {
            self.extend_to_line_bounds();
            self.vim_lines = false;
        }
        // Visual `zf`: the selected lines, folded.
        if c == 'Z' {
            self.extend = false;
            let (from, to) = self.selection();
            let rope = self.current_buffer().rope();
            let (first, last) = (rope.char_to_line(from), rope.char_to_line(to.saturating_sub(1).max(from)));
            return self.fold_lines(first, last);
        }
        // Visual `gc`: the selected lines, commented.
        if c == '#' {
            self.extend = false;
            self.extend_to_line_bounds();
            let start = self.selection().0;
            self.toggle_comment(super::edits::CommentKey::Lines);
            self.sel.set_anchor(start);
            self.sel.set_head(start);
            return self.refresh_goal_column();
        }
        // `gq` leaves the cursor where vim does, on the last line it wrote.
        if c == 'q' {
            self.extend = false;
            self.extend_to_line_bounds();
            self.snapshot();
            return self.fill_selected_lines();
        }
        let start = self.selection().0;
        match c {
            'u' => self.map_selection(|one| one.to_lowercase().collect()),
            'U' => self.map_selection(|one| one.to_uppercase().collect()),
            '~' => self.map_selection(switch_case),
            'p' => self.replace_with_register(),
            _ => {
                let (anchor, head) = self.span();
                self.apply(motion::Operator::Yank, motion::Span::Over { anchor, head });
            }
        }
        self.extend = false;
        self.sel.set_head(start);
        self.sel.set_anchor(start);
        // 光標真的挪了，下一個 `j` 要從這一欄數起。
        self.refresh_goal_column();
        self.clamp_cursor();
    }

    /// **One key of a vim operator's motion** (#429, 2026-09-18).
    ///
    /// `d`, `c` and `y` in vim are operators: they wait, and what they wait
    /// for is a motion. This editor has no operators — a motion here *is* a
    /// selection, so `d` acts on what is already selected — which is why the
    /// translation table could offer `dw` and `dd` and nothing else. The wait
    /// is what this adds, and once there is a wait every motion the editor
    /// already has comes with it: `d$`, `de`, `dG`, `df,`, `di(`.
    ///
    /// **How it is carried out**: 延伸模式, the motion, and then the action —
    /// `v` `w` `D` for `dw`. `v` is what makes the steps add up, so a count
    /// works; `X` is added for the line-wise ones (`dj` takes both lines
    /// whole, as vim does); a text object needs neither, because `mi(` *is*
    /// the selection.
    ///
    /// Warning: **The action is the cutting one.** vim's `d` fills the unnamed
    /// register — `dd` then `p` puts the line back — so it is this editor's
    /// `D` (剪切), not its `d` (刪除，不動寄存器).
    fn vim_operator_key(&mut self, op: char, first: Option<char>, key: Key) {
        // Warning: **Let go of the wait before doing anything** (2026-09-18): what
        // runs below reads `pending`, and an operator still holding the wait
        // answers its own motion as though it were the next key.
        self.pending = Pending::None;
        // A count may be written inside the wait, as vim writes it (`d2w`).
        if let Key::Char(c) = key {
            if first.is_none() && c.is_ascii_digit() && (c != '0' || self.alias_count.is_some()) {
                // Warning: **Its own count, not the one before the operator**: `2d3w`
                // is six words in vim, which is the two multiplied.
                let n = self.alias_count.unwrap_or(0);
                self.alias_count = Some(
                    n.saturating_mul(10)
                        .saturating_add(c.to_digit(10).unwrap_or(0) as usize)
                        .min(1_000_000),
                );
                self.pending = Pending::VimOperator { op, first };
                return;
            }
        }
        // `d<Enter>` is `d+` (`:h <CR>`): the next line, linewise.
        let key = match (key, first) {
            (Key::Enter, None) => Key::Char('+'),
            (other, _) => other,
        };
        let Key::Char(c) = key else {
            // Anything that is not a character — Esc among them — lets go of
            // the operator without pressing anything.
            self.count = None;
            self.alias_count = None;
            return;
        };
        let grain = self.word_grain();
        // **The operator doubled is the line**: `dd`, `yy`, `cc`. `yss`, which
        // is vim-surround's 「wrap this line」, falls out of the same rule.
        //
        // **`_` is the same thing spelled the other way** (`:h _`, 2026-10-06):
        // `d_` is `dd`, and with a count it is that many lines. It is not in
        // [`yumete_cjk::keymap::VIM_MOTIONS`] because there is no motion for
        // 「this line and the n−1 below it」 — the doubled operator *is* that
        // rule, so this is where it belongs.
        if first.is_none() && (c == op || c == '_') {
            return self.run_vim_line(op);
        }
        // **`gqgq`, `gUgU`, `gugu`, `g~g~`** are the `g` operators doubled
        // with their `g` spelled again (`:h gqgq`), the same lines as `gqq`.
        if first == Some('g') && c == op && matches!(op, 'q' | 'w' | 'u' | 'U' | '~') {
            return self.run_vim_line(op);
        }
        // `gcc` is the line, as Neovim's and vim's package spell it.
        if op == '#' && first.is_none() && c == 'c' {
            return self.run_vim_line(op);
        }
        // **`d%`** (`:h %`, inclusive): from here to the other half of the
        // bracket under the cursor. The pair search is the editor's, which is
        // why it is here and not in the pure motion table.
        if first.is_none() && c == '%' {
            self.count = None;
            self.alias_count = None;
            // Off a bracket, `%` looks along the line for the first one, and
            // the span still starts where the cursor is.
            let here = self.sel.head();
            match self.vim_percent_target() {
                // From the cursor to where `%` lands, inclusive: the bracket it
                // found along the line is not part of the span when its partner
                // lies behind (2026-10-30: `jd%` on `a(b⏎x)y` took the `)` too).
                Some((_, other)) => {
                    let (anchor, head) = (here.min(other), here.max(other));
                    return self.do_vim(op, motion::Span::Over { anchor, head });
                }
                None => {
                    self.status = say!("keys.not-a-motion", "%");
                    return;
                }
            }
        }
        // **vim-surround's three** (2026-10-06): `ds{bracket}` takes a pair
        // off, `cs{old}{new}` swaps one, `ys{motion}{bracket}` puts one on.
        //
        // They are written here and not in the preset's table because `d`,
        // `c` and `y` are operators: the table's left-hand side never gets to
        // see the `s`. The editor's own spelling is `md`, `mr` and `ms`, and
        // these three set exactly the same waits — the plugin is a second
        // way to say it, not a second implementation.
        //
        // Warning: **`s` must not be a motion**, or `ds` would mean 「delete
        // the `s` motion」. It is not one ([`VIM_MOTIONS`]), which is the same
        // hole vim-surround moved into in the first place.
        if first.is_none() && c == 's' && matches!(op, 'd' | 'c' | 'y') {
            self.count = None;
            self.alias_count = None;
            self.pending = match op {
                'd' => Pending::SurroundOff,
                'c' => Pending::SurroundFrom,
                // `ys` is still owed a motion; `'s'` is what this editor calls
                // that wait, and [`Editor::do_vim`] is where it lands.
                _ => Pending::VimOperator { op: 's', first: None },
            };
            return;
        }
        // Warning: **`;` 和 `,` 也是動作**（`:h ;`）：`d;` 把剛纔那個 `f`/`t` 再做
        // 一遍，`d,` 反着做。[`crate::vim::step_for`] 是純函數，記不住「剛纔那
        // 個」——那是編輯器的記憶，不是文法的——所以這一句寫在這裏，而不是寫進
        // 那張表。從前 `d;` 說的是「不是一個動作」。
        if first.is_none() && matches!(c, ';' | ',') {
            let Some((kind, ch)) = self.last_find else {
                self.count = None;
                self.alias_count = None;
                self.status = say!("keys.not-a-motion", c.to_string());
                return;
            };
            let kind = match c {
                ',' => kind.flipped(),
                _ => kind,
            };
            // Warning: **`t` 貼着目標的時候要跳過它**（`:h ;`：「when the cursor is
            // just in front of the searched character, the `;` command will
            // find the next occurrence」）。獨立的 `;` 是靠把頭讓開一格做的
            // （`find_nth_char` 的 `again`），帶動詞的不能讓——錨點要留在原處——
            // 所以這裏改成**要下一個**：貼着第一個，就去找第二個。
            let nudge = kind.till() && self.sits_against(kind.forward(), ch);
            self.count = Some(self.count.take().unwrap_or(1).max(1) + usize::from(nudge));
            let told = match kind {
                FindKind::Forward => 'f',
                FindKind::Backward => 'F',
                FindKind::Till => 't',
                FindKind::TillBack => 'T',
            };
            if let Some(step) = crate::vim::step_for(&told.to_string(), grain, Some(ch)) {
                return self.run_vim_step(op, step);
            }
        }
        // A motion still owed a character (`df,`, `di(`) takes this one.
        if let Some(f) = first {
            if let Some(step) = crate::vim::step_for(&f.to_string(), grain, Some(c)) {
                // `dfa` is a find too: `;` and `,` repeat it afterwards
                // (`:h ;`, 2026-10-30).
                let kind = match f {
                    'f' => Some(FindKind::Forward),
                    'F' => Some(FindKind::Backward),
                    't' => Some(FindKind::Till),
                    'T' => Some(FindKind::TillBack),
                    _ => None,
                };
                if let Some(kind) = kind {
                    self.last_find = Some((kind, c));
                }
                return self.run_vim_step(op, step);
            }
        }
        let mut typed = String::new();
        if let Some(f) = first {
            typed.push(f);
        }
        typed.push(c);
        // **A key bound to a motion is that motion here too** (2026-11-01,
        // step 8 of the keymap plan, as Zed's operators read the motion
        // keys): `"Q" = "jump_next_word_end"` makes `dQ` delete to the word's
        // end. A key bound to anything else is read as vim spells it.
        if first.is_none() {
            if let Some(spelled) = self.motion_bound_to(c) {
                typed = spelled.to_string();
            }
        }
        let owed = typed.chars().next_back().unwrap_or(c);
        match crate::vim::step_for(&typed, grain, None) {
            // `f` and `i` are not finished: they are owed a character.
            Some(step) if step.asks => {
                self.pending = Pending::VimOperator { op, first: Some(owed) };
            }
            Some(step) => self.run_vim_step(op, step),
            None if crate::vim::more_ahead(&typed) => {
                self.pending = Pending::VimOperator { op, first: Some(owed) };
            }
            // Not a motion at all. vim beeps; this says which key was not one,
            // because a key that does nothing and says nothing is the thing
            // this editor is most careful never to be.
            None => {
                self.count = None;
                self.alias_count = None;
                self.status = say!("keys.not-a-motion", typed);
            }
        }
    }

    /// **`dd`, `cc`, `yy`** — the operator doubled, which in vim is 「this
    /// line」 and with a count 「this line and the next n−1」.
    /// **The motion a binding puts on `c`**, as vim spells it after an
    /// operator: the key's binding names one command, and that command is a
    /// motion ([`MOTION_STEPS`]).
    fn motion_bound_to(&self, c: char) -> Option<&'static str> {
        let commands = self.aliases().get(&c.to_string())?;
        let [one] = commands.as_slice() else { return None };
        MOTION_STEPS.iter().find(|(name, _)| name.name() == one).map(|(_, spelled)| *spelled)
    }

    fn run_vim_line(&mut self, op: char) {
        self.settle_alias_count();
        let n = self.count.take().unwrap_or(1).max(1);
        let rope = self.current_buffer().rope();
        let here = rope.char_to_line(self.sel.head());
        // A closed fold is one line (`2dd` on a fold: the fold and the line
        // after it, measured in nvim 2026-11-03).
        let (first, last) = self.lines_counted_over_folds(here, n);
        let rope = self.current_buffer().rope();
        // **`yss` is not a linewise `ys`.** vim-surround spells it
        // `^v{count}$h`, which is 「from where the writing begins to the last
        // character of the line」 — the indent stays outside the opener and
        // the newline stays outside the closer. A whole-line span would put
        // the bracket in front of the indent and after the line break.
        if op == 's' {
            let anchor = motion::line_first_non_blank(rope, rope.line_to_char(first));
            let end = rope.line_to_char((last + 1).min(rope.len_lines()));
            let head = end.saturating_sub(1).max(anchor);
            return self.do_vim(op, motion::Span::Over { anchor, head });
        }
        let span = self.line_span(first, last, op == 'c');
        self.do_vim_span(op, span, Shape::Lines);
        // `gUU` `guu` `g~~` leave the cursor at the line's start, where
        // `gUj` keeps its column (nvim 2026-11-04).
        if matches!(op, 'u' | 'U' | '~') {
            self.land_at_column(first, 0);
        }
    }

    /// `n` lines from `here`, a closed fold counting as one: the first line
    /// (a fold's first when `here` is in one) and the last.
    pub(super) fn lines_counted_over_folds(&self, here: usize, n: usize) -> (usize, usize) {
        let lines = self.current_buffer().rope().len_lines();
        let first = self.shut_around(here).map_or(here, |(first, _)| first);
        let end_of = |line: usize| self.shut_around(line).map_or(line, |(_, last)| last);
        let mut last = end_of(first);
        for _ in 1..n {
            if last + 1 >= lines {
                break;
            }
            last = end_of(last + 1);
        }
        (first, last.min(lines.saturating_sub(1)))
    }

    /// A span reaching into a closed fold, widened to the whole lines of
    /// the folds at its ends, and linewise.
    fn over_closed_folds(&self, op: char, span: motion::Span, shape: Shape) -> (motion::Span, Shape) {
        let motion::Span::Over { anchor, head } = span else { return (span, shape) };
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();
        let (a, b) = (rope.char_to_line(anchor.min(head).min(len)), rope.char_to_line(anchor.max(head).min(len)));
        let (fa, fb) = (self.shut_around(a), self.shut_around(b));
        if fa.is_none() && fb.is_none() {
            return (span, shape);
        }
        let first = fa.map_or(a, |(first, _)| first);
        let last = fb.map_or(b, |(_, last)| last);
        (self.line_span(first, last, op == 'c'), Shape::Lines)
    }

    /// The span whole lines cover. `keep_break` is `cc`'s rule: **`cc` keeps
    /// the line's own newline and `dd` takes it** — without that, what is
    /// typed next lands on the front of the line below.
    fn line_span(&self, first: usize, last: usize, keep_break: bool) -> motion::Span {
        let rope = self.current_buffer().rope();
        let anchor = rope.line_to_char(first);
        let end = rope.line_to_char((last + 1).min(rope.len_lines()));
        let head = match keep_break {
            // `cc` stops at the line's last real character…
            true => motion::line_last(rope, rope.line_to_char(last)),
            // …and `dd` takes the newline, which is the character before the
            // next line begins.
            false => motion::prev_grapheme(rope, end),
        };
        motion::Span::Over { anchor, head: head.max(anchor) }
    }

    /// **Operator ＋ motion, vim's way** (B3, 2026-09-20).
    ///
    /// The translation table is gone: nothing is replayed as keys. The motion
    /// is asked for its **caret** reading — where vim would put the cursor —
    /// and the verb then takes everything between here and there, with the
    /// motion's own class ([`crate::vim::Reach`]) deciding whether 「there」 is
    /// inside what it takes. That one word, `exclusive`, is what a table of
    /// keys could never say.
    fn run_vim_step(&mut self, op: char, step: crate::vim::Step) {
        self.settle_alias_count();
        let given = self.count.take();
        let n = given.unwrap_or(1).max(1);
        let start = self.sel.head();
        // **`d5G`, `5dG` and `d5gg` reach line 5** (`:h G`, measured in nvim
        // 2026-11-04). Walked as a motion, the count only repeated `G`, and
        // all three deleted to the file's end or its start.
        if let (Some(n), motion::Motion::FileEnd | motion::Motion::FileStart) = (given, step.motion) {
            let rope = self.current_buffer().rope();
            let (here, to) = (rope.char_to_line(start), n.max(1).saturating_sub(1).min(motion::last_line(rope)));
            return self.do_vim_lines(op, here.min(to), here.max(to));
        }
        // Warning: **`cw` ends the word it is in** — vim's own special case
        // (`:h cw`): 「When the cursor is in a word, `cw` does not include the
        // white space after a word, it only changes up to the end of the
        // word.」 A translation table cannot say this; it is not a key, it is a
        // rule about a pair.
        //
        // Warning: **不是 `ce`。** 那一句話照字面抄就寫成 `e`，而 `e` 站在詞的最後
        // 一格上跳到下一個詞去。見 [`motion::Motion::WordEndHere`]。
        //
        // Warning: **數目只管最後那一個詞。** `c2w` 是「`w` 一趟，然後換到那個詞
        // 的末尾」，不是「換兩次詞尾」——所以規則落在**最後一跳**上，前面幾跳還
        // 是 `w`。從前整個動作被換成詞尾，第二跳問到的還是同一個詞尾，於是
        // `c2w` 只換一個詞（2026-10-02 拿 nvim 逐欄量出來的）。
        let (step, finally) = match (op, step.motion) {
            ('c', motion::Motion::WordForward(grain))
                if !self.char_at_cursor().is_some_and(char::is_whitespace) =>
            {
                let reach = crate::vim::Reach::Inclusive;
                (crate::vim::Step { reach, ..step }, Some(motion::Motion::WordEndHere(grain)))
            }
            _ => (step, None),
        };
        // **An object is already both ends**; everything else is walked to.
        if step.reach == crate::vim::Reach::Object {
            // **數目交給對象**（2026-10-11；2026-10-12 連括號引號一起交）。
            // `vim::object` 造動作的時候數不到它——那個 2 在 `d2iw` 裏是按鍵，而
            // 按鍵在這一層纔收齊。
            //
            // 這裏從前只交給詞對象，註腳寫着「`2i(` 在 vim 裏是『往外第二層』，
            // 不是『兩個括號對』」——那句話是對的，而它是 `pair_span` 該怎麼讀這
            // 個數的事，不是該不該給它。給了之後 `2i(` 真的往外數一層，`2i"` 連
            // 引號一起拿（vim 的 `count < 2` 那一條）。
            let step = match step.motion {
                motion::Motion::Object { what, around, .. }
                    if matches!(
                        what,
                        motion::Object::Word { .. }
                            | motion::Object::Pair { .. }
                            | motion::Object::NearestPair
                    ) =>
                {
                    crate::vim::Step {
                        motion: motion::Motion::Object { what, around, count: n },
                        ..step
                    }
                }
                _ => step,
            };
            let span = self.read_motion(step.motion, motion::Reading::Caret);
            if span == motion::Span::Missed {
                // Warning: **The refusal has to reach the edit, not just the status
                // line**: an empty selection cuts the character under the
                // cursor, so `di(` outside a pair used to say 「没有這一對」
                // *and delete a character anyway*.
                self.status = match step.motion {
                    motion::Motion::Object { what: motion::Object::Word { .. }, .. } => {
                        say!("edit.no-word-here")
                    }
                    motion::Motion::Object { what: motion::Object::Markup, .. } => {
                        say!("edit.no-markup-here")
                    }
                    motion::Motion::Object {
                        what: motion::Object::Pair { open, close },
                        ..
                    } => say!("edit.no-pair-around", open, close),
                    motion::Motion::Object { what: motion::Object::NearestPair, .. } => {
                        say!("edit.no-pair-to-delete")
                    }
                    _ => return,
                };
                self.count = None;
                self.alias_count = None;
                return;
            }
            // Warning: **A paragraph is a run of lines, so it goes down the lines
            // path** (B5, 2026-09-21). Routed through `do_vim` instead, `dip`
            // took the lines' *text* and left their newlines behind — two
            // empty lines where a paragraph had been. The lines path already
            // knows the two rules this needs: `d` swallows the last break,
            // `c` keeps it (the `dd`/`cc` pair).
            if matches!(step.motion, motion::Motion::Object { what: motion::Object::Paragraph, .. })
            {
                if let motion::Span::Over { anchor, head } = span {
                    let rope = self.current_buffer().rope();
                    let (a, b) = (rope.char_to_line(anchor), rope.char_to_line(head));
                    return self.do_vim_lines(op, a.min(b), a.max(b));
                }
            }
            // **A sentence that fills its lines goes linewise** (2026-10-27,
            // measured against vim 9.1): `:h exclusive-linewise` — the object
            // ends in column 1 of the next line and starts at or before the
            // first non-blank, so `dis` on `Two.` between `One.` and a blank
            // line takes the line `Two.` whole. Without a next line it stays
            // characterwise: `dis` on a file's last line `Two.` leaves it
            // empty. The object itself never holds the break (`mis` must
            // not show it selected), so the rule is here, with the operator.
            if matches!(step.motion, motion::Motion::Object { what: motion::Object::Sentence, .. })
            {
                if let motion::Span::Over { anchor, head } = span {
                    let rope = self.current_buffer().rope();
                    let after = motion::next_grapheme(rope, head);
                    let breaks = rope.get_char(after) == Some('\n');
                    let more = rope.char_to_line(head) < motion::last_line(rope);
                    if breaks && more && anchor <= motion::line_first_non_blank(rope, anchor) {
                        let (a, b) = (rope.char_to_line(anchor), rope.char_to_line(head));
                        return self.do_vim_lines(op, a, b);
                    }
                }
            }
            return self.do_vim(op, span);
        }
        // Walk the motion `n` times to find where vim would have left the
        // caret. Warning: **A motion that stops moving has ended**, and one that
        // misses at all takes nothing — `df,` with no comma on the line used
        // to say so *and delete a character anyway*.
        let mut target = None;
        // Where the caret stood before the last hop, for the line rule below.
        let mut before = start;
        // Warning: **`f`/`t` 的數目是「第 n 個」，一次問完**（2026-10-02 拿 nvim 量
        // 出來的）。走底下那個迴圈的話每一趟都從上一個落點重新下錨，`d2f,` 刪的
        // 就是「第一個逗號到第二個」；而數目超出的時候迴圈留下走成的那幾跳，
        // `d9f,`（只有四個逗號）**默默吃掉三十二個字**，nvim 在那裏一格不動。
        let nth = match step.motion {
            // **`H`/`L` 的數目也是「第 n 個」**：`3H` 是從頂上數第三行，不是
            // 「到頂上去三遍」（到了就不動了，第二遍起全是空轉）。
            motion::Motion::Find { .. } | motion::Motion::Screen { .. } => n,
            _ => 1,
        };
        let hops = (n / nth.max(1)).max(1);
        for i in 0..hops {
            let what = match i + 1 == hops {
                true => finally.unwrap_or(step.motion),
                false => step.motion,
            };
            let span = self.read_motion_nth(what, motion::Reading::Caret, nth);
            let Some(head) = span.head() else { break };
            // Warning: **The first hop counts even if it does not move.** `t,` with
            // the caret already one short of the comma lands where it stands,
            // and `dt,` still takes that character — stopping here made `dt,`
            // do nothing at all.
            if i > 0 && head == self.sel.head() {
                // Warning: **數目走不完，段落和句子就整個不做**（2026-10-18 拿 nvim
                // 量出來的）。只有一個空行的時候 `d3}` 在 nvim 裏一個字都不刪，
                // 而留着已經走成的那幾跳會讓它刪到檔尾（`d2{` 在第一行上 177
                // 條錯的也是這個）。`f`/`t` 的數目不走這條路，它們一次問完
                // （上面那個 `nth`），而 `w` 在檔尾是 vim 自己開的特例。
                if matches!(
                    step.motion,
                    motion::Motion::Paragraph { .. } | motion::Motion::Sentence { .. }
                ) {
                    target = None;
                }
                break;
            }
            before = self.sel.head();
            self.sel.set_head(head);
            target = Some(head);
        }
        let landed = target;
        self.sel.set_head(start);
        let Some(target) = landed else {
            self.count = None;
            self.alias_count = None;
            return;
        };
        let rope = self.current_buffer().rope();
        // Warning: **動不了算不算數，問的是動作的類，不是動作本身**（2026-10-02
        // 拿 nvim 逐欄量出來的，`:h exclusive` 寫的也是這個）：
        //
        // - **排他**的落在原處 ＝ 零寬 ＝ 整個不做。`db` 在檔首、`d^` 在第 1 欄、
        //   `d0` 在第 1 欄、`dT,` 緊貼逗號後面，四個 nvim 都不動。
        // - **包含**的落在原處 ＝ 你站的那一格。`d$` 在行末那一格照樣刪掉它。
        // - **整行**的落在原處 ＝ 這一行。`dgg` 在第 1 行照樣刪掉第 1 行。
        //
        // 這個判斷在動作那一層寫過三版，三版都錯：寫成通則（`dl` 在行末紅）、寫
        // 成「偏移 0」（`dt,` 在檔首紅）、寫成「往回的纔算」（漏掉 `^` 和 `T`）。
        // 錯的不是哪一版，是**問錯了層**——動作只說落點，類是文法的事。
        if target == start && step.reach == crate::vim::Reach::Exclusive {
            self.count = None;
            self.alias_count = None;
            return;
        }
        let span = match step.reach {
            crate::vim::Reach::Linewise => {
                let (a, b) = (rope.char_to_line(start), rope.char_to_line(target));
                return self.do_vim_lines(op, a.min(b), a.max(b));
            }
            // Warning: **Backwards takes the target and leaves the caret's own
            // character**, whichever class the motion is: that is what `db`
            // and `dF,` do in vim.
            //
            // Warning: **光標在第 1 欄的時候，頭要退到上一行的末尾**（2026-10-02 拿
            // 這臺機器上的 nvim 量出來的）。`prev_grapheme(start)` 在第 1 欄上**就是
            // 那個換行**，於是 `j0db` 把兩行焊成一行——正文第二行的開頭按一下，
            // 上一行就接上來了。這是上面那條「`dw` 不跨行」的鏡像，而往前那一支
            // （底下的 `Reach::Exclusive`）早就擋了，往回這一支没有。
            //
            // vim 把它寫成一條通則（`:h exclusive`）：「If the motion is exclusive
            // and the end of the motion is in column 1, the end of the motion is
            // moved to the end of the previous line and the motion becomes
            // inclusive.」
            //
            // Warning: **退不動就是整個動作失敗。** 檔首按 `db`，`prev_word_start`
            // 回的是 0（它從不 `Missed`），於是 `target == start` 這一支根本不進
            // 來，落到底下的排他那一支，做出一格的跨度——nvim 在那裏**什麼都不做**。
            //
            // Warning: **只有排他的往回動作走這一支**（2026-10-02）。`ge` 是往回
            // 的**包含**動作，它要連光標自己那一格一起取：第 2 欄按 `dge`，nvim
            // 刪掉 `al` 兩個字，而這一支只給 `a`。
            _ if target < start && step.reach == crate::vim::Reach::Exclusive => {
                let head = motion::prev_grapheme(rope, start);
                match rope.char(head.min(rope.len_chars().saturating_sub(1))) == '\n' {
                    false => motion::Span::Over { anchor: target, head },
                    // Warning: **上一行是空的就別再退**（2026-10-18 拿 nvim 量出來的）。
                    // 退那一步是為了 `j0db`：上一行有字，終點該落在它的末一**格**，
                    // 而不是那個換行符。上一行空着的時候，那個換行符**就是**整行，
                    // 退過去就連下一個換行一起吃，兩行焊成一行——`))d(` 156 條錯的
                    // 都是這個。
                    true if motion::line_start(rope, head) == head => {
                        // 那一行整個就是一個換行符，取的正是「一整行（空的）」，
                        // 所以交給整行那一支——零寬的跨度會被底下那道閘擋掉，
                        // 做出「按了没反應」。
                        let line = rope.char_to_line(head);
                        return self.do_vim_lines(op, line, line);
                    }
                    true => motion::Span::Over {
                        anchor: target,
                        head: motion::prev_grapheme(rope, head),
                    },
                }
            }
            crate::vim::Reach::Inclusive => motion::Span::Over { anchor: start, head: target },
            // `w` is exclusive: 「up to the next word」, not 「including its
            // first character」.
            //
            // Warning: **And it does not cross a line end** — vim's second special
            // case (`:h word-motions`): 「When the last word moved over is at
            // the end of a line, the end of that word becomes the end of the
            // operated text, not the first word in the next line.」 Without
            // it, `dw` on the last word of a line took the newline **and the
            // next line's indent** with it, which is text the hand never saw
            // itself select.
            _ => {
                let crossed = rope.char_to_line(target) > rope.char_to_line(before);
                // Warning: **空行上沒有詞可停，取走的就是那個換行符**（2026-10-13 報
                // 的：「在空行按 dd 和 dw 無效」）。上面那條「詞不跨行」說的是「最後
                // 走過的那個**詞**在行末」——空行上一個詞都沒走過，走的是 vim 排他
                // 的第一條規矩（`:h exclusive`）：終點落在第 1 欄，終點退到上一行的
                // 末尾**而且動作變成包含式**，於是那個換行算在裏頭，空行就此消失。
                //
                // 交給 `Shape::Lines` 是因為這一段正是「一整行（空的）連它的行尾」，
                // 而底下那道閘擋的是**量出來**零寬的那一種。
                //
                // Warning: **這一條也只給詞**（2026-10-18，和底下那一條同一天同一
                // 個病）。它問的是「上一跳停在的那一行是不是空的」，而**每一個**
                // 往前的排他動作走到空行上都會中——`d2}` 的第一跳正停在段落之間
                // 的那個空行上，於是第二跳的落點被丟掉，刪到空行就停（184 條）。
                let word = matches!(step.motion, motion::Motion::WordForward(_));
                if word
                    && crossed
                    && motion::line_end(rope, before) == motion::line_start(rope, before)
                {
                    let at = motion::line_start(rope, before);
                    let span = motion::Span::Over { anchor: start.min(at), head: at };
                    return self.do_vim_span(op, span, Shape::Lines);
                }
                // Warning: **行末那道夾子是給詞的，不是通則**（2026-10-18 拿 nvim 量
                // 出來的）。`:h word-motions` 那一條只說「最後走過的那個**詞**在行
                // 末」，而這裏把它套在了每一個往前的排他動作上，於是 `d}` 184 條全
                // 錯、`d)` 錯 148 條——兩個都該走到下一段/下一句的頭上，卻被收回起
                // 點那一行的末尾。
                //
                // 其餘排他動作走 vim 自己的通則（`:h exclusive`）：終點落在第 1 欄，
                // 終點退到**終點前一行**的末尾、動作變成包含式；起點又在首個非空白
                // 之前的話，整個變成整行。注意退的是**終點**前一行，不是起點那一行
                // ——兩者在跨一行的時候恰好相同，所以這個錯在 `dw` 上看不出來。
                let lands_on_col_1 = target == motion::line_start(rope, target);
                if !word && crossed && lands_on_col_1 {
                    let above = rope.char_to_line(target) - 1;
                    let first = motion::line_first_non_blank(rope, start);
                    if start <= first {
                        return self.do_vim_lines(op, rope.char_to_line(start), above);
                    }
                    let end = motion::line_end(rope, rope.line_to_char(above));
                    let head = motion::prev_grapheme(rope, end).max(start);
                    return self.do_vim(op, motion::Span::Over { anchor: start, head });
                }
                let end = match crossed && word {
                    true => motion::line_end(rope, before),
                    false => target,
                };
                let head = motion::prev_grapheme(rope, end).max(start);
                // Warning: **vi 的那條怪規矩，只有 `d` 有。** vim 自己的註釋就叫它
                // 「Imitate the strange Vi behaviour」（`op_delete`）：刪掉的跨過
                // 一行以上，終點後面只剩空白，起點又在縮進裏，整段就變成整行。
                // 手冊只寫了終點落在第 1 欄那一條（`:h exclusive-linewise`），這
                // 一條只在代碼裏——`}` 走到檔尾的時候終點是**包含**的，第 1 欄那
                // 條夠不着，而 nvim 照樣整行整行地刪（2026-10-18 逐欄量過：
                // `  he said` 第 0/1/2 欄 `d}` 把整個檔刪空，第 3 欄留下 `  h`）。
                // `c}` 和 `y}` 在同一處是逐字的，一併量過。
                let tail_blank = || {
                    let after = motion::next_grapheme(rope, head);
                    let stop = motion::line_end(rope, head);
                    after >= stop || rope.slice(after..stop).chars().all(char::is_whitespace)
                };
                if op == 'd'
                    && rope.char_to_line(head) > rope.char_to_line(start)
                    && start <= motion::line_first_non_blank(rope, start)
                    && tail_blank()
                {
                    let (a, b) = (rope.char_to_line(start), rope.char_to_line(head));
                    return self.do_vim_lines(op, a, b);
                }
                motion::Span::Over { anchor: start, head }
            }
        };
        self.do_vim(op, span);
    }

    /// Whole lines, for a linewise motion (`dj`, `dG`).
    fn do_vim_lines(&mut self, op: char, first: usize, last: usize) {
        let span = self.line_span(first, last, op == 'c');
        self.do_vim_span(op, span, Shape::Lines);
    }

    /// Do the verb, and say what the old `play_vim_motion` said about each.
    fn do_vim(&mut self, op: char, span: motion::Span) {
        self.do_vim_span(op, span, Shape::Chars)
    }

    /// 同上，而且說清楚這一段是**整行**還是行內的一截——那道「落在換行符上就什麼
    /// 都不做」的閘只擋行內的（見底下）。
    fn do_vim_span(&mut self, op: char, span: motion::Span, shape: Shape) {
        // **An operator takes a closed fold whole** (`:h fold-behavior`: 「dl
        // deletes the whole closed fold under the cursor」, measured).
        let (span, shape) = match op {
            's' => (span, shape),
            _ => self.over_closed_folds(op, span, shape),
        };
        // **`ys{motion}{bracket}`**: the motion only says what to wrap, so
        // nothing is edited yet and there is nothing to take a snapshot of.
        // Select what it named and wait for the bracket, which is what `ms`
        // does — [`Editor::surround_add`] takes its own snapshot.
        if op == 's' {
            let motion::Span::Over { anchor, mut head } = span else {
                return;
            };
            // Warning: **The blanks at the tail stay outside the brackets.**
            // vim-surround does this and says so in one line of `s:opfunc`:
            // `let keeper = substitute(keeper,'\_s\@<!\s*$','','')`, with
            // the piece it cut off put back after the closer. So `ysw` on
            // `hello world` gives `(hello) world`, not `(hello )world` —
            // and the hand that typed `ysw` meant the word, not the gap.
            let rope = self.current_buffer().rope();
            while head > anchor && rope.char(head).is_whitespace() {
                head -= 1;
            }
            self.take_object(motion::Span::Over { anchor, head });
            self.pending = Pending::Surround;
            return;
        }
        // **Where the cell is the unit, the motion is already spoken for**
        // (2026-10-23 作者定). The grid claims `d`, `c` and `y` at the whole
        // cell layer and hands them the wait ([`Editor::table_motion`]); the
        // span the motion worked out is a span of characters, and this layer
        // has no characters — it has one cell. `dd` `dw` `d$`, `cc` `cw` `cl`
        // and `yy` therefore all mean that cell.
        if matches!(op, 'd' | 'c' | 'y') && self.cell_is_the_unit() {
            self.count = None;
            self.alias_count = None;
            return self.cell_operator(op);
        }
        // Warning: **動作落在換行符上，vim 就是「什麽都没動」**（2026-10-09 審出來的）。
        //
        // `D` 在這一端拼成 `d$`，而 `$` 走 `line_last`——空行上那一格正是換行符，段寬
        // 是零。可 `apply` 要往後多吃一個字素（helix 的光標含自己那一格），於是
        // `DD` 的第二下把兩行焊成了一行，`D.....` 一路啃下去。`line_last` 自己的註釋
        // 早寫過這件事（#382：「`d` 吃掉那條換行，把兩行焊成一行」），只是零寬那一格
        // 漏在外頭。
        //
        // 只擋「整段就是那個換行符」：非空行的末字上 `dl`／`x` 照舊刪得掉它（那一格
        // 不是換行符），而 `dw` 從空行走到下一行是真的跨行，段寬不是零，不受影響。
        //
        // Warning: **整行那一路不在這道閘底下**（2026-10-13 報的）。`dd` 在空行上
        // 產生的正是同一個形狀的段（anchor ＝ head ＝ 那個換行符），於是一起被擋
        // 住——空行上 `dd` 什麼都不刪，只能進插入模式按退格。而 `dd` 要的恰恰是那
        // 個換行符：vim 的 `dd` 刪的是「這一行連同它的行尾」。
        if matches!(op, 'd' | 'c' | 'y') && shape == Shape::Chars {
            if let motion::Span::Over { anchor, head } = span {
                let rope = self.current_buffer().rope();
                let on_a_break =
                    rope.get_char(anchor).is_some_and(|c| c == '\n' || c == '\r');
                if anchor == head && on_a_break {
                    return;
                }
            }
        }
        // **Whole lines keep the column the cursor wants** (nvim's
        // 'nostartofline', measured 2026-11-04): `dd` `dj` `dG` on the line
        // left where the first one was, `yk` `yH` `gUj` `>k` `zfk` on the
        // first line taken. They used to go to its start.
        let kept = (shape == Shape::Lines).then(|| {
            let rope = self.current_buffer().rope();
            let top = match span {
                motion::Span::Over { anchor, head } => rope.char_to_line(anchor.min(head).min(rope.len_chars())),
                motion::Span::Missed => rope.char_to_line(self.sel.head()),
            };
            (top, self.column_wanted())
        });
        self.snapshot();
        // 見 [`Editor::yank_whole_lines`]：`dd`／`yy` 取的是整行，末行缺終止符就補。
        self.yank_whole_lines = shape == Shape::Lines;
        match op {
            // vim's `d` and `c` fill the unnamed register.
            'd' => {
                self.apply(motion::Operator::Cut, span);
                if let Some((_, want)) = kept {
                    self.land_at_column(self.cursor_line(), want);
                }
            }
            'c' => match (shape, span) {
                (Shape::Lines, motion::Span::Over { anchor, head }) => {
                    let rope = self.current_buffer().rope();
                    let (a, b) = (anchor.min(head).min(rope.len_chars()), anchor.max(head).min(rope.len_chars()));
                    let (first, last) = (rope.char_to_line(a), rope.char_to_line(b));
                    self.change_whole_lines(first, last, true);
                }
                _ => self.apply(motion::Operator::Change { cut: true }, span),
            },
            // `>` and `<` want the selection, because indenting is a
            // line-shaped edit the editor already knows how to do.
            // The cursor keeps its column (nvim's `nostartofline`, measured).
            '>' | '<' => {
                let (line, column) = {
                    let rope = self.current_buffer().rope();
                    let head = self.sel.head();
                    let line = rope.char_to_line(head);
                    (line, head - rope.line_to_char(line))
                };
                self.take_object(span);
                self.extend_to_line_bounds();
                self.indent(op == '>');
                match kept {
                    // On the first line shifted, at the column wanted:
                    // `>k` lands a line up.
                    Some((top, want)) => {
                        self.extend = false;
                        self.sel.set_anchor(self.sel.head());
                        self.land_at_column(top, want);
                    }
                    None => {
                        let to = self.column_on_line(line, column);
                        self.sel.set_anchor(to);
                        self.sel.set_head(to);
                        self.refresh_goal_column();
                    }
                }
                // A closed fold shifted opens (nvim, 2026-11-04).
                if self.shut_around(line).is_some() {
                    self.open_folds_at(line, usize::MAX);
                }
            }
            // **`gq`** — the lines the motion crosses, filled (2026-10-29).
            // `gw` is the same with the cursor put back (vim's, 2026-11-02).
            'q' | 'w' => {
                let kept = (op == 'w').then(|| self.sel.head());
                self.take_object(span);
                self.extend_to_line_bounds();
                self.fill_selected_lines();
                if let Some(at) = kept {
                    let at = at.min(self.current_buffer().rope().len_chars());
                    self.sel.set_anchor(at);
                    self.sel.set_head(at);
                    self.clamp_cursor();
                    self.refresh_goal_column();
                }
            }
            // **`zf`** — the lines the motion crosses, folded (2026-11-03).
            'Z' => {
                if let motion::Span::Over { anchor, head } = span {
                    let rope = self.current_buffer().rope();
                    let (a, b) = (anchor.min(head), anchor.max(head));
                    let (first, last) = (rope.char_to_line(a), rope.char_to_line(b.min(rope.len_chars())));
                    self.fold_lines(first, last);
                }
                if let Some((top, want)) = kept {
                    self.land_at_column(top, want);
                }
            }
            // **`gc`** — the lines the motion crosses, commented as `Space A-c`
            // comments them (vim 9.1's comment package, Neovim's `gc`).
            '#' => {
                self.take_object(span);
                self.extend_to_line_bounds();
                let start = self.selection().0;
                self.toggle_comment(super::edits::CommentKey::Lines);
                self.extend = false;
                self.sel.set_anchor(start);
                self.sel.set_head(start);
                self.refresh_goal_column();
            }
            // **`gu`／`gU`／`g~`** — 換字形，選區就是那一段（2026-10-06）。
            'u' | 'U' | '~' => {
                self.take_object(span);
                match op {
                    'u' => self.map_selection(|one| one.to_lowercase().collect()),
                    'U' => self.map_selection(|one| one.to_uppercase().collect()),
                    _ => self.map_selection(crate::editor::switch_case),
                }
                if let Some((top, want)) = kept {
                    self.extend = false;
                    self.sel.set_anchor(self.sel.head());
                    self.land_at_column(top, want);
                }
            }
            // **`y` leaves the cursor at the head of what it took** — vim's
            // rule, and why `yyp` puts the copy directly under the line.
            _ => {
                let head = match span {
                    motion::Span::Over { anchor, head } => anchor.min(head),
                    motion::Span::Missed => return,
                };
                self.apply(motion::Operator::Yank, span);
                self.set_cursor(head);
                self.sel.set_anchor(head);
                if let Some((top, want)) = kept {
                    self.land_at_column(top, want);
                }
            }
        }
        self.yank_whole_lines = false;
    }

    /// **A key typed in Insert mode**, at every cursor when it types (one
    /// list of completions is one list, not one per cursor).
    pub(super) fn insert_typed(&mut self, key: Key) {
        // Warning: **補全單子開着的時候一次就夠**：那是一張浮在上面的單子，`Tab` 與
        // `C-n` 歸它，不是往 N 處各寫一次。
        let (was_len, was_head) = (self.current_buffer().rope().len_chars(), self.sel.head());
        self.stepped_over = false;
        match multi::types_at_every_selection(key) && self.offers_here().is_none() {
            true => self.edit_each_from(false, |e| e.insert_key_carrying_the_tail(key)),
            false => self.insert_key_carrying_the_tail(key),
        }
        // **A move ends the selection `i` or `a` kept** (2026-10-27,
        // measured: helix's `i` `→` `X` leaves a one-character cursor).
        // From here on typing is plain typing, at every cursor. A closer
        // stepped over is typing (2026-10-29, review).
        if self.mode == Mode::Insert
            && !std::mem::take(&mut self.stepped_over)
            && self.current_buffer().rope().len_chars() == was_len
            && self.sel.head() != was_head
        {
            self.insert_keeps_tail = false;
            self.keep_anchor_while_inserting = false;
        }
    }

    /// **Which cursors a Normal key reaches, when a binding is in it**: as
    /// the key a one-key binding to one key becomes (`"C-q" = "@w"` → `w`),
    /// once for any other key that begins or finishes a binding (its
    /// commands then reach the cursors themselves, see `run_one_binding`),
    /// and as itself otherwise.
    fn binding_fan(&self, key: Key) -> Fan {
        let raw = Fan::Key(self.pending, key);
        // A key of the reader's in a group (`keymap.rs`).
        if self.pending != Pending::None && self.node_takes(key) {
            return Fan::Once;
        }
        if self.pending != Pending::None || self.expanding_alias || (self.aliases().is_empty() && self.tree().root.is_empty()) {
            return raw;
        }
        if !self.alias_held.is_empty() {
            return Fan::Once;
        }
        let typed = match key {
            Key::Char(c) if !(c.is_ascii_digit() && self.count.is_some()) => c.to_string(),
            Key::Char(_) => return raw,
            Key::Alt(c) => format!("{}{c}", yumete_cjk::actions::ALT),
            key => match typed_as(key) {
                Some(c) => c.to_string(),
                None => return raw,
            },
        };
        if self.root_node(&typed).is_some() || self.aliases().keys().any(|k| k.len() > typed.len() && k.starts_with(typed.as_str())) {
            return Fan::Once;
        }
        let Some(commands) = self.aliases().get(&typed) else { return raw };
        if let [one] = commands.as_slice() {
            if let Some(yumete_cjk::actions::Bound::Keys(k)) = yumete_cjk::actions::bound(one) {
                let mut cs = k.chars();
                if let (Some(c), None) = (cs.next(), cs.next()) {
                    return Fan::Key(Pending::None, Key::Char(c));
                }
            }
        }
        Fan::Once
    }

    /// **Run `run` at the cursors a key reaches**: every selection for a
    /// motion, each in turn for an edit, the primary alone otherwise (then,
    /// when `primary`, the others follow the edit made there).
    pub(super) fn fan_out(&mut self, waiting: Pending, acting: Key, primary: bool, run: &dyn Fn(&mut Editor)) {
        // vim's `]p` `[p` put, which is an edit; helix's are paragraph moves.
        let vim_put = self.vim_keymap() && matches!(waiting, Pending::Hop { .. }) && matches!(acting, Key::Char('p' | 'P'));
        if !vim_put
            && (multi::each_selection_key(&waiting, acting)
                || (waiting == self.pending && self.copies_every_selection(acting)))
        {
            self.each_selection(run);
        } else if vim_put || multi::edits_every_selection(&waiting, acting) {
            if let Key::Char(c) = acting {
                if !self.every_selection_has_its_own_pair(waiting, c) {
                    self.pending = Pending::None;
                    return;
                }
            }
            // **`J` joins a line once however many cursors are on it**
            // (2026-10-29, review: two on one line joined it twice;
            // helix sorts the joins and drops the repeats). The first
            // cursor on each line joins; the others ride along.
            let joins = matches!(acting, Key::Char('J') | Key::Alt('J'));
            // Likewise `C-a`/`C-x`: two cursors that reach the same
            // number bump it once.
            let bumps = matches!(acting, Key::Ctrl('a') | Key::Ctrl('x'));
            let owners = ((joins || bumps) && self.sel.is_plural()).then(|| match joins {
                true => self.first_cursor_on_each_line(),
                false => self.first_cursor_on_each_number(),
            });
            self.edit_each(|e| {
                if let Some(owners) = &owners {
                    let rope = e.current_buffer().rope();
                    let here = match joins {
                        true => Some(rope.char_to_line(e.selection().0.min(rope.len_chars()))),
                        false => e.number_to_bump(e.sel.anchor(), e.sel.head()).map(|(from, _)| from),
                    };
                    if here.and_then(|at| owners.get(&at)) != e.edit_nth.as_ref() {
                        return;
                    }
                }
                run(e)
            });
        } else if primary {
            let before = self.before_a_primary_edit();
            run(self);
            self.after_a_primary_edit(before);
        } else {
            run(self);
        }
    }

    /// A Normal command a binding names, at the cursors its own key reaches
    /// (2026-11-01): `"zx" = "delete_selection"` deletes at every cursor, as
    /// `d` does, and so does each command of a list.
    fn run_normal_at_every_cursor(&mut self, name: &str) {
        // In Insert mode a bound name types as its own key would, at every
        // cursor (found in review, 2026-11-02: `"C-l" = "delete_char_backward"`
        // reached the primary alone).
        if self.mode == Mode::Insert && self.sel.is_plural() {
            if let Some(key) = self.insert_key_for(name).filter(|k| multi::types_at_every_selection(*k)) {
                self.insert_typed(key);
                return;
            }
        }
        let place = (self.mode == Mode::Normal && self.sel.is_plural())
            .then(|| super::normal::place_of(name, &self.group_context()).or_else(|| keyless_place(name)))
            .flatten();
        match place {
            Some((waiting, acting)) => self.fan_out(waiting, acting, false, &|e| {
                e.run_normal(name);
            }),
            None => {
                self.run_normal(name);
            }
        }
    }

    /// The Insert-mode key that runs `name` by the default table.
    fn insert_key_for(&self, name: &str) -> Option<Key> {
        super::insert::INSERT_KEYS
            .iter()
            .find(|(_, cmd, when)| cmd.name() == name && when.is_none_or(|holds| holds(self)))
            .map(|(key, _, _)| *key)
    }

    /// **What the right-hand side of a binding says to do** (#429).
    ///
    /// Three kinds, each marked as what it is (see [`yumete_cjk::actions::bound`]):
    ///
    /// 1. `:something` — that command line, exactly as it would be typed.
    /// 2. An **action's name** (`delete_selection`), bare. Names are what a
    ///    reader should write: `x = "delete_selection"` goes on meaning that
    ///    after the day the editor's own `d` moves somewhere else, and it says
    ///    what it does without the reader knowing which key does it today.
    /// 3. `@xxd` — **keys**, played as though typed (#428), which is what the
    ///    vim preset is made of.
    ///
    /// Warning: **A right-hand side that is none of the three plays nothing.** It
    /// used to fall through to 「then it must be keys」, and a misspelt name
    /// therefore typed its own letters into the chapter. The config refuses such
    /// a line and never installs it, so nothing should reach here — and if
    /// something ever does, the manuscript is not where it gets to say so.
    pub(super) fn run_binding(&mut self, commands: &[String]) {
        // **A list runs in order** (helix's), and stops at a quit.
        for one in commands {
            self.run_one_binding(one);
            if self.quit_from_binding {
                break;
            }
        }
    }

    fn run_one_binding(&mut self, bound: &str) {
        match yumete_cjk::actions::bound(bound) {
            Some(yumete_cjk::actions::Bound::Command(line)) => self.run_bound_command(line),
            // `A-d`、`C-o` 這種寫法在這裏纔翻成內部拼法。
            Some(yumete_cjk::actions::Bound::Keys(keys)) => {
                self.play_keys(&yumete_cjk::actions::written(keys))
            }
            Some(yumete_cjk::actions::Bound::Named(action)) => match action.how {
                yumete_cjk::actions::How::Command(line) => self.run_bound_command(line),
                yumete_cjk::actions::How::Normal => self.run_normal_at_every_cursor(action.name),
            },
            None => {}
        }
    }

    /// **A command line a binding names, run as if typed** — and what it
    /// says heard (2026-10-31). It used to be `let _ = self.execute(…)`, so a
    /// bound `:quit` never left and a failing command said nothing; reported
    /// with `"C-q" = ":quit"`.
    pub(super) fn run_bound_command(&mut self, line: &str) {
        match self.execute(&format!(":{line}")) {
            Ok(crate::editor::CommandOutcome::Quit) => self.quit_from_binding = true,
            Ok(crate::editor::CommandOutcome::Continue) => {}
            Err(err) => self.status = err.to_string(),
        }
    }

    /// Press `keys` as though they had been typed, **without** reading them
    /// as aliases again — an alias whose expansion uses its own key would
    /// otherwise call itself forever — and without recording them twice.
    ///
    /// A count typed before the alias goes to the first key that acts, not to
    /// a leading `;`: `2x` is `;` then `2D`, and `3dd` is `3x` then `d`.
    /// **Keys the reader typed and a binding held**, played as typed: the
    /// flavour's gates apply to them (vim's `Zj` with the flavour off is
    /// vim's beep, not helix's sticky view; found 2026-11-03).
    pub(super) fn play_held(&mut self, keys: &str) {
        let outer = std::mem::replace(&mut self.replaying_held, true);
        self.play_keys(keys);
        self.replaying_held = outer;
    }

    fn play_keys(&mut self, keys: &str) {
        // Warning: **Saved and restored, not set and cleared** (2026-09-19). An
        // operator inside an expansion replays its motion through this same
        // function, and the inner call's `= false` on the way out re-armed the
        // alias layer for the **rest of the outer string**: the trailing key of
        // `Q = "dwQ"` was expanded again and the stack went with it — an abort,
        // which takes unsaved buffers with it. Two keys, one written `ddx` and
        // one `wwx`, ended in two different meanings of `x`.
        let outer = self.expanding_alias;
        self.expanding_alias = true;
        let mut count = self.count.take();
        // **`{n}` says where the count goes** (2026-09-18). Without it the
        // count lands on the first key that acts, which is right for
        // `dd` → `xd` and wrong for `dw`: there the count belongs to the `w`
        // that is *extending* the selection, not to the `v` that opened it.
        let placed = keys.contains("{n}");
        let spelled: Vec<char> = keys.chars().collect();
        let mut at = 0;
        while at < spelled.len() {
            if spelled[at] == '{' && spelled.get(at + 1) == Some(&'n') && spelled.get(at + 2) == Some(&'}')
            {
                if let Some(n) = count.take() {
                    self.count = Some(n);
                }
                at += 3;
                continue;
            }
            let c = spelled[at];
            if !placed && c != ';' {
                if let Some(n) = count.take() {
                    self.count = Some(n);
                }
            }
            // Alt 那一種是兩個 `char`（見 `actions::ALT`）：一下按出去，走兩格。
            let key = match c == yumete_cjk::actions::ALT {
                true => match spelled.get(at + 1) {
                    Some(&next) => Key::Alt(next),
                    None => break,
                },
                false => pressed(c),
            };
            at += if c == yumete_cjk::actions::ALT { 2 } else { 1 };
            // A run that reaches a quit (`@:quit<enter>`) stops there, and the
            // `on_key` above this one is told.
            if self.on_key(key) == KeyOutcome::Quit {
                self.quit_from_binding = true;
                break;
            }
        }
        self.expanding_alias = outer;
    }

    /// **The count a sequence was written with**, from before it and from
    /// inside it (2026-09-18).
    ///
    /// vim multiplies the two — `2d3w` is six words — and one of them is
    /// almost always absent, so this is the whole of that rule.
    pub(super) fn settle_alias_count(&mut self) {
        let inside = self.alias_count.take();
        self.count = match (self.count.take(), inside) {
            (Some(before), Some(after)) => Some(before.saturating_mul(after)),
            (before, after) => before.or(after),
        };
    }

    pub(super) fn on_normal_key(&mut self, key: Key) {
        self.status.clear();
        let continuing_zong = std::mem::take(&mut self.zong_motion);

        // A pending multi-key operator consumes this key.
        // 按過的那一串，趁各層還没把自己關掉（見 `no_such_combination`）。
        let typed_before = self.typed_so_far();
        // An `extend_*` wait that was let go of is not asked any more.
        if self.pending == Pending::None && !self.jumping() {
            self.motion_extend = None;
        }
        // **A group's key**: the reader's keys laid over the group, then its
        // own rows (`keymap.rs`, `groups.rs`).
        if self.pending.is_node() {
            self.on_node_key(key, &typed_before, continuing_zong);
            return;
        }
        if self.pending == Pending::None {
            self.node_trail.clear();
        }
        match self.pending {
            Pending::Mark => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.set_mark(c);
                }
                return;
            }
            Pending::Recall => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.go_to_mark(c);
                }
                return;
            }
            // The five that are waiting for a character of the manuscript.
            // They answer in one place because the character may arrive as a
            // key or as an IME commit (#414), and the two used to be written
            // in two files with only `r` in the second one.
            waiting @ (Pending::Find(_)
            | Pending::Replace
            | Pending::MatchPair { .. }
            | Pending::Surround
            | Pending::SurroundOff
            | Pending::SurroundFrom
            | Pending::SurroundTo(_)) => {
                // Spelt out rather than guarded by a predicate, because a
                // guard does not count towards exhaustivity and this match is
                // what catches a new `Pending` nobody handled.
                //
                // Warning: **這張表和 `wants_the_ime` 有意不同**（2026-10-04）。從前
                // 兩張表是同一張，這裏還有一句 `debug_assert` 把它們釘在一起；而
                // `mi`/`ma` 吃一個字符鍵、卻不要輸入法——「吃不吃鍵」和「要不要
                // 輸入法」本來就是兩個問題，那一句斷言是在逼它們答同一個。
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.answer_with_char(waiting, c);
                } else if key == Key::Enter && waiting == Pending::Surround {
                    self.surround_with_lines();
                } else if key == Key::Enter && waiting == Pending::Replace {
                    // `r<Enter>`: a line break, in both keymaps (`:h r`,
                    // helix's `replace` with a newline).
                    self.answer_with_char(waiting, '\n');
                }
                // Spent either way: a count typed before `f` belongs to that
                // `f` and must not be left lying for the next command — unless
                // the answer asked for another key (`mr`'s first one), whose
                // command the count is still waiting for (2026-10-27).
                if self.pending == Pending::None {
                    self.operator_count = None;
                }
                return;
            }
            Pending::Register => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.pending_register = Some(c);
                }
                return;
            }
            // vim's `q{a-zA-Z0-9}` and `@{a-z0-9"@}`: the letter names the
            // register (`qA` adds to `a`; `@@` plays the last one played).
            // Any other key lets the wait go.
            Pending::MacroRegister { record, count } => {
                self.pending = Pending::None;
                match (record, key) {
                    (true, Key::Char(c)) if c.is_ascii_alphanumeric() => self.start_recording(c),
                    (false, Key::Char('@')) => match self.last_played {
                        Some(name) => self.play_register(name, count),
                        None => self.status = say!("macro.nothing-played"),
                    },
                    (false, Key::Char(c)) if c.is_ascii_alphanumeric() || c == '"' => {
                        self.play_register(c.to_ascii_lowercase(), count)
                    }
                    _ => {}
                }
                return;
            }
            // **The pending nobody typed.** `:s …c` opens it and it stays open
            // across many keys — one answer per match — instead of one key
            // finishing it the way every other pending here works.
            Pending::Confirm => {
                self.answer_confirm(key);
                return;
            }
            Pending::SwapRange => {
                self.pending = Pending::None;
                let asked = self.swap_asked.take();
                // `y` 纔改，別的鍵一律是「不動」——同下面那一問的理由：一個没人
                // 想按的鍵不該替一條會改幾行字的命令答「好」。
                if matches!(key, Key::Char('y') | Key::Enter) {
                    if let Some(line) = asked {
                        self.swapping_is_fine = true;
                        let _ = self.execute(&line);
                        self.swapping_is_fine = false;
                    }
                } else {
                    self.status = String::new();
                }
            }
            // **A vim operator, waiting for its motion** (#429, 2026-09-18).
            Pending::VimOperator { op, first } => {
                self.vim_operator_key(op, first, key);
                return;
            }
            // 撤銷斷點只在插入模式裏按，走到 Normal 這一支就是它已經過期了。
            Pending::UndoBreak => self.pending = Pending::None,
            Pending::None => {}
            // The groups, answered above.
            _ => {}
        }


        // Apply user key aliases (config `[keys.normal]`) to command keys only;
        // pending operator targets above are taken literally.
        //
        // The right-hand side may be several keys — `"J" = "gJ"` puts join
        // back — so an alias that is not one character is *played* rather than
        // swapped, and everything downstream sees the keys it would have seen
        // if they had been typed. `self.replaying` stops an alias for a key
        // that its own expansion uses from calling itself forever.
        //
        // **The left may be a sequence too** (#428) — `dd`, which is what a vim
        // preset is made of. A key that begins a longer alias is *held*, and
        // the next key decides: it completes one (played), or it does not, and
        // then what was held is pressed for real and the new key goes through
        // as itself. `Esc` lets go of what was held without pressing it.
        let key = if self.expanding_alias || (self.aliases().is_empty() && self.tree().root.is_empty()) {
            key
        } else {
            let typed = match key {
                // A digit inside a count is the count's, not an alias — `10`
                // must not fire a `0`.
                //
                // Warning: **Unless a sequence is being held** (2026-09-18): in
                // `2d3w` the `3` arrives with a count already under way, and
                // dropping it here handed the held `d` back with the *first*
                // count on it — two characters deleted where vim deletes six
                // words. Inside a held sequence the digit is answered below.
                Key::Char(c)
                    if !(c.is_ascii_digit()
                        && self.count.is_some()
                        && self.alias_held.is_empty()) =>
                {
                    Some(c.to_string())
                }
                // **Alt 也綁得了**（2026-10-08）。鍵位表的鍵是一個 `String`，而
                // Alt 没有自己的字節——借一個私用區的碼位當前綴（`actions::ALT`），
                // 於是 `A-d` 在表裏是兩個 `char`，而查表那一層一個字都不用改。
                //
                // Warning: 從前這裏只認 `Key::Char`，於是配置裏寫 `"A-d" = …` 綁的是
                // `A`、`-`、`d` 三個鍵連按：那個鍵永遠按不出來，而配置照收。
                Key::Alt(c) => Some(format!("{}{c}", yumete_cjk::actions::ALT)),
                // **其餘的鍵也綁得了**（2026-10-12）：`C-x` 那一族、`<tab>`、
                // `<S-tab>`、方向鍵。寫在 [`typed_as`] 裏一處，和 [`pressed`]
                // 逐條對着——少一邊就是「寫得進配置而按不出來」。
                Key::Char(_) => None,
                key => typed_as(key).map(|c| c.to_string()),
            };
            if key == Key::Esc && !self.alias_held.is_empty() {
                self.alias_held.clear();
                self.alias_count = None;
                return;
            }
            // **A key a table of the reader's took opens that group**
            // (`[keys.normal.","]`), the count typed before it kept for its keys.
            if self.alias_held.is_empty() {
                if let Some(n) = typed.as_deref().and_then(|typed| self.root_node(typed)) {
                    self.pending = self.node_pending(n);
                    self.group_count = self.count.take();
                    return;
                }
            }
            let mut held = std::mem::take(&mut self.alias_held);
            if let Some(typed) = typed {
                // 一下按出來的可能是兩個 `char`（Alt 那一種），所以這裏接的是一段
                // 而不是一個字。數字那條規矩只問單字符，Alt 不是數字。
                let c = typed.chars().next_back().unwrap_or('\0');
                let alone = typed.chars().count() == 1;
                // **A digit inside a held sequence is a count** (2026-09-18):
                // vim writes `d10w`, and `d1` is not the name of anything. An
                // alias whose own name has a digit in it still wins, because
                // that is asked first.
                let named = |held: &str, c: char| {
                    let mut longer = held.to_string();
                    longer.push(c);
                    self.aliases().keys().any(|k| k.starts_with(&longer))
                };
                if alone
                    && !held.is_empty()
                    && c.is_ascii_digit()
                    && !named(&held, c)
                    && (c != '0' || self.alias_count.is_some())
                {
                    let n = self.alias_count.unwrap_or(0);
                    self.alias_count = Some(
                        n.saturating_mul(10)
                            .saturating_add(c.to_digit(10).unwrap_or(0) as usize)
                            .min(1_000_000),
                    );
                    self.alias_held = held;
                    return;
                }
                held.push_str(&typed);
                let longer = self
                    .aliases()
                    .keys()
                    .any(|k| k.len() > held.len() && k.starts_with(held.as_str()));
                if longer {
                    self.alias_held = held;
                    return;
                }
                let found = self.aliases().get(&held).cloned();
                // **一個鍵換一個鍵走正路**：換出來的那一下當作人按的接着處理，所以
                // 數字前綴、模式、計數都還在它身上。`@` 底下那一串纔是鍵，所以剝掉
                // 它再數（2026-10-19：從前右邊光着寫就是鍵，現在要說出來）。
                let one_key = found.as_deref().and_then(|b| match b {
                    [one] => Some(one.as_str()),
                    _ => None,
                }).and_then(|b| match yumete_cjk::actions::bound(b) {
                    Some(yumete_cjk::actions::Bound::Keys(k)) => {
                        let mut cs = k.chars();
                        match (cs.next(), cs.next()) {
                            (Some(c), None) => Some(c),
                            _ => None,
                        }
                    }
                    _ => None,
                });
                match found {
                    Some(_) if held.chars().count() == 1 && one_key.is_some() => {
                        self.settle_alias_count();
                        Key::Char(one_key.unwrap_or('\0'))
                    }
                    Some(bound) => {
                        self.settle_alias_count();
                        return self.run_binding(&bound);
                    }
                    None => {
                        // 放掉剛按的那一下——Alt 那一種是兩個 `char`，掉一個會留
                        // 下一個孤零零的前綴。
                        if held.pop() == Some(yumete_cjk::actions::ALT) {
                            // 不可能：前綴在前面。留着這一句是怕以後改了次序。
                        } else if held.ends_with(yumete_cjk::actions::ALT) {
                            held.pop();
                        }
                        // vim's `Z` waits for one more key and beeps at any
                        // but `Z` `Q`, eating it: `Zjx` deletes on the line it
                        // started on (nvim, 2026-11-03). With the flavour on,
                        // `Z` is helix's sticky view instead.
                        if held == "Z" && self.vim_keymap() && !self.flavor_on(yumete_cjk::FlavorItem::HelixKeysUnderVim) {
                            self.alias_count = None;
                            self.no_such_combination("Z", key);
                            return;
                        }
                        // What was held acts first: the reader's binding for
                        // the longest start of it, else the keys pressed for
                        // real; the digits typed after it belong to whatever
                        // follows them.
                        if held.is_empty() {
                            self.settle_alias_count();
                            key
                        } else {
                            if !self.let_go_of_the_root(&held) {
                                self.settle_alias_count();
                                self.play_held(&held);
                            }
                            // The key that did not finish one reaches the
                            // cursors it reaches alone (it came in once).
                            let waiting = self.pending;
                            return self.fan_out(waiting, key, false, &|e| e.on_normal_key(key));
                        }
                    }
                }
            } else {
                if !held.is_empty() {
                    self.settle_alias_count();
                    self.play_held(&held);
                }
                key
            }
        };

        // A digit prefix builds a count (`3w`), Helix-style. `0` only extends a
        // count already under way, so it stays free for other bindings.
        if let Key::Char(c) = key {
            if let Some(digit) = c.to_digit(10) {
                if digit > 0 || self.count.is_some() {
                    let n = self.count.unwrap_or(0);
                    self.count = Some(
                        n.saturating_mul(10)
                            .saturating_add(digit as usize)
                            .min(1_000_000),
                    );
                    return;
                }
            }
        }
        let operator_count = self.count;
        let count = self.take_count();

        // Read as a grid, `hjkl` walk cells. Before the vertical branch because
        // a table is read across, whatever the file's writing layout is.
        if self.table_here() && self.table_motion(key, count) {
            return;
        }

        // **行文那一套是在門口把四個字母對調換出來的**（2026-10-12 定）。
        //
        // 底下這一塊寫的是**視覺**那一套：`h` 往左 ＝ 下一縱，`j` 往下 ＝ 下一個
        // 字。出廠的行文那一套要的是和橫排逐鍵相同——`h` 上一個字、`j` 下一行
        // ——而那正好是把兩對字母對調：
        //
        // ```text
        //   想要 h（上一個字）＝ 視覺的 k        想要 j（下一行）＝ 視覺的 h
        //   想要 l（下一個字）＝ 視覺的 j        想要 k（上一行）＝ 視覺的 l
        // ```
        //
        // Warning: **是一個四元環，不是兩組對調**（h→k→l→j→h）。第一版寫成了
        // h↔k、l↔j 兩組互換，於是 `j` 走的是上一行、`k` 走的是下一行——兩個鍵
        // 正好反着，而且「按 j 不動」看起來像它根本没接上。
        //
        // Warning: **只換字母，不換方向鍵**（原話：「方向鍵永遠視覺，不跟開關」）。
        // 三家參考都按名字分家：Emacs 的 `C-f` 永遠邏輯而 `→` 看方向，CSS 規範裏
        // `forward`/`backward` 是邏輯而 `left`/`right` 是視覺。一個箭頭是方向的
        // 圖，一個字母是一個詞。Insert 模式那一支（`prompt.rs`）本來就只認方向
        // 鍵，所以一個字都不用動。
        //
        // Warning: **別名展開出來的那幾個鍵，永遠按行文那一套算**——而在這個倉
        // 裏那意味着**要換**，不是不換（2026-10-12 量出來的）。
        //
        // 規矩是 vim 的（`normal.c:874`：「Only when typed by the user
        // directly, not when the result of a mapping or `x` translated to
        // `dl`」）：寫 `"x" = "dl"` 的人說的是「刪一個字」，那是 `l` 的通用意
        // 思。vim 的引擎底子是邏輯的，所以「不換」就等於通用意思；**這裏的引擎
        // 底子是視覺的**（底下那一塊），通用意思要換出來。照抄那個 `!` 就反了。
        //
        // 抓到它的是 vim 鍵位的 `+`——它是 `("+", "jgs")`，展開的 `j` 没換，於
        // 是「下一行行首」在竪排下一行都不走。
        let follow_the_text = self.vertical_letters_follow_the_text()
            || (self.layout() == Layout::Vertical && self.expanding_alias);
        // Warning: **vim 鍵位下這四個字母不轉，交還給底下那一層**（2026-10-13 報
        // 的，「我用 helix，朋友用 vim」）。轉一圈進竪排那一支拿到的是**螢幕的單
        // 位**：`j` 轉成 `h` 就是「下一縱」，而 vim 的 `j` 是行文的一行——段落折
        // 成兩縱的時候那兩件事差整整一縱，`j` 只走到同一段的第二縱去。`h`/`l` 同
        // 病：轉成 `k`/`j` 之後走的是全文的字，`whichwrap` 那道閘繞過去了。
        //
        // helix 不動一個字：helix 的 `j` 本來就是**視覺行**，而竪排的一個視覺行
        // 正是一縱——所以轉進去是對的，這一條只分 vim 出來。底下那幾臂
        // （`move_textual_line`／`step_sideways`）本來就是 vim 那一套行文的意思，
        // 交還給它們就是答案，竪排不必再寫一份。
        let vim_has_its_own = follow_the_text && self.keymap_rules().vim_tables;
        let key = match follow_the_text && !vim_has_its_own {
            false => key,
            true => match key {
                Key::Char('h') => Key::Char('k'),
                Key::Char('k') => Key::Char('l'),
                Key::Char('l') => Key::Char('j'),
                Key::Char('j') => Key::Char('h'),
                key => key,
            },
        };

        // Laid out vertically, the arrow keys and `hjkl` keep their *screen*
        // meaning: `j` still reads onward down the 縱, and `h` still steps left,
        // which is now the next 縱 rather than the next line.
        // 方向鍵不在這道閘裏：它們永遠按螢幕走，兩套鍵位、兩套方案都一樣。
        let letter_walks_the_screen =
            !(vim_has_its_own && matches!(key, Key::Char('h' | 'j' | 'k' | 'l')));
        if self.layout() == Layout::Vertical
            && letter_walks_the_screen
            && self.walk_the_vertical_screen(key, count, continuing_zong)
        {
            return;
        }

        // **Every key is a named command** (`normal.rs`), looked up in the
        // default keymap: the vim preset's own entries first, helix's after.
        let cx = super::normal::Cx { key, count, operator_count, continuing_zong };
        if let Some(run) = self.normal_command_for(&cx).and_then(super::normal::normal_command) {
            self.before_a_motion(run.0);
            (run.1)(self, cx);
            self.note_the_group_count(operator_count);
        }
        // Warning: **An unbound key does nothing** (2026-10-08 定). A phrasebook here
        // once answered 「`$` is `gl` here」; that guesses what the person meant,
        // and was removed with its eleven lines of copy.
    }

    /// **A command's meaning where the page rereads its keys, when a binding
    /// or a group runs it by name** (2026-11-01): a table's grid
    /// ([`Self::table_takes`]), then a vertical page whose letters walk the
    /// screen, where `move_char_left` is `h`'s screen step, one 縱 onward,
    /// as `h` itself is. `true` when one of them took it.
    pub(super) fn takes_by_name(&mut self, cmd: NormalCmd, count: usize) -> bool {
        self.table_takes(cmd, count) || self.vertical_takes(cmd, count)
    }

    fn vertical_takes(&mut self, cmd: NormalCmd, count: usize) -> bool {
        if self.mode != Mode::Normal
            || self.expanding_alias
            || self.layout() != Layout::Vertical
            || self.vertical_letters_follow_the_text()
        {
            return false;
        }
        // Only the command a letter itself runs in this keymap: `gj` is
        // `move_line_down` too, and the `g` keys never turn with the page
        // (2026-10-13 定: 「gh gj gk gl 的逻辑性更大，所以不需要更改」).
        //
        // Asked of both tables (found in review, 2026-11-02): `move_char_*`
        // is the letter's command in Normal mode and `extend_char_*` in select
        // mode, and a name reaches either from anywhere. An `extend_*` name
        // extends wherever it is pressed.
        let was = self.extend;
        let letter_in = |e: &Self| {
            ['h', 'j', 'k', 'l'].into_iter().find(|&c| {
                let cx = super::normal::Cx { key: Key::Char(c), count, operator_count: None, continuing_zong: false };
                e.normal_command_for(&cx) == Some(cmd.name())
            })
        };
        self.extend = false;
        let moving = letter_in(self);
        self.extend = true;
        let extending = letter_in(self);
        self.extend = was;
        let (letter, extend) = match (moving, extending) {
            (Some(c), _) => (c, was),
            (None, Some(c)) => (c, true),
            (None, None) => return false,
        };
        self.extend = extend;
        let took = self.walk_the_vertical_screen(Key::Char(letter), count, false);
        self.extend = was;
        took
    }

    /// **One of the four screen keys on a vertical page** (`h j k l` and the
    /// arrows in the visual way): `true` when the key was one of them.
    fn walk_the_vertical_screen(&mut self, key: Key, count: usize, continuing_zong: bool) -> bool {
        // The count applies here too — `10j` is exactly the key a 縱 of
        // thirty-two characters is long for. These arms used to return
        // before `repeat` could see it.
        match key {
            // Only the first step of a run may reset the goal slot; the
            // rest of a `10h` continues from the one it chose.
            Key::Char('h') | Key::Left => {
                let mut first = continuing_zong;
                self.repeat(count, move |e| {
                    e.move_zong_from(true, first);
                    first = true;
                });
            }
            Key::Char('l') | Key::Right => {
                let mut first = continuing_zong;
                self.repeat(count, move |e| {
                    e.move_zong_from(false, first);
                    first = true;
                });
            }
            // Warning: **Across the break, the same as `h`/`l` are横排** (#504).
            // These were `motion::right`/`left`, whose own doc says 「staying
            // within the current line」 — so `j` walked to the end of a
            // paragraph and stopped dead, and the next 縱 could not be
            // reached by the key that walks the text: 「竖排的时候按 JK，无法
            // 跨行」. 橫排's pair crossed the break years ago (「A line is not
            // a wall」, below); this is the same rule on the other axis, and
            // it is the *same* function — 縱書 only swaps which key is which.
            Key::Char('j') | Key::Down => {
                self.repeat(count, |e| e.move_horizontal(motion::next_grapheme));
            }
            Key::Char('k') | Key::Up => {
                self.repeat(count, |e| e.move_horizontal(motion::prev_grapheme));
            }
            // Warning: **`H`/`L` are not here any more** (2026-09-12, #404).
            // They turned the page sideways, and the rule was 「the capital
            // follows the direction its lowercase does」 — `h` is leftward
            // and leftward is onward on a 縱書 page, so `H` turned the page
            // onward. That rule is right, and it is a rule about **screen
            // quantities**: it still governs `J`/`K`.
            //
            // `H`/`L` now take a sentence, which is a **text unit**, and
            // this editor's text units have never flipped — `w`, `e`, `b`
            // and their capitals all read onward in both layouts, and
            // nobody has ever found that strange. So the sentence pair
            // joins them: `H` is the sentence before, `L` the sentence
            // after, whichever way the page is set.
            _ => return false,
        }
        true
    }

    /// Hand a half-finished key the character it was waiting for (#414).
    ///
    /// The one place the six of them are written, because the character has
    /// two ways in: a keystroke, and a commit from the 輸入法 — `f` then
    /// 「，」 is two 拼音 letters, a panel and a choice, and none of that
    /// arrives as [`Key::Char`]. Written twice, the second copy knew only
    /// about `r`, which is why `f`、`ms`、`mi`、`mr` could not take 中文.
    ///
    /// The caller has already cleared [`Editor::pending`] and passes what it
    /// was; `SurroundFrom` sets the next one, so this must run *after* the
    /// clearing, not before.
    pub(super) fn answer_with_char(&mut self, waiting: Pending, c: char) {
        match waiting {
            Pending::Find(kind) => {
                self.last_find = Some((kind, c));
                // The count belongs to the `f`, which has already spent it:
                // `3fx` is the third `x`, not the first.
                let count = self.operator_count.take().unwrap_or(1).max(1);
                // Warning: **一次問完，不是叫 n 遍**（2026-10-02）。從前走 `repeat`，
                // 理由是它的早退（`10000fZ` 只看一眼）——而那個早退現在是
                // `find_char` 自己的事：找不到第 n 個就 `Missed`，一格不動。
                self.as_asked(|e| e.find_nth_char(kind, c, count, false));
            }
            Pending::Replace => self.replace_by_key(c),
            Pending::MatchPair { around } => self.select_pair(c, around, self.pair_count()),
            Pending::Surround => self.surround_add(c),
            Pending::SurroundOff => self.surround_delete(c, self.pair_count()),
            Pending::SurroundFrom => self.show_the_pair_to_change(c),
            Pending::SurroundTo(from) => self.surround_replace(from, c, self.pair_count()),
            // Everything else waits for a letter naming a command, and those
            // are answered where they are read.
            _ => {}
        }
    }

    /// Handle the second key of a goto (`g`) sequence, Helix-style: `gg` to the
    /// buffer start, `ge` to the last line, `gh`/`gl` to line start/end, `gs` to
    /// the first non-blank character.
    /// How many joins `J` should make (#518).
    ///
    /// Three ways to say it, in the order they are looked for:
    ///
    /// * **A selection spanning lines** — `xxx gJ` makes those three lines one,
    ///   which is what Helix's `J` does and what vi's does in visual mode.
    ///   Warning: It used to join the first pair and stop: `join_lines` reads the
    ///   selection's first line and joins it with the next, once, and nothing
    ///   asked it to go on. Selecting a paragraph and pressing `gJ` looked like
    ///   it had worked.
    /// * **`g3J`** — the sequence's own number, which is the order this editor
    ///   settled on (「命令＋選擇＋動作」, §5.2). Warning: It was read from
    ///   `operator_count`, which holds a count typed *before* the `g`, so the
    ///   documented spelling was the one that did nothing: `g4J` joined one
    ///   pair. `g30g` next door had it right all along.
    /// * **`4gJ`** — vi's order, a count before the command. This one worked.
    pub(super) fn join_count(&mut self, asked: Option<usize>) -> usize {
        let rope = self.current_buffer().rope();
        let (start, end) = self.selection();
        // Warning: **The last character *in* the selection, not the one past it.**
        // `x` takes the line break with the line, so the end sits at the start
        // of the next line — counting to `end` says four lines where three are
        // selected, and `xxx gJ` welded a line nobody had picked.
        if end > start {
            let first = rope.char_to_line(start);
            let last = rope.char_to_line((end - 1).min(rope.len_chars().saturating_sub(1)));
            if last > first {
                // N lines make N−1 joins.
                return last - first;
            }
        }
        // **A count says how many lines end up as one** (2026-09-19),
        // which is vim's rule: `3J` welds three lines, and that is two joins.
        // The two ways of writing it used to disagree by one — `g3J` counted
        // joins and `4gJ` counted lines — while the manual promised both meant
        // the same thing. Counting lines is the half that matches vim and the
        // half that matches 「選了幾行就併幾行」 above.
        // Warning: **`asked` 是最上層那一條路**（2026-10-06）。`J` 從 `gJ` 搬到頂層
        // 之後，`3J` 的那個 3 走的是 `on_normal_key` 開頭就取走的那個計數，既不
        // 在 `sequence_span` 裏（没有序列），也不在 `operator_count` 那個**欄位**
        // 裏（那是前綴鍵存的）。不傳進來的話 `3J` 只併一次。
        self.sequence_span()
            .map(|(n, _)| n)
            .or_else(|| self.operator_count.take())
            .or(asked)
            .unwrap_or(2)
            .saturating_sub(1)
            .max(1)
    }

    /// **What a bare count may be finished with** — only the keys where N means
    /// something other than 「do it N times」 (2026-10-14).
    ///
    /// Asked for as: 「I type 10 and HUD shows `10`, can there be a shortcut hint
    /// panel for this too」. Fifty-seven keys in Normal mode take a count, so
    /// listing them all would redraw the whole keymap — and the rule for these
    /// panels is the one the manual already states: list only the keys **you
    /// could not have guessed**. 「N times」 is guessable, so it is one row at
    /// the bottom and the rest of the table is the exceptions.
    ///
    /// The key column is what to press **next**: the number is already typed,
    /// so this says `gg`, not `<N>g`.
    ///
    /// Every description takes the count as `{0}`, which is why these rows are
    /// built with [`crate::messages::say`] and not [`Self::said`].
    ///
    /// **Commands, not keys** (2026-11-01): each row names the key its
    /// command is on now, under either keymap and with the reader's own
    /// bindings, so the vim keymap's `;` and helix's `A-.` are one row. It
    /// was two hand-written tables, one a keymap.
    pub(super) const COUNT_KEYS: &'static [(&'static [&'static [NormalCmd]], &'static str)] = &[
        (&[&[NormalCmd::VimJoinSelections, NormalCmd::JoinSelections]], "hint.count.join"),
        (&[&[NormalCmd::GotoFileStart]], "hint.count.line"),
        (&[&[NormalCmd::VimGotoLine, NormalCmd::GotoLineOrLastLine, NormalCmd::GotoLine]], "hint.count.line"),
        (&[&[NormalCmd::RepeatFindChar, NormalCmd::RepeatLastMotion]], "hint.count.nth-find"),
        (
            &[&[NormalCmd::GotoWindowTop], &[NormalCmd::GotoWindowCenter], &[NormalCmd::GotoWindowBottom]],
            "hint.count.screen-row",
        ),
    ];

    // Under the vim keymap repeating an `f`/`t` is `;`, under helix `A-.`
    // (measured 2026-10-14 — under helix `;` collapses the selection
    // instead, so `3;` goes nowhere): the row asks vim's command first.
    // The screen rows are `H M L` under the vim keymap, which binds them as
    // vim does, and `gt gc gb` under helix's.

    /// The objects `mi` / `ma` take; [`Self::object_rows`] adds the pairs.
    const OBJECT_KEYS: &'static [(&'static str, &'static str)] = &[
        ("w", "hint.obj.word"),
        ("W", "hint.obj.big-word"),
        ("u", "hint.obj.clause"),
        ("s", "hint.obj.sentence"),
        ("p", "hint.obj.paragraph"),
        ("m", "hint.obj.nearest-pair"),
        ("k", "hint.obj.markup"),
        ("f", "hint.obj.function"),
        ("t", "hint.obj.type"),
        ("a", "hint.obj.parameter"),
        ("c", "hint.obj.comment"),
        ("l", "hint.obj.cell"),
        ("T", "hint.obj.test"),
        ("e", "hint.obj.entry"),
        ("x", "hint.obj.element"),
        ("g", "hint.obj.change"),
    ];

    /// The pair rows both menus end with: a key, then the shapes it matches
    /// that the eye can tell apart. The `…` stands for the look-alikes it
    /// matches too (full width, half width, the vertical forms): listed, they
    /// made rows of shapes nobody could tell apart (2026-10-28).
    const PAIR_ROWS: &'static [(&'static str, &'static str)] = &[
        ("(", "( …"),
        ("[", "[ 「 【 〔 …"),
        ("{", "{ 『 〖 …"),
        ("<", "< « 〈 《 …"),
        ("\"", "\" “ 〝 …"),
        ("'", "' ‘ …"),
        ("`", "` …"),
    ];

    /// What `mi` / `ma` may be finished with: the objects, then the pairs.
    pub(super) fn object_rows() -> super::KeyRows {
        Self::pair_rows(Self::OBJECT_KEYS)
    }

    /// The pairs `ms` / `md` / `mr` take, after whatever `lead` adds above
    /// them (`m` for the nearest pair where that means something).
    pub(super) fn pair_rows(lead: &[(&'static str, &'static str)]) -> super::KeyRows {
        let mut rows = Self::said(lead.iter().copied());
        rows.extend(Self::PAIR_ROWS.iter().map(|&(key, shapes)| (key.into(), shapes.to_string())));
        // A space, not nothing: an empty key is drawn as a line across the
        // whole menu, and this row belongs in the description column, as
        // helix's `" "` row does.
        rows.push((" ".into(), say!("hint.pair.any")));
        rows
    }

    /// **The pairs `ms` and `mr`'s second key write**: each key's own pair and
    /// no family (2026-10-29, review: the rows listed `[ 「 【 〔 …` and `ms[`
    /// writes `[…]` alone). Any other character writes itself.
    pub(super) fn made_rows() -> super::KeyRows {
        let mut rows: super::KeyRows = Self::PAIR_ROWS
            .iter()
            .map(|&(key, _)| {
                let c = key.chars().next().unwrap_or(' ');
                let (open, close) = super::pair_of(c).unwrap_or((c, c));
                (key.into(), format!("{open} {close}"))
            })
            .collect();
        rows.push((" ".into(), say!("hint.pair.any")));
        rows
    }

    pub(super) const CONFIRM_KEYS: &'static [(&'static str, &'static str)] = &[
        ("y", "hint.confirm.yes"),
        ("n", "hint.confirm.no"),
        ("a", "hint.confirm.all"),
        ("q", "hint.confirm.stop"),
        ("l", "hint.confirm.last"),
    ];

    /// A key as a menu writes it: the space bar as `␣` (a space drawn is a
    /// blank, and the row looked keyless), Backspace as `⌫`.
    pub(super) fn key_name(key: char) -> std::borrow::Cow<'static, str> {
        match key {
            ' ' => "␣".into(),
            '\u{8}' => "⌫".into(),
            other => other.to_string().into(),
        }
    }

    pub(super) fn said(
        rows: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> KeyRows {
        rows.into_iter()
            .map(|(key, what)| (key.into(), crate::messages::say(what, &[])))
            .collect()
    }

    /// **開命令行**——`:`、`!`、`空格 ?` 都走這一支。
    ///
    /// Warning: 從前是三份抄的，而且抄漏了一行：`:` 不清 `completion`，另外兩個清
    /// （2026-09-23 審出來的）。同一件事兩種收尾，正是這個倉在防的那一族。
    pub(super) fn open_the_command_line(&mut self, line: &str) {
        self.mode = Mode::Command;
        self.command_line = line.to_string();
        self.command_caret = self.command_line.chars().count();
        self.completion = None;
    }

    /// Run a command line for a key, and say what went wrong when it did.
    pub(super) fn run_the_line(&mut self, line: &str) {
        if let Err(err) = self.execute(line) {
            self.status = err.to_string();
        }
    }

}

/// **The motions an operator can take, by command**: each motion
/// command's name and how vim spells it after `d` (the table in
/// [`crate::vim::step_for`] is vim's operator-pending keymap). Helix's names
/// for the same motions take them too.
pub(super) const MOTION_STEPS: &[(NormalCmd, &str)] = &[
    (NormalCmd::JumpNextWordStart, "w"),
    (NormalCmd::MoveNextWordStart, "w"),
    (NormalCmd::JumpNextLongWordStart, "W"),
    (NormalCmd::MoveNextLongWordStart, "W"),
    (NormalCmd::JumpPrevWordStart, "b"),
    (NormalCmd::MovePrevWordStart, "b"),
    (NormalCmd::JumpPrevLongWordStart, "B"),
    (NormalCmd::MovePrevLongWordStart, "B"),
    (NormalCmd::JumpNextWordEnd, "e"),
    (NormalCmd::MoveNextWordEnd, "e"),
    (NormalCmd::JumpNextLongWordEnd, "E"),
    (NormalCmd::MoveNextLongWordEnd, "E"),
    (NormalCmd::JumpPrevWordEnd, "ge"),
    (NormalCmd::JumpPrevLongWordEnd, "gE"),
    (NormalCmd::VimGotoLineEnd, "$"),
    (NormalCmd::GotoLineEnd, "$"),
    (NormalCmd::GotoLineStart, "0"),
    (NormalCmd::GotoFirstNonwhitespace, "^"),
    (NormalCmd::MoveCharRight, "l"),
    (NormalCmd::MoveCharLeft, "h"),
    (NormalCmd::MoveLineDown, "j"),
    (NormalCmd::MoveVisualLineDown, "j"),
    (NormalCmd::MoveLineUp, "k"),
    (NormalCmd::MoveVisualLineUp, "k"),
    (NormalCmd::GotoNextLineNonwhitespace, "+"),
    (NormalCmd::VimGotoLine, "G"),
    (NormalCmd::GotoFileStart, "gg"),
    (NormalCmd::JumpNextParagraph, "}"),
    (NormalCmd::JumpPrevParagraph, "{"),
    (NormalCmd::GotoWindowTop, "H"),
    (NormalCmd::GotoWindowCenter, "M"),
    (NormalCmd::GotoWindowBottom, "L"),
    (NormalCmd::JumpPrevSentence, "("),
    (NormalCmd::JumpNextSentence, ")"),
    (NormalCmd::FindNextChar, "f"),
    (NormalCmd::FindPrevChar, "F"),
    (NormalCmd::FindTillChar, "t"),
    (NormalCmd::TillPrevChar, "T"),
    (NormalCmd::SelectInnerObject, "i"),
    (NormalCmd::SelectAroundObject, "a"),
];

/// **The key a played character stands for** — the other half of
/// [`yumete_cjk::actions::spell`], which prints these for `:keymap`.
///
/// 把一個下標夾進它那一行裏，順帶夾掉檔尾那條虛行。見
/// [`Editor::keep_off_the_newline`]。空行上 `line_last` 回的就是行首，也就是那個
/// 換行符自己——那是唯一去得了的地方，照 vim。
fn hold_in_line(rope: &crate::Rope, at: usize, floor: usize) -> usize {
    let at = at.min(rope.len_chars());
    let line = rope.char_to_line(at);
    let at = match line > floor {
        // 檔尾那條虛行：退到上一行的最後一個字上。
        true => motion::line_last(rope, rope.line_to_char(floor)),
        false => at,
    };
    motion::line_last(rope, at).min(at.max(motion::line_start(rope, at)))
}

/// A key sequence is written as a string (`"gJ"`, `"dw"`), and a chord has no
/// letter of its own, so the table spells it as the control byte it is:
/// `"\u{f}"` is `C-o`, `"\u{1b}"` is Esc. Played as `Key::Char`, those reached
/// the editor as nothing at all — which is why four named actions
/// (`jump_backward`, `collapse_selection`, `increment`, `decrement`) were
/// listed by `:keymap-actions`, bindable, and **dead** until 2026-09-19.
/// A key in the internal spelling a binding's string uses (`A-d` is
/// [`yumete_cjk::actions::ALT`] then `d`, `C-o` its control byte): the
/// inverse of [`pressed`], for [`yumete_cjk::actions::spell`] to write.
pub(crate) fn key_internal(key: Key) -> String {
    match key {
        Key::Char(c) => c.to_string(),
        Key::Alt(c) => format!("{}{c}", yumete_cjk::actions::ALT),
        Key::Ctrl(c) if c.is_ascii_alphabetic() => yumete_cjk::actions::ctrl(c).to_string(),
        Key::Esc => "\u{1b}".to_string(),
        key => typed_as(key).map(String::from).unwrap_or_default(),
    }
}

/// One key in the internal spelling (`\u{17}`, [`yumete_cjk::actions::ALT`]
/// and a letter) back to the key; `None` for a run of several.
pub(super) fn pressed_key(typed: &str) -> Option<Key> {
    let mut chars = typed.chars();
    let key = match chars.next()? {
        yumete_cjk::actions::ALT => Key::Alt(chars.next()?),
        c => pressed(c),
    };
    chars.next().is_none().then_some(key)
}

pub(super) fn pressed(c: char) -> Key {
    if let Some(key) = named_key(c) {
        return key;
    }
    match c {
        '\u{1b}' => Key::Esc,
        yumete_cjk::actions::CTRL_J => Key::Ctrl('j'),
        yumete_cjk::actions::CTRL_I => Key::Ctrl('i'),
        // Warning: **Enter before the control-byte rule**: `\n` is 0x0A, which that
        // rule would read as `C-j`. A keymap that wants Enter writes `\n`, and
        // it means the key a hand presses.
        '\n' | '\r' => Key::Enter,
        c if (c as u32) < 32 => Key::Ctrl((b'a' + c as u8 - 1) as char),
        c => Key::Char(c),
    }
}

/// **有名字的那幾個鍵，兩個方向**（2026-10-12）——`<tab>`／`<S-tab>`／方向鍵那
/// 一族。表在 [`yumete_cjk::actions::NAMED`]，這裏只把它和 [`Key`] 接上。
///
/// 一支函數管兩頭，是因為兩頭必須逐條相同：[`pressed`] 把別名右邊那一串放回
/// 鍵，[`typed_as`] 把按下的鍵翻成查表用的那一串。少一邊，寫得進配置而按不出
/// 來——`C-o` 就是這麼死了一輪（手冊第 810 行一直寫着它能綁）。
fn named_key(c: char) -> Option<Key> {
    use yumete_cjk::actions::named_key as of;
    let is = |name: &str| of(name) == Some(c);
    match () {
        () if is("tab") => Some(Key::Tab),
        () if is("S-tab") => Some(Key::BackTab),
        () if is("enter") => Some(Key::Enter),
        () if is("backspace") => Some(Key::Backspace),
        () if is("delete") => Some(Key::Delete),
        () if is("up") => Some(Key::Up),
        () if is("down") => Some(Key::Down),
        () if is("left") => Some(Key::Left),
        () if is("right") => Some(Key::Right),
        () if is("home") => Some(Key::Home),
        () if is("end") => Some(Key::End),
        () if is("pageup") => Some(Key::PageUp),
        () if is("pagedown") => Some(Key::PageDown),
        _ => None,
    }
}

/// 按下的這個鍵，在鍵位表裏寫成哪一串——[`pressed`] 的反面。
///
/// `None` ＝ 這個鍵綁不了。今天只有 `Esc` 落在這裏，有意的：按鍵序列按了一半要
/// 靠它鬆手（見 `alias_held`），綁掉就没有退路。
pub(super) fn typed_as(key: Key) -> Option<char> {
    use yumete_cjk::actions::named_key as of;
    let named = |name: &str| of(name);
    match key {
        Key::Char(c) => Some(c),
        // **Alt 借一個私用區的碼位**（2026-10-08）：它没有自己的字節。
        Key::Alt(c) => Some(c),
        // Warning: **`C-x` 從前綁不了**（2026-10-12 查出來的）：這一支只認
        // `Key::Char` 和 `Key::Alt`，而 `written("C-o")` 交的是那個控制字節，
        // 兩邊永遠對不上。配置照收、一聲不吭，手冊卻寫着它能綁。
        Key::Ctrl(c) if c.is_ascii_alphabetic() => Some(yumete_cjk::actions::ctrl(c)),
        Key::Ctrl(_) => None,
        Key::Tab => named("tab"),
        Key::BackTab => named("S-tab"),
        Key::Enter => named("enter"),
        Key::Backspace => named("backspace"),
        Key::Delete => named("delete"),
        Key::Up => named("up"),
        Key::Down => named("down"),
        Key::Left => named("left"),
        Key::Right => named("right"),
        Key::Home => named("home"),
        Key::End => named("end"),
        Key::PageUp => named("pageup"),
        Key::PageDown => named("pagedown"),
        Key::Esc => None,
    }
}

/// **The cursors a command reaches when no key of the keymap runs it** —
/// the vim preset's own commands, which its keys reach through the alias
/// layer (found in review, 2026-11-02: vim's `x` deleted at one cursor of
/// two). Each is classed by a key that does the same kind of thing: an edit
/// at each cursor in turn, or a move of every cursor.
fn keyless_place(name: &str) -> Option<(Pending, Key)> {
    let like = match name {
        "vim_delete_char_forward" => Key::Char('d'),
        "vim_paste_after" | "vim_paste_before" | "vim_paste_after_indented" | "vim_paste_before_indented" => Key::Char('p'),
        "vim_match_brackets" | "vim_goto_line_end" | "vim_goto_first_nonwhitespace" => Key::Char('w'),
        _ => return None,
    };
    Some((Pending::None, like))
}

#[cfg(test)]
mod key_table_tests {
    use super::*;

    /// **一個鍵寫得進配置，就要按得出來**——兩個方向逐條對着（2026-10-12）。
    ///
    /// `C-o` 寫得進配置而永遠按不出來，整整漏了一輪：查表那一支只認 `Key::Char`
    /// 和 `Key::Alt`，而手冊第 810 行一直寫着它能綁。
    #[test]
    fn every_bindable_key_round_trips_through_the_table() {
        let every = [
            Key::Char('a'),
            Key::Ctrl('o'),
            Key::Alt('d'),
            Key::Tab,
            Key::BackTab,
            Key::Enter,
            Key::Backspace,
            Key::Delete,
            Key::Up,
            Key::Down,
            Key::Left,
            Key::Right,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
        ];
        for key in every {
            let written = typed_as(key).unwrap_or_else(|| panic!("{key:?} 綁不了"));
            // Alt 在表裏是前綴 ＋ 字母兩個 `char`，其餘一個。
            let back = match key {
                Key::Alt(c) => Key::Alt(c),
                _ => pressed(written),
            };
            assert_eq!(back, key, "{key:?} 轉一圈回來變了");
        }
        // Warning: **`Esc` 有意綁不了**：按鍵序列按了一半要靠它鬆手。
        assert_eq!(typed_as(Key::Esc), None);
    }

    /// 配置裏寫的 `<tab>`，和按下 Tab 時查表用的那一串，要是同一串。
    #[test]
    fn a_named_key_in_the_config_meets_the_key_a_hand_presses() {
        for (name, key) in [
            ("tab", Key::Tab),
            ("S-tab", Key::BackTab),
            ("enter", Key::Enter),
            ("pagedown", Key::PageDown),
            ("left", Key::Left),
        ] {
            let in_the_file = yumete_cjk::actions::written(&format!("<{name}>"));
            let under_the_hand = typed_as(key).map(|c| c.to_string()).unwrap_or_default();
            assert_eq!(in_the_file, under_the_hand, "<{name}>");
        }
        // 大小寫不計，而認不得的名字原樣留着（它可能真是個尖括號）。
        assert_eq!(
            yumete_cjk::actions::written("<TAB>"),
            yumete_cjk::actions::written("<tab>")
        );
        assert_eq!(yumete_cjk::actions::written("<nope>"), "<nope>");
        assert_eq!(yumete_cjk::actions::written("a<b"), "a<b");
    }
}

