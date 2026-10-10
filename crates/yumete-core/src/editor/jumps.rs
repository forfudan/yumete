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
            Some(path) => Spot::InFile(path.to_path_buf(), line),
            None => Spot::InBuffer(self.current_buffer().id(), self.sel.head()),
        };
        self.marks.insert(name, spot);
        self.status = say!("goto.mark-set", name, self.current_buffer().display_name(), line + 1);
    }

    /// Go back to the place a letter names (`' a`).
    pub(super) fn go_to_mark(&mut self, name: char) {
        let Some(spot) = self.marks.get(&name).cloned() else {
            self.status = say!("goto.no-such-mark", name);
            return;
        };
        self.remember_jump();
        match spot {
            Spot::InFile(path, line) => {
                if self.current_buffer().path() != Some(path.as_path()) {
                    if let Err(err) = self.open_file(&path) {
                        self.status = say!("buffer.cannot-open", path.display(), err);
                        return;
                    }
                }
                self.move_to_line(line + 1);
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

    /// **記一筆，可是別把版面挪動** —— 落腳點本來就在屏幕上的那幾種跳法。
    ///
    /// Warning: **「記進跳轉表」和「落在正中」是兩件事**（2026-10-04 報的：「`gw`
    /// 跳转光标后，这一行会被移动到屏幕的中央位置……我总觉得怪怪的」）。
    ///
    /// 居中那一條是給**跳到看不見的地方**的：`n`、`gd`、`:search` 的結果、wiki 條目
    /// ——你落在一個沒見過的地方，貼着邊就只看得見命中的一側。
    ///
    /// `gw`/`gx`/`gz` 正相反：**那個地方本來就在屏幕上，你正盯着它。** 居中等於
    /// 把你盯着的那一行挪走——而「版面不動」正是這一族功能成立的前提（按之前眼睛
    /// 已經鎖定了要去的地方）。
    ///
    /// helix 也不挪：`jump_to_label` 跳完只有一句 `doc.set_selection(...)`，沒有
    /// `align_view`（`helix-term/src/commands.rs`，2026-10-04 讀的）。
    ///
    /// `C-o` 照樣回得來——那一半兩種跳法都要。
    pub(super) fn note_where_we_came_from(&mut self) {
        let here = (self.current_buffer().id(), self.sel.head());
        // Walking away from a place already noted adds nothing.
        if self.jumps.last() == Some(&here) {
            return;
        }
        // A new jump ends the forward history, as it does everywhere else.
        self.jumps.truncate(self.jump_at);
        self.jumps.push(here);
        // Bounded: a session of a thousand jumps does not need a thousandth of
        // them, and the oldest is the one nobody comes back to.
        if self.jumps.len() > JUMPS {
            self.jumps.remove(0);
        }
        self.jump_at = self.jumps.len();
    }

    /// Go back to where a jump came from (`C-o`), or forward again (`C-i`).
    /// **回到這一份稿子最後改動的地方**（`g.`，helix 的 `goto_last_modification`）。
    ///
    /// Warning: **跳轉表答不了這個問題。** 那張表只在**遠距離移動**的時候記一格
    /// （[`Self::remember_jump`]），而「我剛纔改到哪」是寫東西的人一天問一百次的事：
    /// 翻回去查一個名字、看一眼前一章，然後要回到筆停下的地方。`C-o` 只在你「跳」過的
    /// 時候有答案，光是往上滾幾頁再想回來，它一格都沒記。
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

    /// **走改動表**（vim 的 `g;` 往舊、`g,` 往新，2026-10-06）。
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

    pub(super) fn walk_jumps(&mut self, back: bool) {
        if back {
            if self.jump_at == 0 {
                self.status = say!("goto.no-earlier-jump");
                return;
            }
            // Stepping back for the first time has to note where we are, or
            // `C-i` would have nowhere to return to.
            if self.jump_at == self.jumps.len() {
                let here = (self.current_buffer().id(), self.sel.head());
                if self.jumps.last() != Some(&here) {
                    self.jumps.push(here);
                }
            }
            self.jump_at -= 1;
        } else {
            if self.jump_at + 1 >= self.jumps.len() {
                self.status = say!("goto.no-later-jump");
                return;
            }
            self.jump_at += 1;
        }
        let (id, cursor) = self.jumps[self.jump_at];
        // A buffer that has since been closed leaves its jumps behind rather
        // than sending you to whichever file took its place in the list.
        match self.buffer_with(id) {
            Some(i) if i != self.current => self.show_buffer(i),
            Some(_) => {}
            None => {
                self.status = say!("goto.that-file-is-closed");
                return;
            }
        }
        let len = self.current_buffer().rope().len_chars();
        self.move_head(cursor.min(len));
        self.status = say!("goto.jump-list-position", self.jump_at + 1, self.jumps.len());
    }

    /// Find `target` on the current line (`f`/`t`/`F`/`T`), moving the head and
    /// selecting the jumped-over range (unless already extending).
    /// The same, asked for the **nth** one, and told whether this is a repeat.
    ///
    /// Warning: **數目是「第 n 個」，重複要往前挪一個**（2026-10-02 拿 nvim 量出來
    /// 的）。從前兩件事都是靠**把這一支叫 n 遍**做的，而那對 `f`/`t` 兩樣都不對：
    ///
    /// - 每一趟都從上一個落點重新下錨，於是 `2f,` 選的是「第一個逗號到第二個」，
    ///   而 vim 和 helix 選的都是「光標到第二個」。
    /// - `t` 落在目標前一格，再叫一遍又配上同一個目標、頭拉回原處——`;` 於是永遠
    ///   不前進，而 `tdtd` 的第二下把第一下選中的**收成了一點**。
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
    /// 「貼着了沒有」。
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
        let from = match again && kind.till() {
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
        if span == crate::motion::Span::Missed {
            self.status = say!("find.no-such-character-on-this-line", target);
            return;
        }
        // 錨點是你**出發**的地方，不是讓開之後那一格。
        let span = match span {
            crate::motion::Span::Over { head, .. } => {
                crate::motion::Span::Over { anchor: was, head }
            }
            other => other,
        };
        // **vim 下獨立的 `f` 是跳轉，不是選擇**（2026-10-02，補上 B3 漏掉的那一族）。
        // `w B e { } H L` 早就走這一條了（`keys.rs` 那個「A standalone motion, read
        // vim's way」），只有 `f F t T` 四個落在 helix 那一支上，於是 vim 手指按
        // `f,` 之後跳過去的那一整段是選中的，再按 `~` 就把那一段全變了大寫。
        match self.key_preset == yumete_cjk::KeyPreset::Vim && !self.expanding_alias {
            true => self.jump_to(span),
            false => self.take_span(span),
        }
    }

}
