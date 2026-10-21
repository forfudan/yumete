//! Pairs, and moving the cursor (#296).
//!
//! Helix's `m`: jump to the matching bracket, select the pair, add or drop or
//! change what surrounds a selection. The cursor moves live here too — the
//! goal column, the 縱書 axis, and the clamp that keeps a caret on a character
//! boundary — because every one of them is asked for by a `m`/`w`/`h` key.

use super::*;

/// **What is being stepped clear of a table's dead zones** — a caret, or the
/// far edge of a selection (2026-10-17).
///
/// A dead zone is a run of file characters that owns no column on the page:
/// folded tails, the file's surplus alignment spaces, and the one space
/// either side of a `|` wall, which is drawn as part of the single `┆`.
///
/// The two need opposite treatment and both are right:
///
/// * A **caret** must come out of every one of them, onward with the travel.
///   A key that moves the caret somewhere invisible is a key that did nothing
///   — 2026-09-11：「既然没有显示，就应该允许用户直接跳过去。」
/// * A selection's **edge** may rest in the space beside a wall. It is drawn
///   exactly where the last visible character is, so moving it changes
///   nothing on the page and everything about what `d` takes: pushed onward
///   it paints the wall as selected and makes `w` then `d` answer
///   「分隔符删不掉」 for a word that holds no wall; pulled back it sticks,
///   and `w` stops advancing at the end of a cell.
///
/// The wall itself is in neither list. It owns its column, so **every** key
/// stands on it — that is the whole of 2026-10-17's change, and the reason
/// this enum is now about the spaces rather than about the wall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stepping {
    Caret,
    Edge,
}

impl Editor {
    // ---- Match mode (Helix `m`) -------------------------------------------

    /// **光標底下那個括號的另一半在哪**（2026-10-08），字符下標。
    ///
    /// 光標不在括號上、或者那一半不存在（括號没配對）就是 `None`。
    ///
    /// Warning: **一幀問一次，所以它不許把緩衝攤開。** 走的是 `mm` 那兩支
    /// （`find_forward`/`find_backward`），一個 `String`、一個 `Vec` 都不造。
    ///
    /// 最壞的那一檔是「光標停在一個**配不上**的括號上」——那時要從光標一路掃到
    /// 檔尾。量過（2026-10-08，release，四十萬字一行，跑三趟）：**1.3–2.3 ms**。
    /// 原本是 22.8 ms——掃法本來是逐格 `rope.char(i)`，為這一支改成走迭代器了
    /// （理由寫在 `find_forward` 那裏）。配得上的時候掃到那一半就停，正常稿子上
    /// 是幾微秒。所以不設上限：那一檔要湊齊「一行四十萬字」加「光標正停在一個配
    /// 不上的括號上」纔碰得到，而它已經比畫一幀便宜。
    ///
    /// Warning: **引號不算一對**（`closing_of`/`opening_of` 跳過 `open == close`）。
    /// `"` 的兩半長得一樣，站在一個上面說不出另一半在左還是在右——helix 那一支
    /// 走語法樹，也只認括號。
    pub fn matching_bracket(&self) -> Option<usize> {
        let rope = self.current_buffer().rope();
        let here = self.sel.head();
        if here >= rope.len_chars() {
            return None;
        }
        let c = rope.char(here);
        let half = match closing_of(c) {
            Some(close) => find_forward(rope, here, c, close),
            None => find_backward(rope, here, opening_of(c)?, c),
        }?;
        // **全窗裏配對也不出這一格.** The other half of a pair in another cell
        // is not a pair the reader can act on here, and saying so is better
        // than jumping out of the record they are reading.
        match self.cell_the_caret_is_in() {
            Some((first, last)) if !(first..=last).contains(&half) => None,
            _ => Some(half),
        }
    }

    /// Link to the bracket matching the one under the cursor (`mm`).
    ///
    /// **跳的那一格和畫成金的那一格是同一個答案**（2026-10-08）：兩支寫兩遍就意味
    /// 着頁面指着一格而 `mm` 跳到另一格。
    pub(super) fn goto_matching_bracket(&mut self) {
        match self.matching_bracket() {
            Some(pos) => self.move_head(pos),
            // **A key that does nothing says why** (2026-10-17, found in
            // review). The arm that confines a pair to its cell claimed
            // 「saying so is better than jumping out of the record」 and then
            // dropped the `None` on the floor.
            None if self.cell_the_caret_is_in().is_some() => {
                self.status = say!("table.no-pair-in-this-cell")
            }
            None => {}
        }
    }

    /// Select inside (`mi`) or around (`ma`) the pair named by `c`.
    pub(super) fn select_pair(&mut self, c: char, around: bool) {
        // **`w` 是一個對象，不是一對括號**（2026-09-19）。helix 的 `mi w`/`ma w`
        // 是這麽寫的，而 vim 的 `ciw` `diw` `daw` 走的是同一扇門——`i`/`a` 那兩
        // 個動作播的就是 `mi%`/`ma%`（`keymap.rs` 的 `object`）。從前這裏只認
        // 括號，於是 vim 手指最熟的那一組按下去**什麽也不發生**：`ciw` 剪掉光標
        // 底下那一個字就進了插入，比不動還糟。
        // **An object is a motion now** (B1, 2026-09-20). What stays here is
        // what only the editor can do: name the pair the key stands for, and
        // say the sentence when there is nothing to take.
        let what = match c {
            'w' => motion::Object::Word { big: false },
            'W' => motion::Object::Word { big: true },
            // helix has this one too (`mi p`, `commands.rs:6314`), and in a
            // manuscript it is the handier of the two: a 段 is the unit a
            // writer moves around, a word is the unit they fix.
            'p' => motion::Object::Paragraph,
            // `mi s`/`ma s`：光標所在的那一句（2026-09-28）。
            's' => motion::Object::Sentence,
            // `mi m`/`ma m`：光標所在的那一段 Markdown 標記（2026-09-28）。
            'm' => motion::Object::Markup,
            // **`mi f`/`mi t`：光標所在的那個函數/類**（2026-10-06，helix 的
            // `mi f`/`mi t`）。語法樹那一邊的事，所以不走 `Object`——它是
            // 純文本的一支，而這個要問那棵樹。整份是代碼的檔才有，`ma` 和 `mi`
            // 在這裏是同一段：一個定義没有「連着外面那層」可言。
            //
            // Warning: **類是 `t` 不是 `c`**（2026-10-06 當天改正）。先按記憶綁成了
            // `c`，而 helix 的 `c` 是**註釋**、`t` 纔是類
            // （`commands.rs:6307-6310`：`'t' => "class"`、`'c' => "comment"`）。
            // 照抄參考實現，不照記憶。
            'f' | 't' => return self.select_definition(c == 'f'),
            // **`mi a` 參數、`mi c` 註釋**（2026-10-06，helix 的同兩個字母）。用的
            // 是我們自己寫的那份查詢（`code::Language::objects`），不是 helix 的
            // `textobjects.scm`——授權的事見那裏。
            // **`mi c`：光標所在的那一格**（2026-10-17 作者定）。原話：「`mi c`
            // = 这一格」——按字之後，「把這一格整個換掉」是表格裏最常用的動
            // 作，而它本來一個鍵都沒有。
            //
            // Warning: **和註釋那一條不撞。** `'c' if writes_code()` 認的是
            // `Syntax::Code`，而 `.md`／`.csv` 都不是，所以 `mi c` 在表格裏本
            // 來就是空的。真要在一個 `.rs` 裏把一段制表符文字讀成表（`t t`），
            // 格子贏——那是讀者剛剛親手開的那個模式。
            //
            // `ma c` 和 `mi c` 是同一段：一格沒有「連着外面那層」可言，同
            // `mi f`／`mi t`。外面那層是牆，而牆不歸這一格。
            'c' if self.cell_here().is_some() => return self.select_the_cell(),
            'a' if self.writes_code() => return self.select_object(crate::code::Object::Parameter),
            'c' if self.writes_code() => return self.select_object(crate::code::Object::Comment),
            c => match pair_of(c) {
                Some((open, close)) => motion::Object::Pair { open, close },
                None => {
                    self.object_missed = true;
                    return;
                }
            },
        };
        let span = self.run_motion(motion::Motion::Object { what, around, count: 1 });
        if span == motion::Span::Missed {
            self.status = match what {
                motion::Object::Word { .. } => say!("edit.no-word-here"),
                motion::Object::Pair { open, close } => say!("edit.no-pair-around", open, close),
                // 一段永遠在：光標停在空行上，那一段就是那幾個空行。
                motion::Object::Paragraph => say!("edit.no-word-here"),
                motion::Object::Markup => say!("edit.no-markup-here"),
                // 一句永遠在，除非這一行是空的。
                motion::Object::Sentence => say!("edit.no-word-here"),
            };
            self.object_missed = true;
            return;
        }
        self.take_object(span);
    }

    /// **Take both ends exactly as the object named them** (B1).
    ///
    /// The third reading of a span, and the narrowest: an object knows what it
    /// is taking, so neither end is negotiable — no extend, and no clamping to
    /// what the page draws. Warning: Objects have never respected extend mode; that
    /// is preserved here rather than decided, because 「`mi(` while extending」
    /// is a question nobody has asked yet.
    pub(super) fn take_object(&mut self, span: motion::Span) {
        if let motion::Span::Over { anchor, head } = span {
            self.sel.set_anchor(anchor);
            self.sel.set_head(head.max(anchor));
        }
    }

    /// **選中光標所在的那個函數/類**（`mi f`/`mi c`，2026-10-06）。
    ///
    /// 取**最裏面**那一個：巢狀的函數、`impl` 裏的方法，光標站在哪一層就取哪一層
    /// ——和 `mi(` 在同族括號之間挑的規矩一樣。
    /// **`mi c`／`ma c`：光標所在的那一格**（2026-10-17 作者定）。
    ///
    /// 空格子選得中它那一個位置——`row_cells` 把空格子報在一個偏移上，選區收
    /// 在那裏，接着按 `c` 就在那一格裏打字。
    fn select_the_cell(&mut self) {
        // 到不了：那一條 match 臂已經問過 `cell_here()` 了。
        let Some((from, to)) = self.cell_here() else {
            self.status = say!("table.not-in-a-table");
            return;
        };
        let head = motion::prev_grapheme(self.current_buffer().rope(), to).max(from);
        self.sel.set_anchor(from);
        self.sel.set_head(head);
        self.extend = false;
        self.refresh_goal_column();
    }

    fn select_definition(&mut self, function: bool) {
        let want = match function {
            true => crate::code::Define::Function,
            false => crate::code::Define::Class,
        };
        let here = self.sel.head();
        let found = self
            .definitions_here()
            .into_iter()
            .filter(|&(from, to, kind)| kind == want && (from..to).contains(&here))
            .max_by_key(|&(from, _, _)| from);
        let Some((from, to, _)) = found else {
            self.status = say!("code.no-definition-here");
            return;
        };
        let head = motion::prev_grapheme(self.current_buffer().rope(), to).max(from);
        self.sel.set_anchor(from);
        self.sel.set_head(head);
        self.extend = false;
        self.clamp_cursor();
        self.refresh_goal_column();
    }

    /// **選中光標所在的那個參數/註釋**（`mi a`/`mi c`，2026-10-06）。
    ///
    /// 取最裏面那一個，同 `mi f`：巢狀的閉包參數、文檔註釋裏套的註釋，站在哪一層
    /// 取哪一層。
    fn select_object(&mut self, want: crate::code::Object) {
        let here = self.sel.head();
        let found = self
            .objects_here(want)
            .into_iter()
            .filter(|&(from, to)| (from..to).contains(&here))
            .max_by_key(|&(from, _)| from);
        let Some((from, to)) = found else {
            self.status = match want {
                crate::code::Object::Parameter => say!("code.no-parameter-here"),
                crate::code::Object::Comment => say!("code.no-comment-here"),
            };
            return;
        };
        let head = motion::prev_grapheme(self.current_buffer().rope(), to).max(from);
        self.sel.set_anchor(from);
        self.sel.set_head(head);
        self.extend = false;
        self.clamp_cursor();
        self.refresh_goal_column();
    }

    /// **跳到下一段/上一段註釋**（`]c`/`[c`，2026-10-06，helix 的
    /// `goto_next_comment`）。到頭繞回去，同 `]g`。
    pub(super) fn go_to_object_nearby(&mut self, forward: bool, want: crate::code::Object) {
        let starts: Vec<usize> = self.objects_here(want).into_iter().map(|(from, _)| from).collect();
        if starts.is_empty() {
            self.status = match want {
                crate::code::Object::Parameter => say!("code.no-parameter-here"),
                crate::code::Object::Comment => say!("code.no-comments"),
            };
            return;
        }
        let here = self.sel.head();
        let to = match forward {
            true => starts.iter().copied().find(|&s| s > here).unwrap_or(starts[0]),
            false => starts.iter().copied().rfind(|&s| s < here).unwrap_or(starts[starts.len() - 1]),
        };
        self.remember_jump();
        self.sel.collapse_to(to);
        self.clamp_cursor();
        self.refresh_goal_column();
    }

    /// **跳到下一個/上一個診斷**（`]d`/`[d`，`]D`/`[D` 是第一個/最後一個，
    /// 2026-10-06 定，照 helix `default.rs:112-113`）。
    ///
    /// 只看**這一份**的診斷：`空格 d` 那張單子是整個項目的，而這一對是「在這一頁
    /// 上走」，同 `]g`、`]f`。
    pub(super) fn go_to_problem(&mut self, forward: bool, edge: bool) {
        let Some(path) = self.current_buffer().path().map(std::path::Path::to_path_buf) else {
            self.status = say!("code.no-problems");
            return;
        };
        let mut lines: Vec<usize> = self
            .problems_listed()
            .into_iter()
            .filter(|(at, _)| *at == path)
            .map(|(_, said)| said.line)
            .collect();
        lines.sort_unstable();
        lines.dedup();
        if lines.is_empty() {
            self.status = say!("code.no-problems");
            return;
        }
        let here = self.cursor_line();
        let to = match (edge, forward) {
            (true, true) => lines[lines.len() - 1],
            (true, false) => lines[0],
            (false, true) => lines.iter().copied().find(|&l| l > here).unwrap_or(lines[0]),
            (false, false) => {
                lines.iter().copied().rfind(|&l| l < here).unwrap_or(lines[lines.len() - 1])
            }
        };
        self.remember_jump();
        self.goto_line(to + 1);
    }

    /// **跳到屏幕的頂/中/底**（vim 的 `H`/`M`/`L`，2026-10-06 定）。
    ///
    /// 屏幕畫了哪一段是前端每一幀交過來的（`set_page_span`，`gw` 也靠它），所以
    /// 這裏問的是**真畫出來的那一段**，不是「光標那一行加減半屏」。
    /// **Which line of the file the screen's top, middle or bottom is**
    /// (2026-10-06), for `H`/`M`/`L` and for `dH`/`dL` alike.
    ///
    /// Warning: **One answer, two callers.** The key and the operator were two
    /// copies of this arithmetic for an afternoon, and `)` ended up jumping to
    /// the bottom of the screen because only one of them had been corrected.
    ///
    /// `nth` is vim's count: the nth row in from that edge, clamped to the
    /// page. `None` when no frame has been drawn yet — the off-screen path's
    /// first keystroke — so an operator misses rather than guessing.
    pub(super) fn screen_line(&self, which: char, nth: usize) -> Option<usize> {
        let (from, to) = self.page_span;
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();
        if to <= from || from > len {
            return None;
        }
        let first = rope.char_to_line(from.min(len));
        let last = rope.char_to_line(to.min(len).saturating_sub(1).max(from));
        let step = nth.max(1) - 1;
        Some(match which {
            'H' => (first + step).min(last),
            'L' => last.saturating_sub(step).max(first),
            _ => first + (last - first) / 2,
        })
    }

    /// **`zj`／`zk`：視窗滾一行，光標的文字不動**（2026-10-06，helix 的
    /// `scroll_down`／`scroll_up`）。
    ///
    /// 滾視窗和挪光標是同一件事的兩面——文字没動而視窗下去了，光標在屏幕上就
    /// 往上坐了一行。所以這一支只說「坐第幾行」（[`crate::editor::Aim::Row`]），
    /// 不必給核心一個它改不動的視窗位置。
    ///
    /// Warning: **擠到邊上就把光標一起帶走**，同 helix：視窗往下滾而光標已經是最上面
    /// 那一行了，光標只能跟着下去一行，否則它就留在屏幕外面了。
    pub(super) fn scroll_the_page(&mut self, down: bool, n: usize) {
        let Some(first) = self.screen_line('H', 1) else { return };
        let Some(last) = self.screen_line('L', 1) else { return };
        let rope = self.current_buffer().rope();
        let here = rope.char_to_line(self.sel.head());
        let n = n.max(1);
        let top = match down {
            true => (first + n).min(rope.len_lines().saturating_sub(1)),
            false => first.saturating_sub(n),
        };
        // 光標被擠出去了就跟着走一步，落在新視窗的那一邊上。
        let deep = last.saturating_sub(first);
        let line = here.clamp(top, top + deep);
        if line != here {
            self.goto_line(line + 1);
        }
        self.aim = Some(crate::editor::Aim::Row(line.saturating_sub(top)));
    }

    pub(super) fn go_to_screen(&mut self, which: char, nth: usize) {
        let Some(line) = self.screen_line(which, nth) else { return };
        self.remember_jump();
        self.goto_line(line + 1);
    }

    /// **跳到下一個/上一個函數或類**（`]f`/`[f`/`]c`/`[c`，2026-10-06）。
    ///
    /// 到頭繞回去，同 `]g`。
    pub(super) fn go_to_definition_nearby(&mut self, forward: bool, function: bool) {
        let want = match function {
            true => crate::code::Define::Function,
            false => crate::code::Define::Class,
        };
        let starts: Vec<usize> = self
            .definitions_here()
            .into_iter()
            .filter(|&(_, _, kind)| kind == want)
            .map(|(from, _, _)| from)
            .collect();
        if starts.is_empty() {
            self.status = say!("code.no-definitions");
            return;
        }
        let here = self.sel.head();
        let to = match forward {
            true => starts.iter().copied().find(|&s| s > here).unwrap_or(starts[0]),
            false => starts
                .iter()
                .copied()
                .rfind(|&s| s < here)
                .unwrap_or_else(|| starts[starts.len() - 1]),
        };
        self.remember_jump();
        self.sel.collapse_to(to);
        self.clamp_cursor();
        self.refresh_goal_column();
    }

    /// The span a pair of delimiters encloses, `around` taking the marks too.
    fn pair_span(&self, open: char, _close: char, around: bool, count: usize) -> motion::Span {
        let rope = self.current_buffer().rope();
        // Warning: **一個鍵管一族括號**（2026-09-28）：按 `(` 找得到 `()` 也找得到 `（）`，
        // 按 `[` 連 `【】`『』一起找。取**最裏面**那一對——`【他說（不）】` 裏光標在
        // 「不」上按 `di[`，要的是 `（）` 還是 `【】`？答案跟 `md` 一致：最裏面那一對。
        // 這張族表在 `editor.rs` 的 `FAMILIES`，理由記在那裏。
        //
        // Warning: **没有包着光標的那一對，就往前找**（2026-10-12，vim 的
        // `current_block`／`current_quote` 都有這一步）。從前没有，於是
        // `ab (cd) ef` 上光標在行首按 `di(` 一動不動——而 vim 刪掉 `cd`。神諭上
        // 那 433 格差異大半是這一條。
        //
        // 括號和引號是兩支：括號往前找**不限行**，引號只在自己那一行上找，而且
        // 「光標正停在引號上」要從行首一對一對數起（見 `quoted`）。
        let here = self.sel.head();
        let family = super::pair_family(open);
        // Warning: **數目在兩條路上不是同一個意思**（2026-10-12 從 nvim 神諭上量
        // 的）。vim 的 `current_block` 先往回找包着光標的那一對，找到了就把
        // `count` 用在**往外數幾層**上；一對都没包着的時候走另一條路，而那一條
        // 把 `count` 用在**往前數第幾個開括號**上。`a (b (c) d) e` 的行首按
        // `d2a(` 刪的是 `(c)`——往前第二個 `(`，不是外面那一層。
        let enclosing = family
            .iter()
            .filter_map(|&(open, close)| match open == close {
                true => super::quoted(rope, here, open),
                false => super::surrounding(rope, here, open, close),
            })
            // 同族套着的時候取最裏面那一對（`【他說（不）】`）。
            .max_by_key(|&(start, _)| start);
        let ahead = || {
            family
                .iter()
                .filter(|&&(open, close)| open != close)
                .filter_map(|&(open, close)| super::nth_ahead(rope, here, open, close, count))
                .min_by_key(|&(start, _)| start)
        };
        let Some((mut start, mut end)) = enclosing.or_else(ahead) else {
            return motion::Span::Missed;
        };
        // 往前找到的那一對已經把數目用掉了。
        let count = match enclosing.is_some() {
            true => count,
            false => 1,
        };
        // **`2i(` 往外數一層**（2026-10-12 從 nvim 神諭上量的）。vim 的
        // `current_block` 把 `count` 交給 `findmatch` 連問幾次，每一次從上一層的
        // 開括號再往外找。Warning: **層數不夠就一個字都不動**——`a (b) c` 上
        // `d2i(` 在 vim 裏整行不變，而不是退回那一層。同 `d9iw` 那一條。
        //
        // 引號没有「外一層」（一行上的引號不套），它那邊 `count` 另有意思，見下。
        let nests = rope.char(start) != rope.char(end);
        if nests {
            for _ in 1..count {
                let (open, close) = (rope.char(start), rope.char(end));
                let outer = match start {
                    0 => None,
                    _ => super::surrounding(rope, start - 1, open, close),
                };
                match outer {
                    Some((a, b)) => (start, end) = (a, b),
                    None => return motion::Span::Missed,
                }
            }
        }
        // `end` is the closing bracket's own index. The head goes on the last
        // character the selection covers, not one past it — the cursor's
        // grapheme is inside the selection.
        // **引號上數目 ≥2 ＝ 連引號一起拿**（vim 的 `current_quote`：
        // `if (!include && count < 2) ++col_start;`，收那一頭同理）。空白不拿，
        // 那是 `a` 纔有的。
        let whole = around || (!nests && count > 1);
        let (mut anchor, head) = match whole {
            true => (start, end),
            false => (start + 1, end.saturating_sub(1)),
        };
        let mut head = head;
        // **`a"` 連空白一起拿，`a(` 不拿**（vim 的 `current_quote` 末尾那一段，
        // 2026-10-12 量過）：收引號後面那一串空白，没有就拿開引號前面那一串。
        // 和 `aw` 同一條規矩，而括號那一族没有這一條。
        if around && !nests {
            let (from, to) = super::line_bounds(rope, here);
            let mut at = head + 1;
            while at < to && rope.char(at).is_whitespace() {
                at += 1;
            }
            match at > head + 1 {
                true => head = at - 1,
                false => {
                    while anchor > from && rope.char(anchor - 1).is_whitespace() {
                        anchor -= 1;
                    }
                }
            }
        }
        motion::Span::Over { anchor, head: head.max(anchor) }
    }

    /// **光標底下那一段 Markdown 標記**（`mi m`/`ma m`，2026-09-28）。
    ///
    /// `i` 取標記裏面的文字，`a` 連標記一起——`**粗**` 上按 `mi m` 選中「粗」，按
    /// `ma m` 選中「**粗**」。
    ///
    /// Warning: **靠 `construct` 把一族綁起來**。解析器把一個構造拆成三段交出來（開標記、
    /// 文字、閉標記），三段共用一個 `construct` 號。所以 `a` 那一半不必去數星號有幾個，
    /// 取同號那幾段的兩個端點就行——`**` 兩個字符、`` ` `` 一個字符、`](地址)` 一長串，
    /// 同一句話都說得下來。
    ///
    /// Warning: **眼下套不起來，因為解析器不套。** 量過（2026-09-28）：``**粗的`碼`**`` 交出
    /// 來的是 `Marker`/`Strong`/`Marker` 三段，中間那一段連反引號一起算成粗體的正文，
    /// 没有內層的 `Code`。所以這裏「取起點最靠後的那一個」現在永遠只有一個候選。留着這
    /// 一句是因為解析器哪天學會套的時候，這一支不必跟着改。
    ///
    /// Warning: **標記本身（`Kind::Marker`）不算一種**。光標停在星號上按 `mi m`，要的是它
    /// 圍着的那段文字，不是那兩個星號——而星號的 `construct` 和文字是同一個，所以照樣
    /// 找得到。
    fn markup_object_span(&self, around: bool) -> motion::Span {
        use crate::markdown::Kind;
        let rope = self.current_buffer().rope();
        let head = self.sel.head().min(rope.len_chars());
        let line = rope.char_to_line(head);
        let start = rope.line_to_char(line);
        let at = head - start;
        let runs = self.markup_runs(line);
        // 收哪幾種：**有一對標記裹着一段文字**的那些。標題不在裏面（它没有閉標記，
        // 整行就是它，而整行有 `x`），`Marker` 自己也不在（見上面那一條）。
        let takes = |kind: Kind| {
            matches!(
                kind,
                Kind::Strong
                    | Kind::Emphasis
                    | Kind::Code
                    | Kind::Strike
                    | Kind::Highlight
                    | Kind::Link
                    | Kind::WikiLink
                    | Kind::Comment
                    | Kind::Footnote
            )
        };
        // 光標可能正停在標記上，那時 `Marker` 那一段才是包住它的——所以先找出包住光標
        // 的**構造號**，再去那個構造裏取文字那一段。
        let Some(construct) = runs
            .iter()
            .filter(|span| (span.start..span.end).contains(&at))
            .filter(|span| runs.iter().any(|s| s.construct == span.construct && takes(s.kind)))
            .max_by_key(|span| span.start)
            .map(|span| span.construct)
        else {
            return motion::Span::Missed;
        };
        let group: Vec<&crate::markdown::Span> =
            runs.iter().filter(|span| span.construct == construct).collect();
        let (from, to) = match around {
            true => (
                group.iter().map(|span| span.start).min().unwrap_or(at),
                group.iter().map(|span| span.end).max().unwrap_or(at),
            ),
            false => {
                let text = group.iter().find(|span| takes(span.kind));
                match text {
                    Some(span) => (span.start, span.end),
                    None => return motion::Span::Missed,
                }
            }
        };
        let (anchor, head) = (start + from, start + to.saturating_sub(1).max(from));
        motion::Span::Over { anchor, head: head.max(anchor) }
    }

    /// **光標所在的那一句**（`mi s`/`ma s`，vim 的 `cis`/`das`，2026-09-28）。
    ///
    /// Warning: **邊界走 `sentence_starts`**，和 `(`/`)`、`:view-sentence`、`:check-punct`
    /// 同一支。兩個答案就意味着 `mi s` 選的那一段和版面斷行的地方對不上。
    ///
    /// Warning: **一句不跨行**：這個倉的解析是逐行的，`sentence_starts` 也是。一段話寫成一
    /// 行（中文稿子的常態）的時候這没有分別；硬折過行的稿子裏，`mi s` 取的是這一行裏
    /// 的那一句。
    /// **光標所在那一段的字符範圍**（2026-10-10 抽出來的）。
    ///
    /// 「一段」照 [`Self::paragraph_span`] 的判準：連着的同一種行（都空或都不空）。
    /// 句對象要它，因為一句話的邊界是段落不是行。
    fn paragraph_here(&self) -> (usize, usize) {
        let rope = self.current_buffer().rope();
        let here = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        let last = motion::last_line(rope);
        let blank = |line: usize| rope.line(line).to_string().trim().is_empty();
        let same = blank(here);
        let mut first = here;
        while first > 0 && blank(first - 1) == same {
            first -= 1;
        }
        let mut end = here;
        while end < last && blank(end + 1) == same {
            end += 1;
        }
        let start = rope.line_to_char(first);
        // **末行那個換行算段落的一部分**（2026-10-10 量出來的）：vim 的句對象在段
        // 末連它一起取——`dis` 站在一段没有句末標點的話上，整段連換行一起没。
        let stop = match end < last {
            true => rope.line_to_char(end + 1),
            false => rope.len_chars(),
        };
        (start, stop.max(start))
    }

    fn sentence_object_span(&self, around: bool) -> motion::Span {
        let rope = self.current_buffer().rope();
        let head = self.sel.head().min(rope.len_chars());
        // **一句話的邊界是段落，不是行**（2026-10-10 定，照 vim）。從前這裏取的是
        // `line_chars(rope, line)`，於是一行没有句末標點的時候那一句就在換行處結
        // 束——vim 裏它跑到段落結束（`alpha beta, gamma delta` 換行 `xx` 同一段，
        // `das` 兩行一起没）。中文稿一段常常就是一行，所以這一條在那裏看不出來；
        // 英文草稿換行多就常撞。`scripts/oracle/vim_sweep.py` 的 `dis`/`das` 逐格
        // 對 nvim。
        let (start, stop) = self.paragraph_here();
        let chars: Vec<char> = rope.slice(start..stop).chars().collect();
        if chars.is_empty() {
            return motion::Span::Missed;
        }
        let at = (head.saturating_sub(start)).min(chars.len().saturating_sub(1));
        // **站在空白上，`is` 要的就是那一段空白**（同上，照 vim）。從前空白算前一
        // 句的尾巴，於是光標在 `First one.| Second one` 那個空格上按 `dis` 刪掉的
        // 是整句 `First one.`，而 vim 刪的是那一個空格。
        let starts = motion::sentence_starts(&chars);
        let mut at = at;
        if chars[at].is_whitespace() {
            let mut from = at;
            while from > 0 && chars[from - 1].is_whitespace() {
                from -= 1;
            }
            let mut end = at + 1;
            while end < chars.len() && chars[end].is_whitespace() {
                end += 1;
            }
            // Warning: **只有「兩句之間」那一段空白自成一段**。句子**裏面**的空格
            // （`alpha| beta`）照舊屬於那一句——vim 的說法是「a sentence, or the
            // white space between sentences」，而這一段空白是不是「之間」，問的是
            // 它後面緊接着的是不是一句的開頭。2026-10-10 第一版把任何空白都當成
            // 「之間」，於是 `alpha| beta` 上的 `dis` 只刪掉一個空格。
            let between = from > 0 && starts.contains(&end);
            if between {
                match around {
                    // **`as` 站在句間空白上就是「後面那一句的 `as`」**（2026-10-10
                    // 逐格量出來的）：`First one.| Second one…` 上按 `das`，nvim 留
                    // 下 `First one. Third.`——那個空格還在，刪的是後面那一句連它
                    // 自己的尾巴。所以不在這裏另算一遍，把光標挪過去走同一條路：
                    // 「尾巴上没有空格就拿前面的」那條規矩纔跟着一起生效。
                    true => at = end,
                    false => {
                        let (anchor, head) =
                            (start + from, start + end.saturating_sub(1).max(from));
                        return motion::Span::Over { anchor, head: head.max(anchor) };
                    }
                }
            }
        }
        // **段首那一段空白屬於第一句**（2026-10-10 量出來的）：`starts[0]` 是第一個
        // 非空白，所以光標站在它前面的時候没有哪一句「開始得比它早」。從前這裏回
        // `Missed`，於是 `  he said…` 開頭兩格上按 `dis` 什麼都不發生，而 vim 取
        // 的是整句（連那兩格空白）。
        let which = starts.iter().rposition(|&s| s <= at).unwrap_or(0);
        let mut from = match which {
            0 => 0,
            n => starts[n],
        };
        let to = starts.get(which + 1).copied().unwrap_or(chars.len());
        let mut end = to;
        match around {
            // `as`：連句末那一段空白。Warning: 中文句子之間没有空白，所以這一支在中文裏和
            // `is` 拿到同一段——vim 那條規矩在没有空白的文字裏的自然結果，不是算錯。
            // 後面没有空白就取前面的，同 `aw`。
            true => {
                if end == to && to == from {
                    return motion::Span::Missed;
                }
                // Warning: **句末那一段空白裏不算換行**（2026-10-10 量出來的）。
                // `as` 要的是「這一句連着它後面那一段空白」，而行末那個換行屬於
                // 下一行，不是這一句的尾巴：`…here. |Third.\nxx` 上按 `das`，nvim
                // 刪的是 ` Third.`（連**前面**那個空格），換行留着。
                // ……可段**末**那一個換行是這一段的一部分，照取（同 `is`）。
                let last_of_the_paragraph = to == chars.len();
                if !last_of_the_paragraph {
                    while end > from && chars[end - 1] == '\n' {
                        end -= 1;
                    }
                }
                let trails =
                    end > from && chars[end - 1].is_whitespace() && chars[end - 1] != '\n';
                if !trails {
                    while from > 0 && chars[from - 1].is_whitespace() && chars[from - 1] != '\n' {
                        from -= 1;
                    }
                }
            }
            // `is`：句子本身，句末那一段空白不要。
            //
            // Warning: **只有段**末**那個換行算進來**（2026-10-10 逐格量出來的）。
            // 段落**裏面**的換行和句末的空格一樣削掉（`…here. |Third.` 上按 `dis`
            // 留下 `…here. \nxx`），而段落最後那一個換行是這一段的一部分，vim 連它
            // 一起取——「整段没有句末標點」那一種因此整段連換行一起没。
            false => {
                let last_of_the_paragraph = to == chars.len();
                while end > from && chars[end - 1].is_whitespace() {
                    end -= 1;
                }
                if last_of_the_paragraph {
                    end = chars.len();
                }
            }
        }
        let (anchor, head) = (start + from, start + end.saturating_sub(1).max(from));
        motion::Span::Over { anchor, head: head.max(anchor) }
    }

    /// 光標底下那個**段落**（vim 的 `dip`/`dap`，B5 2026-09-21）。
    ///
    /// 一段是「上下都被空行夾着的那幾行」，而光標停在空行上時，那一段**就是
    /// 那幾個空行**——vim 自己的規矩，也是 `dap` 在段與段之間按下去能把多餘的
    /// 空行收掉的原因。
    ///
    /// `around` ＝ `ap`：再加它**下面**那一段空行；下面没有就取上面的。
    /// Warning: 少了這一條，`dap` 會在原地留下一個洞——段落走了，夾着它的兩個空行併
    /// 成一個更大的空當，而 `dap` 讀起來應該是「這一段整個不見了」。
    ///
    /// Warning: **整行，不是一段字符**——區間從頭一行的行首一直到末一行的**換行**，
    /// 那個換行**在裏面**。helix 也是這樣（`textobject.rs` 末尾兩行：`anchor`
    /// 與 `head` 都是 `line_to_char`，也就是行首到行首），而它是對的：少了那個
    /// 換行，`mip` 之後按 `d` 會取走那幾行的正文卻把空行留下，原地多出一個洞。
    fn paragraph_span(&self, around: bool) -> motion::Span {
        let rope = self.current_buffer().rope();
        let here = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        let last = motion::last_line(rope);
        let blank = |line: usize| rope.line(line).to_string().trim().is_empty();
        let same = blank(here);
        // 往兩頭走，走到「不是同一種行」爲止。
        let mut first = here;
        while first > 0 && blank(first - 1) == same {
            first -= 1;
        }
        let mut end = here;
        while end < last && blank(end + 1) == same {
            end += 1;
        }
        if around {
            // 下面那一段異類；没有就換上面那一段。
            let mut after = end;
            while after < last && blank(after + 1) != same {
                after += 1;
            }
            match after > end {
                true => end = after,
                false => {
                    while first > 0 && blank(first - 1) != same {
                        first -= 1;
                    }
                }
            }
        }
        // 末一行的換行本身；没有換行（檔尾没有空行結尾）就退到最後一個字。
        let tail = match end < last {
            true => rope.line_to_char(end + 1).saturating_sub(1),
            false => motion::line_last(rope, rope.line_to_char(end)),
        };
        motion::Span::Over { anchor: rope.line_to_char(first), head: tail }
    }

    /// 光標底下那個**詞**（`mi w`/`ma w`，以及 vim 的 `ciw`/`daw`）。
    ///
    /// `around` ＝ vim 的 `aw`：詞本身，再加它後面那一段空白；後面没有空白就取
    /// 它前面的，這是 vim 自己的規矩，也是 `daw` 讀起來「整個詞連着那道縫一起
    /// 没了」的原因。
    ///
    /// Warning: 用的是走 `w`/`e` 的那一份分詞（`motion::line_words`），**不是**
    /// `segment_line`——那一支只交漢字，標點與拉丁文一個都不交，而 `ciw` 最常
    /// 按在一個拉丁詞上（`delete_selection` 這種）。
    ///
    /// Warning: **粒度跟着 `w` 走，不再寫死**（2026-09-28）。從前這裏是 `Grain::Coarse`，而
    /// `w`/`b` 問的是 `word_grain()`——同一個編輯器對「詞」有兩個答案，於是
    /// 「今天天氣很好」按 `diw` 刪掉六個字，按 `w` 卻走三步。使用者報的原話：「diw，删除
    /// 光标所在词（目前的表现会忽略中文分词器）」。`coarse` 為真的是 `iW`，那個一律粗。
    ///
    /// Warning: **`aw` 在中文裏會退化成 `iw`**：它取的是「詞加它後面那段空白」，而中文詞之間
    /// 没有空白，於是 `daw` 和 `diw` 拿到同一段。這是 vim 那條規矩在中文裏的自然結果，
    /// 不是這一支算錯了。
    fn word_object_span(&self, around: bool, big: bool, count: usize) -> motion::Span {
        let rope = self.current_buffer().rope().clone();
        let line = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        let start = rope.line_to_char(line);
        // Warning: **`iW` 是 `Grain::Big`，不是 `Grain::Coarse`**（2026-10-11 從 nvim
        // 神諭上量出來的，103 格）。兩個名字裏都有一個「粗」字，而它們是兩條規
        // 矩：`Big` 是「一串非空白」（vim 的 `W`），`Coarse` 是「一次一類」
        // （`e` 的規矩，標點自成一段）。挑錯的那一格讓 `diW` 站在 `(no, yes)` 的
        // `(` 上只刪掉一個括號，而 vim 刪 `(no,`——也就是 `iW` 做的是 `iw` 的事。
        //
        // 詞**動作**那一頭一直是對的（`select_word_forward` 的 `big`），所以同一
        // 個編輯器裏 `W` 走一串非空白而 `iW` 只取一類，兩個答案。
        let grain = match big {
            true => crate::motion::Grain::Big,
            false => self.word_grain(),
        };
        let words = crate::motion::line_words(&rope, line, grain, self.segmenter.as_ref());
        let here = words.iter().find(|&&(a, b)| (a..b).contains(&self.sel.head())).copied();
        let Some((from, to)) = here.or_else(|| {
            // **停在空白上的時候，那一串空白就是「詞」**——vim 的規矩，而這裏
            // 特別要緊：`w` 走完光標正停在詞後面那個空格上（本編輯器的 `w` 連
            // 着邊界一起取），於是 `wdiw` 是最順手的一按。`aw` 在空白上再連下
            // 一個詞，也是 vim 的。
            let line_end = start + crate::zong::line_chars(&rope, line).len();
            if self.sel.head() >= line_end || !rope.char(self.sel.head()).is_whitespace() {
                return None;
            }
            let mut a = self.sel.head();
            while a > start && rope.char(a - 1).is_whitespace() {
                a -= 1;
            }
            // Warning: **`aw` 那一串空白跨得過一個換行，`iw` 跨不過**（2026-10-12
            // 從 nvim 神諭上量的）。`ab␣␣` ⏎ `cd ef` 上光標停在行末那個空格，
            // `daw` 刪掉 `␣␣⏎cd` 而 `diw` 只刪掉 `␣␣`。
            //
            // Warning: **至多一個換行。** 下一行要是空的（`ab␣␣` ⏎ ⏎ `cd`），vim
            // 停在那裏——空行在它眼裏自成一個「詞」，不歸這一串空白。
            let mut b = self.sel.head();
            let mut crossed = false;
            while b < rope.len_chars() {
                match rope.char(b) {
                    '\n' if around && !crossed => crossed = true,
                    '\n' => break,
                    c if c.is_whitespace() => {}
                    _ => break,
                }
                b += 1;
            }
            if around {
                // 跟着的那個詞可能在下一行上，所以問的是 `b` 落在哪一行。
                let bline = rope.char_to_line(b.min(rope.len_chars()));
                let ahead = match bline == line {
                    true => words.clone(),
                    false => crate::motion::line_words(
                        &rope,
                        bline,
                        grain,
                        self.segmenter.as_ref(),
                    ),
                };
                if let Some(&(_, end)) = ahead.iter().find(|&&(s, _)| s >= b) {
                    b = end;
                }
            }
            Some((a, b))
        }) else {
            return motion::Span::Missed;
        };
        // 空白那一支已經把 `around` 算進去了，下面那一段只管詞本身。
        // Warning: **原來那個 `around` 還要用**（2026-10-11）：數目那一段每多要一個
        // 單位都要知道這是 `iw` 還是 `aw`，而這裏把它壓成了 `false`。
        let asked_around = around;
        let around = around && here.is_some();
        let (mut a, mut b) = (from, to.saturating_sub(1).max(from));
        let line_end = start + crate::zong::line_chars(&rope, line).len();
        // 這一趟有没有把後面那道縫吃進去——「没有就改取前面那一道」那條退路要
        // 等整個對象取完了纔問一次（見下面）。
        let mut ate_trailing = false;
        if around {
            let mut at = to;
            while at < line_end && rope.char(at).is_whitespace() {
                at += 1;
            }
            if at > to {
                b = at.saturating_sub(1);
                ate_trailing = true;
            }
        }
        // **數目：再要幾個單位**（2026-10-11 從 nvim 神諭上量出來的，182 格）。
        //
        // vim 的 `2iw` 是「兩個單位」，而單位在**詞**和**空白**之間交替——所以
        // `he said` 上的 `d2iw` 刪掉 `he ` 而不是 `he`。`2aw` 數的是兩個「詞 ＋
        // 它後面那道縫」，上面那一段已經替第一個算過了。
        //
        // Warning: **要不夠就一個都不取**（2026-10-11 問過 nvim）。`one two` 上
        // `d9iw` 在 vim 裏**整行不動**——數目湊不齊，整個對象就不成立。從前這裏
        // 「有多少取多少」，於是同一下把一行清空了。寫這一條之前我按常識猜的正
        // 是「取到有的為止」，而那是錯的。
        let mut taken = 1;
        while taken < count {
            // Warning: **換行不算一個單位，跨過去就是了**（2026-10-12 從 nvim 神諭
            // 上量的）。`ab cd;` ⏎ `ef gh` 上光標停在 `;`，vim 的 `d2iw` 刪掉
            // `;⏎ef`——兩個單位是 `;` 和 `ef`，中間那個換行是**被圈進來的**，不
            // 是第二個。從前這裏撞上行尾就 `Missed`，於是跨行的數目一格都不走。
            let mut at = b + 1;
            if at < rope.len_chars() && rope.char(at) == '\n' {
                at += 1;
            }
            if at >= rope.len_chars() {
                return motion::Span::Missed;
            }
            // 單位可能落在下一行上，所以每一趟都問「`at` 在哪一行」。
            let at_line = rope.char_to_line(at);
            let at_end = rope.line_to_char(at_line)
                + crate::zong::line_chars(&rope, at_line).len();
            let ahead = match at_line == line {
                true => words.clone(),
                false => crate::motion::line_words(
                    &rope,
                    at_line,
                    grain,
                    self.segmenter.as_ref(),
                ),
            };
            let mut end = match rope.char(at).is_whitespace() {
                true => {
                    let mut e = at;
                    while e < at_end && rope.char(e).is_whitespace() {
                        e += 1;
                    }
                    e
                }
                false => match ahead.iter().find(|&&(s, e)| (s..e).contains(&at)) {
                    Some(&(_, e)) => e,
                    None => return motion::Span::Missed,
                },
            };
            // **`aw` 的一個單位是一個詞連着它旁邊那道縫。** 從詞起就往後帶縫，
            // 從縫起就往後帶詞——後者正是光標停在空白上那一種（`d2aw` 在
            // `  he said` 的行首：兩個單位是 `  he` 和 ` said`）。
            if asked_around {
                match rope.char(at).is_whitespace() {
                    true => {
                        if let Some(&(_, e)) = ahead.iter().find(|&&(s, _)| s >= end) {
                            end = e;
                        }
                        ate_trailing = false;
                    }
                    false => {
                        let was = end;
                        while end < at_end && rope.char(end).is_whitespace() {
                            end += 1;
                        }
                        ate_trailing = end > was;
                    }
                }
            }
            b = end.saturating_sub(1).max(b);
            taken += 1;
        }
        // **「後面没有縫就取前面那一道」問的是整個對象，不是每一個單位**
        //（2026-10-11 從神諭上量出來的）。`alpha beta, gamma` 上 `2aw` 站在
        // `beta` 的頭上，vim 刪的是 `beta, `——第一個單位後面確實没有縫（跟着的是
        // 逗號），可第二個有，所以那條退路一次都不該走。逐個單位問的話就多吞了
        // `alpha` 後面那個空格。
        if around && !ate_trailing {
            while a > start && rope.char(a - 1).is_whitespace() {
                a -= 1;
            }
        }
        motion::Span::Over { anchor: a, head: b.max(a) }
    }

    /// Wrap the selection in the pair named by `c` (`ms`).
    pub(super) fn surround_add(&mut self, c: char) {
        if self.refuse_readonly() {
            return;
        }
        let Some((open, close)) = pair_of(c) else {
            return;
        };
        let (start, end) = self.selection();
        let end = end.max(start);
        self.snapshot();
        let done = {
            let buffer = self.current_buffer_mut();
            // The closer first, so writing it cannot shift the opener.
            buffer
                .insert(end, &close.to_string())
                .and_then(|()| buffer.insert(start, &open.to_string()))
        };
        if !self.applied(done) {
            return;
        }
        self.sel.set_anchor(start);
        // The wrapped text plus its two marks runs `start ..= end + 1`, and the
        // head sits on the last grapheme of it — not one past. At `end + 2` the
        // character *after* the closing mark was inside the selection, so `ms(`
        // then `d` took one more than the highlight showed.
        self.sel.set_head(end + 1);
        self.clamp_cursor();
    }

    /// **一對標記，兩邊各有多長** —— `(外左, 內左, 內右, 外右)`，都是半開區間的端點。
    ///
    /// 括號那一對是一邊一個字；markdown 的標記一邊是一串（`**` 兩個、`](網址)` 一片）。
    /// `md` 要刪的就是外與內之間那兩段。
    fn marks_around_the_cursor(&self) -> Option<(usize, usize, usize, usize)> {
        // 括號、引號那一族：一邊一個字。
        let pair = self.innermost_pair().map(|(open, close)| (open, open + 1, close, close + 1));
        // markdown 那一族：`ma m` 與 `mi m` 的差就是兩邊的標記。
        let markup = match (
            self.markup_object_span(true),
            self.markup_object_span(false),
        ) {
            (
                motion::Span::Over { anchor: out, head: out_end },
                motion::Span::Over { anchor: inn, head: inn_end },
            ) if out < inn || out_end > inn_end => Some((out, inn, inn_end + 1, out_end + 1)),
            // 兩段一樣寬 ＝ 這個構造没有標記可刪。
            _ => None,
        };
        // **取內層的那一個**，和 `innermost_pair` 自己在 `PAIRS` 之間挑的規矩一樣：
        // `(**粗**)` 站在粗上按 `md` 去掉的是 `**`，不是那對括號。
        //
        // Warning: **起點一樣的時候讓 markdown 贏**（`>=`，2026-10-04 量出來的）。
        // `[字](網址)` 的 `[` 和那個鏈接構造都從同一格起；讓括號贏，摘掉的只是
        // `[` 和 `]`，剩下 `字(網址)` ——一句壞掉的語法。`[[條目]]` 同理，只脫一層
        // 殼。這兩個都該整個構造一起走。
        match (pair, markup) {
            (Some(p), Some(m)) => Some(match m.0 >= p.0 || pair_is_made_of_marks(p, m) {
                true => m,
                false => p,
            }),
            (Some(p), None) => Some(p),
            (None, m) => m,
        }
    }

    /// Remove the innermost pair around the cursor (`md`).
    ///
    /// Warning: **markdown 的標記也算一對**（2026-10-04 定）。`PAIRS` 裏只有括號和
    /// 引號，而這是一部以 markdown 為主的編輯器——要把 `**一句話**` 的星號去掉、
    /// 字留下，從前最快是四步八鍵：`mim` 複製、`mam` 選中、`R` 貼回去。
    ///
    /// 現在 `*斜*`、`**粗**`、`~~刪~~`、`==標==`、`[字](網址)`、`[[條目]]` 都是三個
    /// 鍵。腳註與註釋同理（凡是 `ma m` 與 `mi m` 答得不一樣的構造）。
    /// `which` 說拆哪一種：**`m` ＝ 最內層那一對**，別的字符 ＝ 只拆那一種。
    ///
    /// Warning: **這是 2026-10-06 補上的，從前它不吃字符。** helix 的 `surround_delete`
    /// 一直是吃的（`commands.rs`：`Some('m') => None, // m selects the closest
    /// surround pair`），我們這一支直接拆最內層——**同一個鍵在兩個編輯器裏不是同一件
    /// 事**。定：「不管是 vim 还是 helix，都和他们的行为对齐就好了，不要让用户
    /// 有意外。」順帶 vim 的 `ds(` 就能一行表映過來了：`md` 正好在等那個字符。
    pub(super) fn surround_delete(&mut self, which: char) {
        if self.refuse_readonly() {
            return;
        }
        let found = match which {
            'm' => self.marks_around_the_cursor(),
            // Warning: **一個鍵管一族括號**（2026-10-06 夜審報的）。取對象那條路
            // （`mi(`/`di(`）走的是 `pair_family`——按 `(` 找得到 `（）`，按 `[`
            // 連 `「」`【】一起找。這一支從前逐字符精確，於是 `md(` 在 `（甲乙）`
            // 上答「外面没有成對的符號」，而 `mi(` 選得中。vim 的 `ds[` 在中文稿
            // 子上同病。同族套着的時候取最裏面那一對，同 `pair_span`。
            ch => {
                let rope = self.current_buffer().rope();
                super::pair_family(ch)
                    .into_iter()
                    .filter_map(|(open, close)| surrounding(rope, self.sel.head(), open, close))
                    .max_by_key(|&(open, _)| open)
                    .map(|(open, close)| (open, open + 1, close, close + 1))
            }
        };
        let Some((out, inn, inn_end, out_end)) = found else {
            self.status = say!("edit.no-pair-to-delete");
            return;
        };
        self.snapshot();
        let done = {
            let buffer = self.current_buffer_mut();
            // The closer first, so removing it cannot shift the opener.
            buffer.remove(inn_end..out_end).and_then(|()| buffer.remove(out..inn))
        };
        if !self.applied(done) {
            return;
        }
        // 光標往前挪開頭那一段的長度——`**` 是兩個字，不是一個。
        let opener = inn - out;
        self.sel.set_head(self.sel.head().saturating_sub(opener));
        self.sel.set_anchor(self.sel.head());
        self.clamp_cursor();
    }

    /// Swap the innermost pair around the cursor for another (`mr`).
    pub(super) fn surround_replace(&mut self, from: char, to: char) {
        if self.refuse_readonly() {
            return;
        }
        let Some((new_open, new_close)) = pair_of(to) else {
            return;
        };
        let rope = self.current_buffer().rope();
        // **`m` ＝ 最內層那一對**，同 helix（`surround_replace` 那一支也認它）。
        // 2026-10-06 補的：從前只認具體符號，`mrm[` 一點反應都没有。
        let found = match from {
            'm' => self.innermost_pair(),
            // 同 `surround_delete`：一個鍵管一族，同族套着取最裏面那一對。
            ch => super::pair_family(ch)
                .into_iter()
                .filter_map(|(open, close)| surrounding(rope, self.sel.head(), open, close))
                .max_by_key(|&(open, _)| open),
        };
        let Some((start, end)) = found else {
            let (open, close) = pair_of(from).unwrap_or((from, from));
            self.status = say!("edit.no-pair-around", open, close);
            return;
        };
        self.snapshot();
        let done = {
            let buffer = self.current_buffer_mut();
            // The closer first, so replacing it cannot shift the opener.
            buffer
                .replace(end..end + 1, &new_close.to_string())
                .and_then(|()| buffer.replace(start..start + 1, &new_open.to_string()))
        };
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
    }

    /// The nearest pair of delimiters enclosing the cursor, whichever kind.
    fn innermost_pair(&self) -> Option<(usize, usize)> {
        let rope = self.current_buffer().rope();
        PAIRS
            .iter()
            .filter_map(|&(open, close)| surrounding(rope, self.caret(), open, close))
            .max_by_key(|&(start, _)| start)
    }

    /// **Forget the goal column `j` and `k` aim at**, so the next one works it out.
    ///
    /// Warning: **它從前就地算一次，而那是打字時最貴的一筆**（2026-10-09 查出來的）。
    /// `sample` 指得很準：一行四百萬字的稿子上，插入態每按一個鍵
    /// `on_insert_key` → `refresh_goal_column` → `wrap::position` →
    /// `line_rows_for_caret`，把**整行的字素走一遍**——4559 個採樣裏 4357 個在這一
    /// 條路上，合**每打一個字 75 毫秒**（他那條 1240 萬字的行按此推是 230 毫秒）。
    /// 四十四處呼叫它，每一處都付這一筆，而開不開折行一樣貴。
    ///
    /// 不必算：`j`／`k` 那一支本來就寫着「没記過就現算一次」，而算完它自己記回去
    /// （[`Editor::step_row`] 那一段註釋說的正是這件事）。把這一支改成「忘掉」，
    /// 答案一個字不變——中間没有別的東西動過光標，所以現算出來的就是這裏本來要算
    /// 的那一個——而那筆 O(行長) 挪到了**真按 `j` 的那一下**，付一次。
    ///
    /// 名字照舊叫 refresh：四十四個呼叫方要的本來就是「目標列現在不算數了」。
    pub(super) fn refresh_goal_column(&mut self) {
        self.sel.set_goal(None);
    }

    /// `j`/`k` inside a grid, by cell rather than by screen column.
    ///
    /// Answers whether it took the step: only in a table, only inside a cell
    /// (at the top layer the row is [`Self::move_cell_row`]'s job), and only
    /// when there is a row that way to go to — at the edges the ordinary path
    /// takes over, so walking out of a table into the prose still works.
    fn step_grid_row(&mut self, up: bool) -> bool {
        if !self.table_here() || self.table_layer() != Layer::Inside {
            return false;
        }
        let Some((line, cell)) = self.cell_position() else {
            return false;
        };
        let Some((from, _)) = self.cell_span(line, cell) else {
            return false;
        };
        let into = self.caret().saturating_sub(from);
        let Some(want) = self.next_row(line, !up) else {
            return false;
        };
        // **A row with fewer cells keeps the caret in its last one**
        // (2026-10-17, found in review). This used to give up when the row
        // below had no column of that number, and the generic vertical
        // motion then dropped the caret on that row's closing `|` — a place
        // 全窗 says it never stands, and the grid drew it inside the cell, so
        // nothing warned before the next keystroke.
        let width = self.row_cells(want).len();
        let Some((a, b)) = self.cell_span(want, cell.min(width.saturating_sub(1))) else {
            return false;
        };
        // **Onto a character of that cell, not past its last one**
        // (2026-10-17). `b` is the content's end, so a column walked down into
        // a shorter cell parked the caret on the space beside the wall: 「同
        // 一欄下一格比較短時，光標停在內容後面的空位上，不是最後一個字上」.
        // Insert keeps the far end — that is where appending happens, and it
        // is the same split `on_this_line_under_vim` makes for a line.
        let last = match self.mode {
            crate::editor::Mode::Insert => b,
            _ => crate::motion::prev_grapheme(self.current_buffer().rope(), b).max(a),
        };
        self.move_head((a + into).min(last));
        true
    }

    /// Apply a horizontal motion, moving the head (extending if in select mode).
    pub(super) fn move_horizontal(&mut self, motion: fn(&ropey::Rope, usize) -> usize) {
        self.step_sideways(motion, None)
    }

    /// 同上，而且這一鍵在 `whichwrap` 裏有自己那個字母。
    ///
    /// `flag` ＝ vim 給這一鍵的那個字母（`h`、`l`、`<`、`>`）。`None` ＝ 這一步
    /// 永遠不跨行（命令行上的左右就是，那裏只有一行）。
    pub(super) fn step_sideways(
        &mut self,
        motion: fn(&ropey::Rope, usize) -> usize,
        flag: Option<char>,
    ) {
        let here = self.sel.head();
        let pos = motion(self.current_buffer().rope(), here);
        let forward = pos >= here;
        // **Three steps, and the order is the whole of it** (2026-10-17).
        // Step clear of what the table keeps off; *then* let the line clamp
        // say whether running off the end crosses to the next row or stops;
        // *then* find a place on whichever row that left us. It used to be
        // the first two only, so the clamp put the caret back **on** the
        // row's last wall — a place 全窗 has no column for, which makes
        // `window_at` scroll the cell inside itself and draw `dog` as `<g`.
        let pos = self.past_what_a_table_keeps_off(pos, forward, Stepping::Caret);
        let pos = self.on_this_line_under_vim(pos, flag);
        // …and last of all the cell's own walls, when the keys are inside one
        // (2026-10-21). It goes after `a_place_for_the_caret` and not before:
        // that one looks for somewhere the page really draws, and the place
        // it would find outside this cell is in the next one.
        let pos = self.a_place_for_the_caret(pos, forward);
        self.move_head(self.in_the_cell(pos));
    }

    /// **vim 鍵位下 `h`/`l` 不出這一行**（`:h l`；2026-10-02 定照參考實現）。
    ///
    /// Warning: **要緊的是行末那一格坐不上去。** vim 的普通模式光標停不到換行符
    /// 上，而 yumete 照 helix 的規矩停得上去——於是在一行的最後一個字上按 `x`，
    /// 吃掉的是換行，兩行焊成一行。2026-10-02 拿 nvim 逐欄比，這是最後一條兩家
    /// 真的不一樣的地方（中文分詞那三格除外）。
    ///
    /// 跨行一併擋住，那也是 vim 自己的規矩（`whichwrap` 出廠不含 `<`、`>`）——
    /// **而讀者撥得開它**（2026-10-10）：`flag` 那個字母寫進 `[editor] which_wrap`
    /// 的話，這一步就跨得出去，落在 vim 落的那一格（往右是下一行的頭一個字，往
    /// 左是上一行的最後一個字，兩個都不是換行符本身）。
    /// helix 鍵位照舊：那一邊「走一頁」是有意的，記在上面 `h`/`l` 那一條。
    fn on_this_line_under_vim(&self, pos: usize, flag: Option<char>) -> usize {
        if self.key_preset != yumete_cjk::KeyPreset::Vim {
            return pos;
        }
        // Warning: **是「跨」不是「不攔」。** 直接放行會讓光標停在換行符上，而
        // `keep_off_the_newline` 下一刻就把它拉回來——那條規矩是對的（停在換行上
        // 一個 `x` 就把兩段接起來）。所以走的是底下那條落點規矩：一步落到下一段
        // 的第一個字，換行符本身從來不是終點。
        let across = match flag {
            // **没有自己那個字母的那一步，就是「順着文字讀下去」的那一鍵**，所以
            // 竪排下它跨：竪排的 `j`/`k` 走的是這一支（`keys.rs` 的竪排分支，經
            // `move_horizontal`），而竪排裏「一行」是一整段——攔在行內就是「`j`
            // 走到段尾停死，下一縱根本到不了」，2026-09 報過、橫排那一側早就修好
            // 的那一條（2026-10-10 量出來的）。
            None => self.layout() == yumete_cjk::Layout::Vertical,
            // **有字母的（`h`/`l`/`<`/`>`/`b`）歸 `whichwrap` 管，橫排竪排同一條
            // 規矩**（2026-10-13 改）。從前這裏也是「竪排一律跨」，而那是為了上面
            // 那一支寫的；行文那一套下 vim 的 `h`/`l` 一併落到這裏之後，`h` 就從
            // 一段的頭一個字跳到上一段的末字了——`which_wrap` 空着也跳。
            Some(c) => self.which_wrap.contains(c),
        };
        let rope = self.current_buffer().rope();
        let here = self.sel.head();
        // **插入態坐得上行末那一格**（2026-10-17 報的）。vim 的普通模式停不到換
        // 行符上，所以普通態的右邊界是**最後一個字**（`line_last`）；插入態不
        // 是——那一格正是打字的地方，一行的虛擬欄是 `len+1`。
        //
        // 從前兩態共用普通態那個界，於是 vim 鍵位下插入態按 `→` 卡在最後一個字
        // 上：原話「I cannot use right arrow to navigate to the right side of
        // the last character of the document」。而且不只末尾——一行的末字就走
        // 不過去，多行檔裏 `→` 從此一步也前進不了，要退回普通態按 `a`。
        //
        // nvim --clean 實測（`abc`，插入態從第一欄起，按完插一個 `X` 看落點）：
        // 一下 `aXbc`、兩下 `abXc`、三下 `abcX`、四下還是 `abcX`——到行末停住，
        // 不跨行（出廠 `whichwrap` 不含 `]`）。下面那一段 `across` 管的正是跨不
        // 跨，所以這裏只要把界放對。
        let first = motion::line_start(rope, here);
        let last = match self.mode {
            crate::editor::Mode::Insert => motion::line_end(rope, here),
            _ => motion::line_last(rope, here),
        };
        if (first..=last).contains(&pos) {
            return pos;
        }
        if !across {
            return pos.clamp(first, last);
        }
        let line = rope.char_to_line(here);
        match pos > last {
            // 下一行的頭一個字。没有下一行就停在原地（vim 也停）。
            true => match line + 1 < rope.len_lines() {
                true => rope.line_to_char(line + 1),
                false => last,
            },
            // 上一行的最後一個字——`first - 1` 是上一行的換行符，`line_last`
            // 從那裏退到它前面那個字。
            false => match first > 0 {
                true => motion::line_last(rope, first - 1),
                false => first,
            },
        }
    }

    /// Step `at` clear of anything a table is keeping off the page, in the
    /// direction of travel, for a caret or for an edge ([`Stepping`], #379).
    ///
    /// **A step the reader cannot see is not a step**, and its converse:
    /// a character that owns a drawn column is a character every motion may
    /// stand on. Under `t f` the file's own padding comes off the page
    /// (`cell_slack_against`), so `l` through a cell's trailing spaces moved
    /// the caret and moved nothing on the screen: it sat still for four
    /// presses and then jumped a column.
    /// 2026-09-11：「既然没有显示，就应该允许用户直接跳过去。」
    ///
    /// Only what a table keeps — see [`Editor::cell_hidden_on_line`]. Markup
    /// hidden by 所見即所得 comes back for the caret, so walking into it is
    /// the point rather than a mistake.
    fn kept_off_on_line(&self, line: usize, what: Stepping) -> Vec<(usize, usize)> {
        let rope = self.current_buffer().rope();
        let mut hidden = self.cell_hidden_on_line(line);
        // **And the padding either side of a wall** (2026-09-12, narrowed
        // 2026-10-17). A `|` and the spaces around it are three characters of
        // the file drawn as one `┆`, and the rule above decides each of the
        // three on its own: the wall owns that column, so it is a stop; the
        // two spaces own none, so they are not. Hiding all three is what
        // 2026-09-12 did, and it made `l` change column in one press while
        // the wall stayed on the page unvisitable — the only place in a table
        // where the caret's file position and its drawn position came apart.
        // Warning: **A `|` table only.** There the separator is three characters of
        // the file — `space pipe space` — drawn as one `┆`. A delimited file's
        // separator is a single comma or tab, and its **empty cells sit at the
        // same offset as the separator after them** (`一,,木目`: the blank cell
        // and the second comma are both at 2), so hiding anything there costs
        // the one thing a table editor is most for — putting the caret in a
        // blank cell to fill it in.
        // **The pane is a different kind of thing** (2026-10-17 作者定):
        // 「to, tb, tf are about the texts (content + separators though
        // rendered), and tt is only about the content (we handle the content
        // in a special app)」. So in 全窗 the wall is not an object at all —
        // the whole `space | space` stays off every motion, caret and edge
        // alike. At 全 it is text that happens to be drawn as a rule, and the
        // rule above applies: the `|` owns its column, the spaces do not.
        //
        // **A delimited file's separator counts too, in the pane** (2026-10-17,
        // found in review). The warning above said 「a `|` table only」 because
        // an empty cell in `一,,木目` sits at the same offset as the comma
        // after it, and hiding the comma hid the one slot there is for filling
        // that cell in. The `keep` split below now protects every cell's start
        // outright, so that reason is spent — and leaving the comma a stop
        // broke the pane's own rule twice over: the caret stood on a `┆`, and
        // `cell_position` reported the cell *before* it, so `select_span`'s
        // clamp dragged a forward `w` **backwards** (measured: `lll` onto the
        // comma, then `w` back onto `t`).
        let pane = self.grid_has_the_pane();
        if pane && self.grid_is_drawn() && !self.grid_separator_is_a_pipe() {
            if let Some(Separator::Delimiter(d)) = self.table.as_ref().map(|v| v.separator) {
                let text = crate::motion::line_text(rope, line);
                for (at, _) in text.chars().enumerate().filter(|&(_, c)| c == d) {
                    hidden.push((at, at + 1));
                }
            }
        }
        if (pane || what == Stepping::Caret)
            && self.grid_is_drawn()
            && self.grid_separator_is_a_pipe()
        {
            // Not the whole gap between two cell spans — `row_cells` reports
            // content only, so that gap also holds the cell's own padding, and
            // padding is the cell's (`cell_hidden_on_line` above already says
            // which of it is off the page). What the wall takes with it is
            // exactly the one space either side.
            let text = crate::motion::line_text(rope, line);
            let chars: Vec<char> = text.chars().collect();
            for at in crate::mdtable::pipes_from(&text, false) {
                let from = match at > 0 && chars[at - 1] == ' ' {
                    true => at - 1,
                    false => at,
                };
                let upto = match chars.get(at + 1) == Some(&' ') {
                    true => at + 2,
                    false => at + 1,
                };
                match pane {
                    true => hidden.push((from, upto)),
                    false => {
                        if from < at {
                            hidden.push((from, at));
                        }
                        if upto > at + 1 {
                            hidden.push((at + 1, upto));
                        }
                    }
                }
            }
            hidden.sort_unstable();
        }
        // **An empty cell keeps its one place** (2026-10-17). `| one |  |`
        // draws a cell between the two walls, and `row_cells` reports it at a
        // single offset — one of the two spaces a wall takes with it. Hide
        // that space and the cell has no slot at all: measured, `l` ran
        // `one` → `three` and column 2 was unreachable from the keyboard,
        // which on a 拆分表 (four cells in five are empty) is most of the
        // table. Putting the caret in a blank cell to fill it in is the one
        // thing a table editor is most for, so no span may swallow the offset
        // a cell begins at.
        // **In Insert the cell's far end is kept too** (2026-10-17, found in
        // review). That offset is the space beside the wall, which the pane
        // hides — and it is also the one place appending happens, so `→` in
        // Insert could never reach the end of a cell: it stepped from the
        // last character straight into the next column. `step_grid_row` got
        // the same exception for `j`/`k` when it was written.
        let appending = self.mode == crate::editor::Mode::Insert;
        let keep: Vec<usize> = self
            .row_cells(line)
            .into_iter()
            .flat_map(|(from, to)| match appending && to > from {
                true => vec![from, to],
                false => vec![from],
            })
            .collect();
        if !keep.is_empty() {
            let mut split = Vec::with_capacity(hidden.len());
            for (from, upto) in hidden {
                let mut at = from;
                for &slot in &keep {
                    if (at..upto).contains(&slot) {
                        if slot > at {
                            split.push((at, slot));
                        }
                        at = slot + 1;
                    }
                }
                if at < upto {
                    split.push((at, upto));
                }
            }
            hidden = split;
        }
        hidden
    }

    /// Step `at` clear of those spans, in the direction of travel.
    ///
    /// The answer may leave the line — forward off its end, or back past its
    /// start — and that is the caller's cue that this line has nowhere to
    /// stand that way ([`Self::a_place_for_the_caret`]).
    fn past_what_a_table_keeps_off(&self, at: usize, forward: bool, what: Stepping) -> usize {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(at.min(rope.len_chars()));
        let start = rope.line_to_char(line);
        let hidden = self.kept_off_on_line(line, what);
        if hidden.is_empty() {
            return at;
        }
        // Spans do not overlap and there are a handful of them, so walking
        // them is a loop over the row's cells at worst.
        let mut at = at;
        while let Some(&(from, upto)) = hidden
            .iter()
            .find(|&&(from, upto)| (from..upto).contains(&(at.saturating_sub(start))))
        {
            match forward {
                true => at = start + upto,
                // The character *before* the span: its first is still hidden.
                // `start` itself is the answer 「nothing before this on the
                // line」, which the caller reads as 「look the other way」.
                false if from == 0 => return start.saturating_sub(1),
                false => at = start + from - 1,
            }
        }
        at
    }

    /// **Where the caret may actually stand** (2026-10-17).
    ///
    /// The row's last wall is the case this exists for. `l` off the end of a
    /// cell steps clear of the wall's span and lands on the line's newline —
    /// a place 全窗 has no column for, so `window_at` decides the cell must
    /// scroll inside itself and draws `dog` as `<g`. A second `l` then went
    /// on to the next row, so every row cost one press that moved nothing
    /// and drew wrongly.
    ///
    /// Running out of places on this line is therefore not a reason to stand
    /// still: it is the end of the row, and the next row's first cell is
    /// where the key was going. The table's own rows bound that — walking out
    /// into the prose under a table is what `j` is for, and the pane has no
    /// prose at all.
    fn a_place_for_the_caret(&self, at: usize, forward: bool) -> usize {
        let rope = self.current_buffer().rope();
        let at = at.min(rope.len_chars());
        let line = rope.char_to_line(at);
        if self.kept_off_on_line(line, Stepping::Caret).is_empty() {
            return at;
        }
        // **The line's characters, not its terminator.** A caret left on the
        // newline is a caret the grid cannot place, which is the whole of the
        // fault above.
        let ends = |line: usize| {
            let start = rope.line_to_char(line);
            let mut last = crate::motion::line_last(rope, start);
            if last > start && last < rope.len_chars() && rope.char(last) == '\n' {
                last -= 1;
            }
            (start, last)
        };
        let (start, last) = ends(line);
        let here = at.clamp(start, last);
        let out = self.past_what_a_table_keeps_off(here, forward, Stepping::Caret);
        if (start..=last).contains(&out) {
            return out;
        }
        // Off the end of the row. The next one along, if it is still a row of
        // this table.
        let next = match forward {
            true => line + 1,
            false => line.checked_sub(1).unwrap_or(line),
        };
        let rows = self.table_row_span_at(line);
        let stays = next != line
            && rows.is_some_and(|(first, upto)| (first..=upto).contains(&next))
            && rows == self.table_row_span_at(next);
        if stays {
            let (from, to) = ends(next);
            let edge = match forward {
                true => from,
                false => to,
            };
            let out = self.past_what_a_table_keeps_off(edge, forward, Stepping::Caret);
            if (from..=to).contains(&out) {
                return out;
            }
        }
        // Nowhere onward: the nearest place back the way we came.
        let back = self.past_what_a_table_keeps_off(here, !forward, Stepping::Caret);
        match (start..=last).contains(&back) {
            true => back,
            false => here,
        }
    }
    /// Apply a vertical motion, preserving the goal column and moving the head.
    ///
    /// With soft wrap on, `j` and `k` step one *screen* row rather than one
    /// paragraph, because that is the row the reader is looking at: on a novel,
    /// where a paragraph is one line of several hundred characters, a logical
    /// `j` would jump a whole screen at a time.
    pub(super) fn move_vertical(&mut self, up: bool) {
        self.move_row(up, false);
    }

    /// Step one line **of the file** — a whole paragraph where a paragraph is
    /// one line — keeping the goal column: helix's `gj`/`gk`
    /// (`move_line_down`, 「textual (instead of visual) line」).
    pub(super) fn move_textual_line(&mut self, up: bool) {
        self.move_row(up, true);
    }

    fn move_row(&mut self, up: bool, textual: bool) {
        // **In a grid, the column is the cell** (#357). The goal column is
        // worked out from the *document* — the text and the padding a `|`
        // table carries in it — while `t f` and `t t` draw a grid of their own
        // whose widths this side never sees. So `j` walked the document's
        // columns under a page laid out to different ones, and the caret
        // drifted between columns as it went down. Asked as 「the same cell,
        // the same way into it」 the question needs no widths at all, and it is
        // what a grid means by 「down」 anyway.
        if self.step_grid_row(up) {
            return;
        }
        let (pos, goal) = {
            let hide = |line: usize| self.hidden_on_line(line);
            let fold = |line: usize| self.line_is_folded(line);
            let rope = self.current_buffer().rope();
            // …with the indent and the folds, for the same reason: the page
            // `j` steps through has to be the page on the screen. With wrapping
            // off a row is a paragraph, which is what `NO_WRAP` gives — through
            // this same code, so the two cases cannot answer differently about
            // what is off the page.
            let width = match textual {
                true => crate::wrap::NO_WRAP,
                false => self.wrap_width().unwrap_or(crate::wrap::NO_WRAP),
            };
            let drawn = |line: usize| self.drawn_on_line(line);
            let typed = |line: usize| self.typed_on_line(line);
            // A table row is one row (#275) — and `j` has to be walking the
            // same page the renderer drew, or it steps into a row that is not
            // on the screen.
            let flat = |line: usize| self.table_row_at(line);
            // 一行的前綴寬度索引（2026-10-08）：不折行的長行上，光標的列
            // 不必每動一下就把前面那一整段逐字素走一遍。
            let widths = |rope: &ropey::Rope, line: usize, span: usize| self.line_widths(rope, line, span);
            let m = crate::wrap::Measure::new(width, &hide)
                .with_indent(self.paragraph_indent())
                .with_folds(&fold)
                .with_drawn(&drawn)
                .with_typed_drawn(&typed)
                .with_unwrapped(&flat)
                .with_widths(&widths)
                .with_version(self.current_buffer().id(), self.current_buffer().revision())
                .with_edit(self.current_buffer().edit())
                .with_open_line(self.open_line())
                .with_caret(Some(self.caret_in_line()));
            // Warning: **目標列現在在這一段自己身上**（#405）。没記過就現算一次——新長出來
            // 的選區、剛從別處跳過來的光標都會落到這一支上。
            //
            // Warning: **算完要記回去**：`j` 自己不叫 `refresh_goal_column`（那正是它保得住
            // 目標列的原因），所以這裏不記的話，下一次 `j` 又從**已經被壓到行尾的**那
            // 一列現算，連按兩下就再也回不到原來那一列了。
            let goal = self.sel.goal();
            let goal = goal.unwrap_or_else(|| crate::wrap::column_of(rope, self.sel.head(), m));
            let pos = match up {
                true => crate::wrap::prev_row(rope, self.sel.head(), m, goal),
                false => crate::wrap::next_row(rope, self.sel.head(), m, goal),
            };
            (pos, goal)
        };
        // vim 鍵位下檔尾那條虛行到不了，所以瞄到那裏的 `j` 是一次失手——連目標
        // 列都不動（見 [`Editor::vim_cannot_reach`]）。
        if self.vim_cannot_reach(self.current_buffer().rope(), pos) {
            return;
        }
        let remembered = self.sel.goal();
        self.sel.set_head(pos);
        if !self.extend {
            self.sel.set_anchor(pos);
        }
        // `set_head` 不動目標列，可是上面那一支可能是現算出來的——記回去。
        if remembered.is_none() {
            self.sel.set_goal(Some(goal));
        }
    }

    /// Move to the neighbouring 縱 in vertical layout: `left` steps to the next
    /// 縱 (drawn to the left, since 縱 stack leftward), otherwise to the
    /// previous one. `continuing` says the previous key was also a 縱 motion,
    /// in which case the goal slot is kept, so crossing a short paragraph does
    /// not drag the cursor permanently upwards.
    pub(super) fn move_zong_from(&mut self, left: bool, continuing: bool) {
        // The page is built here, from the same two answers the horizontal
        // side is built from — and it lives only as long as this block, which
        // is what lets the cursor be written after it.
        let (goal, pos) = {
            let hidden = |line: usize| self.markup_hidden_on_line(line);
            let folded = |line: usize| self.line_is_folded(line);
            let drawn = |line: usize| self.drawn_runs_on_line(line);
            let turned = |line: usize| self.line_is_table_row(line);
            let grid = self.grid_with(&hidden, &folded, &drawn, &turned);
            let rope = self.current_buffer().rope();
            // Warning: **目標格每一段各記一份**（2026-09-28，同 `goal`）。從前它是 `Editor`
            // 上的一個 `goal_slot`，於是竪排下 N 段一起按 `h` 會一起瞄準主選區那一格。
            let goal = match continuing {
                true => self
                    .sel
                    .goal_slot()
                    .unwrap_or_else(|| zong::slot_of(rope, self.sel.head(), grid)),
                false => zong::slot_of(rope, self.sel.head(), grid),
            };
            let pos = if left {
                zong::next_zong(rope, self.sel.head(), grid, goal)
            } else {
                zong::prev_zong(rope, self.sel.head(), grid, goal)
            };
            (goal, pos)
        };
        // 同橫排：vim 鍵位下檔尾那條虛行到不了。
        if self.vim_cannot_reach(self.current_buffer().rope(), pos) {
            return;
        }
        self.sel.set_head(pos);
        if !self.extend {
            self.sel.set_anchor(pos);
        }
        self.sel.set_goal_slot(Some(goal));
        self.zong_motion = true;
    }

    /// Move the selection head to `pos`; collapse the selection unless select
    /// (extend) mode is active. Refreshes the goal column.
    pub(super) fn move_head(&mut self, pos: usize) {
        self.sel.set_head(pos);
        if !self.extend {
            self.sel.set_anchor(pos);
        }
        self.refresh_goal_column();
    }

    /// Put **both** ends where a motion says, unless it is extending.
    ///
    /// A motion that means 「take everything from here to there」 leaves the
    /// anchor where the caret was — that is a span whose `anchor` is the old
    /// position. One that knows what it is taking sets both ends itself. `e` is the second kind: the
    /// word it lands on begins somewhere, and beginning the selection at the
    /// old caret instead dragged the previous word's last character — and the
    /// punctuation between them — along with it (#304).
    pub(super) fn select_span(&mut self, from: usize, to: usize) {
        // **全窗裏按詞走的那幾個鍵不出這一格**（2026-10-17 作者定）——見
        // [`Editor::cell_the_caret_is_in`]. This is the one door they all go
        // through: `w W e E b B`, `f F t T`, `H L`, `{ }` and the text
        // objects, so the rule is stated once.
        // **全窗裏按詞走的那幾個鍵不出這一格**（2026-10-17 作者定）——見
        // [`Editor::cell_the_caret_is_in`]. This is the one door they all go
        // through: `w W e E b B`, `f F t T`, `H L`, `{ }` and the text
        // objects, so the rule is stated once.
        //
        // **After the dead-zone step, and short of the cell's end.** Before it,
        // the step undid the clamp — the cell's end is the space beside the
        // wall, which 全窗 hides, so a `to` clamped to it was pushed straight
        // on into the next cell. And the end itself is one short of the span,
        // so the head always stands on a character of this cell: the next
        // press reads its bounds off that head, and a head parked on the
        // boundary read the *next* cell's bounds and escaped on the second
        // press.
        // Only when the caret really is in the cell its row reports. A caret
        // sitting on a separator reports the cell *before* it, and clamping to
        // that one turns a forward motion into a backward one.
        let here = self.sel.head();
        let held = self
            .cell_the_caret_is_in()
            .filter(|&(first, last)| (first..=last).contains(&here));
        let to = self.past_what_a_table_keeps_off(to, to > self.sel.head(), Stepping::Edge);
        let (from, to) = match held {
            Some((first, last)) => {
                let last = last.saturating_sub(1).max(first);
                (from.clamp(first, last), to.clamp(first, last))
            }
            None => (from, to),
        };
        if !self.extend {
            self.sel.set_anchor(from);
        }
        self.sel.set_head(to);
        self.refresh_goal_column();
    }

    /// **Run a motion and say where it reaches** (B1, 2026-09-20).
    ///
    /// The one place a [`motion::Motion`] becomes a [`motion::Span`]. It reads
    /// the editor because a motion needs what only the editor holds — which
    /// dictionary is loaded, and where the caret is — and it writes nothing:
    /// what to *do* with the span is the grammar's, and today there is one
    /// grammar ([`Self::take_span`]).
    pub(super) fn run_motion(&self, what: motion::Motion) -> motion::Span {
        self.read_motion(what, motion::Reading::Selection)
    }

    /// The same, saying **which reading** is wanted (B3, 2026-09-20).
    ///
    /// 「vim 的 `w` 獨立的時候是跳轉，在命令中是選詞；helix 把兩個 `w` 合一了」
    /// — so the two grammars ask the same motion two different questions, and
    /// this is where the question is put. [`motion::Reading::Caret`] answers
    /// with the **primitive**: where the caret lands, collapsed, no rule of
    /// helix's wrapped around it.
    pub(super) fn read_motion(&self, what: motion::Motion, how: motion::Reading) -> motion::Span {
        self.read_motion_nth(what, how, 1)
    }

    /// The same, asked for the **nth** one (2026-10-02).
    ///
    /// Warning: **一個數目不是「做 n 遍」。** `f`/`t` 帶數目是「第 n 個」，做 n 遍
    /// 會每一趟都從上一個落點重新下錨，選中的那一段就從第一個起而不是從光標起；
    /// 而數目超出的時候「做 n 遍」走到最後一個，vim 是整個動作失敗。別的動作做 n
    /// 遍確實就是對的（`3w`），所以只有這一支認得 `nth`。
    pub(super) fn read_motion_nth(
        &self,
        what: motion::Motion,
        how: motion::Reading,
        nth: usize,
    ) -> motion::Span {
        let span = self.read_motion_unwalled(what, how, nth);
        self.walled(what, span)
    }

    /// **The wall round a cell** — the whole of what [`Layer::Inside`] means
    /// (2026-10-21).
    ///
    /// At the middle layer the cell *is* the document: `l` stops at its last
    /// character, `gg` is its first, `$` its end, `f` looks no further than
    /// it, and `d w` cannot reach the delimiter. One clamp, on the way out of
    /// the one funnel every motion comes through, rather than a rule per key
    /// — the three keymap presets spell these differently and neither this
    /// nor `table/layer.rs` may know which preset is loaded. What is walled is
    /// a **motion**, and a motion is a value here (B1).
    ///
    /// Warning: **`j`/`k` are not walled**, and the screen motions are not
    /// either. A step down a column already answers 「which cell」 — it keeps
    /// the one it is in ([`Self::step_grid_row`]) — so it was never the
    /// question the layers exist to settle, and walling it would make `j`
    /// dead inside a one-line cell, which is nearly every cell. Paging is not
    /// here at all: it is not a [`motion::Motion`].
    fn walled(&self, what: motion::Motion, span: motion::Span) -> motion::Span {
        if matches!(what, motion::Motion::Line { .. } | motion::Motion::Screen { .. }) {
            return span;
        }
        let Some((from, to)) = self.cell_walls() else {
            return span;
        };
        match span {
            // **Both ends.** A span is what an operator takes, so clamping
            // only the head would let `d` keep an anchor outside the cell and
            // eat the delimiter from the other side.
            motion::Span::Over { anchor, head } => motion::Span::Over {
                anchor: anchor.clamp(from, to),
                head: head.clamp(from, to),
            },
            // 「nothing there」 stays 「nothing there」: a verb has to be able
            // to tell it from a span of one.
            motion::Span::Missed => span,
        }
    }

    /// **The two places the caret may stand in this cell**, when the keys are
    /// walled inside one — `None` when they are not, which is everywhere
    /// except [`Layer::Inside`].
    ///
    /// Warning: **the far wall is the last grapheme, not the span's end.** A cell's
    /// span runs up to the delimiter, and standing there is standing on the
    /// wall — 全窗 has no column for it, and `x` there would eat the
    /// separator. The exception is Insert, where the caret is a bar between
    /// graphemes and the place after the last character is exactly where the
    /// next one goes; that is the editor's rule everywhere else too.
    fn cell_walls(&self) -> Option<(usize, usize)> {
        if !self.table_layer().walled() || !self.layers_here() {
            return None;
        }
        let (line, cell) = self.cell_position()?;
        let (from, to) = self.cell_span(line, cell)?;
        let last = match self.mode == Mode::Insert || to <= from {
            true => to,
            false => motion::prev_grapheme(self.current_buffer().rope(), to),
        };
        Some((from, last.max(from)))
    }

    /// One position, walled into the cell — the same clamp [`Self::walled`]
    /// puts on a span, for the keys that never make one.
    ///
    /// `h` and `l` are those keys: they step over what a grid keeps off the
    /// page and over the seams between cells (#408), so they do their own
    /// arithmetic rather than asking for a [`motion::Motion`].
    fn in_the_cell(&self, pos: usize) -> usize {
        match self.cell_walls() {
            Some((from, to)) => pos.clamp(from, to),
            None => pos,
        }
    }

    fn read_motion_unwalled(
        &self,
        what: motion::Motion,
        how: motion::Reading,
        nth: usize,
    ) -> motion::Span {
        let rope = self.current_buffer().rope();
        let seg = self.segmenter.as_ref();
        // **落點就是落點**：動了没有、動不了算不算失敗，是**動詞**的問題，不是
        // 動作的。見 `keys.rs::run_vim_step` 裏那一條 `:h exclusive`。
        let at = |p: usize| motion::Span::Over { anchor: p, head: p };
        let caret = how == motion::Reading::Caret;
        match what {
            // Warning: **The one place the two readings really part company.**
            // helix's `w` is the primitive plus 「never just the cell you are
            // on」; vim's is the primitive itself.
            motion::Motion::WordForward(grain) if caret => {
                at(motion::next_word_start(rope, self.sel.head(), grain, seg))
            }
            // Warning: **退不動就是整個動作失敗**（2026-10-02 拿 nvim 量的）：第 1 欄
            // 按 `dge`，nvim 什麼都不做。`ge` 是**包含**的，所以光靠文法那條
            // 「排他的落在原處就是零寬」攔不住它——要動作自己說没動到。
            motion::Motion::WordEndBack(grain) => {
                let here = self.sel.head();
                match motion::prev_word_end(rope, here, grain, seg) {
                    back if back < here => at(back),
                    _ => motion::Span::Missed,
                }
            }
            motion::Motion::WordEndHere(grain) => {
                at(motion::word_end_here(rope, self.sel.head(), grain, seg))
            }
            motion::Motion::WordEnd(grain) if caret => {
                at(motion::next_word_end(rope, self.sel.head(), grain, seg).1)
            }
            motion::Motion::WordBack(grain) if caret => {
                at(motion::prev_word_start(rope, self.sel.head(), grain, seg))
            }
            // **vim's paragraph is a run of lines, ours is one line** (2026-10-18).
            // Same split as the sentence just below, and for the same reason:
            // with soft wrap on, a logical line *is* the paragraph, while vim
            // has no soft wrap and breaks on empty lines. `d}` was 184/184
            // against nvim before this.
            motion::Motion::Paragraph { forward } if caret => {
                match (forward, self.key_preset == yumete_cjk::KeyPreset::Vim) {
                    (true, true) => at(motion::vim_next_paragraph(rope, self.sel.head())),
                    (false, true) => at(motion::vim_prev_paragraph(rope, self.sel.head())),
                    (true, false) => at(motion::next_paragraph(rope, self.sel.head())),
                    (false, false) => at(motion::prev_paragraph(rope, self.sel.head())),
                }
            }
            // **vim 的句子跨行，這個編輯器的不跨**（§5.97，2026-10-18）。兩端各對
            // 各的：`(`/`)` 只有 vim 鍵位按得到，`H`/`L` 只有 helix 鍵位按得到
            // （`keys.rs` 那三支的次序寫明了），所以問一句預設就把兩套分開了，
            // 不必動 #404 那個句子對象。
            motion::Motion::Sentence { forward } if caret => {
                let vim = self.key_preset == yumete_cjk::KeyPreset::Vim;
                match (forward, vim) {
                    (true, true) => at(motion::vim_next_sentence(rope, self.sel.head())),
                    (false, true) => at(motion::vim_prev_sentence(rope, self.sel.head())),
                    (true, false) => at(motion::next_sentence(rope, self.sel.head())),
                    (false, false) => at(motion::prev_sentence(rope, self.sel.head())),
                }
            }
            motion::Motion::WordForward(grain) => {
                motion::word_forward(rope, self.sel.head(), grain, seg)
            }
            motion::Motion::WordEnd(grain) => motion::word_end(rope, self.sel.head(), grain, seg),
            motion::Motion::WordBack(grain) => motion::word_back(rope, self.sel.head(), grain, seg),
            motion::Motion::Find { forward, target, till } => {
                motion::find_char(rope, self.sel.head(), forward, target, till, nth)
            }
            // **The gotos collapse**, so they say so in the span: both ends at
            // the target. A goto is not a selection — 「take me there」, not
            // 「take everything between」 — and that is the same reading vim
            // gives *every* standalone motion (B3).
            // **屏幕的頂/中/底**——整行整行地取，所以答的是那一行的開頭，
            // 同 [`motion::Motion::Line`]。
            motion::Motion::Screen { which } => match self.screen_line(which, nth) {
                Some(line) => at(rope.line_to_char(line.min(rope.len_lines().saturating_sub(1)))),
                None => motion::Span::Missed,
            },
            motion::Motion::FileStart => at(motion::buffer_start(rope, self.sel.head())),
            motion::Motion::FileEnd => at(motion::buffer_end(rope, self.sel.head())),
            motion::Motion::LineStart => at(motion::line_start(rope, self.sel.head())),
            motion::Motion::LineEnd => at(motion::line_last(rope, self.sel.head())),
            motion::Motion::LineFirstNonBlank => {
                at(motion::line_first_non_blank(rope, self.sel.head()))
            }
            // **Paragraphs and sentences keep the same forward rule as words**
            // (B1): it was written three times before this, once per unit.
            motion::Motion::Paragraph { forward } => {
                let vim = self.key_preset == yumete_cjk::KeyPreset::Vim;
                match (forward, vim) {
                    (true, true) => {
                        motion::unit_forward(rope, self.sel.head(), motion::vim_next_paragraph)
                    }
                    (false, true) => {
                        motion::unit_back(rope, self.sel.head(), motion::vim_prev_paragraph)
                    }
                    (true, false) => {
                        motion::unit_forward(rope, self.sel.head(), motion::next_paragraph)
                    }
                    (false, false) => {
                        motion::unit_back(rope, self.sel.head(), motion::prev_paragraph)
                    }
                }
            }
            motion::Motion::Sentence { forward: true } => {
                motion::unit_forward(rope, self.sel.head(), motion::next_sentence)
            }
            motion::Motion::Sentence { forward: false } => {
                motion::unit_back(rope, self.sel.head(), motion::prev_sentence)
            }
            // **One character, and never off this line** — vim's `h`/`l`
            // under an operator. The clamp is the point: `l` on a line's last
            // character answers with that character, not with the newline.
            motion::Motion::Char { forward } => {
                let here = self.sel.head();
                match forward {
                    // Warning: **Forward may stand still and still count.** `l` on a
                    // line's last character cannot move, but `dl` there is
                    // `x` and must take that character — the verb's range is
                    // 「from here, one grapheme past the head」, so a head that
                    // did not move is exactly one character.
                    //
                    // Warning: **帶着動詞的 `l` 可以落到行末的後面一格**，所以這
                    // 裏夾的是 `line_end`（換行那一格）而不是 `line_last`（最後
                    // 一個字）。2026-10-02 拿 nvim 量出來的：`delta` 的 `t` 上按
                    // `d2l` 刪掉 `ta`，夾在 `line_last` 上只刪得掉 `t`。
                    true => at(motion::next_grapheme(rope, here).min(motion::line_end(rope, here))),
                    // …Warning: **and backward may not.** A backward span runs from
                    // the target up to the caret's own character, so a target
                    // that did not move would be 「take the character behind
                    // me」 when there is nothing behind: `dh` in column 0 must
                    // do nothing, the way vim's fails.
                    false => match motion::prev_grapheme(rope, here) {
                        back if back >= here || back < motion::line_start(rope, here) => {
                            motion::Span::Missed
                        }
                        back => at(back),
                    },
                }
            }
            // Warning: **A line motion that cannot move fails; it does not clamp**
            // (2026-10-18 拿 nvim 量出來的). vim's `k` on the first line beeps,
            // and a failed motion takes the whole operator with it — so `dk`
            // there deletes nothing, while clamping made it delete the line
            // the caret is on (184/184 against nvim).
            motion::Motion::Line { down } => {
                let line = rope.char_to_line(self.sel.head());
                let last = rope.len_lines().saturating_sub(1);
                match down {
                    true if line < last => at(rope.line_to_char(line + 1)),
                    false if line > 0 => at(rope.line_to_char(line - 1)),
                    _ => motion::Span::Missed,
                }
            }
            motion::Motion::Object { what: motion::Object::Word { big }, around, count } => {
                self.word_object_span(around, big, count)
            }
            motion::Motion::Object { what: motion::Object::Markup, around, .. } => {
                self.markup_object_span(around)
            }
            motion::Motion::Object { what: motion::Object::Sentence, around, .. } => {
                self.sentence_object_span(around)
            }
            motion::Motion::Object {
                what: motion::Object::Pair { open, close },
                around,
                count,
            } => self.pair_span(open, close, around, count),
            motion::Motion::Object { what: motion::Object::Paragraph, around, .. } => {
                self.paragraph_span(around)
            }
        }
    }

    /// **Go where a span ends, taking nothing** — the caret reading (B1).
    ///
    /// helix has had both readings all along without naming them: `w` takes
    /// what it crosses ([`Self::take_span`]) and `gg` just goes. What B3 adds
    /// is not a second machine — it is **choosing this one** for every
    /// standalone motion, which is what vim does.
    pub(super) fn jump_to(&mut self, span: motion::Span) {
        if let Some(head) = span.head() {
            self.move_head(head);
        }
    }

    /// **Take what a motion asked for** — the selection-first reading of a
    /// [`motion::Span`], and the only one helix needs (B1, 2026-09-20).
    ///
    /// This is the half of the grammar layer that belongs to the editor: a
    /// span says *where*, and this says what a selection-first editor does
    /// with it — set both ends, clamp to what the page really draws, and leave
    /// the anchor alone while extending. vim's reading (take one end, move the
    /// caret, paint nothing) is the other consumer, and it is B3's.
    ///
    /// Warning: [`motion::Span::Missed`] does nothing at all — **not** a collapse.
    /// A verb must be able to tell 「nothing there」 from 「a span of one」.
    pub(super) fn take_span(&mut self, span: motion::Span) {
        match span {
            motion::Span::Over { anchor, head } => self.select_span(anchor, head),
            motion::Span::Missed => {}
        }
    }



    /// Step forward one word, selecting it (`w` / `W`).
    ///
    /// The selection runs from here to *just before* the next word begins — the
    /// character that starts the next word belongs to the next `w`. But a word
    /// of one character (which, with the default segmenter, is every 漢字) ends
    /// where it starts, and stopping just before the next one would leave the
    /// cursor exactly where it was: `w` would not move at all. So when taking
    /// this word cannot advance, `w` takes the next one instead — which is also
    /// what vi's `w` does.
    pub(super) fn select_word_forward(&mut self, big: bool) {
        // **The rule itself lives in `motion`** (B1, 2026-09-20): a motion is a
        // value now, so the one written here can be read by a second grammar
        // without being replayed as keys. What is left here is the question
        // only an editor can answer — which dictionary, and at what grain.
        let grain = match big {
            true => motion::Grain::Big,
            false => self.word_grain(),
        };
        let span = self.run_motion(motion::Motion::WordForward(grain));
        self.take_span(span);
    }

    /// Set the cursor, always collapsing the selection, and refresh the goal
    /// column. Used when entering Insert mode and after a search jump.
    pub(super) fn set_cursor(&mut self, pos: usize) {
        self.sel.set_head(pos);
        self.sel.set_anchor(pos);
        self.refresh_goal_column();
    }

    /// Clamp the cursor and anchor into the valid range of the active buffer.
    pub(super) fn clamp_cursor(&mut self) {
        let len = self.current_buffer().char_count();
        // Warning: **每一段，不只是主選區**（2026-09-28 修，真機上崩出來的）。
        //
        // 撤銷只把主選區挪了回來（`undo` 那一支叫的是 `set_head`/`set_anchor`，那兩支
        // 問的永遠是主選區），剩下幾段還指着已經不存在的位置。下一幀 `draw_horizontal`
        // 拿它們去切 rope，`next_grapheme` 當場 panic，整個編輯器退出。
        //
        // 日誌裏那一條：`Char index out of bounds: char index 4, Rope char length 0`，
        // 棧是 `motion::right` ← `next_grapheme` ← `secondary_selections` ←
        // `draw_horizontal`。
        self.sel.clamp(len);
    }
}

/// **那「一對括號」其實是 markdown 自己的標記嗎** —— `md` 挑內層時的例外。
///
/// `[[條目]]` 裏的內層 `[`…`]` 起點比整個構造晚，照「取內層」的規矩它會贏；可它
/// 不是一對括號，是 `[[`/`]]` 的各一半。摘掉它只脫一層殼，剩下 `[條目]`——一句
/// 壞掉的語法（2026-10-04 量出來的）。
///
/// 判準：兩個端點**都**落在構造的標記那兩段裏（`[外左,內左)` 與 `[內右,外右)`）。
/// `(**粗**)` 不中——那對括號在標記外面，是真的一對。
fn pair_is_made_of_marks(
    pair: (usize, usize, usize, usize),
    markup: (usize, usize, usize, usize),
) -> bool {
    let (open, _, close, _) = pair;
    let (out, inn, inn_end, out_end) = markup;
    (out..inn).contains(&open) && (inn_end..out_end).contains(&close)
}

// Warning: **測試模組擺在檔尾。** `messages.rs` 那張「每個標籤都有條目」的網把源碼切在
// 第一個頂格的 `#[cfg(test)]\nmod ` 處，擺在檔案中間它後面所有的 `say!` 都從網裏消失。
#[cfg(test)]
mod tests {
    use super::*;

    /// 把 `text` 放進一份新的緩衝，光標坐在第 `at` 格上。
    fn at_char(text: &str, at: usize) -> Editor {
        let mut ed = Editor::new();
        ed.current_buffer_mut().replace(0..0, text).unwrap();
        ed.sel.collapse_to(at);
        ed
    }

    /// **三族括號各問一次**，兩頭都認：站在開的那一半上往後找，站在閉的那一半上往前找。
    #[test]
    fn the_partner_of_the_bracket_under_the_cursor() {
        assert_eq!(at_char("a(bc)d", 1).matching_bracket(), Some(4), "( 的另一半");
        assert_eq!(at_char("a(bc)d", 4).matching_bracket(), Some(1), ") 的另一半");
        assert_eq!(at_char("a[bc]d", 1).matching_bracket(), Some(4), "[ 的另一半");
        assert_eq!(at_char("a[bc]d", 4).matching_bracket(), Some(1), "] 的另一半");
        assert_eq!(at_char("a{bc}d", 1).matching_bracket(), Some(4), "花括號的另一半");
        assert_eq!(at_char("a{bc}d", 4).matching_bracket(), Some(1), "花括號，從閉的那一半");
        // 中文稿子裏的括號也一樣——`PAIRS` 十九對全收。
        assert_eq!(at_char("他說（不）了", 2).matching_bracket(), Some(4), "全角括號的另一半");
    }

    /// **巢狀的數層數**：外面那一對配外面那一對。
    #[test]
    fn nesting_is_counted_not_guessed() {
        // 套着的：(a(b)c)
        let text = "(a(b)c)";
        assert_eq!(at_char(text, 0).matching_bracket(), Some(6), "外層");
        assert_eq!(at_char(text, 6).matching_bracket(), Some(0), "外層，從閉的那一半數回去");
        assert_eq!(at_char(text, 2).matching_bracket(), Some(4), "內層");
        assert_eq!(at_char(text, 4).matching_bracket(), Some(2), "內層，從閉的那一半數回去");
        // 並排的：((a)(b))——第 1 格那個 `(` 配的是第 3 格，不是最外面那一個。
        let text = "((a)(b))";
        assert_eq!(at_char(text, 0).matching_bracket(), Some(7), "最外層");
        assert_eq!(at_char(text, 1).matching_bracket(), Some(3), "並排的第一對");
        assert_eq!(at_char(text, 4).matching_bracket(), Some(6), "並排的第二對");
        assert_eq!(at_char(text, 7).matching_bracket(), Some(0), "從最外面那一半數回去");
    }

    /// **光標不在括號上就什麼都不說。** 這一條是整件事的閘：答了就有一格被塗成金的。
    #[test]
    fn an_ordinary_character_has_no_partner() {
        assert_eq!(at_char("a(bc)d", 0).matching_bracket(), None, "字母 a");
        assert_eq!(at_char("a(bc)d", 2).matching_bracket(), None, "括號裏面的 b");
        assert_eq!(at_char("a(bc)d", 5).matching_bracket(), None, "字母 d");
        assert_eq!(at_char("", 0).matching_bracket(), None, "空緩衝");
        // 配不上的括號也没有另一半——掃到底，答 `None`。
        assert_eq!(at_char("a(bc", 1).matching_bracket(), None, "没有閉的那一半");
        // 引號的兩半長得一樣，說不出另一半在哪一邊（見那一支的 Warning）。
        assert_eq!(at_char("a\"bc\"d", 1).matching_bracket(), None, "半角雙引號");
    }

    /// **量一量：最壞的那一檔有多貴。**
    ///
    /// 一幀問一次，所以要緊的是「配不上的括號」那一檔——它要從光標一路掃到檔尾。
    /// 2026-10-08 量的那個數記在 `matching_bracket` 的註釋裏；留着這一支是為了重跑得了。
    #[test]
    #[ignore = "量數用的，不是斷言"]
    fn how_long_does_the_worst_case_take() {
        let text = format!("({}", "字".repeat(400_000));
        let ed = at_char(&text, 0);
        let began = std::time::Instant::now();
        let found = ed.matching_bracket();
        let took = began.elapsed();
        assert_eq!(found, None, "配不上，所以整條掃到底了");
        println!("四十萬字掃到底：{took:?}");
    }
}
