//! Where a key goes (#296).
//!
//! `on_key` is the one door into the editor, and everything under it is the
//! sorting: which mode, which pane, which of the two grids, and the `空格`
//! menu. It sat under 「word segmentation」 for no reason but the order it was
//! written in.

use super::*;

impl Editor {
    /// Handle a single key press according to the current mode.
    pub fn on_key(&mut self, key: Key) -> KeyOutcome {
        // **A question standing takes every key** (#295), before recording and
        // before the sidebar: while one is on the screen there is no keystroke
        // that reaches the manuscript, and none that a macro should capture —
        // the answer is about this file at this moment, not about the sequence
        // being recorded.
        if self.query.is_some() {
            self.answer_query(key);
            return KeyOutcome::Continue;
        }
        // **按「那裏寫的什麼」跳的時候，鍵先歸它**（`go`／`gu`，§5.73）——和底下
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
            // `Q` ends the recording — but only the `Q` that is a *command*.
            // A `Q` that some half-finished sequence is waiting for is an
            // operand: `fQ` is "find Q", and dropping its second half left the
            // macro as a bare `f`, which on replay swallowed whatever came
            // next. One reviewer's macro deleted their buffer that way.
            let ends_it = matches!(key, Key::Char('Q'))
                && self.mode == Mode::Normal
                && self.pending == Pending::None;
            if !ends_it {
                if let Some(keys) = self.recording.as_mut() {
                    keys.push(key);
                }
            }
        }
        // **一份已經不作數的字典答案，這裏丟掉**（2026-09-23 審出來的）。
        //
        // Warning: **在派鍵之前，不是之後。** `dictionary_live()` 問的是「焦點在不在
        // 字典那一側」——`Layer` 還在的時候那等於「在不在下層」，沒了之後同一側
        // 的常駐面板拿到鍵也是這個值。於是：`空格 D` 開字典、`C-w` 出來、光標一
        // 走（字典從畫面上下去了，可 `self.dictionary` 還是 `Some`）、
        // `:wiki panel`、再 `C-w` 進去——焦點一落到右邊，那份**過期的字典就重新
        // 蓋住百科**，而且把鍵接走。掃在派鍵之前，`C-w` 那一下焦點還沒挪過去，
        // 掃得掉；掃在之後就正好掃不掉。
        self.forget_a_dictionary_nobody_is_reading();
        // 同上，釘住的那一條詞條也是：光標一動就鬆開（2026-09-25）。
        self.forget_a_pinned_entry();
        // `空格 k` 問來的那一則說明同理：光標走開就作廢，不許走回去又冒出來
        // （2026-09-29 報的）。
        self.forget_a_hover_nobody_is_looking_at();
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
        // 「…」上卻翻不動，`C-u`／`C-d` 去翻了正文——等於沒有出路。
        //
        // Warning: **和 helix 逐鍵相同**（`ui/popup.rs:289-297`）：浮窗開着的時候
        // 這四個鍵歸浮窗，沒開就照舊翻正文，別的鍵一按浮窗就走。所以這裏**不
        // 攔**——翻不動就讓路，正文那一支自己接住。
        if self.mode == Mode::Normal && self.pending == Pending::None {
            // Warning: **翻頁那一族要整族在這裏**（2026-10-01 報上來的：「你翻页
            // 翻的是 buffer 不是浮窗／边栏」）。從前只攔了四個，而 `C-f`／`C-b`
            // 與當天新加的 `C-n`／`C-p` 沒攔住——同一族鍵，開着浮窗按下去有的
            // 翻浮窗有的翻正文，讀者沒法預測。
            //
            // 行數跟着「翻多少」走：整頁 8、三分之二 6、半頁 4。
            let by = match key {
                Key::PageDown | Key::Ctrl('f') => Some(8),
                Key::PageUp | Key::Ctrl('b') => Some(-8),
                Key::Ctrl('n') => Some(6),
                Key::Ctrl('p') => Some(-6),
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
            // 的任何一句話都賴着不走——而命令行那一格被它佔着，按鍵提示就沒地方
            // 畫。話是說給上一個按鍵的，下一個按鍵一到它就該散。
            self.status.clear();
            self.on_sidebar_key(key);
            return KeyOutcome::Continue;
        }
        // Watch this command, so `.` can play it back. A command begins in
        // Normal mode with nothing pending; it ends when it is back there.
        // A key describes one move, and「was that a jump?」is about *this* one.
        self.jumped = false;
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
                self.edit_revision = (self.current_buffer().id(), self.current_buffer().revision());
            }
            self.edit_keys.push(key);
        }
        // Where the pane's window was pointed before this key — see
        // [`Self::hold_the_pane`].
        let held = self
            .table
            .as_ref()
            .is_some_and(|view| view.takes_the_pane())
            .then(|| (self.current_buffer().id(), self.sel.head()));
        let outcome = match self.mode {
            Mode::Normal => {
                if multi::each_selection_key(&self.pending, key) {
                    self.each_selection(|e| e.on_normal_key(key));
                } else if multi::edits_every_selection(&self.pending, key) {
                    self.edit_each(|e| e.on_normal_key(key));
                } else {
                    self.on_normal_key(key);
                }
                KeyOutcome::Continue
            }
            Mode::Insert => {
                // Warning: **補全單子開着的時候一次就夠**：那是一張浮在上面的單子，`Tab` 與
                // `C-n` 歸它，不是往 N 處各寫一次。
                match multi::types_at_every_selection(key) && self.offers_here().is_none() {
                    true => self.edit_each_from(false, |e| e.on_insert_key(key)),
                    false => self.on_insert_key(key),
                }
                KeyOutcome::Continue
            }
            Mode::Command => self.on_command_key(key),
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
        self.forget_a_guessed_table();
        self.hold_the_pane(held);
        self.find_the_table_here();
        outcome
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
    /// The rule for the second class of file (#275): 「如果一個文件沒
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
            grain: Grain::Char,
            separator: Separator::Pipe,
            pane: false,
            bounds: Bounds::Md,
            // **A `|` table says what it is on every one of its own lines**,
            // so the view is the file's the moment it exists — which is what
            // keeps a table three screens down from wrapping while the cursor
            // is up here in the prose, padded and folded at once.
            reach: Reach::File,
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
        self.last_edit_keys = std::mem::take(&mut self.edit_keys);
    }

    /// Press `keys` as though they had been typed, **without** reading them
    /// as aliases again — an alias whose expansion uses its own key would
    /// otherwise call itself forever — and without recording them twice.
    ///
    /// A count typed before the alias goes to the first key that acts, not to
    /// a leading `;`: `2x` is `;` then `2D`, and `3dd` is `3x` then `d`.
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
        // Warning: **`;` 和 `,` 也是動作**（`:h ;`）：`d;` 把剛纔那個 `f`／`t` 再做
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
                return self.run_vim_step(op, step);
            }
        }
        let mut typed = String::new();
        if let Some(f) = first {
            typed.push(f);
        }
        typed.push(c);
        match crate::vim::step_for(&typed, grain, None) {
            // `f` and `i` are not finished: they are owed a character.
            Some(step) if step.asks => {
                self.pending = Pending::VimOperator { op, first: Some(c) };
            }
            Some(step) => self.run_vim_step(op, step),
            None if crate::vim::more_ahead(&typed) => {
                self.pending = Pending::VimOperator { op, first: Some(c) };
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
    fn run_vim_line(&mut self, op: char) {
        self.settle_alias_count();
        let n = self.count.take().unwrap_or(1).max(1);
        let rope = self.current_buffer().rope();
        let first = rope.char_to_line(self.sel.head());
        let last = (first + n - 1).min(rope.len_lines().saturating_sub(1));
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
        self.do_vim(op, span);
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
        let n = self.count.take().unwrap_or(1).max(1);
        let start = self.sel.head();
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
            // `c` keeps it (the `dd`／`cc` pair).
            if matches!(step.motion, motion::Motion::Object { what: motion::Object::Paragraph, .. })
            {
                if let motion::Span::Over { anchor, head } = span {
                    let rope = self.current_buffer().rope();
                    let (a, b) = (rope.char_to_line(anchor), rope.char_to_line(head));
                    return self.do_vim_lines(op, a.min(b), a.max(b));
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
        // Warning: **`f`／`t` 的數目是「第 n 個」，一次問完**（2026-10-02 拿 nvim 量
        // 出來的）。走底下那個迴圈的話每一趟都從上一個落點重新下錨，`d2f,` 刪的
        // 就是「第一個逗號到第二個」；而數目超出的時候迴圈留下走成的那幾跳，
        // `d9f,`（只有四個逗號）**默默吃掉三十二個字**，nvim 在那裏一格不動。
        let nth = match step.motion {
            motion::Motion::Find { .. } => n,
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
            // （底下的 `Reach::Exclusive`）早就擋了，往回這一支沒有。
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
                let end = match rope.char_to_line(target) > rope.char_to_line(before) {
                    true => motion::line_end(rope, before),
                    false => target,
                };
                motion::Span::Over {
                    anchor: start,
                    head: motion::prev_grapheme(rope, end).max(start),
                }
            }
        };
        self.do_vim(op, span);
    }

    /// Whole lines, for a linewise motion (`dj`, `dG`).
    fn do_vim_lines(&mut self, op: char, first: usize, last: usize) {
        let span = self.line_span(first, last, op == 'c');
        self.do_vim(op, span);
    }

    /// Do the verb, and say what the old `play_vim_motion` said about each.
    fn do_vim(&mut self, op: char, span: motion::Span) {
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
        self.snapshot();
        match op {
            // vim's `d` and `c` fill the unnamed register.
            'd' => self.apply(motion::Operator::Cut, span),
            'c' => self.apply(motion::Operator::Change { cut: true }, span),
            // `>` and `<` want the selection, because indenting is a
            // line-shaped edit the editor already knows how to do.
            '>' | '<' => {
                self.take_object(span);
                self.extend_to_line_bounds();
                self.indent(op == '>');
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
            }
        }
    }

    /// **What the right-hand side of a binding says to do** (#429).
    ///
    /// Three things, tried in this order:
    ///
    /// 1. `:something` — that command line, exactly as it would be typed.
    /// 2. An **action's name** (`delete_selection`). Names are what a reader
    ///    should write: `x = "delete_selection"` goes on meaning that after
    ///    the day the editor's own `d` moves somewhere else, and it says what
    ///    it does without the reader knowing which key does it today.
    /// 3. Anything else is **keys**, played as though typed — layer one
    ///    (#428), which is what the vim preset is made of.
    ///
    /// Warning: **A name wins over keys that spell it.** `search` is an action; it
    /// is also six letters somebody could in principle want pressed one after
    /// another. Nobody wants that, and the ambiguity is worth less than the
    /// names being writable bare. A misspelt name is caught when the config is
    /// read (an underscore means a name was meant), so it cannot quietly type
    /// itself into a chapter.
    fn run_binding(&mut self, bound: &str) {
        if let Some(line) = bound.strip_prefix(':') {
            let _ = self.execute(&format!(":{line}"));
            return;
        }
        if let Some(action) = yumete_cjk::actions::action(bound) {
            match action.how {
                yumete_cjk::actions::How::Keys(keys) => return self.play_keys(keys),
                yumete_cjk::actions::How::Command(line) => {
                    let _ = self.execute(&format!(":{line}"));
                    return;
                }
            }
        }
        self.play_keys(bound)
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
            self.on_key(pressed(c));
            at += 1;
        }
        self.expanding_alias = outer;
    }

    /// **The count a sequence was written with**, from before it and from
    /// inside it (2026-09-18).
    ///
    /// vim multiplies the two — `2d3w` is six words — and one of them is
    /// almost always absent, so this is the whole of that rule.
    fn settle_alias_count(&mut self) {
        let inside = self.alias_count.take();
        self.count = match (self.count.take(), inside) {
            (Some(before), Some(after)) => Some(before.saturating_mul(after)),
            (before, after) => before.or(after),
        };
    }

    fn on_normal_key(&mut self, key: Key) {
        self.status.clear();
        let continuing_zong = std::mem::take(&mut self.zong_motion);

        // A pending multi-key operator consumes this key.
        match self.pending {
            Pending::Table => {
                // 命令＋選擇＋動作: `t20,20g` is 「table · row 20, column 20 ·
                // go」, and the sequence stays open while the digits arrive.
                //
                // A sort names as many columns as it likes before it acts
                // (`t1a2d8as`), so `a`/`d` are asked first: they close one
                // column and leave the sequence open for the next.
                if self.take_sort_key(key) || self.take_sequence_argument(key) {
                    return;
                }
                self.pending = Pending::None;
                // **A chain that has named a column ends in `s` or `S`.**
                // Without this, every `t` verb stays live in the middle of a
                // sort, and `d` — which is both 「降序」 and 「delete this
                // row」, one missing digit apart — took `t1ad` as 「delete」
                // and threw the columns away. So once a column has been
                // named, the only ways out are the action, `Esc`, and being
                // told what went wrong.
                if !self.sort_keys.is_empty() && !matches!(key, Key::Char('s' | 'S') | Key::Esc) {
                    self.sequence = None;
                    self.sort_keys.clear();
                    self.status = say!("table.sort-wants-its-action");
                    return;
                }
                self.table_structure(key);
                self.sequence = None;
                self.sort_keys.clear();
                return;
            }
            Pending::TableConvert => {
                self.pending = Pending::None;
                self.convert_table_key(key);
                return;
            }
            Pending::Hop { forward } => {
                self.pending = Pending::None;
                match key {
                    Key::Char('c') => self.go_to_conflict(forward),
                    // **`]p` / `[p` 是段落**，helix 的 `goto_next_paragraph` /
                    // `goto_prev_paragraph`（2026-10-06 補的）。這裏 `}` / `{`
                    // 早就是同一件事，這兩個是 helix 的手會按的那個拼法。
                    Key::Char('p') => {
                        let span = self.run_motion(motion::Motion::Paragraph { forward });
                        self.take_span(span);
                    }
                    // **`]空格` / `[空格` 加一條空行，人不動**（helix 的
                    // `add_newline_below` / `_above`，2026-10-06 補的）。
                    Key::Char(' ') => self.add_blank_line(forward),
                    _ => {}
                }
                return;
            }
            // **`z` 那一層**（2026-09-28，helix 的 view mode 的那三個）。
            Pending::Aim => {
                self.pending = Pending::None;
                match key {
                    Key::Char('z') | Key::Char('c') => self.aim_the_page(crate::editor::Aim::Middle),
                    Key::Char('t') => self.aim_the_page(crate::editor::Aim::Top),
                    Key::Char('b') => self.aim_the_page(crate::editor::Aim::Bottom),
                    _ => self.status = say!("page.aim-wants-ztb"),
                }
                return;
            }
            Pending::Conflict => {
                self.pending = Pending::None;
                match key {
                    Key::Char('o') => self.resolve_conflict(crate::conflict::Keep::Ours),
                    Key::Char('t') => self.resolve_conflict(crate::conflict::Keep::Theirs),
                    Key::Char('b') => self.resolve_conflict(crate::conflict::Keep::Both),
                    _ => {}
                }
                return;
            }
            // **區域那一組**（2026-09-30 定，照 helix 的 `C-w`）。
            Pending::Region => {
                self.pending = Pending::None;
                self.on_region_key(key);
                return;
            }
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
            Pending::Goto => {
                // **命令＋選擇＋動作.** Inside a sequence the digits are its
                // *argument*, not a repetition: `g30g` is 「goto · line 30」,
                // the way `t20,20g` names a cell and `t1a2d8as` names three
                // columns to sort by. The verb ends the sequence, so no
                // separator and no space is needed — and the sequence stays
                // open while digits are being typed.
                if self.take_sequence_argument(key) {
                    return;
                }
                self.pending = Pending::None;
                self.handle_goto(key);
                self.operator_count = None;
                self.sequence = None;
                return;
            }
            Pending::Space => {
                self.pending = Pending::None;
                self.handle_space(key);
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
                // `mi`／`ma` 吃一個字符鍵、卻不要輸入法——「吃不吃鍵」和「要不要
                // 輸入法」本來就是兩個問題，那一句斷言是在逼它們答同一個。
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.answer_with_char(waiting, c);
                }
                // Spent either way: a count typed before `f` belongs to that
                // `f` and must not be left lying for the next command.
                self.operator_count = None;
                return;
            }
            Pending::Register => {
                self.pending = Pending::None;
                if let Key::Char(c) = key {
                    self.pending_register = Some(c);
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
            Pending::ReplaceAll => {
                self.pending = Pending::None;
                match key {
                    Key::Char('y') | Key::Enter => self.replace_what_was_asked(),
                    // Anything else is no. A question about a hundred files
                    // answers 「no」 to a key nobody meant.
                    //
                    // Warning: **取消之後屏幕不說話**（2026-09-29 定）。問句本來就寫着
                    // 「取消」，按下去問句就沒了——那已經是答覆。
                    _ => self.forget_the_file_it_asked_about(),
                }
                return;
            }
            Pending::Case => {
                self.pending = Pending::None;
                match key {
                    Key::Char('l') => self.map_selection(|c| c.to_lowercase().collect()),
                    Key::Char('u') => self.map_selection(|c| c.to_uppercase().collect()),
                    Key::Char('`') => self.map_selection(switch_case),
                    // **簡繁也是「把選區裏的字換一種寫法」**（2026-09-22）。
                    // 一鍵一檔，第二個字母各不相同——`tw`／`hk` 拼全了，`` `t ``
                    // 就既是命令又是前綴，只能靠超時猜。
                    Key::Char('s') => self.convert_selection(crate::convert::Side::T, crate::convert::Side::S),
                    Key::Char('t') => self.convert_selection(crate::convert::Side::S, crate::convert::Side::T),
                    Key::Char('w') => self.convert_selection(crate::convert::Side::S, crate::convert::Side::Tw),
                    Key::Char('h') => self.convert_selection(crate::convert::Side::S, crate::convert::Side::Hk),
                    Key::Char('c') => self.convert_selection(crate::convert::Side::S, crate::convert::Side::C),
                    Key::Char('g') => self.convert_selection(crate::convert::Side::S, crate::convert::Side::G),
                    // Warning: **這一個從繁體起步，另外六個從簡體。** opencc 只有
                    // `t2jp`／`jp2t` 兩條日文路，沒有 `s2jp`——寫 `S` 的那一版
                    // 按下去什麽都不會發生（2026-09-23 審出來的）。
                    Key::Char('j') => self.convert_selection(crate::convert::Side::T, crate::convert::Side::Jp),
                    _ => {}
                }
                return;
            }
            Pending::Match => {
                self.pending = Pending::None;
                match key {
                    Key::Char('m') => self.goto_matching_bracket(),
                    Key::Char('i') => self.pending = Pending::MatchPair { around: false },
                    Key::Char('a') => self.pending = Pending::MatchPair { around: true },
                    Key::Char('s') => self.pending = Pending::Surround,
                    Key::Char('d') => self.pending = Pending::SurroundOff,
                    Key::Char('r') => self.pending = Pending::SurroundFrom,
                    _ => {}
                }
                return;
            }
            // **A vim operator, waiting for its motion** (#429, 2026-09-18).
            Pending::VimOperator { op, first } => {
                self.vim_operator_key(op, first, key);
                return;
            }
            // 撤銷斷點只在插入模式裏按，走到 Normal 這一支就是它已經過期了。
            Pending::UndoBreak => self.pending = Pending::None,
            Pending::None => {}
        }

        // **vim 的可視模式，那五個按下去會改錯字的鍵**（2026-10-06）。
        //
        // 這一段要寫在別名層**前面**：`x`／`s` 在 vim 鍵位表上是 `;{n}D`／`;{n}Di`，
        // 而那個 `;` 是「收成一點」——它是為 Normal 寫的，可視模式下照收，於是
        // `vjx` 只刪一個字符。vim 的可視模式下這五個都是對**整個選區**動手：
        //
        // | 鍵 | vim | 這裏從前 |
        // | --- | --- | --- |
        // | `o` | 換到選區的另一頭 | 開一行並進插入（`vjo` 在稿子裏留下一行） |
        // | `x` `s` | 刪／改選區 | 先收成一點，只動一個字 |
        // | `u` `U` | 轉小寫／大寫 | 撤銷／重做（`dwvju` 把 `dw` 撤了） |
        // | `p` | 用寄存器換掉選區 | 粘貼在後面 |
        // | `~` | 選區大小寫互換 | 只換一個字 |
        //
        // Warning: **只在 vim 鍵位下**。helix 的 select mode 裏這幾個鍵就是 Normal
        // 那幾件事（它靠 `A-;` 換頭、`` ` `` 轉大小寫），兩邊各跟各的。
        if self.key_preset == yumete_cjk::KeyPreset::Vim && self.extend && !self.expanding_alias {
            match key {
                Key::Char('o') => return self.flip_selection(),
                Key::Char('u') => return self.map_selection(|c| c.to_lowercase().collect()),
                Key::Char('U') => return self.map_selection(|c| c.to_uppercase().collect()),
                Key::Char('~') => return self.map_selection(switch_case),
                Key::Char('p') => return self.replace_with_register(),
                // 刪／改整個選區 ＝ 這個編輯器的 `d`／`c`，原樣交下去。
                Key::Char('x') => return self.on_normal_key(Key::Char('d')),
                Key::Char('s') => return self.on_normal_key(Key::Char('c')),
                // **`y` 複製完就回 Normal，光標落在那一段的開頭**——vim 的可視
                // 模式一動完就結束。helix 的 `y` 把選區留着（它的 select mode 不
                // 為一次動作而開），所以這一條同樣只在 vim 鍵位下。
                Key::Char('y') => {
                    let start = self.selection().0;
                    let (anchor, head) = self.span();
                    self.apply(motion::Operator::Yank, motion::Span::Over { anchor, head });
                    self.extend = false;
                    self.sel.set_head(start);
                    self.sel.set_anchor(start);
                    // 光標真的挪了，下一個 `j` 要從這一欄數起。
                    self.refresh_goal_column();
                    return;
                }
                _ => {}
            }
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
        let key = if self.expanding_alias || self.key_aliases.is_empty() {
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
                    Some(c)
                }
                _ => None,
            };
            if key == Key::Esc && !self.alias_held.is_empty() {
                self.alias_held.clear();
                self.alias_count = None;
                return;
            }
            let mut held = std::mem::take(&mut self.alias_held);
            if let Some(c) = typed {
                // **A digit inside a held sequence is a count** (2026-09-18):
                // vim writes `d10w`, and `d1` is not the name of anything. An
                // alias whose own name has a digit in it still wins, because
                // that is asked first.
                let named = |held: &str, c: char| {
                    let mut longer = held.to_string();
                    longer.push(c);
                    self.key_aliases.keys().any(|k| k.starts_with(&longer))
                };
                if !held.is_empty()
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
                held.push(c);
                let longer = self
                    .key_aliases
                    .keys()
                    .any(|k| k.len() > held.len() && k.starts_with(held.as_str()));
                if longer {
                    self.alias_held = held;
                    return;
                }
                match self.key_aliases.get(&held).cloned() {
                    Some(keys) if held.chars().count() == 1 && keys.chars().count() == 1 => {
                        self.settle_alias_count();
                        Key::Char(keys.chars().next().unwrap())
                    }
                    Some(bound) => {
                        self.settle_alias_count();
                        return self.run_binding(&bound);
                    }
                    None => {
                        held.pop();
                        // What was held is pressed for real, and the digits
                        // typed after it belong to whatever follows them.
                        self.settle_alias_count();
                        if held.is_empty() {
                            key
                        } else {
                            self.play_keys(&held);
                            return self.on_normal_key(key);
                        }
                    }
                }
            } else {
                if !held.is_empty() {
                    self.settle_alias_count();
                    self.play_keys(&held);
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

        // Laid out vertically, the arrow keys and `hjkl` keep their *screen*
        // meaning: `j` still reads onward down the 縱, and `h` still steps left,
        // which is now the next 縱 rather than the next line.
        if self.layout() == Layout::Vertical {
            // The count applies here too — `10j` is exactly the key a 縱 of
            // thirty-two characters is long for. These arms used to return
            // before `repeat` could see it.
            match key {
                // Only the first step of a run may reset the goal slot; the
                // rest of a `10h` continues from the one it chose.
                Key::Char('h') | Key::Left => {
                    let mut first = continuing_zong;
                    return self.repeat(count, move |e| {
                        e.move_zong_from(true, first);
                        first = true;
                    });
                }
                Key::Char('l') | Key::Right => {
                    let mut first = continuing_zong;
                    return self.repeat(count, move |e| {
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
                    return self.repeat(count, |e| e.move_horizontal(motion::next_grapheme));
                }
                Key::Char('k') | Key::Up => {
                    return self.repeat(count, |e| e.move_horizontal(motion::prev_grapheme));
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
                _ => {}
            }
        }

        match key {
            // **Across the break.** A line is not a wall: `l` off the end of
            // one sentence lands on the start of the next, and `h` walks back
            // over the break the same way. The line-bound pair is still what
            // measures a line; it is not what a reader walking a page wants.
            Key::Char('h') | Key::Left => {
                self.repeat(count, |e| e.move_horizontal(motion::prev_grapheme))
            }
            Key::Char('l') | Key::Right => {
                self.repeat(count, |e| e.move_horizontal(motion::next_grapheme))
            }
            Key::Char('k') | Key::Up => self.repeat(count, |e| e.move_vertical(true)),
            Key::Char('j') | Key::Down => self.repeat(count, |e| e.move_vertical(false)),
            Key::Home => {
                let pos = motion::line_start(self.current_buffer().rope(), self.sel.head());
                self.move_head(pos);
            }
            Key::End => {
                let pos = motion::line_last(self.current_buffer().rope(), self.sel.head());
                self.move_head(pos);
            }
            // **`Enter`, `+` and `-`** — vim's three ways of saying 「the next
            // line, where its writing begins」 (B4, 2026-09-20). `Enter` is
            // unbound in Normal here, and `+`／`-` are in the preset's table;
            // this is the one that cannot be spelled there, because the table
            // holds characters.
            Key::Enter if self.key_preset == yumete_cjk::KeyPreset::Vim => {
                self.repeat(count, |e| {
                    let down = e.run_motion(motion::Motion::Line { down: true });
                    e.jump_to(down);
                    let home = e.run_motion(motion::Motion::LineFirstNonBlank);
                    e.jump_to(home);
                });
            }
            // **A standalone motion, read vim's way** (B3, 2026-09-20).
            //
            // 2026-09-20：「vim 的 `w` 獨立的時候是跳轉，在命令中是選詞。
            // helix 就是將跳轉和選擇兩個 `w` 合一了。」 So under the vim preset
            // a bare motion asks for the **caret** reading: it goes to the
            // primitive — the next word's first character — and paints
            // nothing. Extend mode still extends, because `jump_to` moves the
            // head and leaves the anchor where it is: that is vim's visual
            // mode, for free.
            Key::Char(c @ ('w' | 'W' | 'b' | 'B' | 'e' | 'E' | '{' | '}' | 'H' | 'L'))
                if self.key_preset == yumete_cjk::KeyPreset::Vim
                    && !self.expanding_alias
                    && crate::vim::step_for(&c.to_string(), self.word_grain(), None).is_some() =>
            {
                let grain = self.word_grain();
                let step = crate::vim::step_for(&c.to_string(), grain, None)
                    .expect("the guard just asked");
                self.repeat(count, |e| {
                    let span = e.read_motion(step.motion, motion::Reading::Caret);
                    e.jump_to(span);
                });
            }
            // Word motions (Helix `w`/`b`/`e`, and WORD `W`/`B`/`E`).
            Key::Char('w') => self.repeat(count, |e| e.select_word_forward(false)),
            // **`e` is coarse whatever the dictionary says** (#304). Chinese
            // has no spaces, so an `e` that consulted the dictionary would do
            // very nearly what `w` does; left coarse it runs to the next
            // punctuation instead — `w` takes a word, `e` takes a clause.
            //
            // And it sets **both** ends: a span anchored at the old caret
            // would give, from a word's last character, 「that character ＋ the
            // next word」 — the punctuation between them riding along in both
            // directions.
            Key::Char('e') => self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::WordEnd(motion::Grain::Coarse));
                e.take_span(span);
            }),
            // **`b` is `e`'s partner, not `w`'s** (#304). helix's tutor teaches
            // 「to select the word under cursor, combine `e` and `b`」, and the
            // two only compose if they read the same grain. Ours are coarse, so
            // the pair takes a **clause** — which is the more useful unit here:
            // an English word is many letters, a Chinese word is two and a half
            // characters, and the thing a writer wants to grab is the run
            // between two 標點.
            //
            // The cost, taken deliberately: **there is no 「back one word」**.
            // `w` is the only key on the dictionary's grain, and it only goes
            // forward. `C-w` in Insert still deletes a *word* — it says why in
            // its own doc, and deleting a clause there would be brutal.
            Key::Char('b') => self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::WordBack(motion::Grain::Coarse));
                e.take_span(span);
            }),
            // A paragraph is a logical line here, and with soft wrap on `j`
            // and `k` move by visual row — so these are the keys that move by
            // what a writer calls a paragraph, and nothing else does.
            Key::Char('}') => self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::Paragraph { forward: true });
                e.take_span(span);
            }),
            Key::Char('{') => self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::Paragraph { forward: false });
                e.take_span(span);
            }),
            // 。！？ and the closing mark that follows one. The unit a person
            // proofreads in, and the one the manual already teaches by telling you
            // to break the file on 。 with `:%s`.
            //
            // **On `H`/`L` since 2026-09-12** (#404), for two reasons at once.
            // They were on `(`/`)`, which Helix spends on cycling the primary
            // selection — a multi-cursor key we will want when #405 lands, and
            // squatting on it now would mean moving twice. And the pair they
            // moved to reads better than the one they left: capitals are the
            // bigger unit along the same axis, so
            //
            // ```text
            // h  l   one character      j  k   one row
            // H  L   one sentence       J  K   half a page
            // ```
            //
            // Warning: `H`/`L` used to be whole-page paging. Nothing was lost:
            // `C-f` and `C-b` still do it, and the pair a reader actually
            // wears out is the *half* page on `J`/`K`.
            Key::Char('L') => self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::Sentence { forward: true });
                e.take_span(span);
            }),
            Key::Char('H') => self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::Sentence { forward: false });
                e.take_span(span);
            }),
            // A mark is where you meant to come *back* to; the jump list is
            // where you came *from*. `M`/`'` rather than vi's `m`/`'`, because
            // `m` here opens match mode.
            Key::Char('M') => self.pending = Pending::Mark,
            Key::Char('\'') => self.pending = Pending::Recall,
            // 「下一個這種東西」, which is where Helix keeps it too.
            // **`z` 那一層**：把光標這一行挪到屏幕的頂／中／底（2026-09-28）。
            // Warning: `zc` 也是居中，同 helix（它的 `zc` 是 align_view_center）。
            Key::Char('z') => self.pending = Pending::Aim,
            Key::Char(']') => self.pending = Pending::Hop { forward: true },
            Key::Char('[') => self.pending = Pending::Hop { forward: false },
            Key::Char('W') => self.repeat(count, |e| e.select_word_forward(true)),
            Key::Char('E') => self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::WordEnd(motion::Grain::Big));
                e.take_span(span);
            }),
            Key::Char('B') => self.repeat(count, |e| {
                let span = e.run_motion(motion::Motion::WordBack(motion::Grain::Big));
                e.take_span(span);
            }),
            Key::Char('g') => {
                self.pending = Pending::Goto;
                self.operator_count = operator_count;
            }
            // Helix's Space menu: the things that are not motions.
            Key::Char(' ') => self.pending = Pending::Space,
            // In-line character search — **all four of vi's**, `f` `F` `t` `T`.
            //
            // Warning: **`t`／`T` were the table group until 2026-09-21**, retired as
            // till-keys on the reasoning that a verb-last editor puts till
            // 「one keystroke away from find and no more」. Two things undid
            // that: the vim preset puts the verb **first** (`dt,`), and
            // **helix's own `t` is `find_till_char`**
            // (`helix-term/src/keymap/default.rs:14`) — so one key was costing
            // *both* hands their muscle memory to save one keystroke in the
            // one group that can afford to be a keystroke longer. 「表格操作
            // 並不是特別頻繁。」 The table group is `空格 t` now.
            Key::Char('f') | Key::Char('F') | Key::Char('t') | Key::Char('T') => {
                self.pending = Pending::Find(match key {
                    Key::Char('f') => FindKind::Forward,
                    Key::Char('F') => FindKind::Backward,
                    Key::Char('t') => FindKind::Till,
                    _ => FindKind::TillBack,
                });
                self.operator_count = operator_count;
            }
            // Select (extend) mode and collapse (Helix `v` / `;`).
            // **vim's `V` is linewise visual**, not 「select this line」
            // (B3, 2026-09-20): the selection grows by *lines* as the caret
            // moves, and a verb takes those lines entire. The old translation
            // (`V` → `x`) could take three lines only if told `3V`; a vim hand
            // types `Vjj`.
            Key::Char('V') if self.key_preset == yumete_cjk::KeyPreset::Vim => {
                self.select_line();
                self.extend = true;
                self.vim_lines = true;
            }
            Key::Char('v') => {
                self.extend = !self.extend;
                self.vim_lines = false;
            }
            // Esc is every modal editor's way out. It leaves select mode and
            // collapses the selection — and, when the page is showing you
            // something in the other work area, it dismisses that first: a
            // preview is the transient thing on the screen, and Esc is the key
            // every reader presses at a transient thing.
            Key::Esc => {
                // **浮窗先收**（2026-10-01 提）。它是屏幕上最臨時的那一樣，
                // 而 `Esc` 是人對着臨時的東西按的那一鍵。收的只是「這會兒別擋
                // 着」——挪開光標它又回來，邊欄一動不動。
                if self.hush_the_float() {
                    return;
                }
                if self.other.is_some() && self.live_pane == 0 {
                    // **The window goes; the search stays.** Esc dismissed the
                    // pane, and the reader who then presses `n` means the same
                    // 「下一處」 they meant a moment ago — so `n` walks to the
                    // next hit and brings the pane back with it. Throwing the
                    // list away here handed `n` to `/` instead, which answered
                    // about some older pattern, or about nothing at all, and
                    // never said which.
                    self.close_split();
                    return;
                }
                // **Esc 没有別的事可做的時候，就是「把輸入法的挂起再說一
                // 遍」**（2026-09-22）。收窗口、收選區都輪不到這一件；而收不收得
                // 到選區，看的是它本來收不收得起來。
                let idle = !self.extend && self.sel.anchor() == self.sel.head();
                self.extend = false;
                self.sel.set_anchor(self.sel.head());
                if idle {
                    self.say_it_again = true;
                }
            }
            // `;` collapses the selection but leaves select mode standing —
            // Helix's own behaviour, and the reason it looks broken to a
            // reader who has just pressed `v`: the very next motion grows the
            // selection again. So it says which of the two happened.
            // **vim repeats a find with `;` and `,`** (#428, 2026-09-18) —
            // here that is `A-.`, which no vim hand will ever press. `,` is
            // the same search the other way round.
            // Warning: **…but not while an alias is being played** (2026-09-19).
            // The preset writes `;` into its own expansions to mean 「collapse
            // first」 (`x` is `;{n}D`), and once a find had happened that `;`
            // repeated the find instead — so after `f,` the preset's `x` cut
            // everything the `f` had selected rather than one character.
            Key::Char(c @ (';' | ','))
                if self.key_preset == yumete_cjk::KeyPreset::Vim
                    && self.last_find.is_some()
                    && !self.expanding_alias =>
            {
                if let Some((kind, ch)) = self.last_find {
                    let kind = match c {
                        ',' => kind.flipped(),
                        _ => kind,
                    };
                    // 數目是「第 n 個」，重複要往前挪一個——兩件事都在
                    // `find_nth_char` 裏，不靠把它叫 n 遍。
                    self.find_nth_char(kind, ch, count, true);
                }
            }
            Key::Char(';') => {
                self.sel.set_anchor(self.sel.head());
                if self.extend {
                    self.status = say!("selection.collapsed-in-extend");
                }
            }
            // **多選區**（#405）：`C`／`A-C` 往下／往上再加一段，`,` 只留主選區。
            // 三個都是 helix 的拼法。Warning: vim 預設下 `C` 被別名成 `c$`，`,` 是「反向重複
            // 剛纔那個 f」，所以那一端按不到這裏——多選區是原生鍵位的東西。
            Key::Char('C') => self.copy_selection_on_row(true, count),
            Key::Alt('C') => self.copy_selection_on_row(false, count),
            Key::Char(',') => self.keep_primary_selection(),
            // **`,` 的反面，和兩種合併，和「全部轉成正向」**（2026-10-06）。
            // §5.13.6 自己把這四個列成「空着的，直接拿來用」，而四期都沒排到。
            Key::Alt(',') => self.remove_primary_selection(),
            Key::Alt('-') => self.merge_selections(false),
            Key::Alt('_') => self.merge_selections(true),
            Key::Alt(':') => self.face_them_forward(),
            // **`A-s` 把每一段選區按行切開**（helix 的 `split_selection_on_newline`）。
            // Warning: 它是這一族裏唯一不用打正則的，所以不開提示行。
            Key::Alt('s') => self.split_on_newline(),
            // **正則那四個**（#405 Phase 2）。它們開的是**搜索那一扇**提示行，於是拼音、
            // 簡繁、模糊、正則四個開關一起管用，中文也照打——helix 的 `s` 只認正則。
            // **`(` `)` 換主選區，`_` 去兩端空白**（#405 Phase 3）。Warning: 前兩個一段都不
            // 動，動的只是「哪一段是主的」——選了二十處要一處一處看過去靠的就是它。
            // 帶數字走幾格——helix 的 `rotate_selections` 吃計數（`commands.rs`
            // 的 `rotate_selections_impl`），從前這裏 `3)` 只走一格（2026-10-06）。
            Key::Char(')') => self.repeat(count, |e| e.rotate_primary(true)),
            Key::Char('(') => self.repeat(count, |e| e.rotate_primary(false)),
            Key::Char('_') => self.trim_selections(),
            // **`&` 把每一段的開頭對齊到同一列**（helix 的 `align_selections`）。
            // Warning: 算的是顯示寬度，一個漢字兩格。
            Key::Char('&') => self.align_selections(),
            // **`A-(` `A-)` 輪轉的是裝在選區裏的字**，邊界不動（helix 的
            // `rotate_selection_contents_*`）。上面那一對換的是「哪一段是主的」。
            Key::Alt(')') => self.rotate_contents(true),
            Key::Alt('(') => self.rotate_contents(false),
            Key::Char('s') => self.open_sift(multi::Sift::Select),
            Key::Char('S') => self.open_sift(multi::Sift::Split),
            Key::Alt('k') => self.open_sift(multi::Sift::Keep),
            Key::Alt('K') => self.open_sift(multi::Sift::Drop),
            // Selection + changes (Helix: `x` selects the line, `d` deletes the
            // selection, `c` changes it).
            //
            // Warning: **這一族 2026-09-28 翻過一次，現在是 helix 的那一條**（#405）。
            //
            // #492 當初把它定成「小寫刪、大寫剪」，原話是「d 作为剪切功能会污染
            // register。这是我觉得 helix 最不好的地方」——剪貼板只有一個，而編輯器裏
            // 按得最多的那個鍵會花掉它。依據之一是「helix 的 `C`／`D` 都空着」，而**那
            // 句話錯了一半**：helix 的 `C` 是 `copy_selection_on_next_line`，佔着，
            // 只有頂層的 `D` 是空的。多選區（#405）要的正是 `C`。
            //
            // 現在：`d`／`c` 進寄存器（同 helix），`A-d`／`A-c` 不進（同 helix），
            // **`D` 是 `A-d` 的別名**——#492 那個需求（刪掉這段，別蓋掉我剛複製的）
            // 保下來了，只是從最好按的鍵換成了第二好按的。`A-c` 沒有對應的別名，
            // 因為 `C` 讓給了多選區。
            Key::Char('x') => self.repeat(count, |e| e.select_line()),
            // **`d`, `c`, `y` are operators under the vim preset** (#429,
            // 2026-09-18): they wait for a motion. With something already
            // selected they act at once, which is what vim's visual mode does
            // — and it is also what makes `v3wd` go on working for a hand that
            // learnt this editor's own way round.
            Key::Char(op @ ('d' | 'c' | 'y' | '>' | '<'))
                if self.key_preset == yumete_cjk::KeyPreset::Vim && !self.extend =>
            {
                self.pending = Pending::VimOperator { op, first: None };
                self.count = operator_count;
            }
            // Warning: **2026-09-28 整族換成了 helix 的拼法**（#405 要 `C` 這個鍵）。從前是
            // 「小寫刪、大寫剪」，四個鍵一條規矩；現在是 helix 的那一條：
            //
            // - `d` `c`：刪／改，**進寄存器**（helix 一樣）
            // - `A-d` `A-c`：刪／改，**不進**（helix 一樣）
            // - `D`：`A-d` 的別名。寫東西的人最常要的是「刪掉這段，別蓋掉我剛複製的」，
            //   而那在 helix 裏要按 `A-d`。留一個好按的大寫鍵給它。
            //
            // `A-c` 沒有對應的別名，因為 `C` 讓給了「往下再加一個選區」。不對稱，
            // 而「改寫而不進寄存器」遠不如「刪除而不進」常用。
            Key::Char('d')
            | Key::Char('c')
            | Key::Alt('d')
            | Key::Alt('c')
            | Key::Char('D')
                if !matches!(key, Key::Char('D'))
                    || self.key_preset == yumete_cjk::KeyPreset::Vim =>
            {
                self.snapshot();
                // A count deletes that many graphemes when there is nothing
                // selected, the way `3x` does in vim; with a selection it is
                // the selection that goes, once.
                if self.span().0 == self.span().1 && count > 1 {
                    self.extend_by_graphemes(count);
                }
                // **Through the one door** (B2, 2026-09-20): helix hands the
                // verb its own selection, vim will hand it a motion's span,
                // and neither knows the other exists.
                // **`V` means whole lines**, however far along one the caret
                // stopped (B3).
                if self.vim_lines {
                    self.extend_to_line_bounds();
                    self.vim_lines = false;
                }
                // Warning: **`D` 只有 vim 預設下走得到這裏**，而那一端它要進寄存器：那頭的
                // `D` 被別名成 `d$`（`yumete-cjk/src/keymap.rs`），所以原生的 `D` 只在
                // 別名展開裏出現，而 vim 的 `x` 展開成 `;{n}D`，vim 的 `x` 就是進寄存
                // 器的。
                let cut = !matches!(key, Key::Alt('d') | Key::Alt('c'));
                let op = match matches!(key, Key::Char('c') | Key::Alt('c')) {
                    true => motion::Operator::Change { cut },
                    false => match cut {
                        true => motion::Operator::Cut,
                        false => motion::Operator::Delete,
                    },
                };
                let (anchor, head) = self.span();
                self.apply(op, motion::Span::Over { anchor, head });
            }
            // Yank / paste (Helix `y` / `p` / `P`).
            Key::Char('y') => {
                if self.vim_lines {
                    self.extend_to_line_bounds();
                    self.vim_lines = false;
                }
                let (anchor, head) = self.span();
                self.apply(motion::Operator::Yank, motion::Span::Over { anchor, head });
            }
            Key::Char('p') => self.repeat_writing(count, |e| e.paste(true)),
            Key::Char('P') => self.repeat_writing(count, |e| e.paste(false)),
            // Insert (`i` before the selection, `a` after it, `I`/`A` line ends).
            // **vim 預設下，可視模式裏的 `i`／`a` 是物件前綴**（2026-09-28 收到的反饋：
            // 「vi* 进入选择/高亮模式（目前是因为 i 键的关系，进入了插入模式）」）。
            //
            // Warning: **只改 vim 那一端。** helix 的 select 模式是整份繼承 normal 的
            // （`keymap/default.rs:342` 的 `select = normal.clone()`，`i` 沒有被蓋掉），
            // 所以 helix 按 `vi` 也是進插入——原生鍵位現在的行為就是 helix 的行為，
            // 原生那一端要取物件是 `mi(`。vim 的 `vi(` 纔是選中括號裏那一段。
            //
            // 代價是 vim 預設下不能再從可視模式按 `i` 進插入。vim 自己也不能：那一端
            // 的 `i` 在可視模式裏就是物件前綴。要進插入先按 `v` 或 `Esc` 出來。
            Key::Char(key @ ('i' | 'a'))
                if self.extend && self.key_preset == yumete_cjk::KeyPreset::Vim =>
            {
                self.pending = Pending::MatchPair { around: key == 'a' };
            }
            Key::Char('i') => {
                self.snapshot();
                let pos = self.selection().0;
                self.set_cursor(pos);
                self.enter_insert();
                self.type_it_again(count, None);
            }
            Key::Char('a') => {
                self.snapshot();
                let pos = self.append_position();
                self.set_cursor(pos);
                self.enter_insert();
                self.type_it_again(count, None);
            }
            Key::Char('I') => {
                self.snapshot();
                let pos = motion::line_start(self.current_buffer().rope(), self.sel.head());
                self.set_cursor(pos);
                self.enter_insert();
                self.type_it_again(count, None);
            }
            Key::Char('A') => {
                self.snapshot();
                let pos = motion::line_end(self.current_buffer().rope(), self.sel.head());
                self.set_cursor(pos);
                self.enter_insert();
                self.type_it_again(count, None);
            }
            Key::Char('o') => {
                self.snapshot();
                self.open_line_below();
                self.type_it_again(count, Some(true));
            }
            Key::Char('O') => {
                self.snapshot();
                self.open_line_above();
                self.type_it_again(count, Some(false));
            }
            // Undo/redo (Helix: `u` / `U`).
            Key::Char('u') => self.repeat_writing(count, |e| e.undo()),
            Key::Char('U') => self.repeat_writing(count, |e| e.redo()),
            // Search (`/` forward, `?` backward, `n`/`N` repeat).
            // `!` is what it is in vi: send this through a command and take
            // what comes back. It opens the command line with the verb already
            // typed, so the key is a shortcut and not a second mechanism —
            // and so a reader who presses it by accident can see what it was
            // about to do and press Esc.
            Key::Char('!') => self.open_the_command_line("pipe "),
            Key::Char('/') => {
                self.mode = Mode::Search;
                self.search_forward = true;
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
                // A new search takes `n` back from the table's.
                self.hits = None;
            }
            Key::Char('?') => {
                self.mode = Mode::Search;
                self.search_forward = false;
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
                self.hits = None;
            }
            // In a table with a search open, `n` walks *its* answers: they are
            // the last search that happened, which is what `n` has always
            // meant. A plain `/` clears them and takes the key back.
            Key::Char('n') if self.walking_hits() => {
                self.repeat(count, |e| {
                    e.walk_table_hits(true);
                });
            }
            Key::Char('N') if self.walking_hits() => {
                self.repeat(count, |e| {
                    e.walk_table_hits(false);
                });
            }
            Key::Char('n') => self.repeat(count, |e| e.repeat_search(e.search_forward)),
            Key::Char('N') => self.repeat(count, |e| e.repeat_search(!e.search_forward)),
            Key::Char(':') => self.open_the_command_line(""),
            // Match mode (Helix `m`): matching bracket, textobjects, surround.
            Key::Char('m') => self.pending = Pending::Match,
            // Overwrite every character of the selection with the next key.
            Key::Char('r') => self.pending = Pending::Replace,
            // Name the register the next yank, delete or paste will use.
            Key::Char('"') => self.pending = Pending::Register,
            // Record a macro, and play the last one back.
            // **`Q` records, `q` replays** — Helix's way round, and ours since
            // 2026-09-12. It was the other way, and both keys explained
            // themselves, but Warning: **a swapped pair is the worst kind of
            // divergence**: the hand does not read, it just presses, and the
            // wrong one of these two is not a no-op — it starts recording over
            // what you meant to play back (#404).
            Key::Char('Q') => self.toggle_recording(),
            Key::Char('q') => self.replay_macro(count),
            // A page, and half of one, in the direction the text is read.
            // The next region — the panels and the work areas, in the order
            // they sit on the screen. vi spells window motions `C-w`, and this
            // is the one it spells `C-w w` (#293).
            // **`C-w` 是那一組的另一扇門**（2026-09-30 定，同 helix：
            // `default.rs:193` 是 `C-w`，`:260` 是 `空格 w`，兩處同一組）。
            // 走下一區因此是 `C-w w`——vi 本來就是這麽拼的。
            Key::Ctrl('w') => self.pending = Pending::Region,
            // …and the keys a keyboard already has for it. `C-f`/`C-b` are
            // vi's; these are the ones a reader who has never used vi presses.
            // Warning: **浮窗開着的時候它們歸浮窗**（見 `on_key` 開頭，同 helix）。
            Key::Ctrl('f') | Key::PageDown => self.move_page(count, false, 1.0),
            Key::Ctrl('b') | Key::PageUp => self.move_page(count, true, 1.0),
            // Back and forward through the places jumps came from, as in vi
            // and in Helix. Under the Kitty protocol `C-i` and Tab are told
            // apart; without it a terminal sends the same byte for both, and
            // `C-i` simply does whatever Tab does.
            Key::Ctrl('o') => self.walk_jumps(true),
            // Warning: **`Tab` 在 Normal 裏也是這一件事**（2026-10-06，對齊 helix 的
            // `jump_forward`）。終端把 `Tab` 和 `C-i` 發成同一個字節，而這個編輯器
            // 分得開（`kitty` 那套協議），於是從前 `Tab` 在 Normal 裏一個 arm 都
            // 沒有、靜默掉地——偏偏一個終端一換，手上那個 `C-i` 就變成了它。
            Key::Ctrl('i') | Key::Tab => self.walk_jumps(false),
            Key::Ctrl('d') => self.move_page(count, false, 0.5),
            Key::Ctrl('u') => self.move_page(count, true, 0.5),
            // **三分之二頁**（2026-10-01 定）。起因是 `C-u`／`C-d` 在 Mac 上
            // 不好按——那塊鍵盤只有一個 Ctrl，而 `u` 和 `d` 都在左手這一邊；
            // `n` 和 `p` 在右手，左 Ctrl 配右手字母順得多。
            //
            // Warning: **二三頁不是哪一家的慣例。** 兩種慣例是「整頁留一兩行」
            // （vim 的 `C-f`、Emacs 的 `next-screen-context-lines`）與「半頁」
            // （vim 的 `C-d`）。推理和 vim 留那兩行是同一個：留一截做
            // 錨。只是留得大方——「前一页留 1/3 不会断，2/3 的新页增加阅读效
            // 率」。三種都留着，用幾天見分曉。
            Key::Ctrl('n') => self.move_page(count, false, 2.0 / 3.0),
            Key::Ctrl('p') => self.move_page(count, true, 2.0 / 3.0),
            // …and on the capitals of the keys that move, which is a reader's
            // most-used pair and does not deserve a chord. `C-d` and its family
            // still work; these are the same four motions under the fingers
            // already on `hjkl`.
            Key::Char('J') => self.move_page(count, false, 0.5),
            Key::Char('K') => self.move_page(count, true, 0.5),
            // Swap which end of the selection the cursor is on.
            Key::Alt(';') => self.flip_selection(),
            // Whole file, and extending the selection to whole lines.
            Key::Char('%') => self.select_all(),
            // vim's `X` under the vim preset (#428): the character *before*
            // the cursor, cut, and never across the start of the line — which
            // is why it is an action and not a line in the preset's table: a
            // translation into `h` would walk over the break and join two
            // lines, and could not carry `3X` to the cut.
            // **vim's redo is `C-r`** (#428, 2026-09-18): here it is `U`, and
            // a vim hand pressing `C-r` got a line of prose pointing at it.
            // Under the preset it simply redoes.
            Key::Ctrl('r') if self.key_preset == yumete_cjk::KeyPreset::Vim => {
                self.repeat(count, |e| e.redo());
            }
            Key::Char('X') if self.key_preset == yumete_cjk::KeyPreset::Vim => {
                self.snapshot();
                self.cut_before_cursor(count);
            }
            Key::Char('X') => self.extend_to_line_bounds(),
            // 字形變換 (§5.2.3 ②): `` `l `` 小寫, `` `u `` 大寫, `` `` `` 互換.
            // `~` and `` A-` `` are Helix's and are **unbound** here — the
            // phrasebook catches both and points at this group.
            Key::Char('`') => self.pending = Pending::Case,
            // **`~` is `` ` `` `` ` ``, and `*` is `g/`** (#404, 2026-09-11).
            // Both were hints pointing at where the thing had moved to, which
            // is the right answer for a key we deliberately spell differently
            // — but these two are not spelled differently, they are the *same*
            // action under the name vi and Helix both give it. A hint that
            // could have just done it is a hint that costs a keystroke and
            // teaches nothing.
            //
            // Warning: `~` maps to the group's **third** member, not to the group:
            // `` ` `` opens 「小寫／大寫／互換」, and `~` has always meant the
            // last of those on its own.
            Key::Char('~') => {
                self.map_selection(switch_case);
                // **vim 的 `~` 按完往前走一格**（`:h ~`），所以 `~~~` 在 `alpha`
                // 上給 `ALPha`；從前這裏三下都換同一個字，給的是 `Alpha`。
                // helix 的 `~` 不走（它作用在整個選區上），兩邊各跟各的。
                if self.key_preset == yumete_cjk::KeyPreset::Vim && !self.extend {
                    let rope = self.current_buffer().rope();
                    let at = self.selection().1.min(motion::line_end(rope, self.sel.head()));
                    self.sel.set_head(at);
                    self.sel.set_anchor(at);
                    self.clamp_cursor();
                }
            }
            Key::Char('*') => {
                self.definition_preview = false;
                self.search_the_page(false);
            }
            // **vim 的 `#`：往回找光標下這個詞**（2026-10-05 定）。
            //
            // Warning: **只在 vim 鍵位下，而且不走鍵位表。** 那張表是「鍵 → 一串
            // 鍵」，而往回找在 helix 那一套上沒有鍵（`g/` 往前、`g?` 是在副編輯區
            // 給你看，沒有一支是往回），所以表裏沒有東西可以映——這一條直接寫在
            // 鍵上，同 `d`／`c`／`y` 那幾個操作符。helix 鍵位下不給鍵，要的人在配
            // 置裏繫得上。
            Key::Char('#') if self.key_preset == yumete_cjk::KeyPreset::Vim => {
                self.definition_preview = false;
                self.search_the_page(true);
            }
            // Replacing the selection with the register. Joining is on `gJ`:
            // `J` turns the page, which a reader presses a hundred times for
            // every once they join two lines.
            Key::Char('R') => self.replace_with_register(),
            // Search for whatever is selected (Helix `*`).
            // **`30G` goes to line 30**, and a bare `G` to the last line,
            // which is what `G` means in vi; the key was unbound here.
            // `10gg` and `:30` still work; this is the one a reader's fingers
            // already know.
            //
            // Warning: **helix's bare `G` does nothing** (2026-10-06 correction —
            // this comment used to claim it went to the last line there too).
            // `goto_line_impl` opens with `if cx.count.is_some()`
            // (`helix-term/src/commands.rs`), so an un-counted `G` falls
            // through. Going somewhere useful is better than going nowhere,
            // so the key stays as it is; only the claim was wrong.
            Key::Char('G') => {
                self.remember_jump();
                // `operator_count`, not `self.count`: the count was taken at
                // the top of this function, so asking `self.count` here always
                // said "no digits" and `1G` went to the *last* line.
                match operator_count.is_some() {
                    true => self.goto_line(count),
                    false => {
                        let rope = self.current_buffer().rope();
                        self.move_head(motion::buffer_end(rope, self.sel.head()));
                    }
                }
            }

            // Indent / unindent the selected lines.
            Key::Char('>') => self.repeat_writing(count, |e| e.indent(true)),
            Key::Char('<') => self.repeat_writing(count, |e| e.indent(false)),
            // Increment / decrement the number at the cursor.
            Key::Ctrl('a') => self.repeat_writing(count, |e| e.bump_number(1)),
            Key::Ctrl('x') => self.repeat_writing(count, |e| e.bump_number(-1)),
            // Repeat the last insert, and the last `f`/`t`.
            Key::Char('.') => self.repeat_writing(count, |e| e.repeat_edit()),
            Key::Alt('.') => {
                if let Some((kind, c)) = self.last_find {
                    self.find_nth_char(kind, c, count, true);
                }
            }
            // Nothing here does what this key does elsewhere — so say what
            // yumete calls the thing you meant, in the place a reader is
            // already looking. See [`Self::phrasebook`].
            other => {
                if let Some(said) = Self::phrasebook(other) {
                    self.status = said;
                }
            }
        }
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
                self.find_nth_char(kind, c, count, false);
            }
            Pending::Replace => self.replace_chars(c),
            Pending::MatchPair { around } => self.select_pair(c, around),
            Pending::Surround => self.surround_add(c),
            Pending::SurroundOff => self.surround_delete(c),
            Pending::SurroundFrom => self.pending = Pending::SurroundTo(c),
            Pending::SurroundTo(from) => self.surround_replace(from, c),
            // Everything else waits for a letter naming a command, and those
            // are answered where they are read.
            _ => {}
        }
    }

    /// What to say when a key that means something in another editor is
    /// pressed here and means nothing.
    ///
    /// **Not a compatibility layer**: it never *does* the thing. The first
    /// minute in any editor is spent pressing exactly these keys, and a key
    /// that does nothing and says nothing is an hour of guessing. A key that
    /// says 「行尾是 gl」 is an hour of learning.
    ///
    /// Reached only from the fall-through, so it can never contradict a real
    /// binding: bind the key and this stops being consulted.
    fn phrasebook(key: Key) -> Option<String> {
        let c = match key {
            Key::Char(c) => c,
            // Helix's 轉大寫. Its two companions are `~` and `` ` ``; the
            // group took the third, so this is the only one that needs the
            // `Alt` arm.
            Key::Alt('`') => {
                return Some(say!("hint.helix.case-keys"));
            }
            // vi's redo. Ours is `U`, the capital of the key that undoes —
            // a chord for it would be a second name for one action.
            Key::Ctrl('r') => return Some(say!("hint.vi.ctrl-r")),
            // Helix spends two tutor lessons on `C-c` (11.1, 11.2). We have
            // the thing, on the 空格 menu with the other 「do something to
            // this line」 keys, so this says where rather than binding a chord.
            Key::Ctrl('c') => return Some(say!("hint.helix.comment")),
            _ => return None,
        };
        Some(match c {
            '$' => say!("hint.vi.dollar"),
            '^' => say!("hint.vi.caret"),
            // vi's 行首. It gets here only with no count under way — with one,
            // `0` is still the digit it looks like (`20l`).
            '0' => say!("hint.vi.zero"),
            'Z' => say!("hint.vi.z-upper"),
            '@' => say!("hint.vi.at"),
            '+' | '-' => say!("hint.vi.line-motions"),
            '\\' => say!("hint.vi.backslash"),
            // helix 頂層的 `D` 是空的，這裏也空着（2026-09-28）。按它的人分兩種：
            // vim 手要的是「刪到行尾」，helix 手要的是「刪了別動寄存器」，一句話說得
            // 完兩件。Warning: vim 預設下按不到這裏，那一端 `D` 是 `d$`。
            'D' => say!("hint.helix.capital-d"),
            // No `` ` `` arm: it is a real binding now (the 字形 group), so the
            // fall-through never reaches here for it. `hint.vi.backtick` moved
            // into that group's menu, where vi's reader will see it anyway.
            _ => return None,
        })
    }

    /// Handle the second key of a goto (`g`) sequence, Helix-style: `gg` to the
    /// buffer start, `ge` to the last line, `gh`/`gl` to line start/end, `gs` to
    /// the first non-blank character.
    /// How many joins `gJ`/`gK` should make (#518).
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
    fn join_count(&mut self) -> usize {
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
        self.sequence_span()
            .map(|(n, _)| n)
            .or_else(|| self.operator_count.take())
            .unwrap_or(2)
            .saturating_sub(1)
            .max(1)
    }

    fn handle_goto(&mut self, key: Key) {
        // `10gg` is "goto line 10", the way Helix reads a count before `gg`;
        // a bare `gg` is the same thing with the count 1.
        if key == Key::Char('g') {
            // `g30g` — the sequence's own argument — and `30gg`, vi's order.
            let line = self.sequence_span().map(|(n, _)| n).or(self.operator_count.take());
            if let Some(n) = line.filter(|&n| n > 0) {
                return self.goto_line(n);
            }
        }
        // **縱書 turns the four the way it turns `hjkl`** (2026-09-17). On the
        // horizontal page `gj`／`gk` cross lines and `gh`／`gl` run along one;
        // on a 縱書 page the line runs down the 縱 and the lines stack
        // leftward, so running along is `k`／`j` and crossing is `h`／`l` —
        // `h`, leftward, being onward, as it is for `h` alone.
        let key = match (self.layout() == Layout::Vertical, key) {
            (true, Key::Char('h')) => Key::Char('j'),
            (true, Key::Char('l')) => Key::Char('k'),
            (true, Key::Char('j') | Key::Down) => Key::Char('l'),
            (true, Key::Char('k') | Key::Up) => Key::Char('h'),
            (_, key) => key,
        };
        // `gg` and `ge` cross a document; `gh`, `gl` and `gs` cross a line.
        // `remember_jump`'s own doc comment said every far motion went through
        // `goto_line` and so had a way back — and `gg`/`ge` did not, because
        // they are the two that do not name a line number.
        if matches!(key, Key::Char('g') | Key::Char('e')) {
            self.remember_jump();
        }
        // **Five of these are motions now** (B1, 2026-09-20) — named, so the
        // vim grammar can ask for the same five without anybody replaying `g`
        // and then a letter. The consumer is unchanged: a goto **collapses**
        // (`move_head`), which is exactly the reading `jump_to` wraps.
        let go = |e: &mut Self, what: motion::Motion| {
            let span = e.run_motion(what);
            e.jump_to(span);
        };
        match key {
            Key::Char('g') => return go(self, motion::Motion::FileStart),
            // Warning: **vim 鍵位下 `ge`／`gE` 是 vim 的 `ge`**（2026-10-02 定
            // 「照參考實現」）：往回到上一個詞的末尾。helix 鍵位下照舊是「到檔
            // 尾」。兩套鍵位在這一格上真的各說各的，所以這裏分家。
            Key::Char('e' | 'E') if self.key_preset == yumete_cjk::KeyPreset::Vim => {
                let grain = match key {
                    Key::Char('E') => motion::Grain::Big,
                    _ => self.word_grain(),
                };
                let count = self.operator_count.take().unwrap_or(1).max(1);
                return self.repeat(count, |e| {
                    let span = e.read_motion(motion::Motion::WordEndBack(grain), motion::Reading::Caret);
                    e.jump_to(span);
                });
            }
            Key::Char('e') => return go(self, motion::Motion::FileEnd),
            Key::Char('h') => return go(self, motion::Motion::LineStart),
            Key::Char('l') => return go(self, motion::Motion::LineEnd),
            Key::Char('s') => return go(self, motion::Motion::LineFirstNonBlank),
            // helix 的 `goto_last_modification`（`keymap/default.rs:88`）。
            Key::Char('.') => return self.goto_last_modification(),
            // …以及 `goto_last_accessed_file`（同一張表 :84）。
            Key::Char('a') => return self.goto_last_file(),
            _ => {}
        }
        // What is left here either edits (`gJ`, `gK`) or goes somewhere that is
        // not a motion of the page (a file, a definition, the other pane), so
        // every arm answers for itself.
        match key {
            // Joining lines, which vi also spells `gJ`.
            // Joining two lines of a grid makes one row with twice the fields
            // — the one thing table mode promises cannot happen. It went
            // round the two gates because it edits the rope directly.
            Key::Char('J') if self.joining_welds_a_grid() => {
                self.status = say!("table.join-would-change-columns");
            }
            Key::Char('J') => {
                let count = self.join_count();
                self.repeat_writing(count, |e| e.join_lines())
            }
            // …and the other direction, which helix does not have (#485).
            //
            // Warning: **Step up once, then join downwards** (2026-09-19, caught in
            // review). Repeating `join_with_above` walked *out* of what was
            // selected: each call asks where the selection starts now, and
            // after the first join that is one line higher — so `gK` on three
            // selected lines welded two lines nobody had picked and left the
            // selected ones alone. The line above and the selection is the
            // whole of what `gK` may touch.
            Key::Char('K') => {
                let rope = self.current_buffer().rope();
                let (start, end) = self.selection();
                // **A selection joins to the line above it**, so there is one
                // more seam to close than `gJ` has: three lines picked and the
                // one over them make four, which is three joins.
                let spans = end > start
                    && rope.char_to_line(start)
                        < rope.char_to_line((end - 1).min(rope.len_chars().saturating_sub(1)));
                let count = self.join_count() + usize::from(spans);
                let rope = self.current_buffer().rope();
                let line = rope.char_to_line(self.selection().0);
                if line == 0 {
                    self.status = say!("edit.no-line-above");
                    return;
                }
                let above = rope.line_to_char(line - 1);
                self.set_cursor(above);
                self.repeat_writing(count, |e| e.join_lines())
            }
            // **`gj` and `gk` walk lines of the file, as helix's do** — `j`
            // and `k` walk the rows on the screen, and in a manuscript a line
            // of the file is a paragraph, so this is the next paragraph at
            // the same column. Warning: Until 2026-09-17 this was a copy of `j`／`k`,
            // on a comment that had helix the wrong way round (「In helix the
            // plain pair walks logical lines」 — it is `move_visual_line_down`).
            Key::Char('j') | Key::Down => {
                let count = self.operator_count.take().unwrap_or(1).max(1);
                self.repeat(count, |e| e.move_textual_line(false))
            }
            Key::Char('k') | Key::Up => {
                let count = self.operator_count.take().unwrap_or(1).max(1);
                self.repeat(count, |e| e.move_textual_line(true))
            }
            // Goto the next / previous buffer, as Helix binds them.
            Key::Char('n') => self.next_buffer(),
            Key::Char('p') => self.prev_buffer(),
            // Open the file named on this line — a `:grep` hit, or a line
            // pasted in from any other tool that prints `path:line:`.
            Key::Char('f') => self.goto_file_under_cursor(),
            // **`gx` follows what is written here.** vim and Helix both keep
            // 「open the thing under the cursor」 on this key, and in a
            // manuscript the thing under the cursor is a link.
            Key::Char('x') => self.follow_link(),
            // **`gd` goes, `gD` shows.** The pair every editor has: `gd` is
            // *go to definition* — on a footnote that is the note, in a 拆分
            // column the row the component names — and `gD` is the same
            // question answered in the other work area, without leaving.
            //
            // The capital is the whole rule: this editor already says 「the
            // same question, shown over there」 twice (`g/`／`g?`, `t/`／`t?`),
            // and `w` said nothing at all. One letter, and the shift key means
            // 「without leaving」.
            Key::Char('d') => self.show_definition(false),
            Key::Char('D') => self.show_definition(true),
            // **一眼跳到屏幕上任何地方**（#406，2026-09-28）。查定義是 `gd`／`gD`
            // 了（2026-09-09 改的名），而 `gw` 從那天起只剩一句「它搬家了」——
            // Warning: 一個 helix 使用者按 `gw` 收到的是那句話，**看起來像個答案而答的
            // 是另一個問題**。現在它就是 helix 的那個 `gw`。
            Key::Char('w') => self.start_jump(),
            // **按「那裏寫的什麼」跳**（§5.73，2026-10-04 定）。`gw` 是屏幕發號碼，
            // 這兩個是打那裏寫的字。**一個鍵一種文字**：`o` 只問西文、打字面、定長
            // 兩個字母；`u` 只問中文、打讀音、不定長（`zh`／`sh`／`ji` 那幾個擁擠的
            // 聲母兩個字母收不住）。分開的理由是候選面板畫在光標處，會蓋住要跳的
            // 地方——見 `seek.rs` 開頭。
            //
            // Warning: **字母是查出來的，不是挑出來的。** `s`／`S`（flash 用的那兩個）
            // 在這個倉是「選出所有匹配」「拿匹配當分隔符」；`g` 組兩邊都空着的小寫
            // 只有 `o q u v z`——`gb` 會撞 helix 的 goto_window_bottom，`gx` 這裏
            // 已經是「跟着鏈接走」。
            Key::Char('o') => self.start_seek(false),
            Key::Char('u') => self.start_seek(true),
            // **`/` here, `?` over there.** 「這個詞還在哪裏」 — the selection,
            // or what the cursor is on — searched across the whole document.
            // `g/` is the sugar `/` has always wanted: search for *this*,
            // without retyping it. `g?` is the same answer shown in the other
            // work area, so the place you are standing is still on the screen.
            //
            // It used to be `Enter`, which is the key a writer presses by
            // accident: one keystroke too many in Normal mode and the page
            // jumped somewhere else.
            Key::Char('/') => {
                self.definition_preview = false;
                self.search_the_page(false)
            }
            Key::Char('?') => {
                self.definition_preview = true;
                self.search_the_page(false)
            }
            _ => {}
        }
    }

    /// The keys `Space` opens, and what each of them is for — the list the
    /// which-key overlay draws, so what is offered and what happens cannot
    /// drift apart.
    // Warning: **`空格 e` is gone** (2026-09-18): 「我覺得空格 e 和空格 f 重了，我覺得
    // 留空格 f 就够了」. The file *sidebar* is still there — `:sidebar-left`
    // opens it, and `:sidebar-left files` says so exactly — but a key on this
    // menu is the scarcest thing the editor has, and the picker is what
    // 「open a file」 means now.
    pub const SPACE_KEYS: &'static [(char, &'static str)] = &[
        ('o', "hint.goto.outline"),
        ('f', "hint.goto.open-file"),
        // **`空格 F`：搜工作路徑**（2026-10-01，照 helix 的
        // `file_picker_in_current_directory`）。與 `空格 f` 的分別見
        // [`Editor::working_dir`]。
        ('F', "hint.goto.open-file-here"),
        ('b', "hint.goto.switch-buffer"),
        ('/', "hint.goto.advanced-search"),
        ('?', "hint.goto.all-commands"),
        ('y', "hint.goto.copy-to-clipboard"),
        ('p', "hint.goto.paste-from-clipboard"),
        ('P', "hint.space.paste-before"),
        ('n', "hint.goto.dictionary"),
        ('N', "hint.space.dictionary-panel"),
        ('k', "hint.space.what-is-this"),
        ('K', "hint.space.what-is-this-panel"),
        // **診斷在 `d`**（2026-09-30 定，照 helix：`space d` 是 diagnostics
        // picker）。五種信息同一個形狀——小寫浮，大寫進邊欄。
        ('d', "hint.space.problems"),
        ('D', "hint.space.problems-panel"),
        ('i', "hint.space.record"),
        ('I', "hint.space.record-panel"),
        // **`空格 w` 是區域那一組的門**（2026-09-30 定，照 helix 的 `C-w`：
        // `keymap/default.rs:193` 與 `:260` 是同一組，兩扇門）。
        //
        // Warning: **這裏從前躺着三套說同一件事的詞彙**——`1234`（點名去）、
        // `!@#$`（隔空關）、`w/W/q/Q`（走一步／全開／關／只留）。一組一套之
        // 後，數字整塊空了出來，留給緩衝區（ 2026-09-30 定：每個緩衝區在
        // 檔名前帶一個號，十個起補零，於是那是一套不必按空格確認的前綴碼）。
        ('w', "hint.region.title"),
        ('"', "menu.paste.title"),
        ('c', "hint.space.comment-line"),
        ('C', "hint.space.comment-block"),
        // Warning: **`m`, not `c`** (#409, 2026-09-12). Merge conflicts had `空格 c`
        // and gave it up: helix teaches `空格 c` for commenting, and on an
        // editor for novels a conflict is far the rarer of the two. `m` is
        // merge, and the two letters do not compete for the same word.
        ('m', "hint.conflict.title"),
        // Warning: **The table group moved here** (2026-09-21) so that `t`／`T` could
        // go back to being vi's till-keys — helix spells them that way too
        // (`keymap/default.rs:14`). One slot of this menu buys a whole group,
        // which is the best trade a slot here can make.
        ('t', "hint.space.table"),
    ];

    /// What `g` may be finished with.
    pub(super) const GOTO_KEYS: &'static [(&'static str, &'static str)] = &[
        ("g", "hint.goto.start-of-file"),
        ("e", "hint.goto.end-of-file"),
        ("h l", "hint.goto.line-start-or-end"),
        ("s", "hint.goto.first-non-blank"),
        ("f", "hint.goto.open-this-file"),
        ("x", "hint.goto.follow-link"),
        ("w", "hint.goto.jump-labels"),
        ("o", "hint.goto.seek-written"),
        ("u", "hint.goto.seek-reading"),
        ("n p", "hint.goto.next-or-previous-file"),
        ("d D", "hint.goto.follow-note"),
        ("/ ?", "hint.goto.word-elsewhere"),
        ("j k", "hint.goto.by-file-line"),
        (".", "hint.goto.last-change"),
        ("a", "hint.goto.last-file"),
        ("J", "hint.join-with-line-below"),
        ("K", "hint.join-with-line-above"),
    ];

    /// The same menu on a 縱書 page, where the four directions turn (see
    /// `handle_goto`).
    pub(super) const GOTO_KEYS_VERTICAL: &'static [(&'static str, &'static str)] = &[
        ("g", "hint.goto.start-of-file"),
        ("e", "hint.goto.end-of-file"),
        ("k j", "hint.goto.line-start-or-end"),
        ("s", "hint.goto.first-non-blank"),
        ("f", "hint.goto.open-this-file"),
        ("x", "hint.goto.follow-link"),
        ("w", "hint.goto.jump-labels"),
        ("o", "hint.goto.seek-written"),
        ("u", "hint.goto.seek-reading"),
        ("n p", "hint.goto.next-or-previous-file"),
        ("d D", "hint.goto.follow-note"),
        ("/ ?", "hint.goto.word-elsewhere"),
        ("h l", "hint.goto.by-file-line-vertical"),
        (".", "hint.goto.last-change"),
        ("a", "hint.goto.last-file"),
        ("J", "hint.join-with-line-below"),
        ("K", "hint.join-with-line-above"),
    ];

    /// What `m` may be finished with — 「這一對」，以及拿它做什麼.
    pub(super) const MATCH_KEYS: &'static [(&'static str, &'static str)] = &[
        ("m", "hint.match.pair"),
        ("i", "hint.match.inside"),
        ("a", "hint.match.around"),
        ("s", "hint.match.surround"),
        ("d", "hint.match.take-off"),
        ("r", "hint.change"),
    ];

    /// What `` ` `` may be finished with.
    ///
    /// The last row is `hint.vi.backtick`, which had never been read by
    /// anybody: the phrasebook is consulted only for keys that are *not*
    /// bound, and `` ` `` was bound to 轉小寫. As a group prefix it has a menu,
    /// and a vi reader looking for a mark meets the answer here.
    pub(super) const CASE_KEYS: &'static [(&'static str, &'static str)] = &[
        ("l", "hint.case.lower"),
        ("u", "hint.case.upper"),
        ("`", "hint.case.switch"),
        ("s", "hint.case.to-s"),
        ("t", "hint.case.to-t"),
        ("w", "hint.case.to-tw"),
        ("h", "hint.case.to-hk"),
        ("c", "hint.case.to-c"),
        ("g", "hint.case.to-g"),
        ("j", "hint.case.to-jp"),
        ("", "hint.vi.backtick"),
    ];

    /// What a `:s …c` is waiting for at each match.
    ///
    /// vi's five answers, and the reason the flag is worth having: the writer
    /// looks at this one match and says what happens to **it**.
    /// What `R` in the search panel is asking — one yes, not one per match.
    /// Warning: **Its own words, not [`Self::CONFIRM_KEYS`]'.** Those say 「換這一
    /// 處」／「跳過，不換」 because a `:s …c` is asking about one match at a
    /// time; this is asking about all of them at once, and borrowing that
    /// wording would tell a reader the opposite of what `y` does.
    pub(super) const REPLACE_ALL_KEYS: &'static [(&'static str, &'static str)] = &[
        ("y", "hint.replace-all.yes"),
        ("n", "hint.replace-all.no"),
    ];

    pub(super) const CONFIRM_KEYS: &'static [(&'static str, &'static str)] = &[
        ("y", "hint.confirm.yes"),
        ("n", "hint.confirm.no"),
        ("a", "hint.confirm.all"),
        ("q", "hint.confirm.stop"),
        ("l", "hint.confirm.last"),
    ];

    /// What `]` and `[` may be finished with — 「下一個這種東西」.
    pub(super) const HOP_KEYS: &'static [(&'static str, &'static str)] = &[("c", "hint.hop.conflict")];

    /// The two that read differently depending on which way `[` / `]` points.
    ///
    /// Warning: **段落那一行借的是 `:keymap actions` 的名字**（`action.goto-next-paragraph`
    /// / `action.goto-prev-paragraph`）：同一件事在兩張表上，寫成兩句話只會讓
    /// 它們慢慢說岔。
    pub(super) const HOP_KEYS_FORWARD: &'static [(&'static str, &'static str)] =
        &[("p", "action.goto-next-paragraph"), ("␣", "hint.hop.blank-below")];

    pub(super) const HOP_KEYS_BACK: &'static [(&'static str, &'static str)] =
        &[("p", "action.goto-prev-paragraph"), ("␣", "hint.hop.blank-above")];

    /// What `空格 m` may be finished with — which side of the conflict to keep.
    /// **區域那一組**（`C-w`／`空格 w`，2026-09-30 定）。
    ///
    /// 照 helix 的 `C-w`（`keymap/default.rs:193`）：`w` 走一步、`hjkl` 按方向
    /// 走、`s` 切一刀、`q` 關、`o` 只留這一個。`e`／`i`／`E`／`I` 是這一頭自己
    /// 加的——helix 沒有邊欄這件東西。
    pub(super) const REGION_KEYS: &'static [(&'static str, &'static str)] = &[
        ("w", "hint.region.next"),
        ("h j k l", "hint.region.go"),
        ("e i", "hint.region.toggle-bar"),
        ("E I", "hint.region.into-bar"),
        ("s", "hint.region.split"),
        ("q", "hint.region.close"),
        ("o", "hint.region.only"),
    ];

    pub(super) const CONFLICT_KEYS: &'static [(&'static str, &'static str)] = &[
        ("o", "hint.conflict.ours"),
        ("t", "hint.conflict.theirs"),
        ("b", "hint.conflict.both"),
    ];

    /// What `t` offers **wherever the cursor is**.
    ///
    /// **The way in comes first.** `t` is a group in every mode (#206), so most
    /// of the time it is pressed by somebody who is *not* in a table yet — and
    /// the menu used to open with 「照這欄順排」 and never once mention `t t`.
    /// These six work anywhere, so they head every list, and on a page with no
    /// table under the cursor they are the whole list.
    const TABLE_KEYS: &'static [(&'static str, &'static str)] = &[
        // **五個一系列，照它們的次序排**（2026-09-30 定：「t 放到 f 后面，
        // q放到 t后面。这五个是一个系列」）。`o b f t` 是同一個問題的四個答案
        // ——這張表畫成什麼樣——由淺入深，`q` 是從最深那一檔退回來。
        ("o", "hint.table.back-to-prose"),
        ("b", "hint.table.operate-it"),
        ("f", "hint.table.draw-it"),
        ("t", "hint.table.whole-window"),
        ("q", "hint.table.leave-the-window"),
        // 2026-09-30：從頂層的 `T` 搬進來。
        ("T", "hint.table.grain"),
        // 摺格子 asks nothing about where the cursor is standing either — it is
        // a preference about how the page is *drawn* — and it had never once
        // been offered by any of the four lists, in the group whose whole
        // purpose is to say what `t` can be finished with.
        ("w", "hint.table.fold-wide-cells"),
        // 折行 stands beside 摺起 because it answers the same question — and
        // a key offered nowhere is a key nobody finds.
        ("a", "hint.table.wrap-wide-cells"),
        ("] [", "hint.table.next-or-previous"),
        // 2026-10-02 定：命令是 `:convert-table`，鍵位摺在這一組底下。
        ("x", "hint.table.convert"),
    ];

    /// `空格 t x` 底下那三個格式。名字直接寫——它們就是命令上打的那三個詞。
    /// `空格 t x` 底下那三種格式。**小寫是轉換，大寫是把剪貼板貼成那一種**
    /// （2026-10-02 定）——和這一組的習慣一致：大寫是「另一個方向」或者
    /// 「重的那一個」（`r R`、`c C`、`d D`、`F`）。
    pub(super) const TABLE_CONVERT_KEYS: &'static [(&'static str, &'static str)] = &[
        ("p P", "hint.table.convert-pipe"),
        ("c C", "hint.table.convert-csv"),
        ("t T", "hint.table.convert-tsv"),
    ];

    /// What `t` adds inside a fenced block. A block is read where it lies
    /// (#216), so the keys that rewrite a file are not offered — because they
    /// are refused.
    const TABLE_KEYS_BLOCK: &'static [(&'static str, &'static str)] = &[
        ("/", "hint.table.search-columns"),
        ("?", "hint.table.search-columns-elsewhere"),
        ("g", "hint.table.go-to-cell"),
        ("y p", "hint.table.yank-or-paste-column"),
        ("1s 1S", "hint.table.sort-by-column"),
    ];

    /// What `t` adds inside a Markdown table.
    const TABLE_KEYS_MD: &'static [(&'static str, &'static str)] = &[
        ("/", "hint.table.search-columns"),
        ("?", "hint.table.search-columns-elsewhere"),
        ("g", "hint.table.go-to-cell"),
        ("r R", "hint.table.add-row"),
        ("c C", "hint.table.add-column"),
        ("d D", "hint.table.delete-row-or-column"),
        ("j k", "hint.table.move-row"),
        ("h l", "hint.table.move-column"),
        ("y p", "hint.table.yank-or-paste-column"),
        ("1s 1S", "hint.table.sort-by-column"),
        ("< = >", "hint.table.align-column"),
        // `F`, not `f` — the lowercase letter is 完整表格 since #283, and a
        // capital is what a command that rewrites every row of the file should
        // have wanted.
        ("F", "hint.table.line-it-up"),
        ("i", "hint.table.detail-panel"),
    ];

    /// What `t` adds in a delimited file, where the columns are the schema's.
    ///
    /// **What this table can actually do**, not what tables can do: `n`/`D`/
    /// `h`/`l` are not offered here because they are refused here, and the menu
    /// that listed them was the one place the editor said a key existed and
    /// then said it did not.
    const TABLE_KEYS_FILE: &'static [(&'static str, &'static str)] = &[
        ("/", "hint.table.search-columns"),
        ("?", "hint.table.search-columns-elsewhere"),
        ("g", "hint.table.go-to-cell"),
        ("1s 1S", "hint.table.sort-by-column"),
        ("r R", "hint.table.add-row"),
        ("d", "hint.table.delete-row"),
        ("j k", "hint.table.move-row"),
        ("y p", "hint.table.yank-or-paste-column"),
        ("H", "hint.table.first-row-is-data"),
        ("e", "hint.table.schema"),
        ("i", "hint.table.detail-panel"),
    ];

    /// A table of key names read out in the reader's language.
    pub(super) fn said(
        rows: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> KeyRows {
        rows.into_iter()
            .map(|(key, what)| (key.into(), crate::messages::say(what, &[])))
            .collect()
    }

    /// 「`空格 t` 後面可以按這些」——**從那幾張真表生出來**。
    ///
    /// Warning: 從前這是三則手抄的文案（`hint.table.after-t`／`block-keys`／
    /// `csv-keys`）。抄的那一份與真表對不上五處：少了 `b`／`w`／`F`，多了一個
    /// 根本不存在的 `n`，`1s 1S` 抄成了 `s S`——而「光按 `t s` 什麽都不會發生」
    /// 正是 `documented_keys.rs` 的 `DISOWNED_KEYS` 記着的那一條
    /// （2026-09-23 審出來的）。按錯鍵的人看見的就這一行，它不能是另一份說法。
    pub(super) fn table_keys_say(inside: Option<Bounds>) -> String {
        let keys: Vec<&str> = Self::table_keys(inside).into_iter().map(|(k, _)| k).collect();
        say!("hint.table.after-t", keys.join(" "))
    }

    /// The keys `t` offers, given what the cursor is standing in.
    pub(super) fn table_keys(inside: Option<Bounds>) -> Vec<(&'static str, &'static str)> {
        let mut keys = Self::TABLE_KEYS.to_vec();
        keys.extend(
            match inside {
                Some(Bounds::Block) => Self::TABLE_KEYS_BLOCK,
                Some(Bounds::Md) => Self::TABLE_KEYS_MD,
                Some(Bounds::WholeFile) => Self::TABLE_KEYS_FILE,
                None => &[],
            }
            .iter()
            .copied(),
        );
        keys
    }

    /// Every key that may follow `leader`, in **any** context.
    ///
    /// The menus themselves stay context-sensitive on purpose — `t` inside a
    /// Markdown table offers what a Markdown table can do, and a delimited file
    /// offers what a schema can — so this does not replace them. It is their
    /// union, and it has one reader: the test that asks whether a key sequence
    /// the documents print is a key sequence the editor has. §5.2.2 found the
    /// same name wrong in two places eight times over; this is the second place
    /// made answerable.
    ///
    /// `None` means **any character follows**: `f`, `r`, `"`, `M`, `'` and the
    /// `mi`/`ma`/`ms`/`mr` pairs take a letter or a bracket the writer chooses,
    /// so there is no list for a document to be checked against.
    pub fn keys_after(leader: char) -> Option<Vec<String>> {
        let spell = |rows: &[(&'static str, &'static str)]| -> Vec<String> {
            rows.iter()
                .flat_map(|(keys, _)| keys.split_whitespace().map(str::to_string))
                .collect()
        };
        Some(match leader {
            // 空 is the first character of how the documents spell it: `空格 f`.
            ' ' | '空' => Self::SPACE_KEYS.iter().map(|(k, _)| k.to_string()).collect(),
            'g' => spell(Self::GOTO_KEYS),
            'm' => spell(Self::MATCH_KEYS),
            '`' => spell(Self::CASE_KEYS),
            ']' | '[' => spell(Self::HOP_KEYS),
            // Warning: **`t` 不在這張表上了。** 2026-09-21 表格組搬到了 `空格 t`，而
            // 這裏一直還答得出表格鍵——於是教程裏那幾行舊拼法（`t r`、`t1s`）
            // 逐條「驗過」，全綠（2026-09-23 審出來的）。`t` 現在只是 vi 的
            // till：一個吃任何字符的鍵，和 `f`／`r`／`"` 一樣，沒有單子。
            _ => return None,
        })
    }

    /// **開命令行**——`:`、`!`、`空格 ?` 都走這一支。
    ///
    /// Warning: 從前是三份抄的，而且抄漏了一行：`:` 不清 `completion`，另外兩個清
    /// （2026-09-23 審出來的）。同一件事兩種收尾，正是這個倉在防的那一族。
    fn open_the_command_line(&mut self, line: &str) {
        self.mode = Mode::Command;
        self.command_line = line.to_string();
        self.command_caret = self.command_line.chars().count();
        self.completion = None;
    }

    /// Run one key of a `Space` sequence.
    fn handle_space(&mut self, key: Key) {
        match key {
            // **`空格 w` 是區域那一組的門**（2026-09-30 定）。從前這一層躺着三
            // 套說同一件事的詞彙——`空格 1234`（點名去）、`空格 !@#$`（關那
            // 一區）、`空格 w/W/q/Q`（走一步／全開／關／只留）。現在一組一套。
            //
            // Warning: **數字空出來了，留給緩衝區**（ 2026-09-30 定）：
            // 「each buffer will be indexed by 1, 2, 3, 4 before the file
            // name……Then users can use space + N to quickly switch between
            // them.」——檔數上十位就補零，所以那是一套不必按空格確認的前綴碼。
            Key::Char('w') => self.pending = Pending::Region,

            // The outline is the sidebar showing the view that has it.
            Key::Char('o') => self.show_sidebar(crate::sidebar::View::Outline),
            // 定義 (#215): the 拆分表 on the character under the cursor. The
            // table detail panel this key used to open is a table key, and now
            // lives in the table group as `t i`.
            // **`空格 d` 只開浮窗**（2026-09-22 定）：看一眼那個字，邊欄一點都
            // 不動。Warning: 再按一次收起來。
            Key::Char('n') => match self.char_at_cursor() {
                Some(ch) => {
                    self.look_up_afloat(ch);
                }
                None => self.set_status(say!("ui.nothing-to-look-up")),
            },
            // **`空格 D` 在邊欄裏開**——同一份答案，鍵跟過去，讀得完長的那些。
            // 大寫是「同一件事的更大版本」，同 `空格 c`／`空格 C`。
            Key::Char('N') => match self.char_at_cursor() {
                Some(ch) => {
                    self.look_up(ch, true);
                }
                None => self.set_status(say!("ui.nothing-to-look-up")),
            },
            // 旁注 (§5.2.3 ②). A page carries one or two, and a reading is
            // typed at leisure — so it is worth a key, and worth a second one.
            // The levels, `auto` and `format` stay `:ruby` commands: those are
            // said once a document, not once a word.
            // **`空格 k`：這是什麽？**（#53 ③，2026-09-21）helix 的 hover 也是這
            // 一個鍵。只在代碼檔上問得出去；稿子上按到就照實說一句，而不是無聲。
            // `空格 k` 浮窗、`空格 K` 進邊欄——與字典那一對同一條規矩
            // （2026-09-22）。
            Key::Char('k') => {
                if !self.ask_what_this_is(true) {
                    self.set_status(say!("lsp.not-code"));
                }
            }
            // `空格 K`：查一次，**一定畫進邊欄**（沒開就開出來）。`空格 k` 看
            // 那一扇開沒開：開着就畫進去，沒開就浮。
            Key::Char('K') => {
                if !self.ask_what_this_is(false) {
                    self.set_status(say!("lsp.not-code"));
                }
            }
            // **診斷搬到 `d`／`D`**（2026-09-30 定，照 helix：`space d` 是
            // diagnostics picker）。同形：小寫浮，大寫進邊欄。
            Key::Char('d') => self.show_the_problem_here(true),
            Key::Char('D') => self.show_the_problem_here(false),
            // **記錄**——`空格 t i` 的別名（同日定）。`t i` 是它的本家（它是表
            // 格的事），這一對是順手。
            Key::Char('i') => self.show_the_record_here(true),
            Key::Char('I') => self.show_the_record_here(false),
            Key::Char('"') => self.open_paste_picker(),
            // 衝突 (#249): the three keys that end one. Under `空格` rather
            // than a letter of its own because every letter has one already,
            // and because a merge conflict is a thing that happens to a file
            // a few times a year — not a motion a writer's fingers know.
            Key::Char('m') => self.pending = Pending::Conflict,
            // 表格組（2026-09-21 從 `t` 搬來，讓 `t`／`T` 回去當 till）。
            Key::Char('t') => self.pending = Pending::Table,
            Key::Char('c') => self.toggle_comment(crate::comment::Prefer::Line),
            Key::Char('C') => self.toggle_comment(crate::comment::Prefer::Block),
            Key::Char('f') => self.open_file_picker(),
            Key::Char('F') => self.open_file_picker_here(),
            Key::Char('b') => self.open_buffer_picker(),
            // **高級搜索** (#419). It used to prefill `:grep `, and `:grep` is
            // gone: what to look for is typed in the panel's own box, which
            // arrives with the last pattern in it, selected.
            Key::Char('/') => self.open_search(),
            // 空的命令行畫的就是命令一覽，所以這一格和 `:` 是同一扇門。
            Key::Char('?') => self.open_the_command_line(""),
            // 工作區 (Feature #176): one key, three meanings that are the same
            // meaning — 「另一個工作區」. Nothing open: open one, showing this
            // same place. Open: hand it the keys. `W`:收掉，留下你站着的這半。
            // **四個動詞，一個名詞**（2026-09-26 定的）。`w` 走一步、`W` 全開、
            // `q` 關這個、`Q` 只留一個工作區；`1`–`4` 是同一套坐標下的點名。

            Key::Char('y') => self.copy_to_clipboard(),
            Key::Char('p') => self.clipboard_paste(true),
            Key::Char('P') => self.clipboard_paste(false),
            _ => {}
        }
    }
}

/// **The key a played character stands for** — the other half of
/// [`yumete_cjk::actions::spell`], which prints these for `:keymap`.
///
/// A key sequence is written as a string (`"gJ"`, `"dw"`), and a chord has no
/// letter of its own, so the table spells it as the control byte it is:
/// `"\u{f}"` is `C-o`, `"\u{1b}"` is Esc. Played as `Key::Char`, those reached
/// the editor as nothing at all — which is why four named actions
/// (`jump_backward`, `collapse_selection`, `increment`, `decrement`) were
/// listed by `:keymap actions`, bindable, and **dead** until 2026-09-19.
fn pressed(c: char) -> Key {
    match c {
        '\u{1b}' => Key::Esc,
        // Warning: **Enter before the control-byte rule**: `\n` is 0x0A, which that
        // rule would read as `C-j`. A keymap that wants Enter writes `\n`, and
        // it means the key a hand presses.
        '\n' | '\r' => Key::Enter,
        c if (c as u32) < 32 => Key::Ctrl((b'a' + c as u8 - 1) as char),
        c => Key::Char(c),
    }
}
