//! The prompt line, and the keys that only mean something in it (#296).
//!
//! `:` commands, `/` search, `f`/`t` 找字 and the 找詞 menu all type into the
//! same one-line field, so its history, its ghost text and its caret live
//! together here rather than beside the mode each of them belongs to.

use super::*;
use yumete_cjk::actions::PlaceCmd;

/// The two commands that walk a prompt's history: any other key leaves it.
const HISTORY: &[PlaceCmd] = &[PlaceCmd::HistoryPrev, PlaceCmd::HistoryNext];

impl Editor {
    // Warning: **The list's arms are not folded into guards**
    // (`collapsible_match`): `Key::Tab if self.take_the_offer()`
    // would be a guard with a side effect, which changes meaning when an arm
    // is put between.
    #[allow(clippy::collapsible_match)]
    pub(super) fn on_insert_key(&mut self, key: Key) {
        // Warning: **`Pending` 的分派只在 Normal 那一支跑**（`on_normal_key`），所以
        // `C-g` 後面那個 `u` 得在這裏吞——放在那邊是吞不到的，稿子裏會留下一個
        // 游離的「u」。
        if self.pending == Pending::UndoBreak {
            self.pending = Pending::None;
            if key == Key::Char('u') {
                return;
            }
        }
        // `C-r` 在等寄存器的名字。Warning: 和 `Pending::UndoBreak` 一樣，這一格
        // 要在這裏吞：`Pending` 的分派只在 Normal 那一支跑。
        if self.pending == Pending::Register {
            self.pending = Pending::None;
            match key {
                // `"` 是無名的那一個，同 vi。
                Key::Char('"') => self.insert_register(None),
                Key::Char(name) => self.insert_register(Some(name)),
                // 別的鍵當成「算了」，不往下走——不然 `C-r` 之後按 `Esc` 會退出插入。
                _ => self.status = say!("edit.register-cancelled"),
            }
            return;
        }
        // **服務器提的那張單子，開着的時候先歸它**（#53 ④）。
        //
        // Warning: **`Tab` 在這裏本來就有主人**（引用補全，#418）：單子開着 `Tab` 是
        // 「就這一條」，没開纔是原來那件事。這是浮出來的單子的通則——它在，它
        // 收鍵；它不在，什麽都没變。
        //
        // Warning: **IME 開着的時候這裏一個鍵都收不到**：編碼串還在 IME 手裏，根本
        // 不進這個函數。模型是「只有當文字上屏才算字符落到屏幕上」——所以候選欄與這張單子不會同時收同一個鍵，空格與 `2390` 一直是
        // IME 的。
        if self.offers_here().is_some() {
            match key {
                Key::Tab => {
                    if self.take_the_offer() {
                        return;
                    }
                }
                Key::Ctrl('n') | Key::Down => {
                    if self.pick_offer(true) {
                        return;
                    }
                }
                Key::Ctrl('p') | Key::Up => {
                    if self.pick_offer(false) {
                        return;
                    }
                }
                // **`C-x` asks again, by hand**, over the list typing opened:
                // that list never holds the pinyin
                // offers, and `C-n` inside it only moves.
                Key::Ctrl('x') => {
                    if self.offer_from_the_text(true) {
                        return;
                    }
                }
                // Warning: **`Esc` is not the list's**: it closes the list and
                // leaves Insert in one press, as helix's popup
                // (`ignore_escape_key`) and vim's menu (`i_<Esc>` ends
                // completion) do. `leave_insert` puts the list away.
                // 別的鍵照舊：打字就該打進去，而打完這張單子就過期了（它記着
                // 問的時候光標在哪）。
                _ => {}
            }
        } else if key == Key::Ctrl('x')
            || (key == Key::Ctrl('n') && (self.vim_keymap() || self.flavor_on(yumete_cjk::FlavorItem::InsertExtras)))
        {
            // **叫它出來。** 稿子裏没有服務器可問，那就什麽都不做——`C-n` 在
            // 這個編輯器裏還没有別的意思，無聲勝過一句用不上的抱怨。
            //
            // `C-x` 是 helix 的拼法（`default.rs` 的插入態 `completion`）：兩個
            // 拼法開同一張單子，插入態裏 `C-x` 本來什麼都不做。
            if self.offer_from_the_text(true) || self.ask_what_comes_next(true) {
                return;
            }
        }
        // Anything but Tab abandons the reference being walked, so the next Tab
        // starts from what is actually in the buffer — the same rule the
        // command line's completion follows (#418).
        if !matches!(key, Key::Tab | Key::BackTab) {
            self.reference = None;
        }
        let continuing_zong = std::mem::take(&mut self.zong_motion);
        if self.layout() == Layout::Vertical {
            match key {
                Key::Left => return self.move_zong_from(true, continuing_zong),
                Key::Right => return self.move_zong_from(false, continuing_zong),
                Key::Up => return self.move_horizontal(motion::left),
                Key::Down => return self.move_horizontal(motion::right),
                _ => {}
            }
        }
        // Inside a grid, Insert mode is scoped to one cell — that is what "edit
        // this cell" means, and the keys that would join two cells into one or
        // split a row in half stay barred. **Walking out is not joining up**
        // (#376): an arrow key moves and changes nothing, so barring it was a
        // rule about editing applied to walking.
        if let Some((start, end)) = self.insert_bounds() {
            match key {
                // Within the cell these move by character, which is how you
                // reach the middle of a 拆分 sequence; at its edge they step
                // into the cell next door — off the end of a row, into the
                // first cell of the next.
                //
                // **By grapheme and not by [`motion::right`]**. That one stops
                // at the end of a *file* line, and a cell can hold a line break
                // of its own: the arrow would stop at the break and the line
                // under it could not be typed in. The
                // cell's own two ends are what `start` and `end` are for.
                Key::Left => {
                    match self.sel.head() > start {
                        true => self.move_horizontal(motion::prev_grapheme),
                        // **The same step Normal's `h` takes at the edge**
                        // (aligned upwards): one function, so the two modes
                        // cannot drift apart — it is also
                        // the one that knows not to walk onto the header.
                        false => {
                            self.step_into_the_next_cell(false);
                        }
                    }
                    return;
                }
                Key::Right => {
                    match self.sel.head() < end {
                        true => self.move_horizontal(motion::next_grapheme),
                        false => {
                            self.step_into_the_next_cell(true);
                        }
                    }
                    return;
                }
                // `C-a`/`C-e` here too (#516): they are bound in Insert on a
                // page of prose and on the `:` line, and a cell is the third
                // place this editor takes a line of typing. Two out of three is
                // a rule nobody can hold. **The cell's own ends**, not the
                // file line's — inside a grid that is what 「行首」 means.
                Key::Home | Key::Ctrl('a') => return self.set_cursor(start),
                Key::End | Key::Ctrl('e') => return self.set_cursor(end),
                // The same step `j` and `k` take from Normal, keeping to the
                // column — one rule for「which cell is above this one」, not
                // two. **Which is why it has to ask the layer**: inside a cell
                // that step walks the cell's own rows first and leaves it only
                // at the last one, and typing does not change what a direction
                // means, or the arrow in Insert would go straight to the next
                // record and the second line of the field could not be
                // reached. Typing from the top layer has no row inside a
                // cell to walk, so there it is the cell below.
                Key::Up | Key::Down => {
                    return match self.table_layer().walled() {
                        true => self.move_vertical(key == Key::Up),
                        false => self.move_cell_row(key == Key::Down),
                    };
                }
                _ => {}
            }
        }
        if self.table_here() {
            match key {
                Key::Char(c) => {
                    // **The three characters the quotes are for**: anything may
                    // be typed in a cell, and the quotes are the editor's. A quoted
                    // field holds the delimiter and a quote as ordinary
                    // characters, so the keystroke that needs one puts the
                    // quotes there — and a `"` typed into the value is two in
                    // the file, which is RFC 4180's own escape.
                    if c == '"' && self.cell_needs_quoting_for(c) && self.quote_the_cell() {
                        // One quote in the value is two in the file — RFC
                        // 4180's own escape, and the one place this editor
                        // writes a character the reader did not type.
                        self.insert_recording.push(c);
                        self.insert_str("\"\"");
                        return;
                    }
                    if self.cell_needs_quoting_for(c) {
                        self.quote_the_cell();
                    }
                    let mut buf = [0u8; 4];
                    let one = c.encode_utf8(&mut buf);
                    if let Some(why) = self.cell_refuses_text_at(Some(self.sel.head()), one) {
                        self.status = why;
                        return;
                    }
                }
                // Tab is what walks a table in every tool that has one, and
                // it is why a table is quick to fill in: you never reach for a
                // pipe. It reflows the row on the way, so the columns stay
                // lined up while you type rather than after you stop.
                Key::Tab | Key::BackTab => {
                    self.format_md_table();
                    self.step_cell(key == Key::Tab, true);
                    return;
                }
                // **<kbd>Enter</kbd> inside a cell makes a line break** — in a delimited file, where a quoted
                // field may hold one. A `|` table cannot: a row of it is one
                // line by construction, and there is no quoting to lean on.
                Key::Enter if self.cell_takes_a_line_break() && self.quote_the_cell() => {
                    self.insert_recording.push('\n');
                    self.insert_str("\n");
                    // **This row is two rows of the file now.** Every row
                    // number the grid draws comes from the record map, and it
                    // was scanned on the way in.
                    self.rescan_records();
                    return;
                }
                Key::Enter => {
                    self.status = say!("table.enter-makes-no-newline");
                    return;
                }
                // At the cell's own start there is nothing of this cell to
                // delete, and the character before it is the delimiter.
                Key::Backspace if self.at_cell_start() => {
                    self.status = say!("table.backspace-would-join-cells");
                    return;
                }
                _ => {}
            }
        }

        // `[^` and `](#` finished from what the file already holds (#418).
        // Asked here, below the grid's own Tab: inside a cell Tab walks to the
        // next one, and that is the older claim on the key.
        if matches!(key, Key::Tab | Key::BackTab) && self.cycle_reference(match key {
            Key::Tab => 1,
            _ => -1,
        }) {
            return;
        }
        // **Insert mode's keys are a table too** (step 6 of the keymap plan): `INSERT_KEYS` says which command each key runs, and
        // the reader's `[keys.insert]` is asked before any of this (`on_key`).
        // A key no command answers types itself.
        if let Some(name) = self.insert_command_for(key) {
            if let Some(&(_, run, _)) = super::normal::normal_command(name) {
                run(self, super::normal::Cx { key, count: 1, operator_count: None, continuing_zong });
            }
            return;
        }
        if let Key::Char(c) = key {
            self.insert_recording.push(c);
            let mut buf = [0u8; 4];
            let one = c.encode_utf8(&mut buf).to_string();
            if !self.type_paired(c) {
                self.insert_str(&one);
            }
            self.maybe_ask_what_comes_next(&one);
            self.maybe_ask_for_a_signature(&one);
        }
    }

    /// **Leave Insert mode** (`Esc`, and `normal_mode` from a binding such as
    /// `jk`).
    pub(super) fn leave_insert(&mut self) {
        self.take_back_fresh_indent();
        // **`5ix` spends its count here**: what repeats is
        // the whole typing session, and nobody knows what that is until
        // now.
        let again = std::mem::take(&mut self.insert_again);
        let typed = std::mem::take(&mut self.insert_recording);
        if again > 0 && !typed.is_empty() {
            // At every cursor, not at the primary alone.
            let opened = self.insert_opened;
            self.edit_each_from(false, |e| {
                e.insert_opened = opened;
                e.spend_the_insert_count(again, &typed);
            });
        }
        // The session just ended is what `.` replays.
        self.insert_recording.clear();
        let stop = self.sel.head();
        self.current_buffer_mut().set_insert_stop(stop);
        self.mode = Mode::Normal;
        let caret = self.sel.head();
        self.step_back_out_of_insert();
        // A block's `I`, `A` or `c`: copy it down now.
        self.finish_block_insert(caret);
        // 那一則簽名是打字當口的東西，出了插入態就不是答案了。
        self.signature = None;
        self.signature_query = None;
        self.signature_due = None;
        self.signature_on = None;
        // A completion question out is about the typing just ended, and so
        // is the list it brought: whichever key left Insert (`Esc`, `C-c`, a
        // `[keys.insert]` run), it does not stand over Normal mode.
        self.offering = None;
        self.completion_at = None;
        self.completion_query = None;
        self.completion_sent = None;
        // **The quotes are the editor's, on the way out too**: a cell typed into leaves the file written the
        // way `quote_for` would have written it, so a pair that is no
        // longer earning its place goes away with the line break or
        // the comma that asked for it.
        self.tidy_the_cell();
        self.rescan_records();
        // **And the caret back onto a character of the cell**.
        // `step_back_out_of_insert` pulls it one grapheme the way vi
        // does, which off the end of a cell lands on the cell's last
        // character — but `a` at the end of a cell leaves it *on the
        // delimiter*, which the status line would read out as 「this
        // cell holds `,`」. The grid's walls are the
        // cell's, so coming to rest is clamped by them.
        self.put_the_caret_in_the_cell();
        // A cell that grew while it was being typed in made its column
        // too narrow for it. Laying the table out again on the way out
        // is what keeps "aligned" a property of the file rather than a
        // command somebody has to remember.
        self.format_md_table();
    }

    /// A line break, carrying the indent, a list's marker or a pair's inside.
    pub(super) fn insert_newline(&mut self) {
        // A list carries itself down, and an empty item ends (#418).
        if self.continue_the_list() {
            return;
        }
        // The file's own line ending, so that one keystroke does not
        // leave a CRLF manuscript with two kinds of line in it (#309).
        let ending = self.current_buffer().ending().to_string();
        let (line, column) = self.caret_in_line();
        let base = self.indent_of_line(line, Some(column));
        if let Some(typed) = self.open_the_pair(&ending, &base) {
            self.insert_recording.push_str(&typed);
            return;
        }
        // **縮進跟着下來**，同 vim 的 `autoindent`、helix 的 `insert_newline`：
        // 不然代碼檔裏每按一次 Enter 光標就回到第 0 列。
        //
        // 規矩與代價都在 [`Editor::indent_of_line`] 上：只帶光標前面那一截
        // 空白，所以一個字符都不是憑空長出來的；行首按下去什麼都不帶。
        let indent = self.indent_to_carry();
        // An autoindent nothing was typed after goes; the next line still
        // gets it.
        self.take_back_fresh_indent();
        // 重放（`3i…`、`.`）是把記下來的字串原樣插進去的，所以縮進要記在
        // 裏面——不記，`.` 重放出來的那幾行就貼着左邊。
        self.insert_recording.push_str(&ending);
        self.insert_recording.push_str(&indent);
        self.insert_str(&format!("{ending}{indent}"));
        self.note_fresh_indent(indent.chars().count(), true);
    }

        // **`C-h` is Backspace** and **`C-j` is Enter** — the two the
        // terminal has sent as those control codes since before vi, and
        // helix binds them in Insert as well (`default.rs`:
        // `C-h` delete_char_backward, `C-j` insert_newline), rather than
        // falling off the end into 「chords are not text」.
    pub(super) fn insert_backspace(&mut self) {
        self.insert_recording.pop();
        // **覆寫模式的退格是「還回去」**，同 vim：往回走一格，把那個位置
        // 上原來的字放回來。蓋到行尾外面去的那幾個没有原文，就只是刪掉。
        if self.overwriting {
            if let Some(was) = self.overwritten.pop() {
                return self.put_back_what_was_overwritten(was);
            }
        }
        let before = self.sel.head();
        if self.delete_an_empty_pair() {
            return;
        }
        if self.vim_keymap() && self.vim_backspace_in_the_indent() {
            return;
        }
        self.delete_before_cursor();
        // The server's list widens again, as helix's does.
        if self.sel.head() < before && self.narrow_the_server_list(before) == Some(true) {
            self.ask_what_comes_next(false);
        }
    }

    /// The line's start and the caret's column when only blanks (spaces,
    /// TABs) are before it.
    fn caret_in_the_indent(&self) -> Option<(usize, usize)> {
        let rope = self.current_buffer().rope();
        let head = self.sel.head();
        let start = rope.line_to_char(rope.char_to_line(head));
        let ts = self.tab_stop();
        let mut cols = 0;
        for c in rope.slice(start..head).chars() {
            cols = match c {
                '\t' => cols + ts - cols % ts,
                ' ' => cols + 1,
                _ => return None,
            };
        }
        Some((start, cols))
    }

    /// **vim's Backspace in the indent goes back to the last multiple of
    /// `sw`** (nvim's 'smarttab', measured): blanks taken off the
    /// end until the column is no further, spaces put back to reach it.
    /// Answers whether it did.
    fn vim_backspace_in_the_indent(&mut self) -> bool {
        let Some((start, cols)) = self.caret_in_the_indent().filter(|&(start, _)| self.sel.head() > start) else {
            return false;
        };
        let (sw, ts) = (self.indent_width(), self.tab_stop());
        let want = (cols - 1) / sw * sw;
        let head = self.sel.head();
        let blanks: Vec<char> = self.current_buffer().rope().slice(start..head).chars().collect();
        let width = |n: usize| blanks[..n].iter().fold(0, |col, c| match c {
            '\t' => col + ts - col % ts,
            _ => col + 1,
        });
        let mut keep = blanks.len() - 1;
        while keep > 0 && width(keep) > want {
            keep -= 1;
        }
        let pad = " ".repeat(want - width(keep));
        if !self.edit_remove(start + keep..head) || (!pad.is_empty() && !self.edit_insert(start + keep, &pad)) {
            return true;
        }
        self.set_cursor(start + keep + pad.len());
        true
    }

    /// Indentation: `Tab` the kind the file indents with (`tab_spaces`),
    /// `Shift-Tab` the other kind.
    pub(super) fn insert_indent(&mut self, the_files: bool) {
        // **vim's Tab in the indent writes the indent afresh** (nvim's
        // 'smarttab', measured): to the next multiple of `sw`, as
        // TABs and spaces when Tab types TABs. After text, or when `sw` is
        // the tab stop, it is a TAB, as below.
        //
        // With Tab typing spaces nvim goes to the next multiple of `ts` after
        // text; this keeps `sw`, as nvim's `softtabstop=-1` does: there is no
        // 'softtabstop' here, and at the factory `ts=8 sw=4` one Tab would
        // jump eight columns.
        if the_files && self.vim_keymap() && !self.tab_spaces() && self.indent_width() != self.tab_stop() {
            if let Some((start, cols)) = self.caret_in_the_indent() {
                let sw = self.indent_width();
                let text = self.vim_indent_text((cols / sw + 1) * sw);
                let head = self.sel.head();
                if self.edit_remove(start..head) && self.edit_insert(start, &text) {
                    self.set_cursor(start + text.chars().count());
                    self.insert_recording.push('\t');
                }
                return;
            }
        }
        let spaces = the_files == self.tab_spaces();
        let text = match spaces {
            // To the next multiple of the indent, not a fixed run: a
            // Tab after `ab` lands on the same column a Tab after
            // nothing would, which is what makes a column line up.
            true => {
                let rope = self.current_buffer().rope();
                let start = rope.line_to_char(rope.char_to_line(self.sel.head()));
                // A TAB already on the line lands on its own stop.
                // Warning: **字簇，不是 `char`**（#422）：`Warning: ` 是兩個 `char`，逐字
                // 加算一格而終端給兩格，於是對齊到這一欄的縮進會少一格。Tab
                // 要單獨認，它的寬度是「到下一個制表位」，不是字形的寬度。
                let ahead: String = rope.slice(start..self.sel.head()).chars().collect();
                let column = yumete_cjk::graphemes(&ahead).fold(0, |col, g| {
                    match g {
                        "\t" => col + self.tab_stop() - col % self.tab_stop(),
                        g => col + yumete_cjk::grapheme_width(g),
                    }
                });
                let width = self.indent_width();
                " ".repeat(width - column % width)
            }
            false => "\t".to_string(),
        };
        self.insert_recording.push_str(&text);
        self.insert_str(&text);
    }

    pub(super) fn on_command_key(&mut self, key: Key) {
        let commands = self.commands_in(yumete_cjk::actions::Place::Prompt, key);
        let runs = |these: &[PlaceCmd]| {
            commands.as_ref().is_some_and(|c| c.iter().any(|one| PlaceCmd::from_name(one).is_some_and(|cmd| these.contains(&cmd))))
        };
        // Anything but ⇥ abandons the completion in progress, so the next one
        // starts from what is actually on the line.
        //
        // Warning: **`C-n`/`C-p` are not in this list**: they walk the
        // history, and a key that walks the history is a key that abandons
        // the completion like any other.
        if !runs(&[PlaceCmd::CompletionNext, PlaceCmd::CompletionPrev]) {
            self.completion = None;
        }
        // Anything but the four history keys leaves the history where it was:
        // walking back to a line and then editing it is editing *that line*,
        // not browsing.
        if !runs(HISTORY) {
            self.history_at = None;
        }
        let Some(commands) = commands else {
            match key {
                // A second `:` on an **empty** line opens the search over what
                // the commands do (#224). Only on an empty one: `:s/:/：/` is a
                // substitution with two colons in it.
                Key::Char(':') if self.command_line.is_empty() => {
                    self.mode = Mode::Lookfor;
                    self.lookfor_focus = 0;
                }
                Key::Char(c) => {
                    self.type_into_prompt(c);
                    // 行變了：照新的樣子先畫出來（`PromptEvent::Update`）。
                    self.ask_for_a_trial();
                }
                _ => {}
            }
            return;
        };
        self.run_in_place(yumete_cjk::actions::Place::Prompt, &commands, |e, name| e.command_line_command(name));
    }

    /// One of a prompt's commands, on the `:` line.
    fn command_line_command(&mut self, name: PlaceCmd) {
        match name {
            // Warning: **⇥ 也要預覽**。這兩個鍵自己一支，走不到下面那個 `other`
            // 分支——而那裏纔是喊預覽的地方。不在這裏喊的話，手打 `:theme mogao`
            // 預覽得了，⇥ 到同一個名字上一點反應都没有，而 ⇥ 正是挑主題最順手的
            // 那條路（名字就在單子上）。
            // Warning: **`C-n`/`C-p` 走歷史，不走這張單子**（照 helix）。
            //
            // 命令行上這一對在 vim 和 helix 裏都歸歷史，而這個編輯器一按冒號就把
            // 補全攤開——接在單子上的話，那一對永遠輪不到歷史。
            //
            // helix 的分法（`helix-term/src/ui/prompt.rs:711`）：
            // `ctrl!('p') | key!(Up)` 和 `ctrl!('n') | key!(Down)` 在**同一個
            // arm** 裏走歷史，`Tab`/`S-Tab` 走補全。所以兩對鍵各管一樣東西，
            // 和命令行上打了什麼無關——「看得見的那張」始終是 ⇥ 走的那張。
            // A shell line's ghost is the last shell command; Tab takes it.
            PlaceCmd::CompletionNext if self.completion.is_none() && self.shell_ghost().is_some() => {
                let rest = self.shell_ghost().unwrap_or_default();
                self.command_line.push_str(&rest);
                self.command_caret = self.command_line.chars().count();
            }
            // Otherwise it walks the programs or files the word can become.
            PlaceCmd::CompletionNext | PlaceCmd::CompletionPrev if command::shell_start(&self.command_line).is_some() => {
                self.cycle_shell(if name == PlaceCmd::CompletionNext { 1 } else { -1 });
            }
            PlaceCmd::CompletionNext => {
                self.cycle_completion(1);
                self.ask_for_a_trial();
            }
            PlaceCmd::CompletionPrev => {
                self.cycle_completion(-1);
                self.ask_for_a_trial();
            }
            PlaceCmd::NormalMode => {
                // 退出去：把預覽放回原樣（helix 的 `PromptEvent::Abort`）。
                self.end_a_trial(crate::editor::Trial::Undo);
                self.leave_visual_for_the_line();
                self.close_prompt();
            }
            PlaceCmd::HistoryPrev | PlaceCmd::HistoryNext => {
                self.walk_history(name == PlaceCmd::HistoryPrev, false);
                // 翻出來的那一行也是一行：`:theme 墨香` 從歷史裏翻上來，照樣先畫。
                self.ask_for_a_trial();
            }
            PlaceCmd::Confirm => {
                // 按下了：預覽成真，記下的舊樣子不必再留（`PromptEvent::Validate`）。
                self.end_a_trial(crate::editor::Trial::Keep);
                let line = std::mem::take(&mut self.command_line);
                self.command_caret = 0;
                self.mode = Mode::Normal;
                // Not what is typed on the settings panel: its
                // `:w` and `:q` are the panel's, and walking back through the
                // editor's history should not step on them.
                if !self.settings_open {
                    remember_line(&mut self.command_history, &line);
                }
                let visual = self.visual_start_for_the_line();
                let before = (self.sel.anchor(), self.sel.head());
                match self.execute(&line) {
                    Ok(CommandOutcome::Quit) => self.quit_from_binding = true,
                    Ok(CommandOutcome::Continue) => {}
                    Err(err) => self.status = err.to_string(),
                }
                if let Some(start) = visual {
                    self.leave_visual_after_the_line(start, before, line.trim().is_empty());
                }
            }
            other => {
                self.edit_prompt(other);
                // 行變了：照新的樣子先畫出來（`PromptEvent::Update`）。
                self.ask_for_a_trial();
            }
        }
    }

    /// **命令行打到一半，照它先畫一次**（照 helix 的三個事件）。
    ///
    /// Warning: **還原歸這裏管，不歸命令管。** 每一條命令自己記一遍「退出去要放回
    /// 什麼」，就是每一條都有機會記錯；記在這一處，`:theme` 之外再加一條只要
    /// 進那張白名單，不必再寫一次還原。
    ///
    /// 名字被退格退没了也要還原——helix 那邊同一句話：「Ensures that a preview
    /// theme gets cleaned up if the user backspaces until the prompt is empty」。
    fn ask_for_a_trial(&mut self) {
        match crate::command::preview_of(&self.command_line) {
            Some(shown) => {
                self.trial = Some(crate::editor::Trial::Show(shown));
                self.previewing = true;
            }
            None => self.end_a_trial(crate::editor::Trial::Undo),
        }
    }

    /// 收場：`Keep` 留着，`Undo` 放回去。没有預覽在場就什麼都不做。
    fn end_a_trial(&mut self, how: crate::editor::Trial) {
        if self.previewing {
            self.previewing = false;
            self.trial = Some(how);
        }
    }

    /// The `::` line: searching the commands by what they **do** (#224).
    ///
    /// Nothing here runs anything. ⇥ — and ⏎, which is the same gesture aimed
    /// at the same row — writes the whole command back into the `:` line and
    /// goes back there with the caret after it, so what Enter finally runs is
    /// always the line the reader can see. `:q!` is not undoable, and a mode
    /// that guessed which command was meant would eventually guess that one.
    pub(super) fn on_lookfor_key(&mut self, key: Key) {
        // A picker over the commands: the picker's keys, then the prompt's.
        let place = yumete_cjk::actions::Place::Picker;
        match self.commands_in(place, key) {
            Some(commands) => self.run_in_place(place, &commands, |e, name| e.lookfor_command(name)),
            None => {
                if let Key::Char(c) = key {
                    self.type_into_prompt(c);
                }
                self.lookfor_focus = 0;
            }
        }
    }

    /// One of a picker's or a prompt's commands, on the `::` line.
    fn lookfor_command(&mut self, name: PlaceCmd) {
        match name {
            PlaceCmd::NormalMode => {
                self.leave_visual_for_the_line();
                self.close_prompt();
            }
            // Backspacing `::` empty goes back to `:`, not out to the page:
            // the second colon is the last thing there was to take back.
            PlaceCmd::DeleteCharBackward if self.command_line.is_empty() => {
                self.mode = Mode::Command;
                self.lookfor_focus = 0;
            }
            // **⇥ 走一格，Enter 纔拿**。
            //
            // Warning: **⇥ 不是「拿走這一條」**：挪高亮只有方向鍵的話，第二條根本
            // 没有辦法挑。`:` 那一行上的 ⇥ 是「走到下一條」，所以相鄰兩行上同一個
            // 鍵是同一種意思。
            //
            // **走到頭繞回來**：這是一張排過序的短名單，不是一份目錄。
            PlaceCmd::NextEntry => self.walk_lookfor(1),
            PlaceCmd::PrevEntry => self.walk_lookfor(-1),
            PlaceCmd::FirstEntry => self.lookfor_focus = 0,
            PlaceCmd::LastEntry => self.lookfor_focus = lookfor::look(&self.command_line).len().saturating_sub(1),
            PlaceCmd::Confirm => self.adopt_lookfor(),
            // Its list is short and has no page; the history and the
            // completion are the `:` line's.
            PlaceCmd::PageDown | PlaceCmd::PageUp | PlaceCmd::CycleHiddenFiles | PlaceCmd::HistoryPrev | PlaceCmd::HistoryNext | PlaceCmd::CompletionNext
            | PlaceCmd::CompletionPrev | PlaceCmd::ToggleWideSearch => {}
            other => {
                self.edit_prompt(other);
                self.lookfor_focus = 0;
            }
        }
    }

    /// 單子上走一格，走到頭繞回去。
    fn walk_lookfor(&mut self, by: isize) {
        let found = lookfor::look(&self.command_line).len();
        if found == 0 {
            return;
        }
        let at = self.lookfor_focus.min(found - 1) as isize;
        self.lookfor_focus = (at + by).rem_euclid(found as isize) as usize;
    }

    /// Take the highlighted row back to the `:` line, whole.
    fn adopt_lookfor(&mut self) {
        let (found, focus) = self.lookfor_menu();
        let Some(hit) = found.get(focus) else {
            // Nothing found, so there is nothing to take. Saying so beats
            // dropping the reader onto an empty `:` line that looks as if the
            // search had been thrown away.
            self.status = say!("lookfor.nothing-to-take");
            return;
        };
        // **Enter 就做掉挑中的那一條**。⇥ 和 ⇧⇥ 在單子上走，Enter 收尾——三個鍵各做一件事。
        //
        // 鍵就按下去，命令就跑，不是「把命令寫到 `:` 那一行上」再按一次 Enter：
        // 單子上那一行本來就看得見，兩次 Enter 是一次多餘的。
        let what = hit.what.clone();
        self.lookfor_focus = 0;
        self.close_prompt();
        match what {
            // A command's answer is heard, as a binding's is: `save_and_quit` quits, a failure says why.
            lookfor::What::Keys(action) => match action.how {
                yumete_cjk::actions::How::Command(line) => self.run_bound_command(line.trim_start_matches(':')),
                yumete_cjk::actions::How::Normal => {
                    self.run_normal(action.name);
                }
            },
            lookfor::What::Command(choice) => {
                let line = choice.written();
                self.run_bound_command(line.trim_start_matches(':'));
            }
        }
    }

    /// What the `::` line has turned up, and which row is highlighted.
    ///
    /// Worked out afresh from the line rather than kept: 221 rows scored
    /// against a few characters is microseconds, and a cached list is a list
    /// that can disagree with what is on the prompt.
    pub fn lookfor_menu(&self) -> (Vec<lookfor::Hit>, usize) {
        let found = lookfor::look(&self.command_line);
        let focus = self.lookfor_focus.min(found.len().saturating_sub(1));
        (found, focus)
    }

    /// **Where vim's cursor goes when `:` leaves Visual** (`:h v_:`,
    /// measured in nvim): the start of the Visual area, which
    /// under `V` is the first line's start unless the cursor is the line
    /// above. `None` outside vim's Visual mode.
    fn visual_start_for_the_line(&self) -> Option<usize> {
        if !self.vim_keymap() || !self.extend {
            return None;
        }
        let (anchor, head) = (self.sel.anchor(), self.sel.head());
        if !self.vim_lines {
            return Some(anchor.min(head));
        }
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();
        let (top, cursor) = (rope.char_to_line(anchor.min(len)), rope.char_to_line(head.min(len)));
        Some(match cursor < top {
            true => self.column_on_line(cursor, self.vim_line_column),
            false => rope.line_to_char(top),
        })
    }

    /// vim's Visual ends when its `:` line does: the command has taken the
    /// selected lines, and the cursor stays where the command put it, else
    /// at the start of the Visual area. An empty line goes to the last line
    /// of the range (vim's `:'<,'>`), keeping the column.
    fn leave_visual_after_the_line(&mut self, start: usize, before: (usize, usize), empty: bool) {
        if !self.extend {
            return;
        }
        let last = {
            let rope = self.current_buffer().rope();
            let (_, end) = self.selection();
            rope.char_to_line(end.saturating_sub(1).min(rope.len_chars()))
        };
        let at = match (self.sel.anchor(), self.sel.head()) == before {
            true => start,
            false => self.sel.head(),
        };
        self.extend = false;
        self.vim_lines = false;
        self.sel.set_anchor(at);
        self.sel.set_head(at);
        self.clamp_cursor();
        self.refresh_goal_column();
        if empty {
            let want = self.column_wanted();
            self.land_at_column(last, want);
        }
    }

    /// The `:` line given up from vim's Visual mode ends Visual too, the
    /// cursor at the start of the Visual area.
    fn leave_visual_for_the_line(&mut self) {
        if let Some(start) = self.visual_start_for_the_line() {
            let before = (self.sel.anchor(), self.sel.head());
            self.leave_visual_after_the_line(start, before, false);
        }
    }

    /// Shut the prompt and forget what was on it.
    fn close_prompt(&mut self) {
        self.command_line.clear();
        self.command_caret = 0;
        self.history_at = None;
        self.lookfor_focus = 0;
        self.mode = Mode::Normal;
    }

    /// The commands that edit a prompt rather than submit or cancel it.
    ///
    /// One set for `:` and `/` both: a search pattern is as long and as easy to
    /// mistype as a command, and the same fingers type them.
    pub(super) fn edit_prompt(&mut self, name: PlaceCmd) {
        let len = self.command_line.chars().count();
        // **The caret stays after the `!` that `Shell:` stands for**: before
        // it, unseen, a letter typed there would turn the shell line into a
        // command.
        let floor = self.prompt_hidden().min(len);
        self.command_caret = self.command_caret.clamp(floor, len);
        let byte = |line: &str, at: usize| -> usize {
            line.char_indices().nth(at).map(|(i, _)| i).unwrap_or(line.len())
        };
        match name {
            // Forward delete on the prompt: the character *under* the caret,
            // and the caret stays where it is.
            PlaceCmd::DeleteCharForward => {
                if self.command_caret < len {
                    let from = byte(&self.command_line, self.command_caret);
                    let to = byte(&self.command_line, self.command_caret + 1);
                    self.command_line.replace_range(from..to, "");
                }
            }
            PlaceCmd::DeleteCharBackward => {
                if self.command_caret == 0 {
                    // Backspacing past the start leaves the prompt: the line is
                    // the only thing there was to go back over.
                    if len == 0 {
                        if self.mode == Mode::Command {
                            self.leave_visual_for_the_line();
                        }
                        self.close_prompt();
                    }
                    return;
                }
                let from = byte(&self.command_line, self.command_caret - 1);
                let to = byte(&self.command_line, self.command_caret);
                self.command_line.replace_range(from..to, "");
                self.command_caret -= 1;
            }
            PlaceCmd::MoveCharLeft => self.command_caret = self.command_caret.saturating_sub(1).max(floor),
            PlaceCmd::MoveCharRight => self.command_caret = (self.command_caret + 1).min(len),
            PlaceCmd::GotoLineStart => self.command_caret = floor,
            PlaceCmd::GotoLineEnd => self.command_caret = len,
            // The same two keys Insert has, and every terminal prompt.
            PlaceCmd::DeleteWordBackward => {
                let head: String = self.command_line.chars().take(self.command_caret).collect();
                let cut = word_cut(&head).max(floor);
                let keep: String = head.chars().take(cut).collect();
                let tail: String = self.command_line.chars().skip(self.command_caret).collect();
                self.command_caret = keep.chars().count();
                self.command_line = format!("{keep}{tail}");
            }
            PlaceCmd::KillToLineStart => {
                let head = self.command_line.chars().take(floor);
                let tail = self.command_line.chars().skip(self.command_caret);
                self.command_line = head.chain(tail).collect();
                self.command_caret = floor;
            }
            _ => {}
        }
    }

    /// A character typed into a prompt, at the caret.
    pub(super) fn type_into_prompt(&mut self, c: char) {
        let len = self.command_line.chars().count();
        let floor = self.prompt_hidden().min(len);
        self.command_caret = self.command_caret.clamp(floor, len);
        let at = self.command_line.char_indices().nth(self.command_caret).map_or(self.command_line.len(), |(i, _)| i);
        self.command_line.insert(at, c);
        self.command_caret += 1;
    }

    /// Walk through what has been typed at this prompt before, **in either
    /// direction and from either end**.
    ///
    /// `history` runs oldest first. Back is older, forward is newer, and from a
    /// prompt nobody has walked yet the two keys start at opposite ends: back
    /// at the newest, forward at the oldest. So one key reaches the whole list
    /// going one way and the other reaches it going the other way, as helix
    /// cycles both ways.
    ///
    /// This is helix's `Prompt::change_history` exactly (`helix-term/src/ui/
    /// prompt.rs:341`) — one index, clamped at both ends:
    ///
    /// ```text
    /// Forward  => history_pos.map_or(0, |i| i + 1)
    /// Backward => history_pos.unwrap_or(len).saturating_sub(1)
    /// ```
    ///
    /// Warning: **It clamps, it does not come back empty.** vim clears the line
    /// walking forward past the newest (`c_<Down>` restores the line you were
    /// typing). The two references disagree here and helix is the one followed,
    /// so the newest line is the end of the road — `C-u` is the way back to an
    /// empty prompt.
    ///
    /// **A shell line walks only shell lines**, as helix's shell
    /// prompts keep a history of their own: with `!` (or `pipe `…) typed, the
    /// other commands are passed over.
    fn walk_history(&mut self, back: bool, search: bool) {
        // Decided where the walk starts, not at every step: a walk over all
        // the commands that lands on a shell line goes on over all of them.
        if self.history_at.is_none() {
            self.history_shell = !search && command::shell_start(&self.command_line).is_some();
        }
        let shell = self.history_shell;
        let history = match search {
            true => &self.search_history,
            false => &self.command_history,
        };
        let fits: Vec<usize> = (0..history.len())
            .filter(|&i| !shell || command::shell_start(&history[i]).is_some())
            .collect();
        let (Some(&first), Some(&last)) = (fits.first(), fits.last()) else {
            return;
        };
        let at = match (back, self.history_at) {
            (true, None) => last,
            (true, Some(h)) => fits.iter().rev().copied().find(|&i| i < h).unwrap_or(first),
            (false, None) => first,
            (false, Some(h)) => fits.iter().copied().find(|&i| i > h).unwrap_or(last),
        };
        self.history_at = Some(at);
        // **A shell walk keeps the verb it started on**: `:!` + C-p must not
        // bring back `pipe sort`, which edits the buffer. helix's
        // shell prompts share one register holding the command alone.
        let verb_here = command::shell_start(&self.command_line).map(|v| self.command_line[..v].to_string());
        self.command_line = match (shell, verb_here, command::shell_start(&history[at])) {
            (true, Some(verb), Some(from)) => format!("{verb}{}", history[at][from..].trim_start()),
            _ => history[at].clone(),
        };
        self.command_caret = self.command_line.chars().count();
    }

    pub(super) fn on_search_key(&mut self, key: Key) {
        let place = yumete_cjk::actions::Place::Prompt;
        let commands = self.commands_in(place, key);
        let walks_history = |c: &Vec<String>| c.iter().any(|one| PlaceCmd::from_name(one).is_some_and(|cmd| HISTORY.contains(&cmd)));
        if !commands.as_ref().is_some_and(walks_history) {
            self.history_at = None;
        }
        match commands {
            Some(commands) => self.run_in_place(place, &commands, |e, name| e.search_line_command(name)),
            None => {
                if let Key::Char(c) = key {
                    self.type_into_prompt(c);
                }
            }
        }
    }

    /// One of a prompt's commands, on the `/` line.
    fn search_line_command(&mut self, name: PlaceCmd) {
        match name {
            // Warning: 退出去的時候也要把那一格放掉，不然下一次按 `/` 會做上一次那件事。
            PlaceCmd::NormalMode => {
                self.sift = None;
                self.close_prompt();
            }
            // Tab takes the rest of the last pattern, so searching for the same
            // thing again is a keystroke rather than retyping it.
            PlaceCmd::CompletionNext => self.adopt_ghost(),
            // **`A-j`：繁簡拼音通配**。`/` 收不了旗標（`:s` 的
            // `y`／`j` 是那一頭的辦法），所以這一行用一個鍵。兩樣一起翻：這一行
            // 分不開也不必分，要分得開就開那扇面板。
            PlaceCmd::ToggleWideSearch => self.search.wild = !self.search.wild,
            // **`C-p`/`C-n` 也走歷史**，同命令行那一扇。helix 的
            // `/` 和 `:` 共用一支 prompt，所以那一對在兩扇裏是同一件事；這裏兩
            // 扇各一支，所以兩處都要寫——少一處就是「一扇認一扇不認」。
            PlaceCmd::HistoryPrev | PlaceCmd::HistoryNext => self.walk_history(name == PlaceCmd::HistoryPrev, true),
            PlaceCmd::Confirm => {
                let pattern = std::mem::take(&mut self.command_line);
                self.command_caret = 0;
                self.mode = Mode::Normal;
                remember_line(&mut self.search_history, &pattern);
                if !pattern.is_empty() {
                    self.last_search = pattern;
                }
                // **正則那一族借這一扇**（#405 Phase 2）：同一條規則，做的是另一件事。
                if let Some(what) = self.sift {
                    self.search.query = self.last_search.clone();
                    // Still set while the pattern is read: `regex_on` asks it.
                    self.sift(what);
                    self.sift = None;
                    return;
                }
                let forward = self.search_forward;
                self.remember_jump();
                self.repeat_search(forward);
            }
            other => self.edit_prompt(other),
        }
    }

    /// **Open the command row for a value a front end asks for**:
    /// the settings panel's one-line editor and its search. The same line as
    /// `:` — its keys, the IME, a paste at the caret — with `label` before
    /// it; Enter and Esc hand back ([`Editor::take_value_answer`]).
    pub fn open_value_line(&mut self, label: &str, text: &str) {
        self.open_line_for_a_value(label, text, false);
    }

    /// **The command row for a command a front end takes**, so the settings
    /// panel's `:` line does not compose `q` with the IME:
    /// [`Editor::open_value_line`], but keys go in as they are, as on the
    /// editor's own `:` line.
    pub fn open_value_command_line(&mut self, label: &str, text: &str) {
        self.open_line_for_a_value(label, text, true);
    }

    /// Whether the open value line takes keys as they are, the IME out of it.
    pub fn value_line_is_plain(&self) -> bool {
        self.mode == Mode::Value && self.value_plain
    }

    fn open_line_for_a_value(&mut self, label: &str, text: &str, plain: bool) {
        self.value_plain = plain;
        self.value_label = label.to_string();
        self.command_line = text.to_string();
        self.command_caret = self.command_line.chars().count();
        self.value_answer = None;
        self.completion = None;
        self.mode = Mode::Value;
    }

    /// What was typed on the value line: `Some(text)` after Enter, `None`
    /// after Esc; nothing until one of them.
    pub fn take_value_answer(&mut self) -> Option<Option<String>> {
        self.value_answer.take()
    }

    /// The text on an open value line, as typed so far.
    pub fn value_typed(&self) -> Option<&str> {
        (self.mode == Mode::Value).then_some(self.command_line.as_str())
    }

    /// Close the value line without an answer, when what it was for goes.
    pub fn close_value_line(&mut self) {
        if self.mode == Mode::Value {
            self.command_line.clear();
            self.command_caret = 0;
            self.mode = Mode::Normal;
        }
    }

    /// `text` into the prompt at its caret.
    pub(super) fn insert_committed_now_in_prompt(&mut self, text: &str) {
        let len = self.command_line.chars().count();
        self.command_caret = self.command_caret.min(len);
        let at = self
            .command_line
            .char_indices()
            .nth(self.command_caret)
            .map_or(self.command_line.len(), |(i, _)| i);
        self.command_line.insert_str(at, text);
        self.command_caret += text.chars().count();
    }

    pub(super) fn on_value_key(&mut self, key: Key) {
        let place = yumete_cjk::actions::Place::Prompt;
        match self.commands_in(place, key) {
            Some(commands) => self.run_in_place(place, &commands, |e, name| e.value_command(name)),
            None => {
                if let Key::Char(c) = key {
                    self.type_into_prompt(c);
                }
            }
        }
    }

    /// One of a prompt's commands, on the value line.
    fn value_command(&mut self, name: PlaceCmd) {
        match name {
            PlaceCmd::NormalMode => {
                self.close_value_line();
                self.value_answer = Some(None);
            }
            // An empty value is one to hand back (the check says whether it
            // may be), so backspacing to empty does not leave; Esc does.
            PlaceCmd::DeleteCharBackward if self.command_line.is_empty() => {}
            PlaceCmd::Confirm => {
                let text = std::mem::take(&mut self.command_line);
                self.command_caret = 0;
                self.mode = Mode::Normal;
                self.value_answer = Some(Some(text));
            }
            other => self.edit_prompt(other),
        }
    }
}

/// **Where `C-w` cuts back to** in `head`, the text before the caret: the
/// last word and the spaces after it, counted in characters.
///
/// Counted in **characters**, not bytes: a full-width space — which a Chinese
/// writer types without thinking about it — is three bytes, and `byte index
/// + 1` lands inside it.
pub(crate) fn word_cut(head: &str) -> usize {
    let kept = head.trim_end();
    kept.char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| kept[..i + c.len_utf8()].chars().count())
}
