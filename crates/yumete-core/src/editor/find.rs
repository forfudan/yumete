//! The search panel's own half of the editor — Feature #419.
//!
//! Running what the box holds, and the keys of the form it sits in. The
//! panel's *state* is [`crate::search_panel`]; drawing it is the front end's.

use super::*;
use crate::search_panel::{Case, Field, Hit, Where, AROUND, MOST};
use std::path::{Path, PathBuf};

impl Editor {
    /// The search panel, for the front end to draw.
    pub fn search(&self) -> &crate::search_panel::Search {
        &self.search
    }

    /// The pattern `n` and `N` are walking — the panel writes it too (#419).
    pub fn last_search(&self) -> &str {
        &self.last_search
    }

    /// **Open the panel and put a pattern in it, selected** — `空格 /`, #419.
    ///
    /// What goes in the box: the selection if there is one (you marked it, the
    /// intention is on the screen), otherwise the last thing searched for. It
    /// arrives selected, so typing replaces it and `Enter` keeps it — both
    /// intentions in one key, which is how VSCode's box behaves.
    /// `:search-cd`／`-wd`／`-gd`／`:search <path>` — open it looking somewhere
    /// else (#419).
    pub(super) fn open_search_in(&mut self, scope: Where, replacing: bool) {
        // ⚠️ **A folder that is not there is said out loud.** Falling back to
        // 「this file only」 would answer a question nobody asked, and answer
        // it plausibly — a short list that looks like the truth.
        if let Where::Named(path) = &scope {
            let named = path.clone();
            if self.search_root_of(&scope).is_none() {
                self.status = say!("search.no-such-folder", named.display());
                return;
            }
        }
        self.search.scope = scope;
        // ⚠️ **Only ever turned on here.** `:search` after a `:replace` is a
        // reader saying 「just looking」, and leaving the row up would leave
        // `r` and `R` live on a panel nobody meant to change anything with.
        self.search.replacing = replacing;
        // ⚠️ **模糊 comes off when the panel starts changing things**
        // (2026-09-20). A loose match covers characters nobody typed, so
        // 「replace them all」 would hand the manuscript a range the writer
        // cannot predict. The switch is not even in the form while replacing
        // (`Field::step`), and leaving the *flag* on would have made the list
        // loose while the switch that says so was out of sight.
        if replacing {
            self.search.fuzzy = false;
        }
        self.open_search();
    }

    pub(super) fn open_search(&mut self) {
        // ⚠️ A selection wins over the last pattern — but only one somebody
        // **made**. Every motion in this editor leaves a selection and the
        // cursor covers its own grapheme, so 「one character」 is where the
        // cursor is standing, not something marked; taking it would mean the
        // box never once opened holding the last pattern. And half a chapter
        // is a paste, not a search.
        let (from, to) = self.selection();
        let rope = self.current_buffer().rope();
        let marked: String = match to > from + 1 && to - from <= 64 {
            true => rope.slice(from..to.min(rope.len_chars())).chars().collect(),
            false => String::new(),
        };
        // ⚠️ **The box remembers what was typed, not what was compiled.**
        // `last_search` holds the pattern the engine runs — flags and all —
        // and showing `(?i)霜` to a reader who typed 霜 would be showing them
        // the plumbing. The panel's own query is that memory; `last_search` is
        // the fallback for a `/` typed on the page.
        let seed = match (!marked.is_empty() && !marked.contains('\n'), self.search.query.is_empty()) {
            (true, _) => marked,
            (false, false) => self.search.query.clone(),
            (false, true) => self.last_search.clone(),
        };
        self.search.ask(seed);
        // **那一格裏先寫着此刻的範圍**，這樣 `k` 上去 `i` 進去，改的是看得見的
        // 那一份，而不是一個空框（2026-09-23）。
        self.search.scope_text = self.scope_as_typed();
        self.show_sidebar(crate::sidebar::View::Search);
        // The form is entered where a reader would start typing.
        self.mode = Mode::Field;
        // ⚠️ **Opened onto a folder, it looks straight away** rather than
        // waiting for an `Enter` nobody knows to press: the reader just named
        // a place, and the pattern was already in the box.
        match self.search.scope.live() {
            true => self.run_search(),
            false => self.search_now(),
        }
    }

    /// What the box holds, as a pattern the engine understands.
    ///
    /// Four settings fold into one string here rather than into four branches
    /// at the point of use: 正則 off escapes the whole thing, 完整匹配 wraps it
    /// in `\b`, and 大小寫 puts the flag on the front. The engine sees one
    /// pattern and the panel is the only place that knows why.
    /// **換成什麽** —— 和 [`Self::search_pattern`] 同一條規矩的另一半。
    ///
    /// ⚠️ **正則關着的時候，右邊也要照字面。** 左邊一直是照規矩辦的（`regex::
    /// escape`），而右邊從前**無條件走展開**——於是「正則」那一格明明沒勾，
    /// `US$100` 裏的 `$100` 還是被讀成第 100 個捕獲組（空的），換出來只剩 `US`。
    ///
    /// | 查 | 換成 | 從前得到 |
    /// | --- | --- | --- |
    /// | `一百元` | `US$100` | `US` |
    /// | `甲` | `$x^2$` | `^2$` |
    /// | `甲` | `價$x元` | `價元` |
    ///
    /// ⚠️ **這不是小事**：`R` 是「每個檔每一處」，而寫 Typst 的人滿篇 `$…$`
    /// （那是數學），寫稿的人滿篇錢號。橫跨整本書靜靜刪字，而畫面上那一格寫着
    /// 「[ ] 正則」。2026-09-24 審出來的，實測。
    ///
    /// `$$` 是 `regex` 那一頭「一個真的錢號」的寫法，所以照字面就是把 `$` 加倍。
    fn replacement(&self) -> String {
        match self.search.regex {
            true => self.search.replace.clone(),
            false => self.search.replace.replace('$', "$$"),
        }
    }

    fn search_pattern(&self) -> String {
        let mut body = match self.search.regex {
            true => self.search.query.clone(),
            false => regex::escape(&self.search.query),
        };
        // ⚠️ **`\b` is nothing between 漢字.** There is no word boundary
        // there, so this only ever bites on the Western words in a manuscript
        // — which is what it does in VSCode too, and what the manual says.
        if self.search.whole {
            body = format!(r"\b{body}\b");
        }
        // ⚠️ **Sensitive says so out loud** (`(?-i)`), it does not just stay
        // silent. The page's own `n` runs this pattern through smart case
        // again (`compile`), which would put `(?i)` in front of a quiet one
        // and search differently from the panel that produced it. Flags apply
        // in order, so an explicit one at the front wins.
        match self.search.case {
            Case::Sensitive => format!("(?-i){body}"),
            Case::Insensitive => format!("(?i){body}"),
            // The rule the page's own `/` follows: a capital is how you ask.
            Case::Smart => match body.chars().any(char::is_uppercase) {
                true => body,
                false => format!("(?i){body}"),
            },
        }
    }

    /// **Where the search is rooted**, for a scope that is not the buffer.
    ///
    /// `None` when the answer cannot be worked out — no file open to reckon
    /// from, or a named folder that is not there.
    fn search_root(&self) -> Option<PathBuf> {
        self.search_root_of(&self.search.scope)
    }

    /// The same, for a scope that has not been adopted yet.
    fn search_root_of(&self, scope: &Where) -> Option<PathBuf> {
        match scope {
            Where::Buffer => None,
            // The file's own folder. ⚠️ Not the project: a book's drafts, its
            // notes and its exports live under one tree, and 「this folder and
            // what is under it」 is the near thing a reader means.
            Where::Folder => Some(self.here_folder()),
            Where::Workspace => std::env::current_dir().ok(),
            Where::Project => Some(self.project_root()),
            Where::Named(path) => {
                let full = match path.is_absolute() {
                    true => path.clone(),
                    false => self.here_folder().join(path),
                };
                full.is_dir().then_some(full)
            }
        }
    }

    /// The folder the file being written is in, as an absolute path.
    ///
    /// ⚠️ **Resolved against the working directory first.** A buffer opened as
    /// `a.md` has a relative path, and its `parent()` is the *empty* path —
    /// which as a root walks nothing at all, so 「this folder」 quietly found
    /// only the file already open.
    pub(super) fn here_folder(&self) -> PathBuf {
        let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        match self.current_buffer().path() {
            Some(path) => here.join(path).parent().map(Path::to_path_buf).unwrap_or(here),
            None => here,
        }
    }

    /// **Run what the box holds** — #419.
    ///
    /// The buffer is searched on every keystroke: it is in memory and a pass
    /// over it costs nothing worth counting. ⚠️ **Anything wider waits for
    /// `Enter`** — a hundred chapters read off the disk per letter typed is
    /// not a thing to do — and until then the panel says so rather than
    /// showing a list that answers an older question.
    pub(super) fn run_search(&mut self) {
        if !self.search.scope.live() {
            self.search.stale = true;
            return;
        }
        self.search_now();
    }

    /// Run it whatever the scope, walking the disk if that is what it takes.
    pub(super) fn search_now(&mut self) {
        self.search.broken = false;
        if !self.search.asked() {
            self.search.hits.clear();
            self.search.total = 0;
            self.search.selected = 0;
            return;
        }
        let pattern = self.search_pattern();
        let look = match self.search.fuzzy {
            true => Look::Nearby {
                needle: self.search.query.chars().collect(),
                fold: match self.search.case {
                    Case::Insensitive => true,
                    Case::Sensitive => false,
                    // The rule the page's own `/` follows: a capital is how
                    // you ask for case to matter.
                    Case::Smart => !self.search.query.chars().any(char::is_uppercase),
                },
            },
            false => match regex::Regex::new(&pattern) {
                Ok(re) => Look::Pattern(re),
                Err(_) => {
                    // ⚠️ The hits stay, and are drawn quiet. Typing a regular
                    // expression walks through `[`, `(` and every other
                    // unfinished state; emptying the list on each of them
                    // flickers, and a blank list would say 「nothing found」,
                    // which is not true.
                    self.search.broken = true;
                    return;
                }
            },
        };
        self.search.hits.clear();
        self.search.total = 0;
        self.search.selected = 0;
        self.search.folded.clear();
        self.search.stale = false;
        // **The pattern hands the search to the panel, and the page follows.**
        // One 「what am I looking for」 with two ways in: the highlight and
        // `n`/`N` are the same search, which is what [^415]记 `:grep` 不寫
        // `last_search` 為缺口的那條理由.
        // ⚠️ **模糊 does not hand the page a pattern it cannot keep.** `n`/`N`
        // and `:s` run a regular expression, and there is no regular
        // expression for 「these characters, nearly in a row」 — so what they
        // are left with is the query *itself*, exactly. The panel lists the
        // near misses; the page walks the exact ones, which are a subset of
        // them, and neither is lying about the other.
        self.last_search = match self.search.fuzzy {
            true => regex::escape(&self.search.query),
            false => pattern,
        };
        let root = self.search_root();
        // ⚠️ **Compared as absolute paths.** A buffer opened as `a.md` and the
        // same file coming out of the walk as `/…/卷一/a.md` are one file, and
        // the guard that failed to see that searched it twice.
        let here = self
            .current_buffer()
            .path()
            .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()));
        // Where the file being written sits in the answer: under its own name
        // when the answer has names in it, and namelessly when it is the whole
        // of the answer.
        let mine = match (&root, &here) {
            (Some(root), Some(here)) => Some(
                here.strip_prefix(std::fs::canonicalize(root).as_deref().unwrap_or(root))
                    .unwrap_or(here)
                    .to_path_buf(),
            ),
            _ => None,
        };
        let mut hits = Vec::new();
        let mut total = 0usize;
        // It comes first, and it comes from memory: what is on the screen is
        // what is searched, saved or not.
        let rope = self.current_buffer().rope();
        let mut at = 0usize;
        for line in 0..rope.len_lines() {
            let text: String = rope.line(line).chars().collect();
            for (nth, (start, stop)) in look.spans(&text).into_iter().enumerate() {
                total += 1;
                if hits.len() < MOST {
                    hits.push(excerpt(mine.clone(), &text, at, start, stop, line, nth));
                }
            }
            at += text.chars().count();
        }
        if let Some(root) = root {
            let mut files = Vec::new();
            crate::editor::walk(&root, &mut 0, &mut |path| files.push(path.to_path_buf()));
            for path in files {
                // Not twice: the one being written was searched from memory.
                let full = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if Some(&full) == here.as_ref() {
                    continue;
                }
                // **An open file is read from its buffer, not from disk.**
                // Unsaved work is work, and a search that could not see it
                // would send a reader to a line that no longer says that.
                // ⚠️ **Matched on the resolved path.** `/tmp` is a link to
                // `/private/tmp` on this platform, so the walk's path and the
                // buffer's are two spellings of one file — compared as typed,
                // a file just changed in a buffer was re-read off the disk and
                // the change looked as though it had not happened.
                let text = match self.buffers.iter().find(|b| {
                    b.path()
                        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
                        .as_deref()
                        == Some(full.as_path())
                }) {
                    Some(buffer) => buffer.rope().to_string(),
                    None => match std::fs::read_to_string(&path) {
                        Ok(text) => text,
                        // Not text, or not readable: not this writer's prose.
                        Err(_) => continue,
                    },
                };
                let shown = path.strip_prefix(&root).unwrap_or(&path).to_path_buf();
                let mut at = 0usize;
                for (line, text) in text.split_inclusive('\n').enumerate() {
                    for (nth, (start, stop)) in look.spans(text).into_iter().enumerate() {
                        total += 1;
                        if hits.len() < MOST {
                            hits.push(excerpt(
                                Some(shown.clone()),
                                text,
                                at,
                                start,
                                stop,
                                line,
                                nth,
                            ));
                        }
                    }
                    at += text.chars().count();
                }
            }
            self.search.root = Some(root);
        } else {
            self.search.root = None;
        }
        self.search.hits = hits;
        self.search.total = total;
    }

    /// One key while a field of the search panel has them (`Mode::Field`).
    pub(super) fn on_field_key(&mut self, key: Key) {
        match key {
            // **`Enter` 只做一件事：跑一遍搜索**（2026-09-25 定，原話：「按下
            // Enter「只」触发搜索。他不更改光标位置，不更改状态……这样的好处是在
            // 搜的到/搜不到东西的时候，enter的行为都是一样的」）。
            //
            // ⚠️ **它從前還兼着「把鍵交到第一條結果上」**（同一天早些時候定的），
            // 而那讓它在找得到和找不到的時候做兩件不同的事——正是這一條想去掉的
            // 分岔。去結果現在是 `Esc` 然後 `j`，兩個已經學過的鍵。
            Key::Enter => {
                self.search.all_selected = false;
                self.look_again();
                // **打完了，去找——然後把鍵交回面板**（2026-09-25 補的：「enter键
                // 除了触发搜索，还是最好能回到 normal mode」）。它因此讀成一句
                // 完整的話，而不是「跑一遍，然後你還在框裏」。
                //
                // ⚠️ **落在哪一格不動。** 找得到找不到都一樣，這正是這個鍵要的
                // 那份一致；去結果是接着按 `j`。
                self.mode = Mode::Normal;
            }            Key::Char(ch) => {
                self.search.type_char(ch);
                if self.search.field != Field::Scope {
                    self.run_search();
                }
            }
            Key::Backspace => {
                self.search.backspace();
                if self.search.field != Field::Scope {
                    self.run_search();
                }
            }
            Key::Left => {
                let to = self.search.caret.saturating_sub(1);
                self.search.move_caret(to);
            }
            Key::Right => {
                let to = self.search.caret + 1;
                self.search.move_caret(to);
            }
            Key::Home => self.search.move_caret(0),
            Key::End => {
                let to = self.search.query.chars().count();
                self.search.move_caret(to);
            }
            // The word and the line, as they are on the `:` line.
            Key::Ctrl('u') => {
                self.search.ask(String::new());
                self.run_search();
            }
            // ⚠️ **`Tab` is the slot's own key** (2026-09-17): it walks the
            // views that live in this slot, in every panel, and this one had
            // taken it — so a reader who opened 尋找 could not get back to the
            // tree without closing it. 「到了高級搜索的，tab 變成了『下一個』
            // 項目，再也切不到其他面板了」. The next box is `↓`, which is
            // where a form's next field is anyway.
            Key::Down => self.leave_field(false),
            Key::Up => self.leave_field(true),
            // **`Esc` 出框，打的字留着**（2026-09-25 報的：「按下Esc，输入的东西
            // 就还原了。这个不是「退回到normal」，而是放弃编辑」）。
            //
            // ⚠️ **從前也沒有真的還原**——`scope_text` 一直在，是**畫**的時候只在
            // 打字態纔顯示它，出框就退回按真實範圍算出來的名字，看着像還原了。
            // 所以修法是讓它**落地**（作者定：「算，离开格子就落地」），這樣屏幕上
            // 寫着什麼就是什麼。
            Key::Esc => {
                self.search.all_selected = false;
                self.mode = Mode::Normal;
                self.land_the_scope();
            }
            _ => {}
        }
    }

    /// A committed string from the IME lands in the field, not in the page.
    pub(super) fn type_into_field(&mut self, text: &str) {
        self.search.type_text(text);
        self.run_search();
    }

    /// `Tab` out of a box: the next cell.
    ///
    /// ⚠️ **Landing on another box keeps you typing.** 找什麼 and 換成什麼 sit
    /// one above the other and are filled in one after the other; having to
    /// press `i` between them would make `Tab` the wrong key for the commonest
    /// thing anybody does in this panel.
    fn leave_field(&mut self, back: bool) {
        // 離開「位置」那一格，不管走的是哪一條路，都落地。
        self.land_the_scope();
        self.search.field = self.search.field.step(back, self.search.replacing);
        self.search.all_selected = false;
        match self.search.field.takes_text() {
            true => self.search.caret = self.search.typed().chars().count(),
            false => self.mode = Mode::Normal,
        }
    }

    /// One key while the search panel has them and the box does not
    /// (`Mode::Normal`, the keys in the panel).
    pub(super) fn on_search_panel_key(&mut self, key: Key, side: crate::sidebar::Side) {
        match key {
            // `Tab` walks the slot's views, as it does in every other panel;
            // the form's own cells are `hjkl` (below) and `↑`／`↓`.
            Key::Tab => return self.cycle_view(side, false),
            Key::BackTab => return self.cycle_view(side, true),
            // **`hl` 橫着走字，`jk` 竪着走格**（2026-09-25 定，原話：「這一個光標
            // 所在的字是反白的……用戶這樣就能用hl在搜索欄中移動光標」）。和正文
            // 一個感覺：框裏站着一個塊光標，`h`／`l` 挪它，`i` 就從它那裏插。
            //
            // ⚠️ **格子之間從此只有 `jk`**（作者定：「行，格子只用 jk」）。從前
            // `hl` 和 `jk` 走的是同一串格子，那時框裏沒有光標可挪，`hl` 也就沒有
            // 別的事可做。
            //
            // 結果列表是例外：那裏 `h`／`l` 是摺起／打開一個檔，和文件樹、大綱說的
            // 「少一點／多一點」是同一件事；到頂了 `h` 出去，免得困在列表裏。
            Key::Char('h') | Key::Left if self.search.field == Field::Results => {
                if !self.search.fold(true) {
                    let back = self.search.field.step(true, self.search.replacing);
                    self.search.stand_on(back);
                }
            }
            Key::Char('l') | Key::Right if self.search.field == Field::Results => {
                self.search.fold(false);
            }
            Key::Char('h') | Key::Left => {
                let to = self.search.caret.saturating_sub(1);
                self.search.move_caret(to);
            }
            Key::Char('l') | Key::Right => {
                let to = self.search.caret + 1;
                self.search.move_caret(to);
            }
            Key::Char('j') | Key::Down => match self.search.field {
                Field::Results => self.search.step(true),
                _ => {
                    let next = self.search.field.step(false, self.search.replacing);
                    self.search.stand_on(next);
                }
            },
            Key::Char('k') | Key::Up => match self.search.field {
                // ⚠️ **到頂了就出去**（2026-09-23 報的：「我一旦將光標移動到了下面
                // 文件的區域，就沒辦法使用 k 向上移動到選項和輸入框了」）。從前
                // `step(false)` 在第 0 條上飽和，於是列表是個進得去出不來的地
                // 方——`Tab` 走得出去，可沒人會想到去按它。
                Field::Results if self.search.selected == 0 => {
                    let back = self.search.field.step(true, self.search.replacing);
                    self.search.stand_on(back);
                }
                Field::Results => self.search.step(false),
                _ => {
                    let back = self.search.field.step(true, self.search.replacing);
                    self.search.stand_on(back);
                }
            },
            // **The five switches are numbered, top to bottom** (2026-09-24,
            // 原話：「中间五行选项，在normal状态下不是移动上去按空格，而是直接
            // 通过一个字母来选择（在这一行后面显示这个字母快捷键）」). The
            // number is drawn at the end of its row, so the panel says what to
            // press rather than asking anybody to remember it.
            //
            // ⚠️ **模糊 while 替換 is ticked does nothing**, as it did before:
            // the row is drawn quiet, and a quiet row that still flipped would
            // be saying two things at once. `flip_switch` guards it.
            Key::Char(ch) if ch.is_ascii_digit() && ch != '0' => {
                let nth = ch as usize - '1' as usize;
                if let Some(&which) = Field::SWITCHES.get(nth) {
                    if !(self.search.replacing && which == Field::Fuzzy) {
                        self.flip_switch(which);
                    }
                }
            }
            Key::Enter => match self.search.field {
                Field::Scope | Field::Query | Field::Replace => self.mode = Mode::Field,
                Field::Results => self.go_to_hit(),
                // 走不到的格子——開關只認 `1`–`5`。
                other if other.is_switch() => self.flip_switch(other),
                _ => {}
            },
            // **`r` and `R` change things**, and only while the replace row
            // is showing — `:search` is for looking, `:replace` for changing,
            // and the panel says which it is.
            Key::Char('r') if self.search.replacing && self.search.field == Field::Results => {
                match self.search.row() {
                    Some(crate::search_panel::Row::File { path, .. }) => {
                        let done = self.replace_file(Some(&path));
                        self.after_replacing(done);
                    }
                    _ => self.replace_hit(),
                }
            }
            // ⚠️ **Only 「all of them」 asks first.** One hit and one file are
            // changes a reader is looking straight at; every file in a book is
            // not, and that is the one where a slip costs an afternoon.
            Key::Char('R') if self.search.replacing => {
                self.status = say!("search.replace-all-sure", self.search.total);
                // ⚠️ **`ReplaceAll`, not `Confirm`.** The latter is `:s …c`'s
                // per-match walker: with nothing to walk it clears itself on
                // the next key, so the question was asked and the answer went
                // nowhere.
                self.pending = Pending::ReplaceAll;
            }
            // **`F5` runs it again**, for a scope that does not run itself.
            Key::Char('F') if !self.search.scope.live() => self.search_now(),
            // **`/` 回搜索框：進 insert、光標放末尾、框裏的字留着**。
            //
            // ⚠️ **和選擇器（`空格 f`／`空格 b`／`:wiki`）那一扇裏的 `/` 一個
            // 樣**——2026-09-25 作者報的就是這一條：「用户在相似的界面按同样的
            // 快捷键，他的行为应该是一致的」。它原先是「整條選中」（`ask`），於是
            // 整格白底、看不見光標，看着既不像插入模式也不像光標在末尾。
            //
            // ⚠️ **它和 `i` 不同的地方是「不管現在站在哪一格」**：站在換框上、
            // 站在結果上，`/` 都回到搜索框；`i` 只在當前那一格打不了字的時候纔
            // 挪窩，而且是**從光標處**插。改搜索詞是這扇面板裏最常做的事。
            Key::Char('/') => {
                self.search.stand_on(Field::Query);
                self.mode = Mode::Field;
            }
            // `i` opens a box — this one if the keys are on one, the query
            // otherwise. The key that means 「type here」 everywhere else.
            Key::Char('i') => {
                // ⚠️ **從光標那裏插，不再跳到末尾**（2026-09-25 定）。`hl` 挪了
                // 半天光標，一按 `i` 又回末尾，那就是挪了白挪。光標出廠就在末尾，
                // 所以不挪的人感覺一點沒變。
                if !self.search.field.takes_text() {
                    self.search.field = Field::Query;
                    self.search.caret = self.search.typed().chars().count();
                }
                self.mode = Mode::Field;
            }
            Key::Char('g') | Key::Home if self.search.field == Field::Results => {
                self.search.selected = 0
            }
            Key::Char('G') | Key::End if self.search.field == Field::Results => {
                self.search.selected = self.search.hits.len().saturating_sub(1)
            }
            // `C-w` `q` `:` and a bare `Space` are every panel's, not this
            // one's — see `panel_key_in_common`. Tried last, so this panel's
            // own `Space` (flip the switch the keys are on) still wins.
            other => {
                self.panel_key_in_common(other, side);
            }
        }
    }

    /// **The highlighted hit, with room to read it** — the command row (#419).
    ///
    /// `None` unless the keys are actually in the list: the row belongs to
    /// whatever has them, and a panel nobody is standing in has no claim on it.
    #[cfg(test)]
    pub(crate) fn hit_in_context_for_test(&self) -> Option<String> {
        self.hit_in_context().map(|(line, text, _)| format!("{line} {text}"))
    }

    /// 測試要看反白落在哪幾個字上。
    #[cfg(test)]
    pub(crate) fn hit_mark_for_test(&self) -> Option<(String, std::ops::Range<usize>)> {
        self.hit_in_context().map(|(_, text, mark)| (text, mark))
    }

    /// 測試要擺一條命中進去——`search` 本身不是公開的。
    #[cfg(test)]
    pub(crate) fn search_for_test(&mut self) -> &mut crate::search_panel::Search {
        &mut self.search
    }

    /// The highlighted hit, its line number, and where the match sits in it.
    ///
    /// **The text comes back untrimmed and the row does the fitting.** How
    /// much of it fits is a question about the window, and the window is the
    /// front end's to know; what the editor knows is which characters are
    /// around the match and which ones *are* the match. The range counts
    /// characters into the text handed back, so the row can pick the word out
    /// however it likes.
    pub(super) fn hit_in_context(&self) -> Option<(usize, String, std::ops::Range<usize>)> {
        if self.search.field != Field::Results || self.search.broken {
            return None;
        }
        let side = self.panel_focus()?;
        // 光標把別的東西頂上來的時候，鍵雖然在這個邊欄裏，眼前那一個卻不是搜索。
        if self.transient(side).is_some()
            || self.panel(side).map(|p| p.view()) != Some(crate::sidebar::View::Search)
        {
            return None;
        }
        let hit = self.search.here()?;
        // ⚠️ **命中不一定在眼前這個緩衝區裏，而這裏問的是眼前這一個。**
        // `:search .` 搜的是整個文件夾，命中帶着自己的檔（`Hit::file`）；拿一條
        // 第 6496 行的命中去問一份**只有一行**的 scratch，ropey 當場 panic
        // ——2026-09-23 報的：在倉裏 `ye` 空開、`:search .`、Esc、按 `j` 走到結果
        // 列表上，一進去就崩。
        //
        // ⚠️ **行號也要夾。** 就算命中真在這一份裏，搜索是那一刻跑的，而之後
        // 刪掉幾段就能讓行號指到文件外面去。
        let rope = self.current_buffer().rope();
        if hit.file.is_some() || hit.line >= rope.len_lines() {
            // 別的檔（或者已經對不上了）：搜索當時抓下來的那一小段就是答案，
            // 而它本來就是為了「一欄放得下」裁過的。
            return Some((hit.line + 1, hit.excerpt.clone(), hit.mark.clone()));
        }
        // As many characters as a window is wide, centred on the match — far
        // more than the column can hold, which is the whole point.
        let line: String = rope.line(hit.line).chars().filter(|c| *c != '\n').collect();
        let chars: Vec<char> = line.chars().collect();
        // ⚠️ **偏移也要夾，不只是行號。** 上面那一句夾的是 `hit.line`，而
        // `hit.at` 是**搜索那一刻**的全文字符偏移——之後在命中上面刪掉一段，行號
        // 還落在文件裏而偏移已經不在這一行裏了。兩頭各壞一種：
        //
        // | `hit.at` 在哪 | 從前 |
        // | --- | --- |
        // | 這一行**之後** | `from > to`，切片反着來，**當場 panic** |
        // | 這一行**之前** | `saturating_sub` 歸零，不崩，**摘出來的是錯的一段** |
        //
        // 實測（2026-09-24 審出來的）：`空格 /` 搜本檔、Esc `j` 進結果、`C-w` 回
        // 正文、在命中上面 `dd`、`C-w` `j` 走回結果——回去那一幀就崩
        // （`range start index 67 out of range for slice of length 8`）。
        //
        // 對不上就走**上面那條退路**：搜索當時抓下的那一小段。它本來就是為這件事
        // 存的，而一個「差不多對」的摘要比一個錯的摘要還難發現。
        let head = rope.line_to_char(hit.line);
        let Some(at) = hit.at.checked_sub(head).filter(|at| *at <= chars.len()) else {
            return Some((hit.line + 1, hit.excerpt.clone(), hit.mark.clone()));
        };
        let from = at.saturating_sub(AROUND);
        let to = (at + AROUND).min(chars.len());
        let mut text = String::new();
        if from > 0 {
            text.push('…');
        }
        // 前面那個省略號也佔一個字，反白從它之後數起。
        let lead = text.chars().count();
        text.extend(&chars[from..to]);
        if to < chars.len() {
            text.push('…');
        }
        // ⚠️ **命中本身可能比摘出來的這一段還長**（一條 `.*` 規則能匹配整行），
        // 所以尾巴要夾在摘出來的這一段裏，不能照 `hit.end` 直接算。
        let long = (hit.end - hit.at).min(to - at.min(to));
        let mark = lead + (at - from)..lead + (at - from) + long;
        Some((hit.line + 1, text, mark))
    }

    /// Flip one of the five switches, and search again.
    ///
    /// **Which one is an argument**, not 「wherever the keys are」: since
    /// 2026-09-24 the cursor does not stop on a switch at all — `1`–`5` press
    /// them and `j k` walk past them.
    fn flip_switch(&mut self, which: Field) {
        match which {
            // ⚠️ **正則／完整匹配 and 模糊 are alternatives, so asking for one
            // puts the other down** rather than leaving a tick that does
            // nothing. They are drawn quiet while 模糊 is on, and a dimmed
            // switch that still flips would be saying two things at once.
            Field::Regex => {
                self.search.regex = !self.search.regex;
                self.search.fuzzy &= !self.search.regex;
            }
            Field::Case => self.search.case = self.search.case.next(),
            Field::Whole => {
                self.search.whole = !self.search.whole;
                self.search.fuzzy &= !self.search.whole;
            }
            Field::Fuzzy => self.search.fuzzy = !self.search.fuzzy,
            Field::Replacing => return self.flip_replacing(),
            _ => return,
        }
        self.run_search();
    }

    /// **勾上「替換」就長出替換行**（2026-09-23 報的）。
    ///
    /// ⚠️ **和 模糊 互斥，而勾這一個的時候把那一個關掉、畫灰**（作者定：「我傾向
    /// 自動關掉畫灰」）。理由是原來那一條：鬆的匹配蓋住讀者沒打的字，「把它們全
    /// 換掉」交出去的範圍他預測不了。和 `flip_switch` 裏 正則／完整匹配 壓掉 模糊
    /// 是同一個寫法——**要一個就把打架的那個放下**，而不是留一個按了不算數的勾。
    ///
    /// ⚠️ **關掉替換不會自動把 模糊 打開**：它本來就是關着的那一個，替下去再彈
    /// 回來是替讀者做了他沒說過的決定。
    fn flip_replacing(&mut self) {
        self.search.replacing = !self.search.replacing;
        if self.search.replacing {
            self.search.fuzzy = false;
            self.search.replace.clear();
        }
        // 走到一個此刻不存在的格子上就沒地方站了——替換行剛沒，焦點若在它身上。
        if !self.search.replacing && self.search.field == Field::Replace {
            self.search.field = Field::Query;
        }
        self.run_search();
    }

    /// **此刻的範圍，寫成 `:search` 後面那個詞。**
    ///
    /// ⚠️ 三個用命令開的範圍（`-cd`／`-wd`／`-gd`）**沒有**對應的 `:search` 參
    /// 數，所以寫的是它們算出來的那個目錄——那是誠實的，而且改得動。
    fn scope_as_typed(&self) -> String {
        use crate::search_panel::Where;
        match &self.search.scope {
            Where::Buffer => String::new(),
            Where::Named(path) => path.display().to_string(),
            other => self
                .search_root_of(other)
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        }
    }

    /// **把那一格裏寫着的路徑變成真的範圍** —— 和 `:search` 的參數完全一致：
    /// 空着是本文件，別的都當路徑（`Where::Named`）。
    /// **離開「位置」那一格就把它打的那個路徑落地**（2026-09-25 定）。
    ///
    /// 不在那一格上就什麼都不做，所以三條出口（`Esc`、`Enter`、`↑`／`↓`）可以
    /// 一律叫它一次，不必各自先問一句「我是不是站在位置上」。
    fn land_the_scope(&mut self) {
        if self.search.field == Field::Scope {
            self.take_scope();
        }
    }

    /// **跑一遍，該walk盤的就walk盤**——`Enter` 的全部作用。
    ///
    /// 邊打邊搜的那一種（本文件）名單已經是新的，可還是照跑：`Enter` 在兩種範圍
    /// 下要做同一件事，而「這一種其實不必跑」是一句只有寫代碼的人纔知道的話。
    fn look_again(&mut self) {
        match self.search.field == Field::Scope {
            // 位置那一格：落地本身就帶一次搜索。
            true => self.take_scope(),
            false => match self.search.scope.live() {
                true => self.run_search(),
                false => self.search_now(),
            },
        }
    }

    fn take_scope(&mut self) {
        let typed = self.search.scope_text.trim().to_string();
        let scope = match typed.is_empty() {
            true => crate::search_panel::Where::Buffer,
            false => crate::search_panel::Where::Named(typed.clone().into()),
        };
        // **屏幕上寫着什麼就是什麼**：說了一個不存在的文件夾，那一格照樣寫着它，
        // 而狀態欄當場說它不在。
        //
        // ⚠️ **不許悄悄退回「只搜這個文件」。** `search_now` 拿不到根就只掃眼前
        // 這個緩衝區，交出一張**看着像真的**短清單——那是這扇面板從一開始就躲開
        // 的事（`open_search_in` 開頭那一段說的是同一件）。
        let nowhere = !matches!(scope, crate::search_panel::Where::Buffer)
            && self.search_root_of(&scope).is_none();
        self.search.scope = scope;
        if nowhere {
            self.search.hits.clear();
            self.search.total = 0;
            self.search.selected = 0;
            self.search.stale = false;
            self.status = say!("search.no-such-folder", typed);
            return;
        }
        // 換了地方，上一次的答案就不是這個問題的答案了。
        self.search_now();
    }

    // ---- Changing what was found (#419 三) --------------------------------
    //
    // **The safety is the order, and it is the whole design.** There is no
    // project-wide substitute anybody can type blind: the pattern is the one
    // already in the box, its hits are already listed, and every change is
    // made **in a buffer** — not a byte reaches the disk until `:write-all`.
    // So `u` takes any one file back, `gn` walks them, and the moment a person
    // says yes is a moment they choose.

    /// `r` on a hit: change **that one**.
    fn replace_hit(&mut self) {
        let Some(hit) = self.search.here().cloned() else {
            return;
        };
        let Some(re) = self.search_regex() else { return };
        let with = self.replacement();
        match self.buffer_of(&hit) {
            Some(index) => {
                let done = self.with_buffer(index, |ed| ed.swap_one(&re, &with, hit.line, hit.nth));
                match done {
                    true => self.after_replacing(1),
                    // The line has fewer matches than it had when it was read:
                    // somebody has edited it since. Saying so beats changing
                    // whatever is in that position now.
                    false => self.status = say!("search.moved-on"),
                }
            }
            None => self.status = say!("search.moved-on"),
        }
    }

    /// `r` on a file header, and each step of `R`: change **every hit in one
    /// file**.
    fn replace_file(&mut self, rel: Option<&Path>) -> usize {
        let Some(re) = self.search_regex() else { return 0 };
        let with = self.replacement();
        let Some(index) = self.buffer_for(rel) else {
            return 0;
        };
        self.with_buffer(index, |ed| ed.swap_all(&re, &with))
    }

    /// `R`: change every hit there is, once the reader has said yes.
    pub(super) fn replace_all_found(&mut self) {
        let mut files: Vec<Option<PathBuf>> = Vec::new();
        for hit in &self.search.hits {
            if !files.contains(&hit.file) {
                files.push(hit.file.clone());
            }
        }
        let mut done = 0usize;
        for file in files {
            done += self.replace_file(file.as_deref());
        }
        self.after_replacing(done);
    }

    /// Change **one match on one line** of the buffer being worked on.
    ///
    /// Found again by running the pattern over the line as it stands, rather
    /// than by the offsets the hit carries: those were counted when the file
    /// was read, and the first change in a file moves every one after it.
    /// `false` when the line no longer holds that many matches.
    fn swap_one(&mut self, re: &regex::Regex, with: &str, line: usize, nth: usize) -> bool {
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return false;
        }
        let text: String = rope.line(line).chars().collect();
        // `captures_iter` rather than `find_iter`, so `$1` in the replacement
        // works the way it does in `:s`.
        let Some(caps) = re.captures_iter(&text).nth(nth) else {
            return false;
        };
        let Some(m) = caps.get(0) else { return false };
        let at = rope.line_to_char(line);
        let from = at + text[..m.start()].chars().count();
        let to = at + text[..m.end()].chars().count();
        let mut grown = String::new();
        caps.expand(with, &mut grown);
        let mut rebuilt = rope.to_string();
        let (b0, b1) = (rope.char_to_byte(from), rope.char_to_byte(to));
        rebuilt.replace_range(b0..b1, &grown);
        self.write_whole(rebuilt)
    }

    /// Change **every match** in the buffer being worked on; how many.
    fn swap_all(&mut self, re: &regex::Regex, with: &str) -> usize {
        let rope = self.current_buffer().rope();
        let text = rope.to_string();
        let count = re.find_iter(&text).count();
        if count == 0 {
            return 0;
        }
        let rebuilt = re.replace_all(&text, with).into_owned();
        match self.write_whole(rebuilt) {
            true => count,
            false => 0,
        }
    }

    /// Put a rewritten document back, with the guards a `:s` gets.
    fn write_whole(&mut self, rebuilt: String) -> bool {
        // ⚠️ The grid guard, for the same reason `:s` has it: a substitution
        // that changes how many cells a row has turns a 拆分表 into rubbish,
        // and it cannot be seen happening in a file nobody is looking at.
        if let Some(why) = self.substitution_breaks_the_grid(&rebuilt) {
            self.status = why;
            return false;
        }
        // One snapshot per file, so `u` in that file takes the whole of this
        // back — not one match at a time.
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(0..len, &rebuilt));
        if !self.applied(done) {
            return false;
        }
        self.clamp_cursor();
        self.anchor = self.cursor;
        self.refresh_goal_column();
        true
    }

    /// Which buffer a hit is in, opening the file if it is not open yet.
    fn buffer_of(&mut self, hit: &Hit) -> Option<usize> {
        self.buffer_for(hit.file.as_deref())
    }

    /// The same, from a path relative to the search's root.
    fn buffer_for(&mut self, rel: Option<&Path>) -> Option<usize> {
        let Some(rel) = rel else {
            return Some(self.current);
        };
        let full = match &self.search.root {
            Some(root) => root.join(rel),
            None => rel.to_path_buf(),
        };
        let full = std::fs::canonicalize(&full).unwrap_or(full);
        if let Some(i) = self.buffers.iter().position(|b| {
            b.path()
                .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
                .as_deref()
                == Some(full.as_path())
        }) {
            return Some(i);
        }
        // **Opened as a buffer, not rewritten on disk.** That is the whole of
        // why a change across a book is safe here.
        match self.open_file(&full) {
            Ok(()) => Some(self.current),
            Err(err) => {
                self.status = say!("buffer.cannot-open", full.display(), err);
                None
            }
        }
    }

    /// The pattern the panel is running, compiled — `None` if it will not.
    fn search_regex(&self) -> Option<regex::Regex> {
        regex::Regex::new(&self.search_pattern()).ok()
    }

    /// Look again and say how it went. The offsets are all stale now.
    fn after_replacing(&mut self, done: usize) {
        let where_ = self.search.selected;
        match self.search.scope.live() {
            true => self.run_search(),
            false => self.search_now(),
        }
        // Stay where the eye was, or at the end if the list got shorter.
        self.search.selected = where_.min(self.search.rows().len().saturating_sub(1));
        self.status = match done {
            0 => say!("search.replaced-none"),
            n => say!("search.replaced", n),
        };
    }

    /// `Enter` on a row: fold a file, or go to a hit and hand the keys back.
    fn go_to_hit(&mut self) {
        // On a file header, `Enter` is what `h`/`l` are: open or shut.
        if let Some(crate::search_panel::Row::File { path, folded, .. }) = self.search.row() {
            let _ = self.search.fold(!folded);
            let _ = path;
            return;
        }
        let Some(hit) = self.search.here().cloned() else {
            return;
        };
        self.remember_jump();
        // ⚠️ **A hit carries a file name even when it is in the file being
        // written** (it has to, or the tree could not group it), so 「another
        // file」 is a question about the path, not about whether there is one.
        let mine = self
            .current_buffer()
            .path()
            .and_then(|p| std::fs::canonicalize(p).ok());
        let elsewhere = match (&hit.file, &self.search.root) {
            (None, _) => false,
            (Some(rel), Some(root)) => std::fs::canonicalize(root.join(rel)).ok() != mine,
            (Some(_), None) => true,
        };
        // Another file has to be opened first — and if it cannot be, say so
        // rather than walking the cursor to that line of the wrong file.
        if elsewhere {
            let rel = hit.file.clone().unwrap_or_default();
            let full = match &self.search.root {
                Some(root) => root.join(&rel),
                None => rel,
            };
            if let Err(err) = self.open_file(&full) {
                self.status = say!("buffer.cannot-open", full.display(), err);
                return;
            }
        }
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();
        // ⚠️ **A hit in another file is placed by line, not by the offset.**
        // Those offsets were counted in the text as it was read; the buffer
        // just opened may have been edited since, and a stale offset would put
        // the cursor in the middle of a word somewhere else.
        let (at, end) = match elsewhere {
            true => {
                let line = hit.line.min(rope.len_lines().saturating_sub(1));
                let at = rope.line_to_char(line);
                (at, at)
            }
            false => (hit.at.min(len), hit.end.min(len)),
        };
        self.anchor = at;
        self.cursor = motion::prev_grapheme(self.current_buffer().rope(), end.max(at)).max(at);
        self.extend = false;
        self.refresh_goal_column();
        self.panel_focus = None;
    }
}

/// A few characters either side of one match, and where the match is in them.
fn excerpt(
    file: Option<PathBuf>,
    text: &str,
    line_at: usize,
    start: usize,
    stop: usize,
    line: usize,
    nth: usize,
) -> Hit {
    let chars: Vec<char> = text.chars().collect();
    let from = start.saturating_sub(AROUND);
    let to = (stop + AROUND).min(chars.len());
    let mut excerpt = String::new();
    if from > 0 {
        excerpt.push('…');
    }
    let lead = excerpt.chars().count();
    excerpt.extend(chars[from..to].iter().filter(|c| **c != '\n'));
    if to < chars.len() {
        excerpt.push('…');
    }
    Hit {
        file,
        line,
        nth,
        at: line_at + start,
        end: line_at + stop,
        mark: lead + (start - from)..lead + (stop - from),
        excerpt,
    }
}

/// **What the panel is looking for** — a pattern, or 「nearly these characters」.
///
/// One type because the two passes over the prose (the buffer in memory, then
/// the files on the disk) must ask the same question; they were two copies of
/// a `find_iter` loop, and a second way to search would have made them two
/// copies of a `match`.
enum Look {
    /// A regular expression, flags and all (`search_pattern`).
    Pattern(Regex),
    /// The 模糊 switch: [`crate::nearby`], which counts in characters.
    Nearby { needle: Vec<char>, fold: bool },
}

impl Look {
    /// Where it is found in one line, as **character** ranges within it.
    fn spans(&self, text: &str) -> Vec<(usize, usize)> {
        match self {
            Look::Pattern(re) => re
                .find_iter(text)
                .map(|m| (text[..m.start()].chars().count(), text[..m.end()].chars().count()))
                .collect(),
            Look::Nearby { needle, fold } => {
                let hay: Vec<char> = text.chars().collect();
                crate::nearby::spans(&hay, needle, *fold)
            }
        }
    }
}
