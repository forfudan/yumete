//! Search and substitute (#14 / #15).

use super::*;

impl Editor {
    // ---- Search (Feature #14) ---------------------------------------------

    /// Compile a search or substitution pattern, remembering the last one.
    ///
    /// Patterns are **regular expressions**, as they are in vi and Helix: half
    /// the work of revising a manuscript is a pattern rather than a string —
    /// 「行首的『他說』」, 「連續兩個以上的驚嘆號」, 「每個。後面斷行」. The
    /// cost is that `.` `*` `(` mean something; `\.` is a full stop.
    ///
    /// `n` and `N` ask for the same pattern over and over, so the compiled form
    /// is kept until the pattern changes.
    pub(super) fn compile(&self, pattern: &str) -> Result<Regex, String> {
        self.compile_exactly(&self.smart_cased(pattern))
    }

    /// [`Self::compile`]，但**不走**智能大小寫 —— `:s` 走這一條（2026-10-09 作者定）。
    ///
    /// 原話：「我覺得替換不應該智能大小寫的，因為本質他是個正則，既然是正則就應該
    /// 默認嚴格。`/` 放寬了智能大小寫，我們必須說明。」
    ///
    /// 量過三家：helix 的 `/` 是智能大小寫（`search.smart-case` 出廠 `true`，書裏的
    /// 原話是「case-insensitive unless pattern contains upper case characters」，和
    /// 這一頭逐字相同），而 **helix 根本沒有 `:s`**；vim 的 `/` 與 `:s` 兩個都分大小
    /// 寫。所以 `:s` 這一格沒有 helix 的答案可抄，而 vim 的答案是「分」。
    ///
    /// 放寬那一次要在 `:s/…/…/i` 裏明說。
    pub(super) fn compile_exactly(&self, pattern: &str) -> Result<Regex, String> {
        // **Keyed on the pattern that is actually compiled**, not on what was
        // typed: `(?i)` is part of it, so `/todo` and `/TODO` are two entries
        // and never hand each other their answer.
        if let Some((cached, re)) = self.compiled.borrow().as_ref() {
            if cached == pattern {
                return Ok(re.clone());
            }
        }
        match Regex::new(pattern) {
            Ok(re) => {
                *self.compiled.borrow_mut() = Some((pattern.to_string(), re.clone()));
                Ok(re)
            }
            // The writer needs to know *which* part of their pattern is wrong,
            // and regex's own message says so; its multi-line form does not fit
            // a status line.
            Err(err) => Err(format!(
                "bad pattern: {}",
                err.to_string().lines().last().unwrap_or("").trim()
            )),
        }
    }

    /// Smart case (#301): a pattern with no capital in it ignores case.
    ///
    /// Typing a capital is how you ask for the case to matter — nothing else
    /// has to be turned on or off, and the whole rule is one line to explain.
    /// `(?-i)` in front of the pattern is the way to say 「lower case, and mean
    /// it」; `[editor] smart_case = false` turns the rule off for good.
    fn smart_cased(&self, pattern: &str) -> String {
        match self.smart_case && !pattern.chars().any(char::is_uppercase) {
            true => format!("(?i){pattern}"),
            false => pattern.to_string(),
        }
    }

    /// Where the panel's 模糊 switch starts — `[editor] fuzzy_search`.
    ///
    /// Only the starting position: the switch itself is on the panel, and
    /// `:replace` puts it down whatever this says.
    pub fn set_fuzzy_search(&mut self, on: bool) {
        self.search.fuzzy = on;
    }

    /// Turn smart case off (or back on) — `[editor] smart_case`.
    pub fn set_smart_case(&mut self, on: bool) {
        self.smart_case = on;
        // The cache is keyed on the compiled pattern, and the rule that builds
        // it has just changed underneath it.
        *self.compiled.borrow_mut() = None;
    }

    /// Search for [`Self::last_search`] in `forward` direction and move there.
    /// The lines a search may land in: the table's own rows while the grid
    /// holds the pane, and the whole buffer otherwise (#354).
    fn search_rows(&self) -> std::ops::Range<usize> {
        let all = 0..self.current_buffer().rope().len_lines();
        if !self.table.as_ref().is_some_and(|view| view.takes_the_pane()) {
            return all;
        }
        match self.table_row_span() {
            Some((first, last)) => first..last + 1,
            None => all,
        }
    }

    pub(super) fn repeat_search(&mut self, forward: bool) {
        // `n` with nothing to repeat used to do nothing and say nothing, which
        // reads as a key that is broken rather than one with no answer yet.
        if self.last_search.is_empty() {
            self.status = say!("find.nothing-searched-yet");
            return;
        }
        // Jumping back after a search is the whole reason `C-o` exists: you
        // look something up, and you want to be back where you were writing.
        self.remember_jump();
        let pattern = self.last_search.clone();
        let re = match self.compile(&pattern) {
            Ok(re) => re,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();

        // Line by line, not over the whole buffer: a pattern cannot contain a
        // newline (Enter submits the prompt), so a match never straddles a line
        // break, and materialising the document for every `n` costs an 800 KB
        // copy on a novel.
        // **Where a hit is allowed to be** (#354). With the grid holding the
        // pane the cursor is held inside the table (`tables.rs`), so a hit
        // outside it is one `n` can never reach — the reader watches the
        // search find something and the cursor refuse to go, and `n` stops
        // going round. The rows of the table are the whole document as far as
        // this search is concerned.
        let within = self.search_rows();
        let scoped = within != (0..rope.len_lines());
        // Warning: **往回找要從這一處的開頭數起，不是從光標數起**（2026-09-24 報的：
        // 「N 向上搜索這個快捷鍵無效」）。一次搜索落地之後光標停在匹配的**最後
        // 一個字**上，開頭在 `anchor`；而 `search_backward` 找的是「開頭在
        // `from` 之前的最後一處」，拿光標去問，當前這一處的開頭就在光標之前，
        // 於是它把自己又找了一遍，光標紋絲不動。
        //
        // Warning: **匹配只有一個字的時候它是好的**——那時光標就在開頭上。中文搜的多
        // 半是兩個字以上，英文搜一個字母的多，所以這個洞躲了很久。
        let found = if forward {
            search_forward(rope, &re, (self.sel.head() + 1).min(len), within)
        } else {
            search_backward(rope, &re, self.sel.anchor().min(self.sel.head()), within)
        };

        match found {
            // The match itself becomes the selection. Every motion leaves one —
            // that is the first thing the manual says about this editor — and a
            // search that only moved the cursor made `/` the one motion after
            // which `d` did something other than what the screen showed.
            Some((pos, end)) => {
                // On the match's last grapheme, not one past it — the selection
                // covers the cursor's own grapheme.
                let end = end.min(len);
                let head = motion::prev_grapheme(rope, end).max(pos);
                // **選區模式裏搜索是延伸，不是跳走**（2026-10-06，對齊 helix）。
                // helix 的 select 下 `n`/`N` 綁的是 `extend_search_next`/`_prev`
                // （`default.rs:359-360`），而 `/` 的 `search` 在 select 下走
                // `Movement::Extend`、**模式不變**（`commands.rs` 的 `search_impl`）。
                // 從前這裏無條件 `self.extend = false`，於是 `v` 之後按一下 `/`
                // 就掉回 Normal，選區也沒了——實測 `v w w Esc` 之後 `d` 只刪一個
                // 空格。錨點留着，頭走到這一處**去的那一頭**：往前是最後一個字，
                // 往回是第一個字。
                match self.extend {
                    true => self.sel.set_head(match forward {
                        true => head,
                        false => pos,
                    }),
                    false => {
                        self.sel.set_anchor(pos);
                        self.sel.set_head(head);
                    }
                }
                self.refresh_goal_column();
            }
            // Warning: **說給人看的式子裏不許有我們自己加的那一段**（2026-10-09 審出
            // 來的）。智能大小寫把 `(?i)` 寫進式子裏（`smart_cased`，那是正則引擎自
            // 己的旗標，不另外實現一套），而面板那一路把穿好衣服的那一整條存進了
            // `last_search`——於是找不到的時候狀態欄寫着「找不到：(?i)zhongguo」。那
            // 四個字符不是打的人寫的。
            None if scoped => self.status = say!("find.not-in-table", as_typed(&pattern)),
            None => self.status = say!("find.not-found", as_typed(&pattern)),
        }
    }

    // ---- Substitute (Feature #15) -----------------------------------------

    /// Replace `pattern` with `replacement` on the cursor's line, or on every
    /// line when `whole_file`; `global` replaces every match on a line.
    pub(super) fn substitute(&mut self, how: Substitution<'_>) {
        if self.refuse_readonly() {
            return;
        }
        let Substitution {
            pattern,
            replacement,
            global,
            ignore_case,
            literal,
            confirm,
            count_only,
            reshape,
            rows,
        } = how;
        if pattern.is_empty() {
            self.status = say!("find.empty-pattern");
            return;
        }
        // **`f` — 照字面.** A manuscript is full of characters the regex engine
        // reads as instructions: `。` is safe, but `(注)`, `A.B`, `第1章*` and
        // every URL are not, and the writer who wants those characters had to
        // know which ones to backslash. `f` says 「these are the characters」
        // and does the escaping.
        let wanted = match literal {
            true => regex::escape(pattern),
            false => pattern.to_string(),
        };
        // `i` is the regex engine's own flag, so it is written into the
        // pattern rather than reimplemented here.
        let cased = match ignore_case {
            true => format!("(?i){wanted}"),
            false => wanted,
        };
        // **替換不走智能大小寫**（見 [`Self::compile_exactly`]）：`i` 旗標是放寬那
        // 一次的唯一辦法，而 `cased` 上面已經按它寫好了。
        let re = match self.compile_exactly(&cased) {
            Ok(re) => re,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        // The right-hand side goes literal too: with the pattern escaped there
        // are no groups for a `$1` to name, so a `$` can only be the writer's
        // own — a price, a shell line, a LaTeX formula. `\n` and `\t` still
        // resolve, because Enter submits the prompt and there is no other way
        // to type them.
        let replacement = match literal {
            true => unescape_replacement(replacement).replace('$', "$$"),
            false => unescape_replacement(replacement),
        };

        let text = self.current_buffer().text();
        let chosen = match self.substitution_rows(rows) {
            Ok(chosen) => chosen,
            Err(why) => {
                self.status = why;
                return;
            }
        };
        // **`c` — 逐處確認** (#415): 「防止一下子全部都替换了」. `n` still wins
        // when both were written: it says change nothing, which is the safer
        // of the two readings of a line that asks for both.
        if confirm && !count_only {
            self.start_confirming(re, replacement, global, chosen, reshape);
            return;
        }
        let mut count = 0usize;
        let mut rebuilt = String::with_capacity(text.len());

        for (idx, line) in text.split_inclusive('\n').enumerate() {
            if chosen.has(idx) {
                // 換行符不進正則，原樣寫回去（CRLF 也原樣）——見
                // [`crate::editor::line_body`]。
                let (body, eol) = crate::editor::line_body(line);
                let (new_line, n) = replace_in_line(body, &re, &replacement, global);
                count += n;
                rebuilt.push_str(&new_line);
                rebuilt.push_str(eol);
            } else {
                rebuilt.push_str(line);
            }
        }

        // A substitution rewrites whole lines, so the cell guard cannot judge it
        // character by character. What it can check is the thing the guard
        // exists to protect: that no row gained or lost a cell.
        if count > 0 && !reshape {
            if let Some(why) = self.substitution_breaks_the_grid(&rebuilt) {
                self.status = why;
                return;
            }
        }
        // `n` in vi means "count, and change nothing". It used to substitute.
        if count_only {
            self.status = say!("find.substitute-nothing-changed", count);
            return;
        }
        // Warning: **配中了不等於改了**（2026-10-09 審出來的）。零寬的式子（` *$`、
        // `x*`）每一行都配得中，而換上去的和換下來的一模一樣；從前照樣記一個撤銷點、
        // 掛上 `[+]`、`:w` 真的寫一遍盤。報的處數不動（vim 也報配中幾處），只是檔沒
        // 變就什麽都不做。
        if count > 0 && rebuilt != text {
            self.snapshot();
            let len = self.current_buffer().char_count();
            let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(0..len, &rebuilt));
            if !self.applied(done) {
                return;
            }
            self.clamp_cursor();
            self.sel.set_anchor(self.sel.head());
            self.refresh_goal_column();
        }
        self.status = say!("find.substitute-changed", count);
    }

    /// **這個範圍是反的嗎** —— 是就交寫的那兩個行號（2026-10-09）。
    ///
    /// 只有 `Rows::Span` 可能反（`,` 那一種是一張單子，順序不說明什麽）。交的是
    /// **讀者寫的那兩個數**，所以 `:$-1s` 在三行的檔上報的是「第 3 行到第 1 行」，
    /// 而不是 `$`。對調本身照舊由 [`Self::substitution_rows`] 做。
    ///
    /// 行號越界的那一種不在這裏答——那一支是 [`Self::substitution_rows`]，它報
    /// 「沒有第 N 行」，而一個不存在的行談不上方向。
    pub(super) fn a_backwards_range(&self, rows: &crate::command::Rows) -> Option<(usize, usize)> {
        use crate::command::{Bound, Rows};
        let Rows::Span(a, b) = rows else { return None };
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        let shown = |b: &Bound| match b {
            Bound::Line(n) => *n,
            Bound::Cursor => rope.char_to_line(self.caret().min(rope.len_chars())) + 1,
            Bound::Last => last + 1,
        };
        let (first, second) = (shown(a), shown(b));
        (first > second).then_some((first, second))
    }

    /// The lines a `:s` range names, as a question a line number can be put to.
    fn substitution_rows(&self, rows: crate::command::Rows) -> Result<Chosen, String> {
        use crate::command::{Bound, Rows};
        let rope = self.current_buffer().rope();
        let last_line = motion::last_line(rope);
        // Warning: **沒有的那一行不許悄悄夾到末行**（2026-10-09 審出來的）。從前寫的是
        // `n.saturating_sub(1).min(last_line)`，於是三行的檔上 `:99s/a/X/` 改的是**第
        // 三行**，還報「換了 1 處」——把 `:9s` 打成 `:91s` 就是這樣改掉了一章的結尾。
        // `:0s` 同理，它改的是第一行。vim 兩種都報 `E16: Invalid range`，一個字不動。
        let resolve = |b: Bound| -> Result<usize, String> {
            match b {
                Bound::Line(0) => Err(say!("find.no-such-line", 0, last_line + 1)),
                Bound::Line(n) if n > last_line + 1 => {
                    Err(say!("find.no-such-line", n, last_line + 1))
                }
                Bound::Line(n) => Ok(n - 1),
                Bound::Cursor => Ok(rope.char_to_line(self.caret().min(rope.len_chars()))),
                Bound::Last => Ok(last_line),
            }
        };
        Ok(match rows {
            Rows::All => Chosen::Span(0, last_line),
            Rows::Span(a, b) => {
                let (a, b) = (resolve(a)?, resolve(b)?);
                Chosen::Span(a.min(b), a.max(b))
            }
            // `1,5,9` — these and no others. Written in any order, and a line
            // named twice is still one line.
            Rows::List(bounds) => Chosen::These(
                bounds.into_iter().map(resolve).collect::<Result<Vec<_>, _>>()?,
            ),
            // No range written: the lines the *selection* covers. Reading the
            // cursor's line instead meant that after `x` — which leaves the
            // cursor on the line below the one it selected — `:s` edited a
            // line the writer had not selected and could not see was selected.
            Rows::Selection => {
                let (start, end) = self.selection();
                let first = rope.char_to_line(start);
                let last = if end > start {
                    rope.char_to_line(end.saturating_sub(1))
                } else {
                    first
                };
                Chosen::Span(first, last)
            }
        })
    }

    // ---- `:s …c` — one match at a time (#415) -----------------------------

    /// Open the walk and put the first question.
    fn start_confirming(
        &mut self,
        re: Regex,
        replacement: String,
        global: bool,
        chosen: Chosen,
        reshape: bool,
    ) {
        self.ask_or_finish(Confirming {
            re,
            replacement,
            global,
            chosen,
            reshape,
            from: 0,
            changed: 0,
            skipped: 0,
            all: false,
            snapped: false,
        });
    }

    /// The next match still to be decided.
    ///
    /// Recomputed from the buffer each time rather than held as a list: every
    /// accepted change moves everything after it, and a stale offset in a
    /// walk that writes is a walk that writes in the wrong place.
    fn next_hit(&self, walk: &Confirming) -> Option<Hit> {
        let text = self.current_buffer().text();
        // Char index of the start of the line being looked at.
        let mut at = 0usize;
        for (idx, line) in text.split_inclusive('\n').enumerate() {
            if walk.chosen.has(idx) {
                // 換行符不進正則，見 [`crate::editor::line_body`]。
                let (line, _) = crate::editor::line_body(line);
                for caps in walk.re.captures_iter(line) {
                    let m = caps.get(0).expect("group 0 is the whole match");
                    let start = at + line[..m.start()].chars().count();
                    if start >= walk.from {
                        // `$1` resolved **for this match**, so the question
                        // shows what will actually be written.
                        let mut text = String::new();
                        caps.expand(&walk.replacement, &mut text);
                        return Some(Hit {
                            start,
                            end: start + m.as_str().chars().count(),
                            found: m.as_str().to_string(),
                            text,
                        });
                    }
                    // Without `g`, only the first match on a line is offered —
                    // and it stays the only one after it has been answered.
                    if !walk.global {
                        break;
                    }
                }
            }
            at += line.chars().count();
        }
        None
    }

    /// Write every match still undecided, in **one** pass.
    ///
    /// Warning: This is what `a` runs, and it exists because the obvious loop is
    /// quadratic. `next_hit` serialises the whole document and rescans it from
    /// the top for every match, and `write_one` copies it again for the grid
    /// guard — fine at one keystroke per `y`, ruinous when `a` does it N times
    /// with no key in between. Measured before the fix: a 680 KB chapter with
    /// 20,000 matches took **28.5 s** under `a` against 0.02 s for the same
    /// `:s` without `c`, and a 2 MB one took **4 minutes 35 seconds** — on the
    /// main thread, with no progress and no key that could stop it. A writer
    /// watching a frozen screen kills the process, which is how `a` came to
    /// cost both the substitution and the last few seconds of typing.
    ///
    /// One pass, one grid check, one `replace` — the same shape [`substitute`]
    /// has always used, restricted to the matches at or after `walk.from`.
    ///
    /// [`substitute`]: Self::substitute
    fn finish_all(&mut self, walk: &mut Confirming) -> Option<String> {
        let text = self.current_buffer().text();
        let mut rebuilt = String::with_capacity(text.len());
        // Char index of the start of the line being looked at, the way
        // `next_hit` counts it.
        let mut at = 0usize;
        let mut changed = 0usize;
        for (idx, line) in text.split_inclusive('\n').enumerate() {
            if !walk.chosen.has(idx) {
                rebuilt.push_str(line);
                at += line.chars().count();
                continue;
            }
            // 換行符不進正則，原樣寫回去——見 [`crate::editor::line_body`]。
            // `at` 數的仍舊是**整行**，它是跨行的字元計數。
            let (body, eol) = crate::editor::line_body(line);
            // Byte cursor inside `body`: everything before it is already in
            // `rebuilt`. `captures_iter` yields non-overlapping matches in
            // order, so it only ever moves forward.
            let mut copied = 0usize;
            for caps in walk.re.captures_iter(body) {
                let m = caps.get(0).expect("group 0 is the whole match");
                let start = at + body[..m.start()].chars().count();
                if start >= walk.from {
                    rebuilt.push_str(&body[copied..m.start()]);
                    // `$1` resolved for this match, as in `next_hit`.
                    caps.expand(&walk.replacement, &mut rebuilt);
                    copied = m.end();
                    changed += 1;
                }
                // Without `g`, only the first match on a line was ever
                // offered — and one already answered leaves the line alone.
                if !walk.global {
                    break;
                }
            }
            rebuilt.push_str(&body[copied..]);
            rebuilt.push_str(eol);
            at += line.chars().count();
        }
        if changed == 0 {
            return None;
        }
        // The grid guard, once for the whole batch rather than once per match.
        // Refusing the batch is the honest answer: every `y` already given is
        // in the buffer and stays there, and the walk closes saying why.
        if !walk.reshape {
            if let Some(why) = self.substitution_breaks_the_grid(&rebuilt) {
                return Some(why);
            }
        }
        // **One `u` undoes the whole walk** — see `write_one`.
        if !walk.snapped {
            self.snapshot();
            walk.snapped = true;
        }
        let len = self.current_buffer().char_count();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(0..len, &rebuilt));
        if !self.applied(done) {
            return Some(self.status.clone());
        }
        walk.changed += changed;
        None
    }

    /// Show the next match and wait for a key, or close the walk and report.
    fn ask_or_finish(&mut self, mut walk: Confirming) {
        if walk.all {
            let why = self.finish_all(&mut walk);
            self.close_confirming(&walk, why);
            return;
        }
        // `if`, not `while`: with `a` handled above, the body always either
        // puts the question and returns or falls through to the close.
        if let Some(hit) = self.next_hit(&walk) {
            // **The match is the selection**, so the writer is looking at the
            // thing the question is about — and at the sentence around it,
            // which is what they are actually judging.
            let head = {
                let rope = self.current_buffer().rope();
                let end = hit.end.min(rope.len_chars());
                motion::prev_grapheme(rope, end).max(hit.start)
            };
            self.sel.set_anchor(hit.start);
            self.sel.set_head(head);
            self.extend = false;
            self.refresh_goal_column();
            self.status = say!("substitute.confirm-this-one", hit.found, hit.text);
            self.pending = Pending::Confirm;
            self.confirming = Some(walk);
            return;
        }
        self.close_confirming(&walk, None);
    }

    /// Write one match, or say why it cannot be written.
    fn write_one(&mut self, walk: &mut Confirming, hit: &Hit) -> Option<String> {
        if !walk.reshape {
            // The grid guard, asked one match at a time — so a substitution
            // that would break a row is refused **at the match that breaks
            // it**, and every answer already given still stands.
            let rope = self.current_buffer().rope();
            let (from, to) = (rope.char_to_byte(hit.start), rope.char_to_byte(hit.end));
            let mut rebuilt = rope.to_string();
            rebuilt.replace_range(from..to, &hit.text);
            if let Some(why) = self.substitution_breaks_the_grid(&rebuilt) {
                return Some(why);
            }
        }
        // **One `u` undoes the whole walk.** A writer who says 「算了」 after
        // twenty `y`s means all twenty, not the twentieth. Taken at the first
        // accepted change, so answering `n` to everything leaves no undo step
        // standing for a document nothing happened to.
        if !walk.snapped {
            self.snapshot();
            walk.snapped = true;
        }
        let done = self
            .without_cell_guard(|e| e.current_buffer_mut().replace(hit.start..hit.end, &hit.text));
        if !self.applied(done) {
            return Some(self.status.clone());
        }
        walk.changed += 1;
        let past = hit.start + hit.text.chars().count();
        // A pattern that matches nothing (`x*`) would be found at the same
        // place forever, and `a` would never come back.
        walk.from = match hit.start == hit.end {
            true => past.max(hit.start + 1),
            false => past,
        };
        None
    }

    /// Close the walk and say what it did.
    fn close_confirming(&mut self, walk: &Confirming, why: Option<String>) {
        self.pending = Pending::None;
        self.confirming = None;
        self.clamp_cursor();
        self.sel.set_anchor(self.sel.head());
        self.refresh_goal_column();
        self.status = match (why, walk.changed + walk.skipped) {
            (Some(why), _) => why,
            // Nothing matched at all: the ordinary answer, not 「0 換 0 跳」.
            (None, 0) => say!("find.substitute-changed", 0),
            (None, _) => say!("substitute.confirm-done", walk.changed, walk.skipped),
        };
    }

    /// Answer the question a `:s …c` is asking (#415).
    pub(super) fn answer_confirm(&mut self, key: Key) {
        let Some(mut walk) = self.confirming.take() else {
            self.pending = Pending::None;
            return;
        };
        // The match on screen is the next undecided one — nothing can have
        // touched the buffer since, because this pending eats every key.
        let Some(hit) = self.next_hit(&walk) else {
            self.close_confirming(&walk, None);
            return;
        };
        match key {
            Key::Char('y') => {
                if let Some(why) = self.write_one(&mut walk, &hit) {
                    self.close_confirming(&walk, Some(why));
                    return;
                }
            }
            Key::Char('n') => {
                walk.skipped += 1;
                walk.from = hit.end.max(hit.start + 1);
            }
            Key::Char('a') => walk.all = true,
            Key::Char('l') => {
                let why = self.write_one(&mut walk, &hit);
                self.close_confirming(&walk, why);
                return;
            }
            Key::Char('q') | Key::Esc => {
                self.close_confirming(&walk, None);
                return;
            }
            // **Any other key leaves the question standing.** This is the flag
            // whose whole job is not changing what the writer has not looked
            // at; a stray keystroke must not be able to answer it.
            _ => {}
        }
        self.ask_or_finish(walk);
    }
}

/// **一條式子裏，打的人自己寫的那一段** —— 報給他看的時候用這一支。
///
/// 智能大小寫把 `(?i)` 寫在式子前面（見 [`Editor::smart_cased`]：那是正則引擎自己的
/// 旗標，不另外實現一套），而搜索面板把穿好衣服的那一整條存進了 `last_search`。於是
/// 找不到的時候狀態欄寫着「找不到：(?i)zhongguo」——那四個字符是編輯器加的，不該讓
/// 人回去找自己哪裏打錯了。
fn as_typed(pattern: &str) -> &str {
    pattern.strip_prefix("(?i)").unwrap_or(pattern)
}
