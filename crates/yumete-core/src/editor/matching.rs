//! Pairs, and moving the cursor (#296).
//!
//! Helix's `m`: jump to the matching bracket, select the pair, add or drop or
//! change what surrounds a selection. The cursor moves live here too — the
//! goal column, the 縱書 axis, and the clamp that keeps a caret on a character
//! boundary — because every one of them is asked for by a `m`/`w`/`h` key.

use super::*;

impl Editor {
    // ---- Match mode (Helix `m`) -------------------------------------------

    /// Link to the bracket matching the one under the cursor (`mm`).
    pub(super) fn goto_matching_bracket(&mut self) {
        let rope = self.current_buffer().rope();
        if self.sel.head() >= rope.len_chars() {
            return;
        }
        let here = rope.char(self.sel.head());
        let target = if let Some(close) = closing_of(here) {
            find_forward(rope, self.sel.head(), here, close)
        } else if let Some(open) = opening_of(here) {
            find_backward(rope, self.sel.head(), open, here)
        } else {
            None
        };
        if let Some(pos) = target {
            self.move_head(pos);
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
            'w' => motion::Object::Word { coarse: false },
            'W' => motion::Object::Word { coarse: true },
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
            // 在這裏是同一段：一個定義沒有「連着外面那層」可言。
            //
            // Warning: **類是 `t` 不是 `c`**（2026-10-06 當天改正）。先按記憶綁成了
            // `c`，而 helix 的 `c` 是**註釋**、`t` 纔是類
            // （`commands.rs:6307-6310`：`'t' => "class"`、`'c' => "comment"`）。
            // 照抄參考實現，不照記憶。
            'f' | 't' => return self.select_definition(c == 'f'),
            // **`mi a` 參數、`mi c` 註釋**（2026-10-06，helix 的同兩個字母）。用的
            // 是我們自己寫的那份查詢（`code::Language::objects`），不是 helix 的
            // `textobjects.scm`——授權的事見那裏。
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
        let span = self.run_motion(motion::Motion::Object { what, around });
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
    fn pair_span(&self, open: char, _close: char, around: bool) -> motion::Span {
        let rope = self.current_buffer().rope();
        // Warning: **一個鍵管一族括號**（2026-09-28）：按 `(` 找得到 `()` 也找得到 `（）`，
        // 按 `[` 連 `【】`『』一起找。取**最裏面**那一對——`【他說（不）】` 裏光標在
        // 「不」上按 `di[`，要的是 `（）` 還是 `【】`？答案跟 `md` 一致：最裏面那一對。
        // 這張族表在 `editor.rs` 的 `FAMILIES`，理由記在那裏。
        let Some((start, end)) = super::pair_family(open)
            .into_iter()
            .filter_map(|(open, close)| surrounding(rope, self.sel.head(), open, close))
            .max_by_key(|&(start, _)| start)
        else {
            return motion::Span::Missed;
        };
        // `end` is the closing bracket's own index. The head goes on the last
        // character the selection covers, not one past it — the cursor's
        // grapheme is inside the selection.
        let (anchor, head) = match around {
            true => (start, end),
            false => (start + 1, end.saturating_sub(1)),
        };
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
    /// 沒有內層的 `Code`。所以這裏「取起點最靠後的那一個」現在永遠只有一個候選。留着這
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
        // 收哪幾種：**有一對標記裹着一段文字**的那些。標題不在裏面（它沒有閉標記，
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
    /// 行（中文稿子的常態）的時候這沒有分別；硬折過行的稿子裏，`mi s` 取的是這一行裏
    /// 的那一句。
    fn sentence_object_span(&self, around: bool) -> motion::Span {
        let rope = self.current_buffer().rope();
        let head = self.sel.head().min(rope.len_chars());
        let line = rope.char_to_line(head);
        let start = rope.line_to_char(line);
        let chars = crate::zong::line_chars(rope, line);
        if chars.is_empty() {
            return motion::Span::Missed;
        }
        let at = (head - start).min(chars.len().saturating_sub(1));
        let starts = motion::sentence_starts(&chars);
        let Some(which) = starts.iter().rposition(|&s| s <= at) else {
            return motion::Span::Missed;
        };
        let mut from = starts[which];
        let to = starts.get(which + 1).copied().unwrap_or(chars.len());
        let mut end = to;
        match around {
            // `as`：連句末那一段空白。Warning: 中文句子之間沒有空白，所以這一支在中文裏和
            // `is` 拿到同一段——vim 那條規矩在沒有空白的文字裏的自然結果，不是算錯。
            // 後面沒有空白就取前面的，同 `aw`。
            true => {
                if end == to && to == from {
                    return motion::Span::Missed;
                }
                if !(from..to).any(|i| chars[i].is_whitespace()) || to == chars.len() {
                    while from > 0 && chars[from - 1].is_whitespace() {
                        from -= 1;
                    }
                }
            }
            // `is`：句子本身，句末那一段空白不要。
            false => {
                while end > from && chars[end - 1].is_whitespace() {
                    end -= 1;
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
    /// 沒有空白，於是 `daw` 和 `diw` 拿到同一段。這是 vim 那條規矩在中文裏的自然結果，
    /// 不是這一支算錯了。
    fn word_object_span(&self, around: bool, coarse: bool) -> motion::Span {
        let rope = self.current_buffer().rope().clone();
        let line = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        let start = rope.line_to_char(line);
        let grain = match coarse {
            true => crate::motion::Grain::Coarse,
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
            let mut b = self.sel.head();
            while b < line_end && rope.char(b).is_whitespace() {
                b += 1;
            }
            if around {
                if let Some(&(_, end)) = words.iter().find(|&&(s, _)| s >= b) {
                    b = end;
                }
            }
            Some((a, b))
        }) else {
            return motion::Span::Missed;
        };
        // 空白那一支已經把 `around` 算進去了，下面那一段只管詞本身。
        let around = around && here.is_some();
        let (mut a, mut b) = (from, to.saturating_sub(1).max(from));
        if around {
            let line_end = start + crate::zong::line_chars(&rope, line).len();
            let mut at = to;
            while at < line_end && rope.char(at).is_whitespace() {
                at += 1;
            }
            match at > to {
                // 後面有空白：連它一起。
                true => b = at.saturating_sub(1),
                // 没有就取前面那一段——`daw` 在一行的末尾也該把那道縫帶走。
                false => {
                    while a > start && rope.char(a - 1).is_whitespace() {
                        a -= 1;
                    }
                }
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
            // 兩段一樣寬 ＝ 這個構造沒有標記可刪。
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
            // 上答「外面沒有成對的符號」，而 `mi(` 選得中。vim 的 `ds[` 在中文稿
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
        // 2026-10-06 補的：從前只認具體符號，`mrm[` 一點反應都沒有。
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

    /// Recompute the goal column `j` and `k` aim at.
    ///
    /// With soft wrap on it is the column within the *visual row*, not within
    /// the paragraph — otherwise `j` from the middle of a wrapped line would
    /// aim at a column hundreds of cells wide and always land at a row's end.
    pub(super) fn refresh_goal_column(&mut self) {
        // The measure borrows the editor — a row's width depends on what is on
        // the page — so it is built here and dropped before anything is set.
        let column = {
            let hide = |line: usize| self.hidden_on_line(line);
            let fold = |line: usize| self.line_is_folded(line);
            let rope = self.current_buffer().rope();
            // …with the indent, because the indent is where a row *breaks*: a
            // measure without it wraps a different page from the one being
            // drawn, and `j` then lands on the character under a column nobody
            // is looking at. Same for the folds: a row the page does not draw
            // is a row `j` must not stop on.
            //
            // **Wrap off goes the same way**, at [`crate::wrap::NO_WRAP`]: one
            // row per paragraph is what a very large width gives, and the
            // alternative was a second answer to「which column is this」 that
            // did not know what is off the page.
            let width = self.wrap_width().unwrap_or(crate::wrap::NO_WRAP);
            let drawn = |line: usize| self.drawn_on_line(line);
            let typed = |line: usize| self.typed_on_line(line);
            // A table row is one row (#275) — and `j` has to be walking the
            // same page the renderer drew, or it steps into a row that is not
            // on the screen.
            let flat = |line: usize| self.table_row_at(line);
            let m = crate::wrap::Measure::new(width, &hide)
                .with_indent(self.paragraph_indent())
                .with_folds(&fold)
                .with_drawn(&drawn)
                .with_typed_drawn(&typed)
                .with_unwrapped(&flat)
                .with_version(self.current_buffer().id(), self.current_buffer().revision())
                .with_edit(self.current_buffer().edit())
                .with_open_line(self.open_line())
                .with_caret(Some(self.caret_in_line()));
            crate::wrap::column_of(rope, self.sel.head(), m)
        };
        self.sel.set_goal(Some(column));
    }

    /// `j`/`k` inside a grid, by cell rather than by screen column.
    ///
    /// Answers whether it took the step: only in a table, only while `hjkl`
    /// are walking characters (by cell is [`Self::move_cell_row`]'s job), and
    /// only when there is a row that way to go to — at the edges the ordinary
    /// path takes over, so walking out of a table into the prose still works.
    fn step_grid_row(&mut self, up: bool) -> bool {
        let grain = self.table.as_ref().map(|view| view.grain);
        if !self.table_here() || grain != Some(Grain::Char) {
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
        let Some((a, b)) = self.cell_span(want, cell) else {
            return false;
        };
        self.move_head((a + into).min(b));
        true
    }

    /// Apply a horizontal motion, moving the head (extending if in select mode).
    pub(super) fn move_horizontal(&mut self, motion: fn(&ropey::Rope, usize) -> usize) {
        let pos = motion(self.current_buffer().rope(), self.sel.head());
        let pos = self.past_what_a_table_keeps_off(pos, pos > self.sel.head());
        self.move_head(self.on_this_line_under_vim(pos));
    }

    /// **vim 鍵位下 `h`/`l` 不出這一行**（`:h l`；2026-10-02 定照參考實現）。
    ///
    /// Warning: **要緊的是行末那一格坐不上去。** vim 的普通模式光標停不到換行符
    /// 上，而 yumete 照 helix 的規矩停得上去——於是在一行的最後一個字上按 `x`，
    /// 吃掉的是換行，兩行焊成一行。2026-10-02 拿 nvim 逐欄比，這是最後一條兩家
    /// 真的不一樣的地方（中文分詞那三格除外）。
    ///
    /// 跨行一併擋住，那也是 vim 自己的規矩（`whichwrap` 出廠不含 `<`、`>`）。
    /// helix 鍵位照舊：那一邊「走一頁」是有意的，記在上面 `h`/`l` 那一條。
    fn on_this_line_under_vim(&self, pos: usize) -> usize {
        if self.key_preset != yumete_cjk::KeyPreset::Vim {
            return pos;
        }
        let rope = self.current_buffer().rope();
        let here = self.sel.head();
        pos.clamp(motion::line_start(rope, here), motion::line_last(rope, here))
    }

    /// Step `at` clear of anything a table is keeping off the page, in the
    /// direction of travel (#379).
    ///
    /// **A step the reader cannot see is not a step.** Under `t f` the file's
    /// own padding comes off the page (`cell_slack_against`), so `l` through a
    /// cell's trailing spaces moved the caret and moved nothing on the screen:
    /// it sat still for four presses and then jumped a column.
    /// 2026-09-11：「既然没有显示，就应该允许用户直接跳过去。」
    ///
    /// Only what a table keeps — see [`Editor::cell_hidden_on_line`]. Markup
    /// hidden by 所見即所得 comes back for the caret, so walking into it is
    /// the point rather than a mistake.
    fn past_what_a_table_keeps_off(&self, at: usize, forward: bool) -> usize {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(at.min(rope.len_chars()));
        let start = rope.line_to_char(line);
        let mut hidden = self.cell_hidden_on_line(line);
        // **And the seams between the cells** (2026-09-12). A `|` and the
        // spaces around it are three characters of the file drawn as one `┆`,
        // so a caret parked in there is a caret the reader cannot see: press
        // `l` at the end of a cell and it sits still, press it again and it is
        // suddenly in the next column. `w` made it worse by landing there in
        // one press. Same rule as the padding above — **a step the reader
        // cannot see is not a step** — and the same rule the grid itself lives
        // by: `row_cells` already leaves the padding out of a cell's span, so
        // whatever falls between two spans is seam.
        // Warning: **A `|` table only.** There the separator is three characters of
        // the file — `space pipe space` — drawn as one `┆`, so a caret in it is
        // a caret nowhere. A delimited file's separator is a single comma or
        // tab, and its **empty cells sit at the same offset as the separator
        // after them** (`一,,木目`: the blank cell and the second comma are both
        // at 2), so hiding seams there costs the one thing a table editor is
        // most for — putting the caret in a blank cell to fill it in.
        if self.grid_is_drawn() && self.grid_separator_is_a_pipe() {
            // **The wall itself, and the space either side of it.** Not the
            // whole gap between two cell spans — `row_cells` reports content
            // only, so that gap also holds the cell's own padding, and padding
            // is the cell's (`cell_hidden_on_line` above already says which of
            // it is off the page). What is drawn as one rule is exactly
            // `space? | space?`, and that is what is hidden.
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
                hidden.push((from, upto));
            }
            hidden.sort_unstable();
        }
        if hidden.is_empty() {
            return at;
        }
        // Spans do not overlap and there are a handful of them, so walking
        // them is a loop over the row's cells at worst.
        let mut at = at;
        while let Some(&(from, upto)) = hidden
            .iter()
            .find(|&&(from, upto)| (from..upto).contains(&(at - start)))
        {
            match forward {
                true => at = start + upto,
                // The character *before* the span: its first is still hidden.
                false if from == 0 => return start,
                false => at = start + from - 1,
            }
        }
        at
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
            let m = crate::wrap::Measure::new(width, &hide)
                .with_indent(self.paragraph_indent())
                .with_folds(&fold)
                .with_drawn(&drawn)
                .with_typed_drawn(&typed)
                .with_unwrapped(&flat)
                .with_version(self.current_buffer().id(), self.current_buffer().revision())
                .with_edit(self.current_buffer().edit())
                .with_open_line(self.open_line())
                .with_caret(Some(self.caret_in_line()));
            // Warning: **目標列現在在這一段自己身上**（#405）。沒記過就現算一次——新長出來
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
        let to = self.past_what_a_table_keeps_off(to, to > self.sel.head());
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
        let rope = self.current_buffer().rope();
        let seg = self.segmenter.as_ref();
        // **落點就是落點**：動了沒有、動不了算不算失敗，是**動詞**的問題，不是
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
            // 「排他的落在原處就是零寬」攔不住它——要動作自己說沒動到。
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
            motion::Motion::Paragraph { forward } if caret => match forward {
                true => at(motion::next_paragraph(rope, self.sel.head())),
                false => at(motion::prev_paragraph(rope, self.sel.head())),
            },
            motion::Motion::Sentence { forward } if caret => match forward {
                true => at(motion::next_sentence(rope, self.sel.head())),
                false => at(motion::prev_sentence(rope, self.sel.head())),
            },
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
            motion::Motion::Paragraph { forward: true } => {
                motion::unit_forward(rope, self.sel.head(), motion::next_paragraph)
            }
            motion::Motion::Paragraph { forward: false } => {
                motion::unit_back(rope, self.sel.head(), motion::prev_paragraph)
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
            motion::Motion::Line { down } => {
                let line = rope.char_to_line(self.sel.head());
                let last = rope.len_lines().saturating_sub(1);
                let want = match down {
                    true => (line + 1).min(last),
                    false => line.saturating_sub(1),
                };
                at(rope.line_to_char(want))
            }
            motion::Motion::Object { what: motion::Object::Word { coarse }, around } => {
                self.word_object_span(around, coarse)
            }
            motion::Motion::Object { what: motion::Object::Markup, around } => {
                self.markup_object_span(around)
            }
            motion::Motion::Object { what: motion::Object::Sentence, around } => {
                self.sentence_object_span(around)
            }
            motion::Motion::Object {
                what: motion::Object::Pair { open, close },
                around,
            } => self.pair_span(open, close, around),
            motion::Motion::Object { what: motion::Object::Paragraph, around } => {
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
