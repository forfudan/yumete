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

    /// **這一刻的正文指紋**，見 [`crate::search_panel::Mark`]。
    fn search_mark(&self) -> crate::search_panel::Mark {
        crate::search_panel::Mark {
            buffer: self.current_buffer().id(),
            revision: self.current_buffer().revision(),
            every: self.buffers.iter().map(|b| b.revision()).fold(0, u64::wrapping_add),
        }
    }

    /// **手上這張名單答的還是不是眼前這個問題。**
    ///
    /// 兩件事各答一半：`Search::stale` 是「框裏的詞改了、而這個範圍不邊打邊搜」，
    /// `looked_at` 是「跑完之後正文又動過」。前者存着，因為改框的路只有一條；後者
    /// 算出來，因為改正文的路有幾十條。
    ///
    /// 面板拿它畫那句「按 Enter 重新查找」，`Enter` 拿它決定先跑還是直接去。
    pub fn search_is_stale(&self) -> bool {
        self.search.stale
            || self.search.looked_at.is_some_and(|mark| mark != self.search_mark())
    }

    /// **同屏別的那幾處命中**，`(起, 止)` 是全文字符偏移，不含站着的那一處。
    ///
    /// 2026-09-27 定，原話：「同屏幕別的命中的底色可以淡一些，防止混淆」。走到
    /// 一處命中上，正文只把那一處標出來，而同一頁上還有幾處要換、換完剩幾處，
    /// 屏幕上一個字都不說。
    ///
    /// 空的時候：面板沒開、名單過期了（正文改過，偏移不作數）、或者這些命中不
    /// 在眼前這一份緩衝裏。
    pub fn search_marks(&self) -> Vec<(usize, usize)> {
        if self.search_is_stale() || !self.search_panel_is_open() {
            return Vec::new();
        }
        // **命中身上的路徑是相對搜索的根的**，所以要拿眼前這一份的路徑去比同一
        // 把尺。範圍是「本文件」的時候根是空的，那些命中身上也沒有路徑。
        let mine = match &self.search.root {
            None => None,
            Some(root) => {
                let under = self
                    .current_buffer()
                    .path()
                    .and_then(|p| std::fs::canonicalize(p).ok())
                    .zip(std::fs::canonicalize(root).ok())
                    .and_then(|(full, root)| {
                        full.strip_prefix(&root).ok().map(std::path::Path::to_path_buf)
                    });
                // 眼前這一份不在那個根底下，那些命中一處都不屬於它。
                match under {
                    Some(rel) => Some(rel),
                    None => return Vec::new(),
                }
            }
        };
        let here = self.search.here().map(|h| (h.at, h.end));
        self.search
            .hits
            .iter()
            .filter(|h| h.file == mine)
            .map(|h| (h.at, h.end))
            .filter(|span| Some(*span) != here)
            .collect()
    }

    /// 這一節有沒有一扇搜索面板開着。
    pub(super) fn search_panel_is_open(&self) -> bool {
        crate::sidebar::Side::BOTH.iter().any(|side| {
            self.panel(*side).map(|p| p.view()) == Some(crate::sidebar::View::Search)
        })
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
    /// `:search-working`／`-wd`／`-gd`／`:search <path>` — open it looking somewhere
    /// else (#419).
    pub(super) fn open_search_in(&mut self, scope: Where, replacing: bool) {
        // Warning: **A folder that is not there is said out loud.** Falling back to
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
        // Warning: **Only ever turned on here.** `:search` after a `:replace` is a
        // reader saying 「just looking」, and leaving the row up would leave
        // `r` and `R` live on a panel nobody meant to change anything with.
        self.search.replacing = replacing;
        // Warning: **模糊 comes off when the panel starts changing things**
        // (2026-09-20). A loose match covers characters nobody typed, so
        // 「replace them all」 would hand the manuscript a range the writer
        // cannot predict. The switch is not even in the form while replacing
        // (`Field::step`), and leaving the *flag* on would have made the list
        // loose while the switch that says so was out of sight.
        if replacing {
            self.search.fuzzy = false;
        }
        self.open_search();
        // Warning: **命令進來的那一路不許開關一下就走**（2026-10-01 撞到）。
        // `show_sidebar` 是個開關：面板開着又拿着鍵的時候它關掉面板，於是
        // `:search ../稿` 把面板關了，而使用者說的是「去那裏找」。`空格 /`
        // 照舊是開關，那一個鍵按第二下的意思就是「收起來」。
        let side = self.side_for(crate::sidebar::View::Search);
        if self.showing(crate::sidebar::View::Search).is_none() {
            let root = self.root();
            self.open_sidebar_showing(&root, crate::sidebar::View::Search);
        }
        self.focus_slot(side);
        self.mode = Mode::Field;
    }

    pub(super) fn open_search(&mut self) {
        // Warning: A selection wins over the last pattern — but only one somebody
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
        // Warning: **The box remembers what was typed, not what was compiled.**
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
        // Warning: **Opened onto a folder, it looks straight away** rather than
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
    /// Warning: **正則關着的時候，右邊也要照字面。** 左邊一直是照規矩辦的（`regex::
    /// escape`），而右邊從前**無條件走展開**——於是「正則」那一格明明沒勾，
    /// `US$100` 裏的 `$100` 還是被讀成第 100 個捕獲組（空的），換出來只剩 `US`。
    ///
    /// | 查 | 換成 | 從前得到 |
    /// | --- | --- | --- |
    /// | `一百元` | `US$100` | `US` |
    /// | `甲` | `$x^2$` | `^2$` |
    /// | `甲` | `價$x元` | `價元` |
    ///
    /// Warning: **這不是小事**：`R` 是「每個檔每一處」，而寫 Typst 的人滿篇 `$…$`
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
        let mut body = match (self.search.regex, self.search.glyphs) {
            // **正則底下也折字形，只折「原樣打出來的那些字」**（2026-10-01 定，
            // 作者問的：「正则情况下能否也能兼容繁简体？」）。
            //
            // Warning: **整串改寫是不行的**，那正是 2026-09-25 讓兩個開關互斥的理由：
            // 把每個字換成 `[...]` 會把使用者寫的 `.`、`*`、`[` 一起吃掉。
            // [`crate::glyphs::widen_pattern`] 先解析再動手，所以 `.`、`\d`、
            // `^$`、括號一律不碰。解析不了（打了一半）就原樣用，壞式子自有它那
            // 條路。
            (true, true) => {
                crate::glyphs::widen_pattern(&self.search.query)
                    .unwrap_or_else(|| self.search.query.clone())
            }
            (true, false) => self.search.query.clone(),
            // **「書齋」找得到「书斋」**：每個字換成它的字形集（`crate::glyphs`）。
            // 這一支自己就轉義，所以不必再 `escape` 一遍。
            (false, true) => crate::glyphs::widen(&self.search.query),
            (false, false) => regex::escape(&self.search.query),
        };
        // Warning: **`\b` is nothing between 漢字.** There is no word boundary
        // there, so this only ever bites on the Western words in a manuscript
        // — which is what it does in VSCode too, and what the manual says.
        if self.search.whole {
            body = format!(r"\b{body}\b");
        }
        // Warning: **Sensitive says so out loud** (`(?-i)`), it does not just stay
        // silent. The page's own `n` runs this pattern through smart case
        // again (`compile`), which would put `(?i)` in front of a quiet one
        // and search differently from the panel that produced it. Flags apply
        // in order, so an explicit one at the front wins.
        match self.search.case {
            Case::Sensitive => format!("(?-i){body}"),
            Case::Insensitive => format!("(?i){body}"),
            // The rule the page's own `/` follows: a capital is how you ask.
            //
            // Warning: **問 `body` 和問查詢本身是同一個答案，查過了**（2026-10-02）。
            // 一份審閱說這裏該問 `self.search.query`，理由是 `\S`、`\D`、`\B`、
            // `\p{Han}` 都帶着大寫字母。可那幾個大寫**正是讀者自己按的**——
            // 兩邊看見的是同一個 `S`。`body` 裏唯一可能多出來的字符來自
            // `regex::escape`（只加反斜杠）和字形折疊（那張表一個大寫都沒有，
            // 逐字掃過），兩個都進不了這道問句。改了是空操作，所以沒改。
            //
            // 剩下的那半句是真的：正則模式下打一個 `\S` 就等於要求區分大小寫。
            // vim 的 `smartcase` 看的同樣是打出來的那一串，所以這是照抄來的，
            // 不是我們的毛病。
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
            // Warning: **內存那兩檔沒有根。** 本文件與緩衝區都是一張現成的表，不走
            // 磁碟——`search_now` 走的是另一條岔路。
            Where::Buffer | Where::Buffers => None,
            Where::Working => Some(self.working_dir()),
            Where::Project => Some(self.root()),
            // **相對路徑從工作路徑算起，絕對路徑就是絕對路徑**（2026-10-01 定）。
            // 和 `:open`、shell、命令行補全同一條規矩——編輯器裏每一條路徑都從同
            // 一個地方算起，這一支從前是唯一的例外。
            //
            // Warning: **從前它從項目路徑算起，開頭一個 `/` 也當項目根**（2026-09-27
            // 定，當時引的理由是「同 VS Code 的包含文件框」）。2026-10-01 查了源
            // 碼，**VS Code 做的正好相反**：開頭一個 `/` 在它那裏是絕對路徑，自成
            // 一個搜索根、跑到工作區外面去（`queryBuilder.ts:459-472`）。真正拿兩
            // 道槓說「項目根」的是 Sublime，而它同樣把單槓留給絕對路徑。
            //
            // 那條規矩還有一個沒人發現的代價：`strip_prefix("/")` 把**所有**絕對
            // 路徑都接到了項目根後面，於是 `:search /usr/share/dict` 報「沒有這個
            // 文件夾」——絕對路徑根本打不進來。
            //
            // 要搜項目根就按號碼換到「項目路徑」那一檔，本來就有。
            Where::Named(path) => {
                // **開頭那個 `~` 要展開**（2026-10-01 作者報的：`:s ~/Dropbox` 無效）。
                // 從前它一路當相對路徑接在根後面，狀態欄說「沒有這個文件夾」——
                // 聽着像那個目錄不在，其實是我們根本沒去那裏找。
                let path = Self::expand_tilde(&path.to_string_lossy());
                let full = match path.is_absolute() {
                    true => path,
                    false => self.working_dir().join(path),
                };
                // Warning: **是個檔就另說一句**（2026-10-02 審出來的）。`:search 卷一.md`
                // 指着一個真的存在的東西，而狀態欄說「沒有這個文件夾」——正是
                // `~` 那一條修的時候說過的毛病：聽着像它不在，其實是我們沒去。
                if full.is_file() {
                    return None;
                }
                full.is_dir().then_some(full)
            }
        }
    }

    /// The folder the file being written is in, as an absolute path.
    ///
    /// Warning: **Resolved against the working directory first.** A buffer opened as
    /// `a.md` has a relative path, and its `parent()` is the *empty* path —
    /// which as a root walks nothing at all, so 「this folder」 quietly found
    /// only the file already open.
    pub(super) fn here_folder(&self) -> PathBuf {
        // 相對的緩衝區路徑按[工作路徑][`Editor::working_dir`]算，同 `:open`
        // （2026-10-01）。
        let here = self.working_dir();
        match self.current_buffer().path() {
            Some(path) => here.join(path).parent().map(Path::to_path_buf).unwrap_or(here),
            None => here,
        }
    }

    /// **Run what the box holds** — #419.
    ///
    /// The buffer is searched on every keystroke: it is in memory and a pass
    /// over it costs nothing worth counting. Warning: **Anything wider waits for
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

    /// **正在寫的那一份裏的命中**，連同它真正有幾處。
    ///
    /// `search_now` 的第一段，抽出來是因為「只重搜被改過的那一份」走的也是它
    /// （[`Editor::rescan_the_open_one`]）。Warning: **它從內存讀**：屏幕上是什麼就搜
    /// 什麼，存沒存盤不算數。
    ///
    /// `room` 是還能往名單裏放幾處；超出的只數不放，所以第二個回值纔是真數。
    fn scan_the_open_one(
        &self,
        look: &crate::editor::find::Look,
        mine: &Option<std::path::PathBuf>,
        room: usize,
    ) -> (Vec<crate::search_panel::Hit>, usize) {
        let rope = self.current_buffer().rope();
        let mut hits = Vec::new();
        let mut total = 0usize;
        let mut at = 0usize;
        for line in 0..rope.len_lines() {
            let text: String = rope.line(line).chars().collect();
            for (nth, (start, stop)) in look.spans(&text).into_iter().enumerate() {
                total += 1;
                if hits.len() < room {
                    hits.push(excerpt(mine.clone(), &text, at, start, stop, line, nth));
                }
            }
            at += text.chars().count();
        }
        (hits, total)
    }

    /// **搜每一個打開着的緩衝區** —— 緩衝區那一檔（2026-10-01 定）。
    ///
    /// 正在寫的那一份排在最前面，於是 `mine` 那一段還是名單開頭連續的一段，
    /// 只重搜那一份的那條快路照舊走得通。
    ///
    /// Warning: **沒有名字的草稿也在裏面**（作者定）。它沒有路徑可以回去，所以每一處
    /// 命中身上記的是**緩衝區的號**（[`Hit::buffer`]），不是路徑。
    fn scan_every_buffer(&mut self, look: &Look) {
        let here = self.working_dir();
        let order: Vec<usize> = std::iter::once(self.current)
            .chain((0..self.buffers.len()).filter(|&i| i != self.current))
            .collect();
        let mut hits = Vec::new();
        let mut total = 0usize;
        let mut mine = 0usize;
        let mut mine_total = 0usize;
        for i in order {
            let id = self.buffers[i].id();
            // 有名字的按工作路徑縮短，縮不動就印全名；沒名字的印它在狀態欄上
            // 的那個名字。
            let label = match self.buffers[i].path() {
                Some(path) => {
                    let full = here.join(path);
                    full.strip_prefix(&here).unwrap_or(&full).to_path_buf()
                }
                None => std::path::PathBuf::from(self.buffers[i].display_name()),
            };
            let rope = self.buffers[i].rope();
            let mut at = 0usize;
            for line in 0..rope.len_lines() {
                let text: String = rope.line(line).chars().collect();
                for (nth, (start, stop)) in look.spans(&text).into_iter().enumerate() {
                    total += 1;
                    if hits.len() < MOST {
                        let mut hit =
                            excerpt(Some(label.clone()), &text, at, start, stop, line, nth);
                        hit.buffer = Some(id);
                        hits.push(hit);
                    }
                }
                at += text.chars().count();
            }
            if i == self.current {
                mine = hits.len();
                mine_total = total;
            }
        }
        self.search.mine = mine;
        self.search.mine_total = mine_total;
        self.search.cut = false;
        self.search.root = None;
        self.search.hits = hits;
        self.search.total = total;
        self.search.looked_at = Some(self.search_mark());
    }

    /// **面板底下那三格做成一套篩子** —— `None` ＝ 有一條 glob 寫錯了。
    fn sieve(&self) -> Option<crate::editor::Sieve> {
        let sieve = crate::editor::Sieve {
            hidden: self.search.hidden,
            include: self.search.include.clone(),
            exclude: self.search.exclude.clone(),
        };
        match self.search_root() {
            Some(root) => sieve.is_sound(&root).then_some(sieve),
            None => Some(sieve),
        }
    }

    /// Run it whatever the scope, walking the disk if that is what it takes.
    pub(super) fn search_now(&mut self) {
        self.search.broken = false;
        if !self.search.asked() {
            self.search.hits.clear();
            self.search.total = 0;
            self.search.selected = 0;
            self.search.looked_at = None;
            // Warning: **那三個數也要歸零。** 名單空了而 `mine_total` 還留着上一次的
            // 數，`rescan_the_open_one` 拿 `total - mine_total` 一減就是負數
            // （`usize` 下溢，debug 當場 panic，release 畫出個天文數字）。
            self.search.mine = 0;
            self.search.mine_total = 0;
            self.search.cut = false;
            return;
        }
        let pattern = self.search_pattern();
        let Some(look) = self.looker() else {
            // Warning: The hits stay, and are drawn quiet. Typing a regular
            // expression walks through `[`, `(` and every other unfinished
            // state; emptying the list on each of them flickers, and a blank
            // list would say 「nothing found」, which is not true.
            self.search.broken = true;
            return;
        };
        self.search.hits.clear();
        self.search.total = 0;
        self.search.selected = 0;
        self.search.folded.clear();
        self.search.stale = false;
        self.search.cut = false;
        // **The pattern hands the search to the panel, and the page follows.**
        // One 「what am I looking for」 with two ways in: the highlight and
        // `n`/`N` are the same search, which is what [^415]记 `:grep` 不寫
        // `last_search` 為缺口的那條理由.
        // Warning: **模糊 does not hand the page a pattern it cannot keep.** `n`/`N`
        // and `:s` run a regular expression, and there is no regular
        // expression for 「these characters, nearly in a row」 — so what they
        // are left with is the query *itself*, exactly. The panel lists the
        // near misses; the page walks the exact ones, which are a subset of
        // them, and neither is lying about the other.
        self.last_search = match self.search.fuzzy {
            true => regex::escape(&self.search.query),
            false => pattern,
        };
        if matches!(self.search.scope, Where::Buffers) {
            return self.scan_every_buffer(&look);
        }
        let root = self.search_root();
        // Warning: **Compared as absolute paths.** A buffer opened as `a.md` and the
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
        // It comes first, and it comes from memory: what is on the screen is
        // what is searched, saved or not.
        let (mut hits, mut total) = self.scan_the_open_one(&look, &mine, MOST);
        self.search.mine = hits.len();
        self.search.mine_total = total;
        if let Some(root) = root {
            let sieve = self.sieve();
            if sieve.is_none() {
                self.search.hits.clear();
                self.search.total = 0;
                // Warning: **同上，那兩個數要跟着歸零**（2026-10-02 審出來的）。這一支
                // 在 `mine`／`mine_total` 已經填好之後纔走到，漏了它們就是一次
                // 真的下溢：打一條寫錯的 glob，再在正文裏打一個字就撞上。
                self.search.mine = 0;
                self.search.mine_total = 0;
                self.search.root = Some(root);
                self.search.looked_at = Some(self.search_mark());
                self.status = say!("search.bad-glob");
                return;
            }
            let mut files = Vec::new();
            let walked = crate::editor::walk_prose(&root, &sieve.unwrap_or_default(), &mut |path| {
                files.push(path.to_path_buf())
            });
            self.search.cut = walked.cut;
            for path in files {
                // Not twice: the one being written was searched from memory.
                let full = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if Some(&full) == here.as_ref() {
                    continue;
                }
                // **An open file is read from its buffer, not from disk.**
                // Unsaved work is work, and a search that could not see it
                // would send a reader to a line that no longer says that.
                // Warning: **Matched on the resolved path.** `/tmp` is a link to
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
        self.search.looked_at = Some(self.search_mark());
    }

    /// One key while a field of the search panel has them (`Mode::Field`).
    pub(super) fn on_field_key(&mut self, key: Key) {
        match key {
            // **`Enter` 只做一件事：跑一遍搜索**（2026-09-25 定，原話：「按下
            // Enter「只」触发搜索。他不更改光标位置，不更改状态……这样的好处是在
            // 搜的到/搜不到东西的时候，enter的行为都是一样的」）。
            //
            // Warning: **它從前還兼着「把鍵交到第一條結果上」**（同一天早些時候定的），
            // 而那讓它在找得到和找不到的時候做兩件不同的事——正是這一條想去掉的
            // 分岔。去結果現在是 `Esc` 然後 `j`，兩個已經學過的鍵。
            Key::Enter => {
                self.search.all_selected = false;
                self.look_again();
                // **打完了，去找——然後把鍵交回面板**（2026-09-25 補的：「enter键
                // 除了触发搜索，还是最好能回到 normal mode」）。它因此讀成一句
                // 完整的話，而不是「跑一遍，然後你還在框裏」。
                //
                // Warning: **落在哪一格不動。** 找得到找不到都一樣，這正是這個鍵要的
                // 那份一致；去結果是接着按 `j`。
                self.mode = Mode::Normal;
            }
            // **`Tab` 在框裏是「下一格」**（2026-09-27 三個試用的人都撞上這一
            // 條）。從前它在框裏什麼都不做，於是打完「搜」的詞按一下 Tab 再打
            // 「換」的詞，兩個詞連成了一個：`搜: 洞庭鄱陽`。而面板上方那一行
            // 正寫着 `Tab 文件 > 緩衝 > 大綱 > 搜索`——一個寫在屏幕上、在這個
            // 狀態下按了沒反應的鍵。
            //
            // Warning: **只在框裏。** 出了框 `Tab` 還是走邊欄那幾個視圖，那是它在每
            // 一扇面板裏的老意思。
            Key::Tab => {
                let next = self.search.field.step(false, self.search.replacing, self.search.on_disk());
                self.search.stand_on(next);
                // 走到名單上就不是打字了，鍵交回面板。
                if self.search.field == Field::Results {
                    self.mode = Mode::Normal;
                    self.show_hit();
                }
            }
            Key::BackTab => {
                let back = self.search.field.step(true, self.search.replacing, self.search.on_disk());
                self.search.stand_on(back);
                if self.search.field == Field::Results {
                    self.mode = Mode::Normal;
                    self.show_hit();
                }
            }
            Key::Char(ch) => {
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
            // Warning: **`Tab` is the slot's own key** (2026-09-17): it walks the
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
            // Warning: **從前也沒有真的還原**——`scope_text` 一直在，是**畫**的時候只在
            // 打字態纔顯示它，出框就退回按真實範圍算出來的名字，看着像還原了。
            // 所以修法是讓它**落地**（定的，原話：「算，离开格子就落地」），這樣屏幕上
            // 寫着什麼就是什麼。
            // **`Esc` 在結果名單上是「退一步」**（2026-09-27 報的：「Esc — dead
            // in the result list……The universal escape hatch is a no-op」）。
            // vi 用戶最確定的那一個鍵，在這一格從前什麼都不做。退到搜索框上，
            // 再按一次就沒有別的可退了，那時它是下面那一支。
            Key::Esc if self.mode == Mode::Normal && self.search.field == Field::Results => {
                self.search.stand_on(Field::Query);
            }
            Key::Esc => {
                // 已經在 Normal 了，這一下 `Esc` 沒有別的事可做——那就是「把輸入
                // 法的挂起再說一遍」，同正文與邊欄那兩處（2026-09-27）。
                if self.mode == Mode::Normal {
                    self.say_it_again = true;
                }
                self.search.all_selected = false;
                self.mode = Mode::Normal;
            }
            _ => {}
        }
    }

    /// **換一個範圍**——`6`，或者在位置那一格上按左右。
    ///
    /// 本文件 → 緩衝區 → 工作路徑 → 項目路徑 → 回本文件（2026-10-01 定的四檔）。
    ///
    /// Warning: **指定文件夾不在圈上**：它只能 `:search 某目錄` 進來，進來之後按一下
    /// 就回本文件，回不去。一個「下一個」走不到的值不該卡在環上。
    ///
    /// Warning: **號碼 2026-10-01 從 `0` 挪到了 `6`**：位置那一格跟着搬到了包含／排除
    /// 上面，和那三格合成「搜哪裏、搜哪些」一組，號碼就接着往下排。
    fn step_the_scope(&mut self) {
        self.search.scope = self.search.scope.next();
        self.search.scope_text = self.scope_as_typed();
        self.search.field = crate::search_panel::Field::Scope;
        self.look_again();
    }

    /// A committed string from the IME lands in the field, not in the page.
    pub(super) fn type_into_field(&mut self, text: &str) {
        self.search.type_text(text);
        self.run_search();
    }

    /// `Tab` out of a box: the next cell.
    ///
    /// Warning: **Landing on another box keeps you typing.** 找什麼 and 換成什麼 sit
    /// one above the other and are filled in one after the other; having to
    /// press `i` between them would make `Tab` the wrong key for the commonest
    /// thing anybody does in this panel.
    fn leave_field(&mut self, back: bool) {
        self.search.field = self.search.field.step(back, self.search.replacing, self.search.on_disk());
        self.search.all_selected = false;
        match self.search.takes_text() {
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
            Key::Tab => self.cycle_view(side, false),
            Key::BackTab => self.cycle_view(side, true),
            // **`hl` 橫着走字，`jk` 竪着走格**（2026-09-25 定，原話：「這一個光標
            // 所在的字是反白的……用戶這樣就能用hl在搜索欄中移動光標」）。和正文
            // 一個感覺：框裏站着一個塊光標，`h`／`l` 挪它，`i` 就從它那裏插。
            //
            // Warning: **格子之間從此只有 `jk`**（定的，原話：「行，格子只用 jk」）。從前
            // `hl` 和 `jk` 走的是同一串格子，那時框裏沒有光標可挪，`hl` 也就沒有
            // 別的事可做。
            //
            // 結果列表是例外：那裏 `h`／`l` 是摺起／打開一個檔，和文件樹、大綱說的
            // 「少一點／多一點」是同一件事；到頂了 `h` 出去，免得困在列表裏。
            Key::Char('h') | Key::Left if self.search.field == Field::Results => {
                if !self.search.fold(true) {
                    let back = self.search.field.step(true, self.search.replacing, self.search.on_disk());
                    self.stand_on_and_look(back);
                }
            }
            Key::Char('l') | Key::Right if self.search.field == Field::Results => {
                self.search.fold(false);
            }
            // **位置那一格是個四選一，所以 `hl` 在它上面是換檔**（2026-09-27
            // 定）。那一格沒有字可以挪光標，`hl` 也就沒有別的事可做；而「這一格
            // 能換」本來就該用左右來說。
            //
            // Warning: **2026-10-01 起它永遠打不了字**：指定文件夾只能用命令進來，進來
            // 之後面板裏改不了，所以 `takes_text()` 在這一格上恆假。
            Key::Char('h') | Key::Left | Key::Char('l') | Key::Right
                if self.search.field == Field::Scope && !self.search.takes_text() =>
            {
                self.step_the_scope();
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
                // **走一步就看一眼**（2026-09-27）：正文跳到那一處、選區蓋上去，
                // 而鍵留在這裏。見 [`Self::show_hit`]。
                Field::Results => {
                    self.search.step(true);
                    self.show_hit();
                }
                _ => {
                    let next = self.search.field.step(false, self.search.replacing, self.search.on_disk());
                    self.stand_on_and_look(next);
                }
            },
            Key::Char('k') | Key::Up => match self.search.field {
                // Warning: **到頂了就出去**（2026-09-23 報的：「我一旦將光標移動到了下面
                // 文件的區域，就沒辦法使用 k 向上移動到選項和輸入框了」）。從前
                // `step(false)` 在第 0 條上飽和，於是列表是個進得去出不來的地
                // 方——`Tab` 走得出去，可沒人會想到去按它。
                Field::Results if self.search.selected == 0 => {
                    let back = self.search.field.step(true, self.search.replacing, self.search.on_disk());
                    self.stand_on_and_look(back);
                }
                Field::Results => {
                    self.search.step(false);
                    self.show_hit();
                }
                _ => {
                    let back = self.search.field.step(true, self.search.replacing, self.search.on_disk());
                    self.stand_on_and_look(back);
                }
            },
            // **The five switches are numbered, top to bottom** (2026-09-24,
            // 原話：「中间五行选项，在normal状态下不是移动上去按空格，而是直接
            // 通过一个字母来选择（在这一行后面显示这个字母快捷键）」). The
            // number is drawn at the end of its row, so the panel says what to
            // press rather than asking anybody to remember it.
            //
            // Warning: **模糊 while 替換 is ticked does nothing**, as it did before:
            // the row is drawn quiet, and a quiet row that still flipped would
            // be saying two things at once. `flip_switch` guards it.
            // **站在一個換不動的格子上按了改字的鍵**（2026-09-27）：位置那一格
            // 平常是四選一，`i`／`a`／`d` 在它上面沒有東西可改。
            Key::Char('i' | 'a' | 'c' | 'I' | 'A' | 'd' | 'D' | 'C')
                if self.search.field == Field::Scope && !self.search.takes_text() =>
            {
                self.status = say!("search.scope-is-a-pick");
            }
            // **號碼就是從上往下數的行次**（`1`–`9`）。位置那一行 2026-10-01 起是
            // `8`：它挪到了「包含／排除」上面，和那三格合成「搜哪裏、搜哪些」
            // 一組，號碼也就接着往下排，不再是從前那個 `0`。
            Key::Char(ch) if ch.is_ascii_digit() && ch != '0' => {
                let nth = ch as usize - '1' as usize;
                if let Some(&which) = Field::SWITCHES.get(nth) {
                    self.flip_switch(which);
                }
            }
            // **`Enter` 就是「再跑一遍」**（2026-09-25 定，原話：「enter 在非結果
            // 位置（包括查詢框上）都是觸發重搜。如果 enter 在結果上，那麼在文檔
            // 更新之後，確實應該先觸發一次重搜再跳」）。
            //
            // 起因是「改完正文回到面板按 `Enter`，名單還是舊的」：那時 `Enter` 在
            // 框上只是「進編輯」，在開關上是「翻一下」，沒有一個格子是重跑。
            //
            // Warning: **結果那一格上，過期了先跑、不跳**。屏幕上此刻寫着「按 Enter
            // 重新查找」，那 `Enter` 就照它說的做；而且正文動過之後，那一處的行號
            // 與位置都已經不準，跳過去多半落在別的字上。跑完名單是新的，再按一次
            // 纔去。
            //
            // Warning: **進編輯不缺入口**：`i` `a` `I` `A` `c` 和 `/` 六個；開關也不缺，
            // `1`–`7` 和空格都翻得動。騰出 `Enter` 沒有讓誰沒路走。
            Key::Enter if self.search.field == Field::Results && !self.search_is_stale() => {
                self.go_to_hit();
            }
            Key::Enter => self.look_again(),
            // **`r` and `R` change things**, and only while the replace row
            // is showing — `:search` is for looking, `:replace` for changing,
            // and the panel says which it is.
            // Warning: **站錯地方一聲不吭**（2026-09-29 定，原話：「光标不在结果上，
            // 不应该显示『r ....』的提示。因此如果用户按了 r，也不需要任何提示」）。
            //
            // 2026-09-25 那一輪是反過來的：站在框上按 `r` 要出一句「先 j 走到一
            // 條命中上」，理由是「按了沒反應的鍵讀者會以為自己記錯了」。Warning: **那
            // 條理由的前提是提示行上寫着 `r`** ——現在不寫了（見 `hint.rs`），
            // 於是它就是一個沒綁的鍵，和別的沒綁的鍵一樣不必解釋。
            Key::Char('r') if self.search.replacing && self.search.field == Field::Results => {
                match self.search.row() {
                    // Warning: **整個檔也要先問一句**（2026-09-27 三個試用的人都指出
                    // 這一條）。從前只有 `R` 問，而 `r` 站在檔名那一行上一聲不吭
                    // 就換掉整個檔——兩個鍵差一個 Shift，兩行差一個 `j`，而不問的
                    // 那一個標籤最短、最容易被窄窗口截掉。
                    Some(crate::search_panel::Row::File { path, hits, .. }) => {
                        let name = path.display().to_string();
                        self.status = say!("search.replace-file-sure", hits, name);
                        // 緩衝區那一檔要連號一起記下來，見 `replace_file`。
                        let id = self
                            .search
                            .hits
                            .iter()
                            .find(|h| h.file.as_deref() == Some(path.as_path()))
                            .and_then(|h| h.buffer);
                        self.replace_this_file = Some((path, id));
                        self.pending = Pending::ReplaceAll;
                    }
                    _ => self.replace_hit(),
                }
            }
            // Warning: **Only 「all of them」 asks first.** One hit and one file are
            // changes a reader is looking straight at; every file in a book is
            // not, and that is the one where a slip costs an afternoon.
            // **`u` 撤回剛纔那一次替換**（2026-09-27 定）。
            //
            // 換完鍵還在面板裏，而撤回的是正文——所以這個鍵得在這裏接住，否則讀
            // 者要先 `C-w` 出去、按 `u`、再走回來，而那一路上他站在名單的哪一條
            // 早就忘了。換完再看一眼那一行，覺得不對就按 `u`：這纔是「預覽」該有
            // 的樣子。
            Key::Char('u') if self.search.replacing => {
                let batch = std::mem::take(&mut self.replaced_in);
                let mut back = 0usize;
                match batch.is_empty() {
                    // 沒有記在案的那一批：撤回當前這一份，和從前一樣。
                    true => self.undo(),
                    // Warning: **一次 `R` 能動好幾個檔，而 `undo` 只管當前那一份。**
                    // 逐份撤回，光標不動——`with_buffer` 只是借那一格站一下。
                    false => {
                        for id in batch {
                            if let Some(i) = self.buffer_with(id) {
                                self.with_buffer(i, |ed| ed.undo());
                                back += 1;
                            }
                        }
                    }
                }
                self.after_replacing_undone(back);
            }
            // Warning: **`R` 也只在名單那一邊活着**（2026-09-29 定，同 `r`）。提示行
            // 上不寫它的時候，它就是一個沒綁的鍵；名單空着也一樣。
            Key::Char('R')
                if self.search.replacing
                    && self.search.field == Field::Results
                    && !self.search.hits.is_empty() =>
            {
                // **問句要說清楚動的是幾個檔**（2026-09-27 報的）：工作區範圍下
                // 那 8 處散在 4 個檔裏，而從前這句話和只改一個檔的時候一字不差。
                let mut files: Vec<&Option<PathBuf>> = Vec::new();
                for hit in &self.search.hits {
                    if !files.contains(&&hit.file) {
                        files.push(&hit.file);
                    }
                }
                self.replace_this_file = None;
                self.status = match files.len() {
                    0 | 1 => say!("search.replace-all-sure", self.search.total),
                    n => say!("search.replace-all-sure-files", self.search.total, n),
                };
                // Warning: **`ReplaceAll`, not `Confirm`.** The latter is `:s …c`'s
                // per-match walker: with nothing to walk it clears itself on
                // the next key, so the question was asked and the answer went
                // nowhere.
                self.pending = Pending::ReplaceAll;
            }
            // **`F5` runs it again**, for a scope that does not run itself.
            Key::Char('F') if !self.search.scope.live() => self.search_now(),
            // **`/` 回搜索框：進 insert、光標放末尾、框裏的字留着**。
            //
            // Warning: **和選擇器（`空格 f`／`空格 b`／`:wiki`）那一扇裏的 `/` 一個
            // 樣**——2026-09-25 報的就是這一條：「用户在相似的界面按同样的
            // 快捷键，他的行为应该是一致的」。它原先是「整條選中」（`ask`），於是
            // 整格白底、看不見光標，看着既不像插入模式也不像光標在末尾。
            //
            // Warning: **它和 `i` 不同的地方是「不管現在站在哪一格」**：站在換框上、
            // 站在結果上，`/` 都回到搜索框；`i` 只在當前那一格打不了字的時候纔
            // 挪窩，而且是**從光標處**插。改搜索詞是這扇面板裏最常做的事。
            Key::Char('/') => {
                self.search.stand_on(Field::Query);
                self.mode = Mode::Field;
            }
            // `i` opens a box — this one if the keys are on one, the query
            // otherwise. The key that means 「type here」 everywhere else.
            // **框裏的 Normal 也編輯得了**（2026-09-25 報的：「normal模式的时候没
            // 办法用一些按键，比如 d 删除光标选区……用户必须移到最后，i进入
            // insertmode，然后从后向前删除」）。
            //
            // Warning: **鍵全是正文裏同名同義的那幾個，一個都沒新發明**：`d` 在正文裏
            // 刪選區，框裏光標壓着一個字，那就是那一個；`c` 刪了進插入；
            // `a`／`I`／`A` 是正文的三個入口。
            //
            // Warning: **`gh`／`gl` 和 `w b e` 沒有搬進來**——它們是為一長行散文準備的，
            // 而這是個兩三個字的框；`A`／`I` 本來就把行首行尾這兩個去處帶上了。
            // 再說 `g` 在結果那一格已經是「到第一條」，在框裏當引導鍵要多引一套
            // 待決狀態。
            Key::Char('d') if self.search.takes_text() => {
                self.search.delete_here();
                self.after_editing_the_box();
            }
            Key::Char('D') if self.search.takes_text() => {
                self.search.delete_to_end();
                self.after_editing_the_box();
            }
            Key::Char('c') if self.search.takes_text() => {
                self.search.delete_here();
                self.after_editing_the_box();
                self.mode = Mode::Field;
            }
            Key::Char('C') if self.search.takes_text() => {
                self.search.delete_to_end();
                self.after_editing_the_box();
                self.mode = Mode::Field;
            }
            Key::Char('a') if self.search.takes_text() => {
                let to = self.search.caret + 1;
                self.search.move_caret(to);
                self.mode = Mode::Field;
            }
            Key::Char('I') if self.search.takes_text() => {
                self.search.move_caret(0);
                self.mode = Mode::Field;
            }
            Key::Char('A') if self.search.takes_text() => {
                let end = self.search.typed().chars().count();
                self.search.move_caret(end);
                self.mode = Mode::Field;
            }
            // Warning: **站在結果上按 `i`，從前靜悄悄跳回搜索框接着打字**
            // （2026-09-27 兩個試用的人都撞上）：`server` 變成 `serverXX`、名單
            // 變成「無結果」，而屏幕上一個字都沒說。vi 用戶站在一條命中上第一個
            // 按的就是 `i`，他想的是「去那裏改」——那是 `Enter`。
            Key::Char('i' | 'a' | 'c' | 'I' | 'A' | 'd' | 'D' | 'C')
                if self.search.field == Field::Results =>
            {
                self.status = say!("search.that-edits-the-box");
            }
            Key::Char('i') => {
                // Warning: **從光標那裏插，不再跳到末尾**（2026-09-25 定）。`hl` 挪了
                // 半天光標，一按 `i` 又回末尾，那就是挪了白挪。光標出廠就在末尾，
                // 所以不挪的人感覺一點沒變。
                if !self.search.takes_text() {
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
            // `C-w` `q` `:` 和光禿禿的 `Space` 是每一扇面板都有的，不是這一扇
            // 的——見 `panel_key_in_common`。最後纔試，所以這一扇自己的鍵先贏。
            //
            // Warning: **接不住就往邊欄那一層再遞一手**（2026-09-26 報的：「搜索侧栏按
            // w 变宽后，再按就没办法变窄了」）。從前這裏把回報值丟掉，於是 `w`
            // 「寬窄」和 `R`「重讀」在這一扇裏是**啞的**——而狀態欄那一行照樣寫着
            // 「`w` 寬窄」。一個寫在屏幕上、按下去沒反應的鍵，讀者只會以為自己記
            // 錯了（同 §5.12.39 那一族）。
            other => {
                if !self.panel_key_in_common(other, side) {
                    self.on_sidebar_key_after_the_list(other);
                }
            }
        }
    }

    /// 測試要擺一條命中進去——`search` 本身不是公開的。
    #[cfg(test)]
    pub(crate) fn search_for_test(&mut self) -> &mut crate::search_panel::Search {
        &mut self.search
    }

    /// The highlighted hit, its line number, and where the match sits in it.
    ///
    /// Warning: **2026-09-27 走過一趟又回來了**，同 `fit_around`。它本來是命令行畫前後文
    /// 用的，而預覽挪進正文之後那一行改寫鍵位，於是連它一起刪了。回來是因為**「換後」
    /// 那一塊要它**：那一塊畫的是「這一處換完長什麼樣」，而要畫得出來就得先有這一處
    /// 前後的字。同一件事，換了個地方。
    ///
    /// **The text comes back untrimmed and the row does the fitting.** How
    /// much of it fits is a question about the window, and the window is the
    /// front end's to know; what the editor knows is which characters are
    /// around the match and which ones *are* the match. The range counts
    /// characters into the text handed back, so the row can pick the word out
    /// however it likes.
    fn hit_in_context(&self) -> Option<(usize, String, std::ops::Range<usize>)> {
        if self.search.field != Field::Results || self.search.broken {
            return None;
        }
        let side = self.panel_focus()?;
        if self.panel(side).map(|p| p.view()) != Some(crate::sidebar::View::Search) {
            return None;
        }
        let hit = self.search.here()?;
        // Warning: **命中不一定在眼前這個緩衝區裏，而這裏問的是眼前這一個。**
        // `:search .` 搜的是整個文件夾，命中帶着自己的檔（`Hit::file`）；拿一條
        // 第 6496 行的命中去問一份**只有一行**的 scratch，ropey 當場 panic
        // ——2026-09-23 報的：在倉裏 `ye` 空開、`:search .`、Esc、按 `j` 走到結果
        // 列表上，一進去就崩。
        //
        // Warning: **行號也要夾。** 就算命中真在這一份裏，搜索是那一刻跑的，而之後
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
        // Warning: **偏移也要夾，不只是行號。** 上面那一句夾的是 `hit.line`，而
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
        // Warning: **命中本身可能比摘出來的這一段還長**（一條 `.*` 規則能匹配整行），
        // 所以尾巴要夾在摘出來的這一段裏，不能照 `hit.end` 直接算。
        let long = (hit.end - hit.at).min(to - at.min(to));
        let mark = lead + (at - from)..lead + (at - from) + long;
        Some((hit.line + 1, text, mark))
    }

    /// **「換後」那一塊要畫的三段**：這一處前後的文字、換下來的那一段在裏面的
    /// 位置、以及換上去的那一段。
    ///
    /// 2026-09-27 定。面板那一欄窄，一行摘要放不下「從什麼變成什麼」，所以那件
    /// 事挪到名單底下一塊自己的地方去說，而那裏放得下更多上下文。
    ///
    /// `None` 的時候：沒在替換、沒站在一處命中上、或者「換」那一格是空的——三個
    /// 條件缺一，那幾行就還給名單。
    ///
    /// Warning: **換上去的那一段是算出來的，不是框裏那幾個字**：正則那一路的 `$1` 要
    /// 展開，不然預覽寫的和 `r` 換出來的不是同一個東西。
    pub fn replace_preview(&self) -> Option<(String, std::ops::Range<usize>, String)> {
        if !self.search.replacing || self.search.replace.is_empty() {
            return None;
        }
        let (_, text, mark) = self.hit_in_context()?;
        let look = self.looker()?;
        let was: String = text
            .chars()
            .skip(mark.start)
            .take(mark.end.saturating_sub(mark.start))
            .collect();
        let now = look.expand(&was, &self.replacement());
        Some((text, mark, now))
    }

    /// Flip one of the switches, and search again.
    ///
    /// **Which one is an argument**, not 「wherever the keys are」: since
    /// 2026-09-24 the cursor does not stop on a switch at all — `1`–`7` press
    /// them and `j k` walk past them.
    fn flip_switch(&mut self, which: Field) {
        match which {
            Field::Case => self.search.case = self.search.case.next(),
            // **匹配模式三選一**（2026-10-01 定）：字面 → 正則 → 模糊 → 回字面。
            // 從前是兩個獨立的勾，而「兩個都關」纔是默認——那一檔沒有名字。
            Field::Matching => {
                // Warning: **替換開着的時候轉不到模糊**（2026-09-20 定的那一條，合併之後
                // 要在這裏守住）：鬆的匹配蓋住讀者沒打的字，「把它們全換掉」交
                // 出去的範圍他預測不了。那時只在 字面 和 正則 之間轉。
                let loose = !self.search.replacing;
                let (regex, fuzzy) = match (self.search.regex, self.search.fuzzy) {
                    (false, false) => (true, false),
                    (true, _) => (false, loose),
                    (false, true) => (false, false),
                };
                self.search.regex = regex;
                self.search.fuzzy = fuzzy;
            }
            // **中文匹配四態**（2026-10-01 定）：繁簡+拼音 → 繁簡 → 拼音 → 無。
            // Warning: **從前這兩個在正則和模糊底下會失效**，所以不敢合；同日補上
            // `glyphs::widen_pattern` 與 `nearby` 的字形折疊之後，兩者再無例外。
            Field::Chinese => {
                let (glyphs, pinyin) = match (self.search.glyphs, self.search.pinyin) {
                    (true, true) => (true, false),
                    (true, false) => (false, true),
                    (false, true) => (false, false),
                    (false, false) => (true, true),
                };
                self.search.glyphs = glyphs;
                self.search.pinyin = pinyin;
            }
            // Warning: **模糊底下它不起作用，也就翻不動**——畫灰的鍵按下去該什麼都不
            // 發生，不然它是在說兩句相反的話。和正則疊得起來，所以只問模糊。
            Field::Whole if !self.search.fuzzy => self.search.whole = !self.search.whole,
            Field::Whole => return,
            Field::Replacing => return self.flip_replacing(),
            // 位置不是一個勾，是四選一；按它的號碼就是「換一檔」。
            Field::Scope => return self.step_the_scope(),
            // **連隱藏文件和 `.gitignore` 裏的一起搜**（2026-10-01 定）。範圍不走
            // 磁碟的時候它畫灰，按下去什麼都不發生——同 模糊 在替換底下那一條。
            Field::Hidden if self.search.on_disk() => self.search.hidden = !self.search.hidden,
            Field::Hidden => return,
            _ => return,
        }
        self.run_search();
    }

    /// **勾上「替換」就長出替換行**（2026-09-23 報的）。
    ///
    /// Warning: **和 模糊 互斥，而勾這一個的時候把那一個關掉、畫灰**（定的，原話：「我傾向
    /// 自動關掉畫灰」）。理由是原來那一條：鬆的匹配蓋住讀者沒打的字，「把它們全
    /// 換掉」交出去的範圍他預測不了。和 `flip_switch` 裏 正則／完整匹配 壓掉 模糊
    /// 是同一個寫法——**要一個就把打架的那個放下**，而不是留一個按了不算數的勾。
    ///
    /// Warning: **關掉替換不會自動把 模糊 打開**：它本來就是關着的那一個，替下去再彈
    /// 回來是替讀者做了他沒說過的決定。
    fn flip_replacing(&mut self) {
        // **一行三態**（2026-10-01 定）：關 → 字面替換 → 智能大小寫 → 回關。
        // 次序是作者定的：第二檔是出廠那一種（打什麼就寫什麼），按一下到的是平常
        // 要的，再按纔是特殊的。
        let (replacing, keep) = match (self.search.replacing, self.search.preserve_case) {
            (false, _) => (true, false),
            (true, false) => (true, true),
            (true, true) => (false, false),
        };
        let opening = replacing && !self.search.replacing;
        self.search.replacing = replacing;
        self.search.preserve_case = keep;
        if opening {
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
    /// Warning: 三個用命令開的範圍（`-cd`／`-wd`／`-gd`）**沒有**對應的 `:search` 參
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
    /// **框裏改完一個字之後**：邊打邊搜的那一格照樣邊改邊搜。
    ///
    /// 和 `on_field_key` 裏打字那一支同一條規矩——「位置」那一格是按了纔算，別的
    /// 兩格改一個字就重找一遍。
    fn after_editing_the_box(&mut self) {
        match self.search.scope.live() {
            true => self.run_search(),
            // Warning: **改了 包含／排除 不自己重走一趟磁碟**（2026-10-01）。那兩格
            // 只在走磁碟的範圍下打得了字，而磁碟那一趟是 `Enter` 的事。改了就
            // 記成過期，面板上那句「按 Enter」自己會出來。
            false => self.search.stale = true,
        }
    }

    /// **跑一遍，該walk盤的就walk盤**——`Enter` 的全部作用。
    ///
    /// 邊打邊搜的那一種（本文件）名單已經是新的，可還是照跑：`Enter` 在兩種範圍
    /// 下要做同一件事，而「這一種其實不必跑」是一句只有寫代碼的人纔知道的話。
    fn look_again(&mut self) {
        match self.search.scope.live() {
            true => self.run_search(),
            // Warning: **走磁盤那一趟不在這裏跑**（2026-09-27）：先記一筆，讓前端畫完
            // 一幀「正在找…」再回頭跑。見 `Editor::owed_search`。
            false => self.owed_search = true,
        }
    }

    /// 欠着的那一趟搜索還欠着嗎——問一次就算還了。
    pub fn take_owed_search(&mut self) -> bool {
        std::mem::take(&mut self.owed_search)
    }

    /// 欠着的那一趟，跑掉。
    pub fn run_owed_search(&mut self) {
        self.search_now();
    }

    /// 屏幕上要不要寫「正在找…」。
    pub fn is_scanning(&self) -> bool {
        self.owed_search
    }

    /// **欠着就當場還掉。** 給沒有主循環的那些呼叫方用——測試，以及任何一個
    /// 「按完就要答案」的地方。
    ///
    /// Warning: 前端不要用這一支：它跑的就是那件慢事，而前端的辦法是先畫一幀說「正在
    /// 找…」再跑（見 [`Editor::owed_search`]）。
    pub fn settle_search(&mut self) {
        if self.take_owed_search() {
            self.run_owed_search();
            return;
        }
        self.refresh_the_edited_file();
    }

    /// **正文改過之後，把改過的那一份重搜一遍**——只有那一份（2026-09-29 定）。
    ///
    /// 原話：「正文修改后，可以及时刷新侧栏重新搜索（只重新搜索**被修改的文件**
    /// 以防止不必要的搜索）。這樣只要用戶在主工作區修改了什麼，側欄能夠及時反饋。
    /// 我們也不需要回側欄先得按一下 enter 刷新才能再按 enter 跳轉了。」
    ///
    /// 每一幀畫之前問一次。Warning: **不是每一幀都做事**：指紋對得上就立刻回來，而指紋
    /// 是三個整數的比較。
    ///
    /// 走這條捷徑要三個條件都成立，否則照舊掛着「按 Enter 重新查找」：
    ///
    /// 1. **換的不是稿子**（`buffer` 沒變）。換一份稿子在寫，每一處命中身上的檔名
    ///    都要重寫——那不是「重搜一份」，那是重搜。
    /// 2. **動的只有正在寫的這一份**（`every` 的增量等於 `revision` 的增量）。
    ///    別處也動了就交給整趟重搜，它知道怎麼把別人的那幾段也擺對。
    /// 3. **名單沒有被 `MOST` 砍過。** 砍過就不知道後面漏了哪些，接不回去。
    pub fn refresh_the_edited_file(&mut self) {
        let Some(was) = self.search.looked_at else { return };
        // `stale` 是「框裏的詞改了、而這個範圍不邊打邊搜」，那件事等 Enter。
        if self.search.stale || !self.search_panel_is_open() {
            return;
        }
        let now = self.search_mark();
        if now == was {
            return;
        }
        let only_the_open_one = now.buffer == was.buffer
            && now.every.wrapping_sub(was.every) == now.revision.wrapping_sub(was.revision);
        if !only_the_open_one {
            return;
        }
        self.rescan_the_open_one();
    }

    /// 把名單開頭那一段——正在寫的那一份的命中——換成新的。
    fn rescan_the_open_one(&mut self) {
        // Warning: **緩衝區那一檔不走這條快路**（2026-10-02 審出來的）。這一支拿
        // `my_label()` 給新命中貼名字，而那一支開頭就是 `search.root.as_ref()?`
        // ——緩衝區那一檔沒有根（`scan_every_buffer` 有意設成 `None`），於是整
        // 段換上去的命中 `file` 和 `buffer` 全是 `None`：自己那幾處的檔頭變成
        // 一個空名字，而要是只剩自己那幾處，名單還會從樹悄悄塌成平的。
        //
        // 那一檔全在內存裏，重跑一趟不碰盤，所以直接重跑——比兩處各維護一套
        // 貼名字的規矩靠得住。
        if matches!(self.search.scope, Where::Buffers) {
            return self.search_now();
        }
        let Some(look) = self.looker() else { return };
        let mine = self.my_label();
        let was = self.search.mine.min(self.search.hits.len());
        let others = self.search.hits.len() - was;
        let (fresh, total) = self.scan_the_open_one(&look, &mine, MOST.saturating_sub(others));
        // 砍過就接不回去：後面漏了哪些沒人知道。整趟重跑，它自己會把數擺對。
        if self.search.total > MOST || total + (self.search.total - self.search.mine_total) > MOST {
            return self.search_now();
        }
        let grew = fresh.len() as isize - was as isize;
        self.search.hits.splice(0..was, fresh);
        self.search.total = self.search.total - self.search.mine_total + total;
        self.search.mine = self.search.hits.len() - others;
        self.search.mine_total = total;
        // **站着的那一行跟着挪。** 站在別人那一段上的時候，前面長了幾行就往下挪
        // 幾行——那一行說的還是同一處命中。站在自己這一段裏就只夾住，那幾處本來
        // 就被剛纔那一筆改動挪動了。
        let rows = self.search.rows().len();
        self.search.selected = match self.search.selected >= was && grew != 0 {
            true => self.search.selected.saturating_add_signed(grew),
            false => self.search.selected,
        }
        .min(rows.saturating_sub(1));
        self.search.looked_at = Some(self.search_mark());
    }

    /// **正在寫的那一份在名單上叫什麼**——相對搜索的根，沒有根就沒有名字。
    fn my_label(&self) -> Option<std::path::PathBuf> {
        let root = self.search.root.as_ref()?;
        let here = self.current_buffer().path()?;
        let here = std::fs::canonicalize(here).unwrap_or_else(|_| here.to_path_buf());
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.clone());
        Some(here.strip_prefix(&root).unwrap_or(&here).to_path_buf())
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
        let Some(look) = self.looker() else { return };
        let with = self.replacement();
        match self.buffer_of(&hit) {
            Some(index) => {
                let done =
                    self.with_buffer(index, |ed| ed.swap_one(&look, &with, hit.line, hit.nth));
                if done {
                    self.replaced_in = vec![self.buffers[index].id()];
                }
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
    /// `rel` 是名單上那個名字，`id` 是緩衝區那一檔記下的號——**有號就認號**。
    ///
    /// Warning: **2026-10-02 審出來的：從前它只認路徑。** 緩衝區那一檔的「名字」是
    /// 標籤不是路徑，沒有名字的草稿更是字面的 `[scratch]`；拿它去
    /// `buffer_for`，`open_file` 會給出一份**空的新緩衝**（`Buffer::open` 對不
    /// 存在的路徑回的是 `Ok` 加一條空繩），於是 `R` 換了 0 處、一聲不吭、還多
    /// 出一個叫 `[scratch]` 的空檔。
    fn replace_file(&mut self, rel: Option<&Path>, id: Option<u64>) -> usize {
        let Some(look) = self.looker() else { return 0 };
        let with = self.replacement();
        let index = match id {
            Some(id) => self.buffers.iter().position(|b| b.id() == id),
            None => self.buffer_for(rel),
        };
        let Some(index) = index else {
            return 0;
        };
        let done = self.with_buffer(index, |ed| ed.swap_all(&look, &with));
        if done > 0 {
            let id = self.buffers[index].id();
            if !self.replaced_in.contains(&id) {
                self.replaced_in.push(id);
            }
        }
        done
    }

    /// `R`: change every hit there is, once the reader has said yes.
    /// 那句「換不換」得到的是別的鍵：問的那個檔忘掉。
    pub(super) fn forget_the_file_it_asked_about(&mut self) {
        self.replace_this_file = None;
    }

    /// 那句「換不換」得到了 `y`：換問的是哪一批。
    pub(super) fn replace_what_was_asked(&mut self) {
        match self.replace_this_file.take() {
            Some((path, id)) => {
                self.replaced_in.clear();
                let done = self.replace_file(Some(&path), id);
                self.after_replacing(done);
            }
            None => self.replace_all_found(),
        }
    }

    pub(super) fn replace_all_found(&mut self) {
        self.replaced_in.clear();
        // Warning: **按（名字, 號）一對去重，不是只按名字**（2026-10-02 審出來的）。
        // 緩衝區那一檔裏兩份沒有名字的草稿標籤都是 `[scratch]`，只按名字去重會
        // 把第二份整個漏掉。
        let mut files: Vec<(Option<PathBuf>, Option<u64>)> = Vec::new();
        for hit in &self.search.hits {
            let one = (hit.file.clone(), hit.buffer);
            if !files.contains(&one) {
                files.push(one);
            }
        }
        let mut done = 0usize;
        for (file, id) in files {
            done += self.replace_file(file.as_deref(), id);
        }
        self.after_replacing(done);
    }

    /// Change **one match on one line** of the buffer being worked on.
    ///
    /// Found again by running the pattern over the line as it stands, rather
    /// than by the offsets the hit carries: those were counted when the file
    /// was read, and the first change in a file moves every one after it.
    /// `false` when the line no longer holds that many matches.
    fn swap_one(&mut self, look: &Look, with: &str, line: usize, nth: usize) -> bool {
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return false;
        }
        let text: String = rope.line(line).chars().collect();
        let cuts = byte_cuts(&text);
        let Some(&(a, b)) = look.spans(&text).get(nth) else {
            return false;
        };
        let (Some(&b0), Some(&b1)) = (cuts.get(a), cuts.get(b)) else {
            return false;
        };
        let grown = look.expand(&text[b0..b1], with);
        let at = rope.line_to_char(line);
        let mut rebuilt = rope.to_string();
        let here = rope.char_to_byte(at);
        rebuilt.replace_range(here + b0..here + b1, &grown);
        self.write_whole(rebuilt)
    }

    /// Change **every match** in the buffer being worked on; how many.
    fn swap_all(&mut self, look: &Look, with: &str) -> usize {
        let rope = self.current_buffer().rope();
        let text = rope.to_string();
        let spans = look.spans(&text);
        if spans.is_empty() {
            return 0;
        }
        let cuts = byte_cuts(&text);
        // **從後往前換。** 換上去的那一段和換下來的那一段不一樣長，從前往後走會
        // 把後面每一處的位置都推着走；倒着走，還沒動到的那幾處位置一格不變。
        let mut rebuilt = text.clone();
        for &(a, b) in spans.iter().rev() {
            let (Some(&b0), Some(&b1)) = (cuts.get(a), cuts.get(b)) else {
                continue;
            };
            let grown = look.expand(&text[b0..b1], with);
            rebuilt.replace_range(b0..b1, &grown);
        }
        match self.write_whole(rebuilt) {
            true => spans.len(),
            false => 0,
        }
    }

    /// Put a rewritten document back, with the guards a `:s` gets.
    fn write_whole(&mut self, rebuilt: String) -> bool {
        // Warning: The grid guard, for the same reason `:s` has it: a substitution
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
        self.sel.set_anchor(self.sel.head());
        self.refresh_goal_column();
        true
    }

    /// Which buffer a hit is in, opening the file if it is not open yet.
    ///
    /// Warning: **緩衝區那一檔記的是號，不是路徑**——沒有名字的草稿沒有路徑可以回去，
    /// 而兩個草稿的名字是同一個 `[scratch]`。
    fn buffer_of(&mut self, hit: &Hit) -> Option<usize> {
        if let Some(id) = hit.buffer {
            return self.buffers.iter().position(|b| b.id() == id);
        }
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
        // Warning: **盤上沒有這個檔就別開**（2026-10-02 審出來的）。`Buffer::open`
        // 對不存在的路徑回的是 `Ok` 加一條空繩——那是 `:open` 要的規矩（開一個
        // 新檔），在這裏卻是「名單上那個名字解錯了」被悄悄變成一份空緩衝。
        if !full.is_file() {
            self.status = say!("search.moved-on");
            return None;
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
    /// **這一問怎麼問**——找的時候問一次，換的時候拿同一支再問一次。
    ///
    /// Warning: **從前換那一步自己編一個正則**（`Regex::new(&self.search_pattern())`），
    /// 於是**拼音和 模糊 找到的那幾處換不了**：`search_pattern` 只折字形，不折讀
    /// 音也不管「差不多」，`sifuqi` 當正則在「伺服器」那一行一個字都配不上，`r`
    /// 按下去只報一句「那一處已經不在那裏了」——看着像文稿被人改過，其實是兩邊問
    /// 的不是同一個問題（2026-09-27 報的：「我还是不知道搜索面板中 replace 该怎么
    /// 做」）。
    ///
    /// `None` ＝ 式子寫壞了。
    pub(super) fn looker(&self) -> Option<Look> {
        // **拼音只在查詢全是 ASCII 字母的時候纔跑**，所以開着它不影響搜英文。
        // Warning: 正則開着也照跑：它自己走一趟，不往正則裏塞東西（不像簡繁異體）。
        let said = match self.search.pinyin {
            true => crate::pinyin::as_query(&self.search.query),
            false => None,
        };
        let how = match self.search.fuzzy {
            true => How::Nearby {
                needle: self.search.query.chars().collect(),
                // **模糊底下簡繁異體照樣算數**（2026-10-01 補）。挑選器一直是這樣，
                // 而這一扇從前不是——同一個開關兩種行為。
                shapes: self.search.glyphs,
                fold: match self.search.case {
                    Case::Insensitive => true,
                    Case::Sensitive => false,
                    // The rule the page's own `/` follows: a capital is how
                    // you ask for case to matter.
                    Case::Smart => !self.search.query.chars().any(char::is_uppercase),
                },
            },
            false => How::Pattern(regex::Regex::new(&self.search_pattern()).ok()?),
        };
        Some(Look { how, said, keep_case: self.search.preserve_case })
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
        // Warning: **一處都沒換就不說話**（2026-09-29）：`R` 只在名單上有東西的時候
        // 纔畫得出來，所以 `done == 0` 是走不到的；真走到了也不必解釋。
        if done > 0 {
            self.status = say!("search.replaced", done);
        }
    }

    /// **站到某一格上，站到的要是結果名單就順手看一眼。**
    ///
    /// 走進名單的那一下和在名單裏走的每一下是同一件事（2026-09-27）：光是
    /// `jk` 那兩支加預覽的話，**第一條命中會是唯一看不見的那一條**——`j` 從
    /// 「搜」那一格走進名單，站的就是它。
    fn stand_on_and_look(&mut self, field: Field) {
        self.search.stand_on(field);
        if self.search.field == Field::Results {
            self.show_hit();
        }
    }

    /// 撤回一次替換之後，名單要跟着回來——被換掉的那幾處又在了。
    fn after_replacing_undone(&mut self, files: usize) {
        let where_ = self.search.selected;
        match self.search.scope.live() {
            true => self.run_search(),
            false => self.search_now(),
        }
        self.search.selected = where_.min(self.search.rows().len().saturating_sub(1));
        // **撤回也要出聲。** `r` 說「換掉 1 處」、`R` 說「換掉 8 處」，而 `u` 從前
        // 一個字都不說——剛按錯一次 `R` 的人最需要聽見的就是這一句
        // （2026-09-27 審出來的）。
        self.status = match files {
            0 | 1 => say!("search.undone"),
            n => say!("search.undone-files", n),
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
        self.remember_jump();
        if self.show_hit() {
            // **`Enter` 就是「我留在這兒」**——鍵交給正文。走到這一處是 `jk` 早就
            // 做過的事（[`Self::show_hit`]），這個鍵只多做這一件。
            //
            // 順帶把那一份釘住：它不再是「路過看一眼」的那一份，下一次預覽不許
            // 把它收走。
            self.search_preview = None;
            self.panel_focus = None;
        }
    }

    /// **走到這一處，鍵留在面板裏**——`jk` 每走一步都做這件事（2026-09-27 定，
    /// 原話：「在搜索栏结果列表中移动的时候，编辑区应当也跳转到对应的行……用户也
    /// 不需要按 enter 就能预览到」）。
    ///
    /// 開檔、滾過去、把選區蓋在那一處上。和 [`Self::go_to_hit`] 只差最後那一下
    /// ——交不交鍵。從前兩件事焊在 `Enter` 上，於是「我只想看一眼要不要換」必須
    /// 先離開面板，看完再走回來。
    ///
    /// 回 `false` ＝ 站着的那一行不是一處命中（檔頭，或者名單空着）。
    fn show_hit(&mut self) -> bool {
        let Some(hit) = self.search.here().cloned() else {
            return false;
        };
        // Warning: **A hit carries a file name even when it is in the file being
        // written** (it has to, or the tree could not group it), so 「another
        // file」 is a question about the path, not about whether there is one.
        let mine = self
            .current_buffer()
            .path()
            .and_then(|p| std::fs::canonicalize(p).ok());
        // Warning: **緩衝區那一檔記的是號，不是路徑**（2026-10-02 審出來的）。那一檔
        // `search.root` 是 `None` 而每一處都帶着一個標籤，於是下面那張表把**每
        // 一處**都判成「在別的檔裏」——連你正在寫的這一份也算——然後拿
        // `[scratch]` 這個標籤去 `open_file`，開出一份空的新緩衝給你看。
        if let Some(id) = hit.buffer {
            match self.buffers.iter().position(|b| b.id() == id) {
                Some(i) if i == self.current => {}
                Some(i) => {
                    self.show_buffer(i);
                    let now = self.current_buffer().id();
                    self.let_go_of_the_search_preview(now);
                }
                // 面板開着的時候那一份被關掉了。
                None => {
                    self.status = say!("search.moved-on");
                    return false;
                }
            }
        }
        let elsewhere = match (&hit.file, &self.search.root, hit.buffer) {
            (None, _, _) => false,
            // 號已經把人送到那一份上了，offsets 也是從同一條繩上量的。
            (Some(_), _, Some(_)) => false,
            (Some(rel), Some(root), None) => std::fs::canonicalize(root.join(rel)).ok() != mine,
            (Some(_), None, None) => true,
        };
        // Another file has to be opened first — and if it cannot be, say so
        // rather than walking the cursor to that line of the wrong file.
        if elsewhere {
            let rel = hit.file.clone().unwrap_or_default();
            let full = match &self.search.root {
                Some(root) => root.join(&rel),
                None => rel,
            };
            // **開之前先問它本來開着沒有**：本來就開着的那一份不是我們開的，走
            // 開的時候不許收走。
            let was_open = self.buffer_showing(&full).is_some();
            if let Err(err) = self.open_file(&full) {
                self.status = say!("buffer.cannot-open", full.display(), err);
                return false;
            }
            let now = self.current_buffer().id();
            self.let_go_of_the_search_preview(now);
            if !was_open {
                self.search_preview = Some(now);
            }
        }
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();
        // Warning: **A hit in another file is placed by line, not by the offset.**
        // Those offsets were counted in the text as it was read; the buffer
        // just opened may have been edited since, and a stale offset would put
        // the cursor in the middle of a word somewhere else.
        //
        // Warning: **落在行首不算「指給人看」**（2026-09-27 審出來的）：一本二十章的
        // 小說，每一章的第一處命中都只把光標放在那一行的開頭，那幾個字不反白，
        // 看圖的人自己找。行號靠不住而**這一行的文字是現成的**——所以在這一行上
        // 把同一個問題再問一遍，取第 `nth` 段，指到字上。問不出來纔退回行首。
        let (at, end) = match elsewhere {
            true => {
                let line = hit.line.min(rope.len_lines().saturating_sub(1));
                let head = rope.line_to_char(line);
                let text: String = rope.line(line).chars().collect();
                match self.looker().and_then(|look| look.spans(&text).get(hit.nth).copied()) {
                    Some((a, b)) => (head + a, head + b),
                    None => (head, head),
                }
            }
            false => (hit.at.min(len), hit.end.min(len)),
        };
        self.sel.set_anchor(at);
        self.sel.set_head(motion::prev_grapheme(self.current_buffer().rope(), end.max(at)).max(at));
        self.extend = false;
        self.refresh_goal_column();
        true
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
        buffer: None,
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
pub(super) struct Look {
    /// 字面那一路：一個正則，或者「差不多是這幾個字」。
    how: How,
    /// 換上去的那一段要不要跟着原文的大小寫。見 [`Field::PreserveCase`]。
    keep_case: bool,
    /// **拼音那一路**，`None` ＝ 不跑（開關關着，或者查詢不全是字母）。
    ///
    /// Warning: **它是加出來的，不是替掉的**（2026-09-25 定的，原話：「兩種命中合並」）。
    /// 搜 `hello` 的人要的是文稿裏那個 `hello`，而搜 `shuzhai` 的人要的是「書齋」
    /// ——兩種都給，讀者自己認得出哪一條是他要的。
    said: Option<Vec<char>>,
}

/// 字面那一路怎麼問。
pub(super) enum How {
    /// A regular expression, flags and all (`search_pattern`).
    Pattern(Regex),
    /// The 模糊 switch: [`crate::nearby`], which counts in characters.
    Nearby { needle: Vec<char>, fold: bool, shapes: bool },
}

impl Look {
    /// **換上去的那一段**，`matched` 是換下來的那一段。
    ///
    /// Warning: **`$1` 只有正則那一路認得。** 拼音和 模糊 沒有分組可以展開，那兩路換
    /// 的就是字面——而讀者在那兩種模式下也寫不出一個有分組的式子。
    fn expand(&self, matched: &str, with: &str) -> String {
        let grown = match &self.how {
            How::Pattern(re) => match re.captures(matched) {
                Some(caps)
                    if caps.get(0).is_some_and(|m| m.start() == 0 && m.end() == matched.len()) =>
                {
                    let mut out = String::new();
                    caps.expand(with, &mut out);
                    out
                }
                // 這一處是拼音那一路配上的（`spans` 把兩路合並了），正則配不上它。
                _ => with.to_string(),
            },
            _ => with.to_string(),
        };
        match self.keep_case {
            true => follow_the_case_of(matched, &grown),
            false => grown,
        }
    }

    /// Where it is found in one line, as **character** ranges within it.
    ///
    /// Warning: **兩路合並之後要排序去重**：`excerpt` 按這個次序編號（`nth`），而讀者
    /// 看見的是一行一行往下走的單子。同一段被兩路都配上，只算一條。
    pub(super) fn spans(&self, text: &str) -> Vec<(usize, usize)> {
        let mut out = match &self.how {
            How::Pattern(re) => re
                .find_iter(text)
                .map(|m| (text[..m.start()].chars().count(), text[..m.end()].chars().count()))
                .collect(),
            How::Nearby { needle, fold, shapes } => {
                let hay: Vec<char> = text.chars().collect();
                crate::nearby::spans(&hay, needle, *fold, *shapes)
            }
        };
        if let Some(said) = &self.said {
            let also = crate::pinyin::spans(text, said);
            if !also.is_empty() {
                out.extend(also);
                out.sort_unstable();
                out.dedup();
            }
        }
        out
    }
}

/// 每一個字在一串文字裏的字節位置，末尾多一格（第 `n` 個字 ＝ `cuts[n]..cuts[n+1]`）。
///
/// 一次算好，省得每換一處都從頭數一遍——一本書裏幾百處，逐處數就是平方。
fn byte_cuts(text: &str) -> Vec<usize> {
    text.char_indices().map(|(i, _)| i).chain(std::iter::once(text.len())).collect()
}


/// **換上去的那一段，跟着換下來的那一段寫**（2026-09-27 定）。
///
/// 三檔，同 VS Code 的 `AB`：原文**全大寫**就全大寫、**首字母大寫**就首字母大
/// 寫、別的照打的寫。判的是換下來那一段，不是整行。
///
/// Warning: **沒有一個字母就原樣交回去。** 漢字沒有大小寫，所以中文那一路走到這裏
/// 什麼都不做——這個開關對它是空的，而不是會出怪事。
fn follow_the_case_of(was: &str, now: &str) -> String {
    let letters: Vec<char> = was.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.is_empty() {
        return now.to_string();
    }
    if letters.iter().all(|c| c.is_uppercase()) && letters.len() > 1 {
        return now.to_uppercase();
    }
    if letters[0].is_uppercase() && letters.iter().skip(1).all(|c| !c.is_uppercase()) {
        let mut out = String::new();
        let mut chars = now.chars();
        if let Some(head) = chars.next() {
            out.extend(head.to_uppercase());
        }
        out.push_str(chars.as_str());
        return out;
    }
    now.to_string()
}
