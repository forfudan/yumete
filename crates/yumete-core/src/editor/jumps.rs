//! Marks and the jump list (#45).

use super::*;

impl Editor {
    // ---- Marks (Feature #45) ----------------------------------------------

    /// Name this place by a letter (`M a`).
    ///
    /// The jump list remembers where you *came from*; a mark remembers where
    /// you meant to come back **to** — the scene you are rewriting, the note
    /// at the end of the file, the chapter you keep checking against. `M` and
    /// `'` rather than vi's `m` and `'`, because `m` here opens match mode.
    pub(super) fn set_mark(&mut self, name: char) {
        if !name.is_alphanumeric() {
            self.status = say!("goto.mark-name-one-character");
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        let spot = match self.current_buffer().path() {
            Some(path) => Spot::InFile(path.to_path_buf(), line, self.sel.head() - rope.line_to_char(line)),
            None => Spot::InBuffer(self.current_buffer().id(), self.sel.head()),
        };
        self.marks.insert(name, spot);
        self.status = say!("goto.mark-set", name, self.current_buffer().display_name(), line + 1);
    }

    /// **Where mark `name` stands in this buffer**, if it was set here: the
    /// end of a vim operator's `'a` and `` `a ``.
    pub(super) fn mark_here(&self, name: char) -> Option<usize> {
        let rope = self.current_buffer().rope();
        match self.marks.get(&name)? {
            Spot::InFile(path, line, column) if self.current_buffer().path() == Some(path.as_path()) => {
                let line = (*line).min(rope.len_lines().saturating_sub(1));
                let width = crate::motion::line_text(rope, line).chars().count();
                Some(rope.line_to_char(line) + (*column).min(width))
            }
            Spot::InBuffer(id, pos) if *id == self.current_buffer().id() => Some((*pos).min(rope.len_chars())),
            _ => None,
        }
    }

    /// Go back to the place a letter names (`' a`): its line, or with
    /// `exact` the very place (vim's `` ` ``).
    pub(super) fn go_to_mark(&mut self, name: char, exact: bool) {
        // `''` and ``` `` ```: where the last jump came from, and pressed
        // again, back (vim's).
        if matches!(name, '\'' | '`') {
            let Some((index, pos)) =
                self.jump_at.checked_sub(1).and_then(|at| self.jumps.get(at)).and_then(|jump| self.jump_place(jump))
            else {
                self.status = say!("goto.no-such-mark", name);
                return;
            };
            let id = self.buffers[index].id();
            self.remember_jump();
            let Some(i) = self.buffer_with(id) else {
                self.status = say!("goto.mark-buffer-closed", name);
                return;
            };
            self.show_buffer(i);
            let rope = self.current_buffer().rope();
            let pos = pos.min(rope.len_chars());
            // `''` goes to the line's first non-blank, ``` `` ``` to the very
            // spot (`index.txt`: `''` "cursor to the first CHAR of the line
            // where the cursor was before the latest jump").
            let pos = match name {
                '\'' => {
                    let start = rope.line_to_char(rope.char_to_line(pos));
                    let line = crate::motion::line_text(rope, rope.char_to_line(pos));
                    start + line.chars().take_while(|c| matches!(c, ' ' | '\t' | '\u{3000}')).count()
                }
                _ => pos,
            };
            self.set_cursor(pos);
            return;
        }
        let Some(spot) = self.marks.get(&name).cloned() else {
            self.status = say!("goto.no-such-mark", name);
            return;
        };
        self.remember_jump();
        match spot {
            Spot::InFile(path, line, _) => {
                if self.current_buffer().path() != Some(path.as_path()) {
                    if let Err(err) = self.open_file(&path) {
                        self.status = say!("buffer.cannot-open", path.display(), err);
                        return;
                    }
                }
                self.move_to_line(line + 1);
                if let Some(at) = self.mark_here(name).filter(|_| exact) {
                    self.set_cursor(at);
                }
                self.status = say!("goto.mark-in-another-file", name, self.current_buffer().display_name(), line + 1);
            }
            Spot::InBuffer(id, pos) => {
                let Some(i) = self.buffer_with(id) else {
                    self.status = say!("goto.mark-buffer-closed", name);
                    return;
                };
                self.show_buffer(i);
                self.set_cursor(pos.min(self.current_buffer().rope().len_chars()));
                self.status = say!("goto.mark-in-this-file", name);
            }
        }
    }

    // ---- The jump list (Feature #45) ---------------------------------------

    /// Note where the cursor is, before a jump takes it somewhere far.
    ///
    /// Every far motion in this editor goes through `goto_line` — `gg`, `G`,
    /// `:1200`, `:toc`, a heading in the outline, a `#include` followed with
    /// `gf`, a component followed with `Enter` in a table — so one call here
    /// gives all of them a way back. `C-o` walks back through them, `C-i`
    /// forward again, as they do in vi and in Helix.
    ///
    /// Before this, a table's `Enter` was a one-way door: following 螭 → 虫
    /// and coming back meant remembering 螭 and searching for it. The footnote
    /// panel had its own private way back; this is that idea, generalised.
    pub(super) fn remember_jump(&mut self) {
        // **This move was a jump**, which is what the page needs to know to
        // decide where to put the cursor: a jump lands in the middle, because
        // a search hit `scrolloff` from an edge shows nothing on one side of
        // the thing that was looked for. Cleared on the next key, so it
        // describes the move that just happened and nothing after it.
        self.jumped = true;
        self.note_where_we_came_from();
    }

    /// **helix's page centres this place, on the page or off it**
    /// ([`crate::editor::Centring::At`]): a definition, a picker's place, a
    /// file opened at a line.
    pub(super) fn centre_here(&mut self) {
        self.centring = Some(crate::editor::Centring::At(self.sel.head()));
    }

    /// **helix's page centres this place if it is off the page**
    /// ([`crate::editor::Centring::OffPage`]): a search hit, the jump list.
    /// yumete's own ways to a place in another file or another part of
    /// this one (`gf` on `file:12`, the outline) do the same.
    pub(super) fn centre_off_the_page(&mut self) {
        self.centring = Some(crate::editor::Centring::OffPage);
    }

    /// **記一筆，可是別把版面挪動** —— 落腳點本來就在屏幕上的那幾種跳法。
    ///
    /// Warning: **「記進跳轉表」和「落在正中」是兩件事**。
    ///
    /// 居中那一條是給**跳到看不見的地方**的：`n`、`gd`、`:search` 的結果、wiki 條目
    /// ——你落在一個没見過的地方，貼着邊就只看得見命中的一側。
    ///
    /// `gw`/`gx`/`gz` 正相反：**那個地方本來就在屏幕上，你正盯着它。** 居中等於
    /// 把你盯着的那一行挪走——而「版面不動」正是這一族功能成立的前提（按之前眼睛
    /// 已經鎖定了要去的地方）。
    ///
    /// helix 也不挪：`jump_to_label` 跳完只有一句 `doc.set_selection(...)`，没有
    /// `align_view`（`helix-term/src/commands.rs`）。
    ///
    /// `C-o` 照樣回得來——那一半兩種跳法都要。
    pub(super) fn note_where_we_came_from(&mut self) {
        // The place a buffer switch just opened at is where the switch put
        // the cursor, not a place anyone jumped from. A preview has no place
        // but itself, and `C-o` comes back to it.
        if self.just_switched && !self.buffers[self.current].is_preview() {
            return;
        }
        let here = self.jump_here();
        let vim = self.keymap_rules().vim_jump_list;
        let line_of = |e: &Editor, jump: &Jump| {
            e.jump_place(jump).map(|(_, at)| e.current_line_of(jump.buffer, at))
        };
        match vim {
            // vim keeps a line once: the older entry for it goes (`:h jumplist`).
            true => {
                let line = line_of(self, &here);
                let mut i = 0;
                while i < self.jumps.len() {
                    if self.jumps[i].buffer == here.buffer && line_of(self, &self.jumps[i]) == line {
                        self.jumps.remove(i);
                        if self.jump_at > i {
                            self.jump_at -= 1;
                        }
                    } else {
                        i += 1;
                    }
                }
            }
            // helix drops a repeat of the last one, and what was ahead.
            false => {
                if self.jumps.last().is_some_and(|last| self.same_place(last, &here)) {
                    return;
                }
                self.jumps.truncate(self.jump_at);
            }
        }
        self.jumps.push(here);
        // Bounded: a session of a thousand jumps does not need a thousandth of
        // them, and the oldest is the one nobody comes back to.
        if self.jumps.len() > JUMPS {
            self.jumps.remove(0);
        }
        self.jump_at = self.jumps.len();
    }

    /// This place, as a jump list keeps it.
    fn jump_here(&self) -> Jump {
        let buffer = self.current_buffer();
        Jump {
            buffer: buffer.id(),
            path: buffer.path().map(Path::to_path_buf),
            revision: buffer.revision(),
            ranges: self.sel.iter().map(|one| (one.anchor, one.head)).collect(),
            primary: self.sel.primary_index(),
        }
    }

    /// Where a jump's buffer is, and its primary head carried to now; `None`
    /// once the buffer is closed.
    pub(super) fn jump_place(&self, jump: &Jump) -> Option<(usize, usize)> {
        let index = self.buffer_with(jump.buffer)?;
        let head = jump.ranges.get(jump.primary).map_or(0, |r| r.1);
        Some((index, self.buffers[index].carried(jump.revision, head)))
    }

    fn current_line_of(&self, buffer: u64, at: usize) -> usize {
        self.buffer_with(buffer).map_or(0, |i| {
            let rope = self.buffers[i].rope();
            rope.char_to_line(at.min(rope.len_chars()))
        })
    }

    fn same_place(&self, a: &Jump, b: &Jump) -> bool {
        a.buffer == b.buffer && self.jump_place(a) == self.jump_place(b)
    }

    /// Go back to where a jump came from (`C-o`), or forward again (`C-i`).
    /// **回到這一份稿子最後改動的地方**（`g.`，helix 的 `goto_last_modification`）。
    ///
    /// Warning: **跳轉表答不了這個問題。** 那張表只在**遠距離移動**的時候記一格
    /// （[`Self::remember_jump`]），而「我剛纔改到哪」是寫東西的人一天問一百次的事：
    /// 翻回去查一個名字、看一眼前一章，然後要回到筆停下的地方。`C-o` 只在你「跳」過的
    /// 時候有答案，光是往上滾幾頁再想回來，它一格都没記。
    ///
    /// Warning: **每一份稿子各記各的**（欄位在 `Buffer` 上），所以換了檔再按 `g.`，回的是那
    /// 一份自己最後改的地方，不是上一份的。
    pub(super) fn goto_last_modification(&mut self) {
        let Some(at) = self.current_buffer().last_edit() else {
            self.status = say!("goto.nothing-changed-yet");
            return;
        };
        // 改完之後又撤銷、又刪掉別的，那個下標可能已經在檔尾外面了。
        let at = at.min(self.current_buffer().rope().len_chars());
        self.remember_jump();
        self.sel.collapse_to(at);
        self.clamp_cursor();
        self.refresh_goal_column();
    }

    /// **走改動表**（vim 的 `g;` 往舊、`g,` 往新）。
    ///
    /// 和 `g.` 是兩件事：那一個回**最後**改的地方，這一對是在改過的地方之間走。
    /// 一行只記一條（見 `Buffer::note_change`），所以打完一段話按 `g;` 是挪一段，
    /// 不是挪一個字。
    pub(super) fn walk_changes(&mut self, back: bool) {
        let list = self.current_buffer().changes().to_vec();
        if list.is_empty() {
            self.status = say!("goto.nothing-changed-yet");
            return;
        }
        let at = self.current_buffer().changes_at();
        let next = match back {
            true => at.saturating_sub(1),
            false => (at + 1).min(list.len() - 1),
        };
        if next == at && !back {
            self.status = say!("goto.newest-change");
            return;
        }
        self.current_buffer_mut().set_changes_at(next);
        let to = list[next].min(self.current_buffer().rope().len_chars());
        self.remember_jump();
        self.sel.collapse_to(to);
        self.clamp_cursor();
        self.refresh_goal_column();
    }

    pub(super) fn walk_jumps(&mut self, back: bool, count: usize) {
        let count = count.max(1);
        loop {
            let target = match back {
                true => {
                    // Stepping back from the newest place notes where we are,
                    // or `C-i` would have nowhere to come back to.
                    if self.jump_at == self.jumps.len() {
                        let here = self.jump_here();
                        if !self.jumps.last().is_some_and(|last| self.same_place(last, &here)) {
                            self.jumps.push(here);
                        }
                        self.jump_at = self.jumps.len();
                    }
                    let Some(mut target) = self.jump_at.checked_sub(count) else {
                        self.status = say!("goto.no-earlier-jump");
                        return;
                    };
                    // Never a step to where we already are (helix's
                    // `JumpList::backward`): `C-s` then `C-o` goes back.
                    if self.same_place(&self.jumps[target], &self.jump_here()) {
                        let Some(earlier) = target.checked_sub(1) else {
                            self.status = say!("goto.no-earlier-jump");
                            return;
                        };
                        target = earlier;
                    }
                    target
                }
                false => {
                    let target = self.jump_at + count;
                    if target >= self.jumps.len() {
                        self.status = say!("goto.no-later-jump");
                        return;
                    }
                    target
                }
            };
            let jump = self.jumps[target].clone();
            if self.buffer_with(jump.buffer).is_none() {
                // A closed file: vim opens it again by its name; helix forgets
                // its places. A buffer that never had a file can only be forgotten.
                match (self.keymap_rules().vim_jump_list, jump.path.clone()) {
                    (true, Some(path)) => {
                        self.walking_jumps = true;
                        let opened = self.open_file(&path);
                        self.walking_jumps = false;
                        if let Err(err) = opened {
                            self.status = say!("buffer.cannot-open", path.display(), err);
                            return;
                        }
                        let id = self.current_buffer().id();
                        let revision = self.current_buffer().revision();
                        for one in self.jumps.iter_mut().filter(|j| j.path.as_ref() == Some(&path)) {
                            one.buffer = id;
                            one.revision = revision;
                        }
                    }
                    _ => {
                        let gone = jump.buffer;
                        let behind = self.jumps[..self.jump_at.min(self.jumps.len())].iter().filter(|j| j.buffer == gone).count();
                        self.jumps.retain(|j| j.buffer != gone);
                        self.jump_at -= behind;
                        continue;
                    }
                }
            }
            self.jump_at = target;
            self.go_to_jump(&self.jumps[target].clone());
            // helix centres a place off the page (`Editor::jump_to`).
            self.centre_off_the_page();
            self.status = say!("goto.jump-list-position", self.jump_at + 1, self.jumps.len());
            return;
        }
    }

    /// Put the selection where a jump says: the whole of it under helix,
    /// the cursor under vim.
    fn go_to_jump(&mut self, jump: &Jump) {
        let Some(index) = self.buffer_with(jump.buffer) else { return };
        if index != self.current {
            self.walking_jumps = true;
            self.show_buffer(index);
            self.walking_jumps = false;
        }
        let buffer = &self.buffers[index];
        let carry = |at: usize| buffer.carried(jump.revision, at);
        match self.keymap_rules().vim_jump_list {
            true => {
                let head = jump.ranges.get(jump.primary).map_or(0, |r| carry(r.1));
                self.move_head(head);
            }
            false => {
                let ranges: Vec<crate::selection::Range> =
                    jump.ranges.iter().map(|&(a, h)| crate::selection::Range::new(carry(a), carry(h))).collect();
                self.sel.rebuild(ranges, jump.primary);
                self.clamp_cursor();
                self.refresh_goal_column();
            }
        }
    }

    /// Find `target` on the current line (`f`/`t`/`F`/`T`), moving the head and
    /// selecting the jumped-over range (unless already extending).
    /// The same, asked for the **nth** one, and told whether this is a repeat.
    ///
    /// Warning: **數目是「第 n 個」，重複要往前挪一個**（拿 nvim 量過）。兩件事都不許
    /// 靠**把這一支叫 n 遍**做，那對 `f`/`t` 兩樣都不對：
    ///
    /// - 每一趟都從上一個落點重新下錨，於是 `2f,` 選的是「第一個逗號到第二個」，
    ///   而 vim 和 helix 選的都是「光標到第二個」。
    /// - `t` 落在目標前一格，再叫一遍又配上同一個目標、頭拉回原處——`;` 於是永遠
    ///   不前進，而 `tdtd` 的第二下把第一下選中的**收成一點**。
    ///
    /// nvim 量的（`a,b,c,d,e`，逗號在 2、4、6、8 欄）：`2t,`→3、`3t,`→5、
    /// `t,;`→3、`t,;;`→5、`9f,`→1（整個不動）。
    ///
    /// `again` 是 `;`/`,`/`A-.`：vim 把它寫在 `:h ;` 裏——「when the cursor is
    /// just in front of the searched character, the `;` command will find the
    /// next occurrence」。只有重複要這一下，`t` 自己不要（`t,` 停在原地就是停在
    /// 原地）。
    /// **光標是不是正貼着 `target`**，往 `forward` 那一邊看一格。
    ///
    /// `t,` 停在逗號前面一格，於是「再來一次」問的是下一個逗號——這一句就是
    /// 「貼着了没有」。
    pub(super) fn sits_against(&self, forward: bool, target: char) -> bool {
        let rope = self.current_buffer().rope();
        let here = self.sel.head();
        let at = match forward {
            true => crate::motion::next_grapheme(rope, here),
            false => crate::motion::prev_grapheme(rope, here),
        };
        at != here && at < rope.len_chars() && rope.char(at) == target
    }

    pub(super) fn find_nth_char(&mut self, kind: FindKind, target: char, nth: usize, again: bool) {
        // 重複的時候，先從「已經貼着的那一個」上讓開一格。
        //
        // **Under helix every `t`/`T` steps off first** (measured: `t,` on
        // `a,b,c` from `a` selects `a,b` there).
        // Helix's `find_char` starts an exclusive search one past the next
        // character, 「to enable repeated application」. vim's `t` stops
        // where it is, and only its `;` steps off.
        let steps_off = again || !self.keymap_rules().till_stands_still;
        let from = match steps_off && kind.till() {
            true => match kind.forward() {
                true => crate::motion::next_grapheme(
                    self.current_buffer().rope(),
                    self.sel.head(),
                ),
                false => crate::motion::prev_grapheme(
                    self.current_buffer().rope(),
                    self.sel.head(),
                ),
            },
            false => self.sel.head(),
        };
        let was = self.sel.head();
        self.sel.set_head(from);
        let span = self.read_motion_nth(
            crate::motion::Motion::Find {
                forward: kind.forward(),
                target,
                till: kind.till(),
            },
            crate::motion::Reading::Selection,
            nth,
        );
        self.sel.set_head(was);
        // **全窗裏這一行是這一格**。`f`/`t` 是按詞走的那
        // 一族，所以它們看不見格子外面——找不到就說找不到，而不是把光標挪到格
        // 子邊上（`select_span` 那條夾在這裏是錯的：讀者按的是「跳到那個字」，
        // 沒有那個字就該一動不動）。
        let outside = match (span, self.cell_the_caret_is_in()) {
            (crate::motion::Span::Over { head, .. }, Some((first, last))) => {
                !(first..=last).contains(&head)
            }
            _ => false,
        };
        if span == crate::motion::Span::Missed || outside {
            // **The sentence names the stretch that was searched.** 全窗裏
            // 「這一行」是「這一格」; under helix the search
            // runs on over line breaks, so outside a cell it is
            // everything ahead or behind; under vim it is still the line — and
            // inside a cell that a line break splits, vim's bound is that line,
            // not the cell, so 「這一格裏没有」 would be untrue there.
            let across = !self.keymap_rules().find_in_line;
            let cell = self.cell_the_caret_is_in();
            let rope = self.current_buffer().rope();
            let one_line = |(a, b): (usize, usize)| rope.char_to_line(a) == rope.char_to_line(b);
            self.status = match (cell, across) {
                (Some(_), true) => say!("find.no-such-character-in-this-cell", target),
                (Some(span), false) if one_line(span) => {
                    say!("find.no-such-character-in-this-cell", target)
                }
                (Some(_), false) | (None, false) => {
                    say!("find.no-such-character-on-this-line", target)
                }
                (None, true) => match kind.forward() {
                    true => say!("find.no-such-character-ahead", target),
                    false => say!("find.no-such-character-behind", target),
                },
            };
            return;
        }
        // 錨點是你**出發**的地方，不是讓開之後那一格。
        let span = match span {
            crate::motion::Span::Over { head, .. } => {
                crate::motion::Span::Over { anchor: was, head }
            }
            other => other,
        };
        // **vim 下獨立的 `f` 是跳轉，不是選擇**，同 `w B e { } H L`（`keys.rs` 那個
        // 「A standalone motion, read vim's way」）。`f F t T` 要是落在 helix 那一支
        // 上，vim 手指按 `f,` 之後跳過去的那一整段是選中的，再按 `~` 就把那一段全變
        // 了大寫。
        match self.keymap_rules().find_moves_caret && !self.expanding_alias {
            true => self.jump_to(span),
            false => self.take_span(span),
        }
    }

}
