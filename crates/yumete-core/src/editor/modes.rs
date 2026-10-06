//! The mode, the selection, and the prompt line's own shape (#5).

use super::*;

impl Editor {
    // ---- Modal editing (Feature #5) ---------------------------------------

    /// The current editing mode.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Whether a half-finished key is waiting for **a character of the
    /// document** — `f`, `r`, `ms`, `mr` (§5.2.3 ②, #414).
    ///
    /// The front end asks so the IME may run for it: `f` then 中文 opens the
    /// candidate panel, and what is chosen is what `f` looks for. Which
    /// pendings those are is [`Pending::wants_the_ime`]'s to say; this is
    /// only the door it is asked through.
    ///
    /// Warning: **`mi`／`ma` 不在裏面**（2026-10-04）：它們等的是物件的名字。
    pub fn wants_the_ime(&self) -> bool {
        self.pending.wants_the_ime()
    }

    /// A status-line label for the current mode — `None` when the row below
    /// already says it ([`Mode::label`]).
    ///
    /// Three questions in one word, in this order:
    ///
    /// 1. **Where do the keys go?** `PAN.` when a side panel has them. That is
    ///    asked first because it is the one with teeth: `d` in the results
    ///    list clears the search word, `d` in the page deletes a line of the
    ///    novel, and both read `NORMAL` until 2026-09-27.
    /// 2. **Is the selection being extended?** `SEL`, the way Helix writes it.
    /// 3. **Otherwise the mode's own word** — three letters.
    pub fn mode_label(&self) -> Option<String> {
        if self.mode == Mode::Normal && self.sidebar_focused() {
            return Some("PAN.NOR".to_string());
        }
        // **選擇器也報狀態**（2026-10-01 定：「PICKER 下也可以显示状态，就像
        // PANEL 一样……这样的好处是用户可以马上知道现在是插入状态与否」）。
        //
        // Warning: **2026-09-27 定的是「不報」**，理由寫在 [`Mode::label`] 上：
        // 「選擇器是一扇蓋住正文的窗，再寫一個模式詞是同一句話說兩遍」。那一句
        // 說的是「哪一種模式」，而這裏報的是**鍵在哪一層**——選擇器真有兩層
        // （`picker.typing()`：打查詢詞 ／ 站在單子上），兩層的 `j` 不是一回
        // 事，正如 `PAN.` 之於正文。
        //
        // 順帶把那八個點與檔名之間那一格空氣也補回來了：模式一欄不空，狀態行
        // 那條「借一格給點站」的路就走得通（`draw_status`）。
        if self.mode == Mode::Picker {
            return Some(match self.picker.as_ref().is_some_and(|p| p.typing()) {
                true => "PIC.INS".to_string(),
                false => "PIC.NOR".to_string(),
            });
        }
        if self.extend && self.mode == Mode::Normal {
            return Some("SEL".to_string());
        }
        // **覆寫模式報 `REP`**（vim 的 `R`，2026-10-06）。vim 寫 `-- REPLACE --`；
        // 這一欄的詞一律三個大寫字母（`NOR` `INS` `SEL`），所以取頭三個。
        if self.overwriting && self.mode == Mode::Insert {
            return Some("REP".to_string());
        }
        self.mode.label().map(str::to_string)
    }

    /// Whether select (extend) mode is active.
    pub fn is_extending(&self) -> bool {
        self.extend
    }

    /// The cursor position in the active buffer, as a character index.
    pub fn cursor(&self) -> usize {
        self.caret()
    }

    /// The current selection as a character range `(start, end)` with
    /// `start <= end`. When `start == end` the selection is collapsed (just the
    /// cursor). Helix treats the cursor as a one-wide selection, so `d` still
    /// deletes the grapheme under a collapsed cursor.
    pub fn selection(&self) -> (usize, usize) {
        let (start, end) = (self.mark().min(self.caret()), self.mark().max(self.caret()));
        // The grapheme the cursor sits on is *inside* the selection, as it is
        // in Helix. Without this the block cursor covers a character that an
        // edit would not touch — `f。d` left the 。 behind, `e` never reached
        // the end of its word, and what the screen showed was not what `d` took.
        //
        // Insert mode is the exception: there the cursor is a bar between two
        // graphemes and covers nothing.
        if self.mode == Mode::Insert {
            return (start, end);
        }
        (
            start,
            motion::next_grapheme(self.current_buffer().rope(), end),
        )
    }

    /// **手上有不止一段選區沒有。**
    ///
    /// 前端問它是為了一件事：Warning: **多選區下不畫內嵌的 preedit**（2026-09-28 定，原話：
    /// 「我觉得是不是 multiselection 的时候应当禁止 pre-edit 而是用候选面板。这样的话防止
    /// 一堆非确定的修改」）。
    /// 只給測試：手上等的是不是一個字符（前端問的是 `Pending` 上同名的那一支）。
    #[cfg(test)]
    pub fn wants_the_ime_for_test(&self) -> bool {
        self.wants_the_ime()
    }

    pub fn has_many_selections(&self) -> bool {
        self.sel.is_plural()
    }

    /// **主選區以外的那幾段**，各自照 [`Self::selection`] 的辦法撐開一個字素
    /// （#405，2026-09-28）。
    ///
    /// Warning: **塌着的那些也要交出來。** 主選區塌着的時候有光標替它說話——橫排是終端自己
    /// 那個，竪排是畫進頁面的反白方塊——而次選區沒有光標，它塌着的時候能被看見的只有
    /// 它站的那一格底色。`C` 往下複製出來的那一串空光標全靠這個。
    ///
    /// 只有一段的時候回空，所以在沒有人造出第二段之前，畫面一格不動。
    pub fn secondary_selections(&self) -> Vec<(usize, usize)> {
        if !self.sel.is_plural() {
            return Vec::new();
        }
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();
        let insert = self.mode == Mode::Insert;
        self.sel
            .secondaries()
            .map(|one| {
                // Warning: **畫面這一支永遠不許崩**（2026-09-28 修）。狀態那一頭已經在
                // `clamp_cursor` 裏收過了，可是「繪製不許 panic」不能靠別人守規矩——
                // 一個指得太遠的下標最壞是畫錯一格，而 `next_grapheme` 拿到它是整個
                // 編輯器退出。日誌裏那一條就是這麼來的。
                let (start, end) = one.span();
                let (start, end) = (start.min(len), end.min(len));
                match insert {
                    true => (start, end),
                    false => (start, motion::next_grapheme(rope, end)),
                }
            })
            .collect()
    }

    /// The half-open range the cursor and anchor literally span, before the
    /// cursor's own grapheme is added. What motions and the caret work in.
    pub(super) fn span(&self) -> (usize, usize) {
        (self.mark().min(self.caret()), self.mark().max(self.caret()))
    }

    /// Whether the writer has actually selected a range, rather than merely
    /// standing on a character.
    ///
    /// [`Self::selection`] is never empty — the cursor's own grapheme is always
    /// in it — so it cannot answer this. The renderer needs the difference: a
    /// bare cursor is drawn as a cursor, not as a one-character highlight.
    pub fn has_selection(&self) -> bool {
        self.mark() != self.caret()
    }

    /// The text typed so far in Command mode (without the leading `:`).
    pub fn command_line(&self) -> &str {
        &self.command_line
    }

    /// How far into the prompt the caret is, in characters.
    pub fn prompt_caret(&self) -> usize {
        self.command_caret.min(self.command_line.chars().count())
    }

    /// The prompt's text up to the caret — what the front end measures to put
    /// the terminal's cursor in the right cell.
    pub fn prompt_before_caret(&self) -> String {
        self.command_line.chars().take(self.prompt_caret()).collect()
    }

    /// The active prompt (Command, Search, Ruby or `::`): what is written
    /// before it and the text typed so far, or `None` when no prompt is open.
    ///
    /// A **string** rather than a character, because `::` is two of them
    /// (#224) and a prompt that drew itself as `:` would be lying about which
    /// of the two lines the next Enter belongs to.
    pub fn prompt(&self) -> Option<(&'static str, &str)> {
        match self.mode {
            Mode::Command => Some((":", &self.command_line)),
            Mode::Lookfor => Some(("::", &self.command_line)),
            // Warning: **正則那一族借這一扇**（#405 Phase 2），所以前面寫的不一定是斜槓。
            // 按下去之後屏幕上那一行要說得出它要做什麼：選出、切開、只留、去掉，四件事
            // 做完的樣子差得很遠。
            Mode::Search => Some((
                match self.sift {
                    Some(what) => what.prefix(),
                    None if self.search_forward => "/",
                    None => "?",
                },
                &self.command_line,
            )),
            Mode::Ruby => Some(("注", &self.command_line)),
            _ => None,
        }
    }

    /// What to **draw** before the open prompt — the label, not the key (#505).
    ///
    /// [`Self::prompt`] answers with the character that was typed, which is
    /// what the caret arithmetic and the mode checks want and what the tests
    /// read. This is what the reader sees, and for a search it says the word:
    /// helix writes `search:` there, and a lone `/` on an otherwise empty row
    /// says less than the row costs. 「Helix 按下 / 命令行出现了 search:，我們是
    /// 不是可以對齊一下？」
    ///
    /// `:` and `::` keep their colons — those *are* the words, and `:` is the
    /// one prompt whose prefix a reader retypes.
    pub fn prompt_label(&self) -> Option<String> {
        let (prefix, _) = self.prompt()?;
        Some(match self.mode {
            Mode::Search => match self.search_forward {
                true => say!("ui.prompt-search"),
                false => say!("ui.prompt-search-back"),
            },
            _ => prefix.to_string(),
        })
    }

    /// What the open prompt is about to complete to — the part not yet typed,
    /// shown after the caret in a lighter ink and adopted with Tab.
    ///
    /// On the command line it is the rest of the best-matching command name; in
    /// a search it is the rest of the last pattern, so repeating a search is a
    /// keystroke rather than retyping it. Empty when there is nothing to guess,
    /// once arguments have started, or once Tab has already picked something —
    /// at that point the line *is* the completion.
    pub fn prompt_ghost(&self) -> String {
        if self.completion.is_some() {
            return String::new();
        }
        // **An empty search prompt already guesses** (#274).
        // 2026-09-05: 「`/` 搜索，enter 確認，再次按下 `/` 搜索，這個時候是不是
        // 應該預填寫（灰色）上次搜索過內容？」 — `Enter` on an empty line has
        // always repeated the last pattern, and the only thing missing was
        // *saying so*: the guess is the whole of it from the first keystroke,
        // so `/⏎` reads as「再找一次這個」rather than as a prompt you have to
        // remember what you last put in. Typing narrows it the way it always
        // did, and the first character that does not match takes it away.
        if self.mode == Mode::Search && self.command_line.is_empty() {
            return self.last_search.clone();
        }
        // The guess completes the *word* being typed, so a line with arguments
        // on it can still be guessed at: `:yume sch` guesses `eme`.
        let (start, _) = command::complete_at(&self.command_line);
        let typed = &self.command_line[start.min(self.command_line.len())..];
        if typed.is_empty() {
            return String::new();
        }
        let whole = match self.mode {
            // `written()`, the same as Tab writes (`cycle_completion`). A deep
            // match carries its parent — `:sch` is answered with `yume scheme`
            // — and offering the bare `name` guessed `:scheme`, a line that
            // does not parse, while Tab on the same keystroke wrote
            // `:yume-scheme`. Where the parent is not what was typed the guess
            // is now simply not offered, and Tab still says the whole thing.
            Mode::Command => command::complete(&self.command_line)
                .first()
                .map(|e| e.written()),
            Mode::Search => Some(self.last_search.clone()),
            _ => None,
        };
        whole
            .filter(|whole| whole.len() > typed.len() && whole.starts_with(typed))
            .map(|whole| whole[typed.len()..].to_string())
            .unwrap_or_default()
    }

    /// Take the prompt's guess, if there is one.
    pub(super) fn adopt_ghost(&mut self) {
        let drawn = self.prompt_ghost();
        self.command_line.push_str(&drawn);
        self.command_caret = self.command_line.chars().count();
    }

    /// The commands to offer for the open command line, and which one Tab has
    /// selected.
    pub fn command_menu(&self) -> (Vec<command::Choice>, Option<usize>) {
        match &self.completion {
            Some((prefix, i)) => (command::complete(prefix), Some(*i)),
            None => (command::complete(&self.command_line), None),
        }
    }

    #[cfg(test)]
    pub(super) fn recorded_keys_for_test(&self) -> String {
        self.macro_keys
            .iter()
            .map(|k| match k {
                Key::Char(c) => *c,
                _ => '?',
            })
            .collect()
    }

        /// The current transient status message (may be empty).
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Put a message on the status line (used by the shell for things the core
    /// cannot see, such as the IME's answer to `:chaifen`).
    pub fn set_status(&mut self, message: String) {
        self.status = message;
    }

    /// **一句說完就該走的話**——幾秒之後自己從命令行上下去。
    ///
    /// 2026-09-27 定，原話：「有些不是特别重要的消息可以有个参数「显示时间」，
    /// 比如几秒，过了这个时间就会从命令行消失。比如那个宽度 1/4。这样不遮挡按键
    /// 提示。」
    ///
    /// 誰該用它：**確認一下剛纔那個按鍵做了什麼**的那一類。邊欄寬了一檔、換了
    /// 一個區——按的人自己看得見結果，那句話只是個回聲。錯誤、問句、以及「這件事
    /// 沒做成」不許用：那幾種要一直站在那裏，等人讀到。
    pub(crate) fn murmur(&mut self, message: String) {
        self.status_fades = Some((std::time::Instant::now() + Self::MURMUR, message.clone()));
        self.status = message;
    }

    /// 一句話站多久。
    const MURMUR: std::time::Duration = std::time::Duration::from_secs(3);

    /// 還有多久輪到它走。`None` ＝ 沒有欠着的。
    pub fn status_due_in(&self) -> Option<std::time::Duration> {
        let (at, _) = self.status_fades.as_ref()?;
        Some(at.saturating_duration_since(std::time::Instant::now()))
    }

    /// 鐘響了：那句話真還在的話，把它拿下去。
    ///
    /// Warning: **比對過內容纔動手。** 這三秒裏狀態行可能已經被別的事寫過了，而那一句
    /// 不是這個鐘管的。
    pub fn status_tick(&mut self) {
        let Some((at, said)) = self.status_fades.as_ref() else {
            return;
        };
        if std::time::Instant::now() < *at {
            return;
        }
        if self.status == *said {
            self.status.clear();
        }
        self.status_fades = None;
    }

    /// What opening a file had to say, if it had to say anything (#380).
    ///
    /// Taken, not read: the front end clears the status on its way out of
    /// setup — deliberately, so that none of the installation chatter reaches
    /// the first frame — and this is the one line that has to survive that
    /// and be put back. Only a door that *guessed* leaves anything here.
    pub fn take_open_notice(&mut self) -> Option<String> {
        self.open_notice.take()
    }

    /// The 0-based line the cursor is on.
    pub fn cursor_line(&self) -> usize {
        self.current_buffer().rope().char_to_line(self.caret())
    }

    /// The 0-based **character** column the cursor is at within its line.
    ///
    /// Not [`Self::cursor_visual_column`], which is cells: this is the column
    /// a drawn run is anchored at, and those are counted in characters the way
    /// `hidden` is (Feature #211).
    pub fn cursor_column(&self) -> usize {
        let rope = self.current_buffer().rope();
        let at = self.caret().min(rope.len_chars());
        at - rope.line_to_char(rope.char_to_line(at))
    }

    /// The cursor's visual column (summed display width within its line).
    ///
    /// **The page's column, not the text's** (#374). What is drawn before the
    /// caret is part of where the caret *is*: the space a tab advances over,
    /// the padding that squares a table up. Counting only the characters put
    /// the readout at 3 while the screen had the caret at 8 — and a caret
    /// standing in a column the page does not have is the one thing #212 says
    /// may never happen.
    pub fn cursor_visual_column(&self) -> usize {
        let text = motion::visual_column(self.current_buffer().rope(), self.caret());
        let line = self.cursor_line();
        let col = self.cursor_column();
        let drawn: usize = self
            .drawn_runs_on_line(line)
            .iter()
            // A run at the caret's own anchor is the one case that splits:
            // what the writer **typed** stands before the caret, and what is
            // derived — the padding reaching on to the pipe — stands after it.
            .filter(|run| run.column < col || (run.column == col && run.ink == crate::drawn::Ink::Typed))
            .map(|run| yumete_cjk::str_width(&run.text))
            .sum();
        // Warning: **藏起來的那些也要減掉**（2026-10-02 一輪掃查報來的）。這一支加
        // 了畫出來的（註號、補齊的格寬），卻從來沒減過藏起來的（`**`、`(url)`、
        // `[^1]` 的本體）——於是 `:render full` 底下報出來的列號**比那一行畫出來
        // 的還長**：五百七十三個光標位置裏有四十九個這樣，最遠超出二十三格。
        //
        // 畫的那一邊問的是 [`crate::wrap::position`]，它**先濾掉藏起來的**再加畫
        // 出來的。兩邊是同一條規矩的兩份推導，只在「什麼都沒藏」的時候一致——
        // `:render off`／`basic` 下一直對，所以沒人發現。
        let gone: usize = match self.line_text(line) {
            Some(text) => {
                let chars: Vec<char> = text.chars().collect();
                self.hidden_on_line(line)
                    .iter()
                    .map(|&(from, to)| (from.min(col), to.min(col)))
                    .filter(|(from, to)| to > from)
                    .map(|(from, to)| {
                        chars[from.min(chars.len())..to.min(chars.len())]
                            .iter()
                            .map(|&c| yumete_cjk::char_width(c))
                            .sum::<usize>()
                    })
                    .sum()
            }
            None => 0,
        };
        (text + drawn).saturating_sub(gone)
    }

    /// The character under the cursor, for the status line to name.
    ///
    /// At the end of a line — where Insert mode spends most of its time —
    /// there is nothing under the cursor, so the character *before* it is the
    /// answer instead: what a writer wants named is the 字 they are looking at,
    /// and having just typed it counts as looking at it.
    pub fn char_at_cursor(&self) -> Option<char> {
        let rope = self.current_buffer().rope();
        let here = (self.caret() < rope.len_chars()).then(|| rope.char(self.caret()));
        match here {
            Some(c) if c != '\n' && c != '\r' => Some(c),
            _ => (self.caret() > 0)
                .then(|| rope.char(self.caret() - 1))
                .filter(|&c| c != '\n' && c != '\r'),
        }
    }
}
