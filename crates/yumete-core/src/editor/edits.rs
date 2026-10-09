//! Changing the text, and the registers and macros around it (#296).
//!
//! Features #9 / #10: every edit goes through `edit_insert` / `edit_remove`,
//! which is where undo, the table's cell guard and the readonly refusal all
//! hang. Yank, paste, the paste picker and macro recording came to sit here
//! because they are the same subject seen from the keyboard.

use super::*;

impl Editor {
    // ---- Editing (Features #9 / #10) --------------------------------------

    /// Put `text` into the buffer, unless a grid says it must not go in.
    ///
    /// **One gate, not ten.** The first version of this checked the delimiter
    /// where a character is typed, and a review found seven other ways in — a
    /// paste, the clipboard, an IME commit, `:s`, `r`, `R` — every one of which
    /// wrote a comma into a cell and then wrote the file out, silently shifting
    /// every column after it. Text reaches the buffer through exactly two
    /// calls; this is one of them, and the check lives here so that adding an
    /// eighth way in cannot reopen the hole.
    pub(super) fn edit_insert(&mut self, at: usize, text: &str) -> bool {
        if self.refuse_readonly() {
            return false;
        }
        if let Some(why) = self.cell_refuses_text_at(Some(at), text) {
            self.status = why;
            return false;
        }
        let done = self.current_buffer_mut().insert(at, text);
        self.applied(done)
    }

    /// Whether a rewritten document would change any row's shape.
    ///
    /// `:s` is the one edit that rewrites whole lines at once, so it is checked
    /// as a whole: same number of rows, and each row with the same number of
    /// cells it had. A substitution that only changes what is *inside* cells
    /// passes, which is the useful kind — `:%s/⿰木/⿰禾/g` over a 拆分表.
    pub(super) fn substitution_breaks_the_grid(&self, rebuilt: &str) -> Option<String> {
        // **A table is a table whether or not `:table` was typed.** This check
        // used to open with `self.table.as_ref()?`, so `:replace` — which
        // reaches every file `:grep` found, including files never opened — went
        // through 13 rows of this project's own `development.md` and broke them.
        let separator = self.grid_shape_here()?;
        let rows_only = separator == Separator::Pipe;
        // A delimited file is all cells. A document is not: only its table
        // rows are, and a paragraph that gains a `|` has gained a character.
        // Counting the whole document refused `:%s/前文/前 | 文/` on a line
        // nowhere near the table.
        let before = self.current_buffer().rope().to_string();
        // Numbered by the **document's** lines, not by the filtered list: for a
        // Markdown table the filtered index is a table-row number, and 「第 3
        // 行」 then names a line the writer cannot find.
        let count = |text: &str| -> Vec<(usize, usize)> {
            let rows = crate::mdtable::row_lines(text);
            text.lines()
                .enumerate()
                .filter(|(n, _)| !rows_only || rows.get(*n).copied().unwrap_or(false))
                .map(|(n, l)| {
                    // Unescaped only: `\|` is a pipe *inside* a cell, and the
                    // manual promises it works — so a substitution that adds
                    // one must not be refused as if it split a row.
                    let cells = match separator {
                        Separator::Pipe => crate::mdtable::pipes_from(l, false).len(),
                        Separator::Delimiter(d) => l.chars().filter(|&c| c == d).count(),
                    };
                    (n, cells)
                })
                .collect()
        };
        let (was, now) = (count(&before), count(rebuilt));
        if was.len() != now.len() {
            return Some(say!(
                "table.substitution-would-change-rows",
                was.len(),
                now.len()
            ));
        }
        let (line, from, to) = was
            .iter()
            .zip(&now)
            .find(|((_, a), (_, b))| a != b)
            .map(|((n, a), (_, b))| (*n, *a, *b))?;
        // **A refusal that names no way through is a wall.** It used to say
        // 「先 :table-render off」, which stopped being an escape the moment the check
        // stopped asking whether table mode was on.
        Some(say!(
            "table.substitution-would-change-width",
            line + 1,
            from + 1,
            to + 1
        ))
    }

    /// Write `text` over `start..end`, unless a grid says either half must not
    /// happen.
    ///
    /// Both halves are checked *before* either runs, so a refusal leaves the
    /// buffer exactly as it was rather than half-edited.
    pub(super) fn overwrite(&mut self, start: usize, end: usize, text: &str) -> bool {
        if let Some(why) = self
            .cell_refuses_cut(start..end)
            .or_else(|| self.cell_refuses_text(text))
        {
            self.status = why;
            return false;
        }
        // **This is the one that had no gate of its own** (§5.2.3 ⑤): six
        // callers reach it, and the ones that did not refuse first wrote
        // nothing and then moved the cursor over it. It asks the buffer now,
        // and the buffer answers.
        let done = self.current_buffer_mut().replace(start..end, text);
        self.applied(done)
    }

    /// Take a range out of the buffer, unless it would take a cell boundary
    /// with it.
    ///
    /// The other half of the invariant: **a row's delimiter count never
    /// changes while it is being read as a grid.** Deleting is how it was most
    /// easily broken — `d` on an empty cell sits exactly on the delimiter, so
    /// the collapsed selection covered it and two cells became one.
    pub(super) fn edit_remove(&mut self, range: std::ops::Range<usize>) -> bool {
        if self.refuse_readonly() {
            return false;
        }
        if let Some(why) = self.cell_refuses_cut(range.clone()) {
            self.status = why;
            return false;
        }
        let done = self.current_buffer_mut().remove(range);
        self.applied(done)
    }

    /// Take the buffer's answer to an edit, and say so if it refused.
    ///
    /// `true` means the text moved and the caller may move the state around it
    /// — the cursor, the anchor, the status line. `false` means the buffer is
    /// read-only, the status line already says so, and **the caller must not
    /// touch any of that**: moving a cursor over text that was never written
    /// is the whole fault this returns a value to prevent (§5.2.3 ⑤).
    ///
    /// Most callers reach it after [`Editor::refuse_readonly`] has already
    /// turned them back, so the `Err` arm is unreachable there. That is the
    /// intent: the guard is the message, this is the proof.
    #[must_use]
    pub(super) fn applied(&mut self, done: crate::buffer::Edit) -> bool {
        match done {
            Ok(()) => true,
            Err(crate::buffer::ReadOnly) => {
                self.status = say!("readonly.refused");
                false
            }
        }
    }

    /// Say why nothing happened, when the buffer is locked (Feature #213).
    ///
    /// `true` means the caller must not edit. The refusal that *matters* is in
    /// [`Buffer::insert`](crate::buffer::Buffer::insert) — the rope does not
    /// move whatever anyone here forgets. This one exists so the writer is told:
    /// an editor that swallows keystrokes in silence is one you stop trusting
    /// long before you work out why.
    pub(super) fn refuse_readonly(&mut self) -> bool {
        if !self.current_buffer().is_readonly() {
            return false;
        }
        self.status = say!("readonly.refused");
        true
    }

    /// Whether the buffer on screen refuses to be edited (Feature #213).
    ///
    /// What draws `[只讀]` on the status line.
    pub fn is_readonly(&self) -> bool {
        self.current_buffer().is_readonly()
    }

    /// Lock every file this session opens, including the ones already open
    /// (`--readonly`).
    pub fn set_readonly_default(&mut self, on: bool) {
        self.readonly_default = on;
        if on {
            for buffer in &mut self.buffers {
                buffer.set_readonly(true);
            }
        }
        // Turning it *off* unlocks nothing on its own: a buffer the disk
        // itself calls read-only is locked for a reason of its own, and
        // `:readonly off` is how one buffer says otherwise.
    }

    /// Whether a clean buffer re-reads itself when the file changes underneath
    /// it (Feature #214).
    pub fn reload_auto(&self) -> bool {
        self.reload_auto
    }

    /// Insert `text` at the cursor and advance past it.
    pub(super) fn insert_str(&mut self, text: &str) {
        let at = self.sel.head();
        // **覆寫模式先讓位**（vim 的 `R`，2026-10-06）。一個進去的字蓋一個，蓋到
        // 行尾就不再蓋——vim 的 `R` 在行尾是接着寫，不吃換行。蓋掉的記下來，退格
        // 照着還回去。
        if self.overwriting {
            let rope = self.current_buffer().rope();
            let room = motion::line_end(rope, at).saturating_sub(at);
            let want = text.chars().count();
            let take = want.min(room);
            let gone: Vec<Option<char>> = (0..want)
                .map(|n| (n < take).then(|| rope.char(at + n)))
                .collect();
            if take > 0 && !self.edit_remove(at..at + take) {
                return;
            }
            self.overwritten.extend(gone);
        }
        if !self.edit_insert(at, text) {
            return;
        }
        self.sel.set_head(at + text.chars().count());
        self.sel.set_anchor(self.sel.head());
        self.refresh_goal_column();
    }

    /// Carry a Markdown list marker down to the next line, or end the list
    /// (#418). Whether it answered the Enter.
    ///
    /// **The Enter that ends the list is the half that matters.** Continuing a
    /// list is a convenience; without a way out of one, the only way to stop
    /// is to backspace the marker the editor just wrote, and a writer who has
    /// to undo the help twice a paragraph turns the help off. So an Enter on
    /// an item with nothing typed into it clears that line instead — which is
    /// what every editor that does this does, and what the hand already
    /// expects.
    ///
    /// It costs no undo point of its own, in either half: an Insert session
    /// announces one snapshot when it opens and this writes inside it, the
    /// same as every character typed.
    pub(super) fn continue_the_list(&mut self) -> bool {
        // Only where a `- ` *is* a list. In a novel it is a dash, and a
        // manuscript that gains markers it did not ask for is worse off than
        // one that carries none down.
        if self.syntax() != crate::syntax::Syntax::Markdown {
            return false;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        let start = rope.line_to_char(line);
        let text = self.line_text(line).unwrap_or_default();
        let text = text.trim_end_matches(['\n', '\r']).to_string();
        let Some(open) = crate::markdown::opening(&text) else {
            return false;
        };
        // Inside the marker itself the Enter is splitting `- [` in half, and
        // that is a thing the writer typed on purpose.
        if self.sel.head() < start + open.width {
            return false;
        }
        // A listing quoted in a fence is written out as it is; the markers in
        // it are somebody's example.
        //
        // Warning: **Asked last, and only of a line that already looks like an
        // item.** `block_of` walks from the top of the file, and its cache is
        // keyed on the revision — which every Enter has just changed — so
        // asking it first made every line break in a Markdown buffer re-scan
        // the whole document, and the test fixtures that type a chapter in
        // through `Key::Enter` stopped finishing at all. Down here it is paid
        // for by lists only, where one walk per item is a walk per paragraph.
        //
        // Warning: `:render off` reports every line as prose, so under it a fenced
        // list does carry down — the same blind spot
        // [`Self::replacement_reshapes_the_grid`] has, and the same reason:
        // the scan is only kept warm while there is markup on the screen.
        if self.block_of(line).is_literal() {
            return false;
        }
        if open.empty {
            let end = start + text.chars().count();
            let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(start..end, ""));
            if !self.applied(done) {
                return false;
            }
            self.sel.set_head(start);
            self.sel.set_anchor(start);
            self.refresh_goal_column();
            return true;
        }
        // One insert, so it is one edit: the line ending the file already uses
        // (#309) and the marker after it.
        let carried = format!("{}{}", self.current_buffer().ending(), open.next);
        self.insert_recording.push_str(&carried);
        self.insert_str(&carried);
        true
    }

    /// Open a new line below the cursor and enter Insert mode (`o`).
    /// **這一行開頭那一段空白**——`o`、`O` 和 Enter 拿它把縮進帶下去。
    ///
    /// vim 的 `autoindent`、helix 的 `insert_newline` 都做這件事，而這一頭 2026-10-08
    /// 之前一件都不做：代碼檔裏每按一次 Enter 光標就回到第 0 列（報的原話：「In
    /// `go file`, pressing enter in a code block won't auto indent to the same
    /// indentation level of the previous line」）。
    ///
    /// Warning: **不許「發明」空白。** `upto` 是光標在這一行的第幾個字——光標停在縮進
    /// *裏面*的時候只帶它前面那一截，於是 `\t\tfoo` 在兩個 tab 中間按 Enter 得到
    /// `\t` ＋ `\tfoo`，兩行加起來一個字符不多不少。行首（第 0 列）按下去因此什麼
    /// 都不帶，而被推下去的那一行本來就帶着自己的縮進——同 vim。
    ///
    /// Warning: **只在代碼檔裏**（`Syntax::Code`）。報的那一條就是代碼的
    /// （「In `go file`, pressing enter …」），而散文這一頭本來就有自己的首行縮進
    /// （`paragraph_indent`，那是畫出來的，檔案裏沒有空白）——給一本小說自動帶
    /// 縮進，是往稿子裏寫作者沒打的空白。Markdown 的列表另有一條路
    /// （[`Self::continue_the_list`]），它在這一支之前就答完了。
    ///
    /// 全角空格（U+3000）也算空白：它在代碼註釋裏真出現過。
    pub(super) fn indent_of_line(&self, line: usize, upto: Option<usize>) -> String {
        if !self.writes_code() {
            return String::new();
        }
        let text = self.line_text(line).unwrap_or_default();
        let indent: String = text.chars().take_while(|c| *c == ' ' || *c == '\t' || *c == '\u{3000}').collect();
        match upto {
            Some(n) => indent.chars().take(n).collect(),
            None => indent,
        }
    }

    /// 眼下這一行要帶下去的那一段縮進（Enter 用的，截到光標為止）。
    ///
    /// **外加語法樹說的那幾級**（2026-10-08 第二期）：行尾一個 `(`、`{`、`[`，下一
    /// 行就多縮一級；新那一行開頭是 `}` 之類就少一級。差由 [`Editor::levels_to_open`]
    /// 算（helix 的 `Hybrid`），這裏只負責把它換成真的空白。
    pub(super) fn indent_to_carry(&self) -> String {
        let (line, at) = self.caret_in_line();
        let carried = self.indent_of_line(line, Some(at));
        if !self.writes_code() {
            return carried;
        }
        let levels = self.levels_to_open(None);
        // **一級是多少格，照這一行自己的寫法**：它用 tab 就加一個 tab，用空格就加
        // `indent_width` 個空格。問這一行比問設置準——一份檔混着兩種的時候，跟着
        // 眼前這一段走纔不會把兩種拌在一起。
        let unit = match carried.contains('\t') || (carried.is_empty() && !self.tab_spaces) {
            true => "\t".to_string(),
            false => " ".repeat(self.indent_width),
        };
        match levels {
            0 => carried,
            n if n > 0 => format!("{carried}{}", unit.repeat(n as usize)),
            // 退級：從帶下來的那一段尾巴上拿掉幾個單位。
            n => {
                let mut kept = carried;
                for _ in 0..(-n) {
                    match kept.strip_suffix(&unit) {
                        Some(less) => kept = less.to_string(),
                        None => break,
                    }
                }
                kept
            }
        }
    }

    pub(super) fn open_line_below(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let end = motion::line_end(self.current_buffer().rope(), self.sel.head());
        let row = self.blank_row();
        // 縮進跟着下來，同 vim 的 `o`（2026-10-08）。
        let indent = self.indent_of_line(self.cursor_line(), None);
        // **The file's own line ending** (#309), not a literal `\n`.
        let ending = self.current_buffer().ending();
        let done = self.without_cell_guard(|e| {
            e.current_buffer_mut().insert(end, &format!("{ending}{indent}{row}"))
        });
        if !self.applied(done) {
            return;
        }
        self.sel.set_head(end + ending.chars().count() + indent.chars().count());
        self.sel.set_anchor(self.sel.head());
        self.enter_insert();
    }

    /// Open a new line above the cursor and enter Insert mode (`O`).
    pub(super) fn open_line_above(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let start = motion::line_start(self.current_buffer().rope(), self.sel.head());
        let row = self.blank_row();
        // 同 `o`：縮進照這一行的來（2026-10-08）。
        let indent = self.indent_of_line(self.cursor_line(), None);
        let ending = self.current_buffer().ending();
        let done = self.without_cell_guard(|e| {
            e.current_buffer_mut().insert(start, &format!("{indent}{row}{ending}"))
        });
        if !self.applied(done) {
            return;
        }
        self.sel.set_head(start + indent.chars().count());
        self.sel.set_anchor(self.sel.head());
        self.enter_insert();
    }

    /// **A blank line, and stay where you are** — helix's `]空格` / `[空格`
    /// (`add_newline_below` / `add_newline_above`), 2026-10-06.
    ///
    /// Not `o` / `O`: those open a line *and start typing in it*. This one is
    /// for pushing a paragraph apart without leaving where the eye is, which
    /// is why helix gives it a key of its own and why it does not move the
    /// cursor. **It takes no indent either** — a blank line with trailing
    /// spaces on it is a blank line that greps wrong.
    /// Warning: **每一段選區各插一條，而且不碰選區**（2026-10-06 夜審報的）。helix 的
    /// `add_newline_impl` 對每一段各做一次、吃計數、一段選區都不動；這裏從前只讀
    /// 主選區的頭，末尾還把選區塌成一個光標——函數自己的話是「不動光標」，頭確實
    /// 沒動，錨動了。
    pub(super) fn add_blank_line(&mut self, below: bool, count: usize) {
        if self.refuse_readonly() {
            return;
        }
        self.edit_each(|e| e.add_one_blank_line(below, count));
    }

    /// One selection's worth: `count` breaks, and the selection stays where
    /// it was pointing at the same text.
    fn add_one_blank_line(&mut self, below: bool, count: usize) {
        // Warning: **插在行首，不是行尾**，而且問的是**選區跨到哪一行**，不是光標那
        // 一行——逐條照 helix 的 `add_newline_impl`：`Open::Below` 算的是
        // `line_to_char(最後一行 + 1)`，也就是下一行的開頭（換行符後面）。插在
        // `line_end`（換行符上）等於插進了這一行的末尾，一個頭停在換行符上的選區
        // 會被撐大一格。
        let rope = self.current_buffer().rope();
        let (from, to) = self.selection();
        let first = rope.char_to_line(from);
        let last = rope.char_to_line(to.saturating_sub(1).max(from));
        let at = match below {
            true => rope.line_to_char((last + 1).min(rope.len_lines())),
            false => rope.line_to_char(first),
        };
        let ending = self.current_buffer().ending().to_string();
        let breaks = ending.repeat(count.max(1));
        let grew = breaks.chars().count();
        let (anchor, head) = (self.sel.anchor(), self.sel.head());
        self.snapshot();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().insert(at, &breaks));
        if !self.applied(done) {
            return;
        }
        // Inserting above pushes everything from the line's start onward, this
        // selection included; inserting below is past both ends and moves
        // nothing. Either way **both** ends travel together — the selection is
        // on the same text it was on.
        let shift = |one: usize| match one >= at {
            true => one + grew,
            false => one,
        };
        self.sel.set_anchor(shift(anchor));
        self.sel.set_head(shift(head));
        self.clamp_cursor();
    }

    /// **`5ix<Esc>` types five `x`** (`:h count`, 2026-10-06).
    ///
    /// Remembered on the way *in* and spent on the way out, because what gets
    /// repeated is the whole typing session and nobody knows what that is
    /// until `Esc`. `opened` says whether a new line is part of the thing
    /// being repeated: `3ohi<Esc>` is three new lines, not one line with
    /// `hihihi` on it.
    ///
    /// Warning: **vim only.** helix's `i` ignores a count.
    pub(super) fn type_it_again(&mut self, count: usize, opened: Option<bool>) {
        self.insert_again = match self.key_preset == yumete_cjk::KeyPreset::Vim {
            true => count.saturating_sub(1),
            false => 0,
        };
        self.insert_opened = opened;
    }

    /// Put `typed` in `again` more times, as the insert session ends.
    ///
    /// One undo point for the whole thing: `5ix` is one command, so `u` takes
    /// back all five — the same rule [`Self::repeat`] keeps for a counted
    /// motion. The snapshot was already taken when `i` was pressed.
    pub(super) fn spend_the_insert_count(&mut self, again: usize, typed: &str) {
        let opened = self.insert_opened.take();
        let ending = self.current_buffer().ending().to_string();
        for _ in 0..again {
            match opened {
                // `i a I A` — 接着打，就在光標這裏。
                None => self.insert_str(typed),
                // Warning: **`o`/`O` 這兩路要自己拼字串，不許叫 `add_blank_line`**
                // （2026-10-06 夜審報的，兩個病）。① 那一支自己 `snapshot()`，於是
                // 「整段算一個命令」是假的——`3o` 打完要按三下 `u` 纔回得去；
                // ② 它把光標跟着原來那段文字往下挪，`O` 那一路算出來的落點是**已經
                // 有字的那一行**，三遍全寫在同一行上（`3O` 打 `hi` 得
                // `\n\nhihihi\nX`，而 vim 給 `hi\nhi\nhi\nX`）。
                Some(true) => {
                    let at = motion::line_end(self.current_buffer().rope(), self.sel.head());
                    let text = format!("{ending}{typed}");
                    if !self.edit_insert(at, &text) {
                        return;
                    }
                    self.set_cursor(at + text.chars().count());
                }
                Some(false) => {
                    let at = motion::line_start(self.current_buffer().rope(), self.sel.head());
                    let text = format!("{typed}{ending}");
                    if !self.edit_insert(at, &text) {
                        return;
                    }
                    // 光標留在剛插進去那一行的頭上，下一遍就插在它前面——三遍寫出
                    // 來的是三行一樣的字，所以次序看不出來，而落點必須對。
                    self.set_cursor(at);
                }
            }
        }
        self.refresh_goal_column();
    }

    /// 覆寫模式下退格：往回一格，把 `was` 放回去（`None` 是那裏本來就沒有字）。
    pub(super) fn put_back_what_was_overwritten(&mut self, was: Option<char>) {
        let at = self.sel.head();
        let rope = self.current_buffer().rope();
        let back = motion::prev_grapheme(rope, at).max(self.insert_floor());
        if back >= at {
            return;
        }
        if !self.edit_remove(back..at) {
            return;
        }
        match was {
            Some(c) => {
                let mut buf = [0u8; 4];
                let one = c.encode_utf8(&mut buf).to_string();
                if self.edit_insert(back, &one) {
                    self.set_cursor(back);
                }
            }
            None => self.set_cursor(back),
        }
    }

    /// Take back the word before the cursor (`C-w` in Insert).
    ///
    /// The word is the segmenter's, not a run of non-space: this is an editor
    /// for a language that does not put spaces between words, and `C-w` that
    /// deleted the whole paragraph would be worse than not having it.
    pub(super) fn delete_word_before_cursor(&mut self) {
        let at = self.sel.head();
        let rope = self.current_buffer().rope();
        let mut from = motion::prev_word_start(rope, at, self.word_grain(), self.segmenter.as_ref());
        // At the start of a word, the word to take back is the one before it.
        if from >= at {
            from = motion::line_start(rope, at);
        }
        // Never out of the cell it is typing in, and never over a line break:
        // both are the invariants Insert mode already keeps.
        from = from.max(self.insert_floor());
        if from >= at {
            return;
        }
        self.snapshot();
        if self.edit_remove(from..at) {
            self.set_cursor(from);
            // What `.` replays has to match what happened.
            let taken = at - from;
            for _ in 0..taken {
                self.insert_recording.pop();
            }
        }
    }

    /// **Take the word ahead** (`A-d` in Insert) — the other half of `C-w`.
    ///
    /// helix binds it (`delete_word_forward`), readline binds it, and it was
    /// one of four chords Insert mode here swallowed whole (2026-10-06).
    ///
    /// The word is the segmenter's, for the reason `C-w` gives: a rule written
    /// as 「up to the next space」 eats a whole Chinese paragraph. And it never
    /// crosses the line, which is what every other Insert-mode edit here does
    /// — `next_word_start` is deliberately a crossing motion, so the ceiling
    /// is applied here.
    pub(super) fn delete_word_after_cursor(&mut self) {
        let at = self.sel.head();
        let rope = self.current_buffer().rope();
        let mut to = motion::next_word_start(rope, at, self.word_grain(), self.segmenter.as_ref());
        // Nothing further on this line: take the rest of the line's text.
        let line_end = motion::line_end(rope, at);
        to = to.min(line_end).min(self.insert_ceiling());
        if to <= at {
            return;
        }
        self.snapshot();
        if self.edit_remove(at..to) {
            self.set_cursor(at);
        }
    }

    /// The far side of what Insert mode may touch — a cell's end inside a
    /// grid, and the line's end outside one. The twin of [`Self::insert_floor`].
    fn insert_ceiling(&self) -> usize {
        match self.insert_bounds() {
            Some((_, end)) => end,
            None => motion::line_end(self.current_buffer().rope(), self.caret()),
        }
    }

    /// Take back everything from the start of the line to the cursor (`C-u`).
    /// **刪到行尾**（Insert 裏的 `C-k`，2026-09-28）。
    ///
    /// `C-u` 是它的另一半，早就有了。這一對在 readline 裏是一起的，而一個只做了一半的
    /// 對子比兩個都沒有更難記。
    ///
    /// Warning: **不跨行**：光標已經在行尾的時候什麼都不做，不去吃那個換行。readline 的
    /// `C-k` 在行尾也不吃下一行。
    pub(super) fn delete_to_line_end(&mut self) {
        let at = self.sel.head();
        let rope = self.current_buffer().rope();
        // Warning: **不是 `line_last`**：那一支回的是最後一個字自己的下標，用它當上界會把行
        // 末那一個字留下。要的是這一行的**內容**有多長（不含換行）。
        let line = rope.char_to_line(at);
        let to = rope.line_to_char(line) + crate::zong::line_chars(rope, line).len();
        if to <= at {
            return;
        }
        self.snapshot();
        if self.edit_remove(at..to) {
            self.set_cursor(at);
        }
    }

    /// **把一個寄存器的內容插進來**（Insert 裏的 `C-r`，2026-09-28）。
    ///
    /// Warning: **省掉的是一個撤銷點。** 從前寫到一半要放一個剛複製的人名，得 `Esc`、`p`、
    /// 再 `i` 回來——而這個倉的規矩是「一次插入是一次撤銷」（§5.12.3），那一出一進
    /// 白白多出一個撤銷點，一句話從此要按兩次 `u` 纔退得乾淨。
    pub(super) fn insert_register(&mut self, name: Option<char>) {
        let text = match name {
            Some(name) => self.registers.get(&name).cloned().unwrap_or_default(),
            None => self.register.clone(),
        };
        if text.is_empty() {
            self.status = say!("edit.register-is-empty");
            return;
        }
        let at = self.sel.head();
        if self.edit_insert(at, &text) {
            self.set_cursor(at + text.chars().count());
        }
    }

    pub(super) fn delete_to_line_start(&mut self) {
        let at = self.sel.head();
        let from = motion::line_start(self.current_buffer().rope(), at).max(self.insert_floor());
        if from >= at {
            return;
        }
        self.snapshot();
        if self.edit_remove(from..at) {
            self.set_cursor(from);
            for _ in 0..(at - from) {
                self.insert_recording.pop();
            }
        }
    }

    /// The earliest character an Insert-mode deletion may reach.
    ///
    /// The start of the cell when typing in a grid, and the start of the line
    /// otherwise — the two places where deleting one character further would
    /// join two things the file keeps apart.
    fn insert_floor(&self) -> usize {
        match self.insert_bounds() {
            Some((start, _)) => start,
            None => motion::line_start(self.current_buffer().rope(), self.caret()),
        }
    }

    /// Where `a` (append) places the cursor: after the selection, or one grapheme
    /// past the cursor when the selection is collapsed.
    pub(super) fn append_position(&self) -> usize {
        self.selection().1
    }

    /// Select the current line, extending line-wise on repeated presses (`x`).
    pub(super) fn select_line(&mut self) {
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        let (start, end) = self.selection();
        let anchor_line = rope.char_to_line(start);
        let cursor_line = rope.char_to_line(end);
        let sel_start = rope.line_to_char(anchor_line);
        let next_line = cursor_line + 1;
        let sel_end = if next_line > last {
            rope.len_chars()
        } else {
            rope.line_to_char(next_line)
        };
        let sel_end = motion::prev_grapheme(rope, sel_end).max(sel_start);
        self.sel.set_anchor(sel_start);
        self.sel.set_head(sel_end);
        self.refresh_goal_column();
    }

    /// Grow a collapsed selection rightward by `n` graphemes, so a count in
    /// front of `d` or `c` names how much to take.
    pub(super) fn extend_by_graphemes(&mut self, n: usize) {
        let rope = self.current_buffer().rope();
        let mut end = self.sel.head();
        // `n` graphemes counted from the cursor's own, which is already in the
        // selection, so the head moves `n - 1` further.
        for _ in 1..n {
            let next = motion::right(rope, end);
            if next == end {
                break;
            }
            end = next;
        }
        self.sel.set_anchor(self.sel.head());
        self.sel.set_head(end);
    }

    /// Delete the current selection (`d`), **leaving the register alone**.
    ///
    /// A collapsed selection deletes the grapheme under the cursor. The caller
    /// takes the undo snapshot.
    ///
    /// Warning: **Deleting is not copying** — the reason this half exists at all
    /// (#492): 「d 作为剪切功能会污染 register」. The register is a clipboard of
    /// one, so under a yanking `d` every tidy-up between a copy and a paste
    /// silently throws the copy away — yank a paragraph, take out a stray 、
    /// before pasting it, and the paragraph is gone.
    ///
    /// Warning: **Which key this is under changed on 2026-09-28** (#405), and this
    /// comment went on claiming the old answer until 2026-10-07. It is `A-d`
    /// and `A-c`, which is Helix's own spelling; `d` and `c` yank, which is
    /// also Helix's. In a grid it is `D` (`tables.rs`), because a grid has no
    /// `A-d`.
    pub(super) fn delete_selection(&mut self) {
        self.cut_selection(false);
    }

    /// Take the selection out **and put it in the register** (`D`).
    ///
    /// The other half of the pair above: cutting is a deliberate word, and it
    /// gets the deliberate key. `D` and `C` were unbound in Helix, so this
    /// costs nothing that was already spoken for.
    pub(super) fn cut_selection_to_register(&mut self) {
        self.cut_selection(true);
    }

    /// Cut up to `count` graphemes **before** the cursor into the register,
    /// stopping at the start of the line — vim's `X`. The cursor ends on the
    /// character it was on.
    pub(super) fn cut_before_cursor(&mut self, count: usize) {
        let (start, last) = {
            let rope = self.current_buffer().rope();
            let line_start = motion::line_start(rope, self.sel.head());
            let mut start = self.sel.head();
            for _ in 0..count.max(1) {
                if start <= line_start {
                    break;
                }
                start = motion::prev_grapheme(rope, start);
            }
            (start, motion::prev_grapheme(rope, self.sel.head()))
        };
        if start == self.sel.head() {
            return;
        }
        self.sel.set_anchor(start);
        self.sel.set_head(last);
        self.cut_selection_to_register();
    }

    /// The one implementation of both: `yanks` says whether the text taken out
    /// goes into the register on its way.
    fn cut_selection(&mut self, yanks: bool) {
        self.cut_range(self.selection(), yanks)
    }

    /// **The knife** — and since B2 (2026-09-20) it is handed the range rather
    /// than reading the selection itself.
    ///
    /// That one parameter is the whole of what a second grammar needed: vim's
    /// `dw` has no selection to read, only the span its motion just returned.
    /// One knife, two grammars; `cut_selection` is helix passing its own.
    fn cut_range(&mut self, (start, end): (usize, usize), yanks: bool) {
        if end > start {
            // Deleting yanks, as it does in Helix: `d` then `p` moves text.
            if self.cell_refuses_cut(start..end).is_some() {
                self.status = say!("table.divider-cannot-be-deleted");
                return;
            }
            if yanks {
                let text = self.current_buffer().rope().slice(start..end).to_string();
                self.store(text);
            }
            // **Nothing below this line may run on a refusal**: collapsing the
            // selection onto `start` over text that is still there is the
            // fault §5.2.3 ⑤ was decided to close, and this is where it was.
            let done = self.current_buffer_mut().remove(start..end);
            if !self.applied(done) {
                return;
            }
        }
        self.sel.set_head(start);
        self.sel.set_anchor(start);
        self.extend = false;
        self.clamp_cursor();
        self.refresh_goal_column();
    }

    /// Move by whole pages, or half of one.
    ///
    /// A page means what is on screen, and *which way* it runs depends on the
    /// layout: down the lines when set horizontally, across the 縱 when set
    /// vertically. Both are "onward through the text", which is what the key
    /// means.
    pub(super) fn move_page(&mut self, count: usize, back: bool, fraction: f64) {
        let vertical = self.layout() == Layout::Vertical;
        let page = if vertical {
            self.page_columns
        } else {
            self.page_lines
        };
        let steps = ((page as f64 * fraction).round() as usize).max(1) * count;
        for _ in 0..steps {
            let before = self.sel.head();
            if vertical {
                self.move_zong_from(!back, true);
            } else {
                self.move_vertical(back);
            }
            if self.sel.head() == before {
                break;
            }
        }
    }

    /// Start recording keys, or stop and keep what was recorded (Helix `q`).
    pub(super) fn toggle_recording(&mut self) {
        match self.recording.take() {
            Some(keys) => {
                let n = keys.len();
                self.macro_keys = keys;
                self.status = say!("macro.recorded-keys", n);
            }
            None => {
                self.recording = Some(Vec::new());
                self.status = say!("macro.recording");
            }
        }
    }

    /// Play the last recorded macro back (Helix `Q`).
    ///
    /// A macro cannot start while one is playing, and cannot play inside
    /// itself: `Q` recorded into a macro would otherwise recurse until the
    /// stack ran out.
    pub(super) fn replay_macro(&mut self, count: usize) {
        if self.replaying {
            return;
        }
        if self.macro_keys.is_empty() {
            self.status = say!("macro.nothing-recorded");
            return;
        }
        let keys = self.macro_keys.clone();
        self.replaying = true;
        // A macro may write, so it answers to the writing ceiling; and a round
        // that moved nothing and changed nothing will never move anything on
        // the next one either, so stop (#318). `revision`, not `char_count`:
        // a macro that types a character and rubs it out has still worked.
        let clipped = count.min(Self::WRITING_MAX);
        // **數目是一條命令，和 `100p` 同一條規矩**（2026-10-02 補，#323 當初只改了
        // [`Editor::repeat`]）。第一趟照常掙它自己那幾個撤回點——巨集裏每一條命令
        // 各算一步，vim 也是這樣；第二趟起就不再掙了，所以 `10q` 和 `1q` 按同樣
        // 多下 `u` 撤得掉。從前 `10q` 一個改動的巨集要按十一下。
        //
        // Warning: **開在哪一份上就關在哪一份上。** 巨集裏什麼鍵都可能有，包括換稿子
        // 的那幾個；拿 `current_buffer_mut()` 關的話，換過之後關的是別人，而開着
        // 那一份的 `grouping` 永遠是 `true`——**從此一個撤回點都不記**，而且一聲
        // 不吭。所以記下號碼，照號碼關。
        let mut group: Option<(u64, bool)> = None;
        for _ in 0..clipped {
            let before = (self.current, self.sel.head(), self.sel.anchor(), self.current_buffer().revision());
            for &key in &keys {
                self.on_key(key);
            }
            if (self.current, self.sel.head(), self.sel.anchor(), self.current_buffer().revision()) == before {
                break;
            }
            if group.is_none() {
                let id = self.current_buffer().id();
                group = Some((id, self.current_buffer_mut().begin_undo_group()));
            }
        }
        if let Some((id, was)) = group {
            if let Some(i) = self.buffer_with(id) {
                self.with_buffer(i, |e| e.current_buffer_mut().end_undo_group(was));
            }
        }
        self.replaying = false;
        if count > clipped {
            self.status = say!("count.writing-ceiling", Self::WRITING_MAX);
        }
    }

    /// Read the register the next command should use, and forget the request.
    fn take_register(&mut self) -> Option<char> {
        self.pending_register.take()
    }

    /// Put `text` in the register named by a pending `"`, or the unnamed one.
    pub(super) fn store(&mut self, text: String) {
        match self.take_register() {
            Some(name) => {
                self.registers.insert(name, text);
            }
            None => {
                // The ring keeps what the register is about to lose. Not
                // duplicates of the top — `yy` twice is one thing you took, not
                // two — and not nothing.
                if !text.is_empty() && self.yanks.first() != Some(&text) {
                    self.yanks.insert(0, text.clone());
                    self.yanks.truncate(YANKS);
                }
                // **一段選區一份**（2026-10-09，見 [`Editor::register_parts`]）。
                //
                // `edit_nth` 是這一段在**文檔次序**裏的序號（`#` 寄存器讀的也是它），
                // 所以收下來那幾份本來就是按順序排好的，哪一趟先跑不要緊。
                //
                // Warning: **「這一趟」要分得出來**：`edit_each` 每開一輪就把 `edit_round`
                // 往上加一格，而這裏記下存的時候是第幾輪。不記的話，下一個命令的第
                // 一段會接在上一個命令留下的那幾份後面。
                match self.edit_nth {
                    None => {
                        self.register_parts.clear();
                        self.register = text;
                    }
                    Some(nth) => {
                        if self.parts_round != self.edit_round {
                            self.parts_round = self.edit_round;
                            self.register_parts.clear();
                        }
                        if self.register_parts.len() <= nth {
                            self.register_parts.resize(nth + 1, String::new());
                        }
                        self.register_parts[nth] = text;
                        self.register = self.register_parts.concat();
                    }
                }
            }
        }
    }

    /// Open the picker over everything that could be pasted (`Space \"`).
    pub(super) fn open_paste_picker(&mut self) {
        let mut items = vec![crate::picker::Item::Paste(
            None,
            say!("register.system-clipboard"),
        )];
        items.extend(
            self.paste_menu()
                .into_iter()
                .enumerate()
                .map(|(i, (name, text))| {
                    crate::picker::Item::Paste(Some(i), format!("{name}  {}", one_line(&text)))
                }),
        );
        if items.len() == 1 {
            self.status = say!("edit.nothing-yanked-yet");
        }
        self.picker = Some(crate::picker::Picker::new(&say!("picker.paste"), items));
        self.mode = Mode::Picker;
    }

    /// Everything that could be pasted, newest first, for the picker to show.
    ///
    /// The unnamed register's history, then the named ones. The system
    /// clipboard is offered too but is not in this list: only the front end can
    /// read it, so it is a row that asks rather than a row that holds.
    pub fn paste_menu(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (i, text) in self.yanks.iter().enumerate() {
            out.push((format!("{i}"), text.clone()));
        }
        let mut named: Vec<(&char, &String)> = self.registers.iter().collect();
        named.sort();
        for (name, text) in named {
            out.push((format!("\"{name}"), text.clone()));
        }
        out
    }

    /// Paste one of the things `paste_menu` offered.
    pub(super) fn paste_from_menu(&mut self, which: usize) {
        let menu = self.paste_menu();
        let Some((name, text)) = menu.get(which) else {
            return;
        };
        let (name, text) = (name.clone(), text.clone());
        self.register = text;
        self.pending_register = None;
        self.paste(true);
        self.status = say!("edit.pasted", name);
    }

    /// The contents of the register a command should read from.
    pub(super) fn recall(&mut self) -> String {
        match self.take_register() {
            // **`#` 是選區的序號**（#405 Phase 3，helix 也是這個名字）。它不是一個存東
            // 西的格子，是一個問題的答案：「這是第幾段」。`"#p` 於是在第一段貼「1」、
            // 第二段貼「2」——編號列表那個用例就是這麼解開的。
            //
            // Warning: **只有一段的時候它是「1」**，不是空的：`"#p` 在一個光標上貼一個 1 是
            // 說得通的，而貼一個空字符串看起來像鍵沒按上。
            Some('#') => (self.edit_nth.unwrap_or(0) + 1).to_string(),
            Some(name) => self.registers.get(&name).cloned().unwrap_or_default(),
            // **一段選區一份**（2026-10-09）：N 段剪下來、N 段貼回去，逐字節還原
            // ——`d` 再 `P` 是「把它放回去」的那一下，而它從前只放得回一段。
            //
            // 段數對不上的時候交整份（接起來的）：一個光標上 `P` 貼的是剪下來的全部，
            // 真 helix 也是這樣。
            None => match self.edit_nth {
                Some(nth) if self.register_parts.len() > 1 => {
                    self.register_parts.get(nth).cloned().unwrap_or_default()
                }
                _ => self.register.clone(),
            },
        }
    }

    /// **Do `op` to `span`** — the one door every verb goes through (B2,
    /// 2026-09-20).
    ///
    /// helix hands it the selection; vim (B3) will hand it whatever its motion
    /// returned, without either of them knowing the other exists. Warning: A
    /// [`motion::Span::Missed`] does **nothing** — not 「operate on an empty
    /// range」, which is how a failed motion used to eat one character.
    pub(super) fn apply(&mut self, op: motion::Operator, span: motion::Span) {
        let motion::Span::Over { anchor, head } = span else { return };
        // Warning: **A span is in the caret's coordinates, a range is not** — and the
        // difference is one grapheme. `Span::Over { head }` names a cell the
        // caret lands *on*; what a verb takes runs one grapheme past it,
        // because the cursor's own grapheme is inside the selection in this
        // editor (see `selection`). Passing the span through unchanged left
        // the last character of every `d` behind — `f。d` and the 。 again,
        // which is the bug `selection` exists to prevent.
        //
        // A span may also run either way (`b` selects backwards); a range does
        // not.
        let (start, last) = (anchor.min(head), anchor.max(head));
        let range = (start, motion::next_grapheme(self.current_buffer().rope(), last));
        match op {
            motion::Operator::Delete => self.cut_range(range, false),
            motion::Operator::Cut => self.cut_range(range, true),
            motion::Operator::Change { cut } => {
                self.cut_range(range, cut);
                self.enter_insert();
            }
            motion::Operator::Yank => self.yank_range(range),
        }
    }

    /// Copy a range into the register and say how much (`y`).
    ///
    /// Warning: No special case for a collapsed selection: there is no such thing —
    /// the cursor's own grapheme is always inside one.
    pub(super) fn yank_range(&mut self, (start, end): (usize, usize)) {
        let text = self.current_buffer().rope().slice(start..end).to_string();
        self.store(text);
        self.status = say!("edit.yanked-characters", end - start);
    }

    /// Paste the register after (`p`) or before (`P`) the selection, and select
    /// the pasted text. Does nothing when the register is empty.
    pub(super) fn paste(&mut self, after: bool) {
        // Read before `recall`, which *takes* it: asking afterwards always
        // answered `None`, so the reader who named a register was told 「nothing
        // has been yanked」 about a register they had just named.
        let named = self.pending_register;
        let text = self.recall();
        if text.is_empty() {
            // A key that does nothing and says nothing is indistinguishable
            // from a key that is broken — and the reader whose `y` went to a
            // named register is the one most likely to press this.
            self.status = match named {
                Some(name) => say!("register.empty", name),
                None => say!("register.nothing-yanked"),
            };
            return;
        }
        self.snapshot();
        let (start, end) = self.selection();
        // Whole lines go back as whole lines. `xy` copies a line *with* its
        // break, and dropping that in the middle of another line cuts it in
        // two — which is exactly what the standard way of moving a paragraph
        // (`xy`, move, `p`) does most.
        let line_wise = text.ends_with('\n');
        let (start, end) = (start, end.max(start));
        let at = if line_wise {
            let rope = self.current_buffer().rope();
            let line = rope.char_to_line(if after { end.max(start) } else { start });
            if after {
                let next = line + 1;
                if next < rope.len_lines() {
                    rope.line_to_char(next)
                } else {
                    rope.len_chars()
                }
            } else {
                rope.line_to_char(line)
            }
        } else if after {
            if start == end {
                motion::right(self.current_buffer().rope(), self.sel.head())
            } else {
                end
            }
        } else {
            start
        };
        let len = text.chars().count();
        if !self.edit_insert(at, &text) {
            return;
        }
        // The pasted text becomes the selection, ending on its last grapheme.
        let rope = self.current_buffer().rope();
        let head = motion::prev_grapheme(rope, at + len).max(at);
        self.sel.set_anchor(at);
        self.sel.set_head(head);
        self.refresh_goal_column();
    }

    /// Delete the grapheme before the cursor (Insert-mode Backspace).
    pub(super) fn delete_before_cursor(&mut self) {
        if self.sel.head() == 0 {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.sel.head());
        let line_start = rope.line_to_char(line);
        let start = if self.sel.head() == line_start {
            // At the start of a line: delete the preceding newline (join lines).
            self.sel.head() - 1
        } else {
            motion::left(rope, self.sel.head())
        };
        let range = start..self.sel.head();
        if !self.edit_remove(range) {
            return;
        }
        self.sel.set_head(start);
        self.sel.set_anchor(self.sel.head());
        self.refresh_goal_column();
    }

    /// Delete the grapheme **after** the cursor (Insert-mode Delete).
    ///
    /// Backspace's mirror: it takes the character the cursor is sitting in
    /// front of and leaves the cursor where it is. At the end of a line it
    /// takes the newline, joining the line below — which is what Backspace
    /// does at the start of one.
    pub(super) fn delete_at_cursor(&mut self) {
        let rope = self.current_buffer().rope();
        if self.sel.head() >= rope.len_chars() {
            return;
        }
        let line = rope.char_to_line(self.sel.head());
        let line_end = rope.line_to_char(line) + rope.line(line).len_chars();
        let end = match self.sel.head() + 1 >= line_end {
            // At the end of the line: the newline itself.
            true => self.sel.head() + 1,
            false => motion::right(rope, self.sel.head()),
        };
        let end = end.min(rope.len_chars());
        if end <= self.sel.head() {
            return;
        }
        if !self.edit_remove(self.sel.head()..end) {
            return;
        }
        self.sel.set_anchor(self.sel.head());
        self.refresh_goal_column();
    }

    /// Add a buffer and make it active.
    ///
    /// If the only open buffer is the pristine, empty scratch buffer that
    /// [`Editor::new`] starts with, it is *replaced* rather than stacked on top
    /// of, so `yumete file` results in exactly one buffer.
    pub(super) fn add_buffer(&mut self, buffer: Buffer) {
        // Remember where the buffer being left had its cursor, so coming back
        // to it returns to the same place.
        let at = self.sel.head();
        self.buffers[self.current].save_cursor(at);
        // **剛纔看的是哪一份**（`ga`）。開一份新的也算換過，所以這裏和
        // [`Editor::show_buffer`] 都要記——Warning: 那一支只收「已經開着的」，開新檔走的是
        // 這裏，兩條路都記上 `ga` 纔一直有答案。
        let leaving = self.current_buffer().id();
        if self.buffers.len() == 1
            && self.buffers[0].path().is_none()
            && self.buffers[0].char_count() == 0
        {
            // 被頂掉的那一份不存在了，回不去。
            self.buffers[0] = buffer;
            self.current = 0;
            self.last_file = None;
        } else {
            self.buffers.push(buffer);
            self.current = self.buffers.len() - 1;
            self.last_file = Some(leaving);
        }
        // What this file held when the session first saw it, so that a day
        // whose row is opened at four in the afternoon counts from the morning
        // (Feature #244). Asked once per path: reopening a file that is already
        // in the map is the same session still writing it.
        let opened = self.buffers[self.current].path().map(Path::to_path_buf);
        if let Some(path) = opened {
            if !self.opened_with.contains_key(&path) {
                let han = self.han_in_rope(self.buffers[self.current].rope());
                self.opened_with.insert(path, han);
            }
        }
        // A freshly focused buffer starts at the top in Normal mode. The
        // segmentation cache is keyed by line number, and these are the lines
        // of a different document now.
        self.segment_memo.forget();
        self.sel.set_head(0);
        // Whether *this* buffer is a grid is asked again, the way `show_buffer`
        // asks it. Without this, `:table` and then `:!wc -l` left the shell
        // output being edited as a table: `o` opened `|  |  |` in it and `:s`
        // was guarded against a table that was in another file.
        self.leave_table_quietly();
        self.md_cache.borrow_mut().take();
        self.md_tables.borrow_mut().take();
        self.table_on_open();
        self.sel.set_anchor(0);
        self.sel.set_goal(None);
        self.mode = Mode::Normal;
        self.extend = false;
        self.pending = Pending::None;
        // A half-answered `:s …c` belonged to the buffer being left; what it
        // already wrote stands, and the questions it had left do not follow
        // the writer into another file.
        self.confirming = None;
        self.operator_count = None;
        // A half-typed table command belonged to the buffer that is being left
        // — `t1a` and then `:e other.md` must not leave a column named for the
        // next file's table. From the keyboard the key that opens a buffer has
        // already cleared these; a command line and a restored session have
        // not.
        self.sequence = None;
        self.sort_keys.clear();
        // 問服務器的那幾句記的是字元下標，換了文檔就不算了——見
        // [`Editor::forget_what_was_asked`]，換檔那一條路也叫它。
        self.forget_what_was_asked();
    }
    /// Comment the selected lines out, or bring them back (#409).
    ///
    /// **Whole lines, always.** A comment that begins in the middle of a line
    /// and ends in the middle of another is a thing nobody can read back, and
    /// a line comment has nowhere else to go. So the selection is widened to
    /// the lines it touches before anything is written, and the selection that
    /// comes back covers the same lines.
    pub(super) fn toggle_comment(&mut self, prefer: crate::comment::Prefer) {
        if self.refuse_readonly() {
            return;
        }
        let Some(form) = crate::comment::form(self.syntax(), prefer) else {
            // **Said, not silently skipped.** A key that writes nothing and
            // says nothing is the bug this project has fixed most often.
            self.status = say!("comment.none");
            return;
        };
        let rope = self.current_buffer().rope().clone();
        let (from, to) = self.selection();
        let start = crate::motion::line_start(&rope, from.min(rope.len_chars()));
        // **The last line the selection covers, not the one after it** (丟字第
        // 二輪, 2026-10-07). A selection made with `x` ends on the *start* of
        // the following line, so `line_end` there reached past the last line's
        // break: the break went inside the comment (a blank line before the
        // closing mark) and the file came back with no final newline.
        let to = to.min(rope.len_chars());
        let to = match to > start && crate::motion::line_start(&rope, to) == to {
            true => to - 1,
            false => to,
        };
        let end = crate::motion::line_end(&rope, to);
        let text: String = rope.slice(start..end).to_string();
        let rebuilt = match form {
            crate::comment::Form::Line(mark) => {
                // Splitting on `\n` leaves each piece carrying its own `\r`,
                // so a CRLF file comes back out of this one unchanged.
                let lines: Vec<&str> = text.split('\n').collect();
                crate::comment::toggle_line(&lines, mark).join("\n")
            }
            crate::comment::Form::Block(open, close) => {
                let ending = self.current_buffer().ending();
                crate::comment::toggle_block(&text, open, close, ending)
            }
        };
        if rebuilt == text {
            return;
        }
        self.snapshot();
        if !self.overwrite(start, end, &rebuilt) {
            return;
        }
        // The same lines stay selected, so the key can be pressed twice and
        // the second press undoes the first — which is what a toggle is.
        let last = start + rebuilt.chars().count();
        self.sel.set_anchor(start);
        self.sel.set_head(crate::motion::prev_grapheme(self.current_buffer().rope(), last).max(start));
        self.refresh_goal_column();
    }
}
