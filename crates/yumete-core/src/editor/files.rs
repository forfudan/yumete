//! The buffers, the files behind them, and going between them (#296).
//!
//! Opening, closing, naming, counting, grepping, following a link, exporting.
//! The 字數 report and the progress log sit here because they are questions
//! about the document as a file, not about the text under the cursor.

use super::*;

impl Editor {
    /// A count of what has been written, for `:count`.
    ///
    /// Reported three ways, because "how long is it" has three answers in
    /// Chinese: a publisher counts **字** — the 漢字 themselves — while a word
    /// processor counts every character including punctuation, and in dialogue the
    /// two differ by ten per cent or more. With a selection it counts that
    /// instead of the whole file, which is how a scene gets measured rather
    /// than a book.
    ///
    /// Ruby markup is not writing: `<ruby>永和<rt>えいわ</rt></ruby>` is two 字
    /// and two 字符, not the twenty-odd characters the tags take on disk.
    pub(super) fn count_report(&self) -> String {
        let rope = self.current_buffer().rope();
        let (start, end) = self.selection();
        // Whether the writer *made* a selection is a question about the span
        // they dragged, not about the range an edit would take — that one is
        // never empty, since it always holds the cursor's own grapheme.
        let (text, what) = if self.span().0 != self.span().1 {
            (rope.slice(start..end).to_string(), say!("count.of-selection"))
        } else {
            (rope.to_string(), say!("count.whole-file"))
        };
        let (han, chars, paragraphs) = self.counts_of(&text);
        say!("count.report", what, han, chars, paragraphs)
    }

    /// **`:info`：這份檔案是什麼**（2026-10-08 定）。
    ///
    /// 由來：`:info docs` 那個名字讓人以為是「查看這個文檔的信息」，所以那條改名
    /// `:instant-info`，而這個名字讓給真正回答那件事的一頁。
    ///
    /// `servers` 是**前端填的那一行**——哪幾個語言服務器在看着這一份，只有它知道
    /// （`Servers::watching`）。核心這一頭把整張表排好，留那一格給它，於是表的形
    /// 狀只有一處說得算。
    /// `:info` 問出去了沒有——前端每輪取一次。
    pub fn take_file_info_request(&mut self) -> bool {
        std::mem::take(&mut self.file_info_request)
    }

    pub fn file_report(&self, servers: Option<String>) -> String {
        let buffer = self.current_buffer();
        let path = buffer.path().map(|p| p.to_path_buf());
        let text = buffer.rope().to_string();
        let (han, chars, _) = self.counts_of(&text);
        let yes_no = |yes: bool| match yes {
            true => say!("file.yes"),
            false => say!("file.no"),
        };
        let mut out = format!("# {}  {}\n\n", say!("file.title"), buffer.display_name());
        out.push_str("| | |\n| --- | --- |\n");
        let mut row = |name: String, what: String| {
            out.push_str(&format!("| {name} | {what} |\n"));
        };
        match &path {
            Some(path) => row(say!("file.where"), format!("`{}`", path.display())),
            None => row(say!("file.where"), say!("file.never-saved")),
        }
        let on_disk = path.as_ref().and_then(|p| std::fs::metadata(p).ok());
        row(
            say!("file.size"),
            match &on_disk {
                Some(it) => how_big(it.len()),
                None => say!("file.never-saved"),
            },
        );
        row(say!("file.unsaved"), yes_no(buffer.is_modified()));
        row(say!("file.readonly"), yes_no(buffer.is_readonly()));
        row(say!("file.lines"), buffer.line_count().to_string());
        row(say!("file.chars"), chars.to_string());
        row(say!("file.han"), han.to_string());
        row(say!("file.language"), buffer.syntax().name().to_string());
        // 換行符：檔案自己那一種（`\r\n` 的檔存回去還是 `\r\n`，#309）。
        row(
            say!("file.ending"),
            match buffer.ending() {
                "\r\n" => "CRLF".to_string(),
                _ => "LF".to_string(),
            },
        );
        if let Some(when) = on_disk.as_ref().and_then(|it| it.modified().ok()) {
            row(say!("file.modified-at"), when_was(when));
        }
        row(say!("file.root"), format!("`{}`", self.project_root().display()));
        if let Some(servers) = servers {
            row(say!("file.servers"), servers);
        }
        out
    }

    /// **一段文字有幾個漢字、幾個字、幾段**——`:count` 和 `:info` 共用這一支。
    ///
    /// 「字」不含空白，而且**不含標記**（見 [`Self::without_markup`]：注音那一族
    /// 算它注的那幾個字，不算標籤）。兩處各數一遍就是兩個答案，而它們說的是同一
    /// 件事。
    pub(super) fn counts_of(&self, text: &str) -> (usize, usize, usize) {
        let paragraphs = text.lines().filter(|l| !l.trim().is_empty()).count();
        let prose = self.without_markup(text);
        // **一個字是一個字簇，不是一個碼位**（2026-10-09 作者定）。
        //
        // 原話：「嚴格意義上 char 應該是 grapheme cluster 而不是 codepoint。」量出來的：
        // `葛`＋U+E0100（異體字選擇符）是**一個**字，而從前這一句按碼位數，`:count`
        // 報「漢字 1  字數 2」。列數一直是對的（它本來就按字簇算），於是同一幀裏兩個
        // 數說的不是一件事。
        //
        // 漢字那一格不用改：選擇符不是漢字，`is_han` 本來就只數到基字那一個。
        let whole: String = prose.iter().collect();
        let chars = yumete_cjk::graphemes(&whole)
            .filter(|g| !g.chars().all(char::is_whitespace))
            .count();
        let han = prose.iter().filter(|&&c| is_han(c)).count();
        (han, chars, paragraphs)
    }

    /// `text` with every ruby group reduced to the base it annotates — what a
    /// reader would see on the page, which is what a word count is of.
    fn without_markup(&self, text: &str) -> Vec<char> {
        let dialects = self.ruby;
        if dialects.is_empty() {
            return text.chars().collect();
        }
        let mut out = Vec::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let chars: Vec<char> = line.chars().collect();
            let groups = crate::ruby::groups(&chars, dialects);
            let mut at = 0;
            for group in groups {
                out.extend_from_slice(&chars[at..group.start]);
                // The base as it reads, not as it is written: a Typst call
                // holds `\"` where the sentence has a quote (#332).
                let base: String = group.base_text(&chars).iter().collect();
                out.extend(group.dialect.unescape(&base).chars());
                at = group.end;
            }
            out.extend_from_slice(&chars[at..]);
        }
        out
    }

    /// **一行裏的 字** — the publisher's count, ruby markup reduced to its base.
    ///
    /// The same rule [`Editor::count_report`] answers with, so 進度 and `:count`
    /// can never disagree about how long a chapter is.
    fn han_in_line(&self, line: &str) -> usize {
        let dialects = self.ruby;
        // 沒有注音方言就沒有標記要還原——一趟掃過去，一個字節都不用抄。
        if dialects.is_empty() {
            return line.chars().filter(|&c| is_han(c)).count();
        }
        let chars: Vec<char> = line.chars().collect();
        let han = |slice: &[char]| slice.iter().filter(|&&c| is_han(c)).count();
        let mut n = 0;
        let mut at = 0;
        for group in crate::ruby::groups(&chars, dialects) {
            n += han(&chars[at..group.start]);
            // The base as it reads, not as it is written: a Typst call
            // holds `\"` where the sentence has a quote (#332).
            let base: String = group.base_text(&chars).iter().collect();
            n += group.dialect.unescape(&base).chars().filter(|&c| is_han(c)).count();
            at = group.end;
        }
        n + han(&chars[at..])
    }

    /// **一整份稿子有幾個字，不抄一份出來數**（#343，2026-10-01）。
    ///
    /// Warning: 從前是 `han_in(&rope.to_string())`——**每一次存檔**一次。八 MB 的稿
    /// 子於是每存一次先抄一份八 MB 的 `String`，`without_markup` 再抄一份
    /// `Vec<char>`（一個 `char` 四字節，三十二 MB）。這一支按行走，峯值是最長
    /// 那一行；沒有注音方言的時候連行都不抄。
    pub(super) fn han_in_rope(&self, rope: &ropey::Rope) -> usize {
        if self.ruby.is_empty() {
            return rope.chars().filter(|&c| is_han(c)).count();
        }
        (0..rope.len_lines()).map(|i| self.han_in_line(&rope.line(i).to_string())).sum()
    }

    /// Today, as the writer's own calendar has it (Feature #244).
    ///
    /// The offset is asked of the system **once** and kept: it costs a process,
    /// and a session that outlives a daylight-saving change is a session where
    /// one day's rows are an hour out — which no writing log has ever cared
    /// about.
    fn today(&mut self) -> String {
        let offset = match self.time_offset {
            Some(offset) => offset,
            None => {
                let offset = crate::progress::local_offset();
                self.time_offset = Some(offset);
                offset
            }
        };
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        crate::progress::today(secs, offset)
    }

    /// Where this book keeps its 寫作進度, whether or not it is there yet.
    ///
    /// The log belongs to the **book**, not to the chapter, so the search walks
    /// up for an existing log or for the `.yumete/` a book already has — a
    /// novel written as twenty files in one directory gets one ledger, and
    /// `第一章.md` opened from anywhere finds it.
    /// Warning: **問[項目根][`Editor::root`]，不自己再走一趟**（2026-10-01）。從前
    /// 它當前緩衝區排第一、再數別的緩衝區，認「已經有 `progress.tsv`」或「有
    /// 一個 `.yumete/` 目録」——和項目根那一支的走法不一樣（那一支還認 `.git`）。
    /// 五份各走各的「往上找 `.yumete`」，這是其中一份。順帶解決了那條註釋說的
    /// 事：清單沒有自己的檔名，而項目根本來就不問緩衝區。
    fn progress_path(&self) -> Option<PathBuf> {
        Some(self.root().join(".yumete").join("progress.tsv"))
    }

    /// The directory a project-wide command should walk — this book, not this
    /// shell (#361).
    ///
    /// `:grep`, `Space f`, the sidebar and `:word-discover` all mean 「every
    /// file in this book」, and the book is where the manuscript is, not where
    /// the terminal happened to be standing when it started. Rooting them at
    /// the working directory held only by coincidence — the coincidence that a
    /// reader `cd`s to the book before opening a chapter. Opened from anywhere
    /// else it searched the wrong tree, silently and plausibly: writing #308's
    /// test, a `:grep` over sixty chapters in a temporary directory answered
    /// out of this repository instead, 68 files and 179 hits.
    ///
    /// The walk goes up from the file for a `.yumete` first — that is where a
    /// book already keeps its config, its `words.txt` and its `tables/` — then
    /// for a `.git`, and settles for the directory the file itself is in. Only
    /// a session with no named file left in it falls back to the working
    /// directory.
    ///
    /// **Public because a language server needs it.** A server is started
    /// *in* the project (#53/#54) — asked to analyse a crate from anywhere
    /// else it finds no `Cargo.toml` and answers about nothing, silently.
    /// **這一節坐在哪個項目上。**
    ///
    /// 文件樹的根、`空格 f`、「項目」這個搜索範圍、詞表、百科、表格規格、語言
    /// 服務器的 `rootUri` 問的都是這一支——屏幕上那幾處說的必須是同一個地方，
    /// 否則樹裏看得見的和搜得到的不是一批檔。
    ///
    /// Warning: **它是[工作路徑][`Editor::working_dir`]的函數，不存**（2026-10-01
    /// 照 helix 重做）。從前有三個答案：一格釘死的 `workspace_root`、一支按打開
    /// 的緩衝區現算的 `project_root`、再加一格 `working_dir`——而全樹只有七處
    /// 問第一個，整個 TUI 一次都沒問過它。helix 只有一個可變的東西（cwd），工作
    /// 區是 `find_workspace()` 當場從它往上算出來的；這裏照辦。
    ///
    /// 於是：`gd` 跳進別人的源碼它**不動**（工作路徑沒動），`:cd` 一改它**跟着
    /// 改**。
    pub fn root(&self) -> PathBuf {
        self.project_root()
    }

    /// 家目錄——`:cd` 不帶參數去的地方（照 helix 與 vim）。
    ///
    /// Warning: **`yumete-core` 不依賴 `yumete-config`**，所以那一頭那支
    /// `home_dir` 借不過來。同 `downloads_dir`（`editor.rs`）的那幾行。
    pub(super) fn home_dir() -> Option<PathBuf> {
        std::env::var("HOME")
            .ok()
            .filter(|h| !h.is_empty())
            .or_else(|| std::env::var("USERPROFILE").ok().filter(|h| !h.is_empty()))
            .map(PathBuf::from)
    }

    /// 開頭那個 `~` 換成家目錄；沒有 `~` 就原樣。
    ///
    /// Warning: **只認 `~` 自己和 `~/`**（2026-10-01 審出來的）。從前它剝掉任何
    /// 開頭的 `~`，於是 `~notyou/x.md`（別人的家目錄，shell 的寫法）成了
    /// `$HOME/notyou/x.md`，而一個真的叫 `~草稿.md` 的檔成了 `$HOME/草稿.md`。
    /// `yumete_config::expand_tilde` 早就是這條規矩（有一條測試釘着），這一支
    /// 跟上。
    /// **一個打進來的路徑，解成實在的那一個**——`~` 展開，相對的按
    /// [工作路徑][`Editor::working_dir`]算。
    ///
    /// Warning: **一條規矩，一處寫**（2026-10-02）。`:open` 2026-10-01 就改對了
    /// （從前它原樣交給作業系統，按**進程的 cwd** 解，於是 `:pwd` 報的那個目録
    /// `:open` 不認），可**寫出去的那幾個從來沒改**：`:cd sub` 之後
    /// `:write-as z.md` 把檔存進了上一層，而狀態欄說「存了 z.md」。同一個會話裏
    /// 讀和寫對「相對於哪裏」給出兩種答案。
    ///
    /// `~` 也是這裏展開的：`:write-as ~` 從前真的造出一個**叫 `~` 的檔**。
    pub(super) fn here_path(&self, path: &str) -> PathBuf {
        let full = Self::expand_tilde(path.trim());
        match full.is_absolute() {
            true => full,
            false => self.working_dir().join(full),
        }
    }

    pub(super) fn expand_tilde(path: &str) -> PathBuf {
        let rest = match path {
            "~" => Some(""),
            _ => path.strip_prefix("~/"),
        };
        match (rest, Self::home_dir()) {
            (Some(rest), Some(home)) => home.join(rest),
            _ => PathBuf::from(path),
        }
    }

    /// **工作路徑**——`空格 F` 搜的那個目錄（2026-10-01）。
    ///
    /// 和[項目路徑][`Editor::root`]是兩件事：項目路徑是**從打開的那個檔往上找
    /// `.git`/`.yumete` 找到的那一層**，啓動時定一次；工作路徑是**你敲 `ye`
    /// 的那個目錄**，`:cd` 改得動。在項目根上敲 `ye` 的話兩者相同，從子目錄
    /// 進去纔分得開。
    pub fn working_dir(&self) -> PathBuf {
        self.working_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    }

    /// `:cd <路徑>`。回 `false` ＝ 那裏沒有這個目錄。
    pub fn set_working_dir(&mut self, at: &Path) -> bool {
        let full = match at.is_absolute() {
            true => at.to_path_buf(),
            false => self.working_dir().join(at),
        };
        if !full.is_dir() {
            return false;
        }
        let now = std::fs::canonicalize(&full).unwrap_or(full);
        self.working_dir_before = Some(self.working_dir());
        self.working_dir = Some(now);
        true
    }

    /// `:cd -`。回 `None` ＝ 還沒有上一個。
    pub fn working_dir_back(&mut self) -> Option<PathBuf> {
        let back = self.working_dir_before.take()?;
        self.working_dir_before = Some(self.working_dir());
        self.working_dir = Some(back.clone());
        Some(back)
    }

    /// **命令行給的是一個文件夾，那就定[工作路徑][`Editor::working_dir`]。**
    ///
    /// 給一個**檔**不動——照 helix（`helix-term/src/main.rs`：只有目錄參數與
    /// `-w` 設 cwd）。項目路徑不另存，[`Editor::root`] 當場從工作路徑往上算。
    ///
    /// Warning: **2026-10-01 這裏「讓檔也動一格」過，當天撤了。** 當時想的是
    /// `ye 卷一/一.md` 從 `/tmp` 敲時 `空格 f` 會列出 `/tmp`。可**那一改對項目
    /// 路徑毫無幫助**：`cd /repo && ye docs/manual.md` 兩種規矩算出來的項目都是
    /// `/repo`（往上走本來就會走過 `docs/`），它只在「從項目外面敲一個項目裏的
    /// 檔」那一種纔有用。而代價是每一次從子目録啓動都要付：`:pwd` 報一個你沒打
    /// 過的目録、`:open README.md` 解到 `docs/` 底下去（那個檔不在就**一聲不吭
    /// 地**開一個空緩衝，`:w` 在錯的地方造檔）、`!cargo test` 跑在 `docs/` 裏。
    /// 判詞：撤回去。
    pub fn set_root(&mut self, at: &Path) {
        let full = match at.is_absolute() {
            true => at.to_path_buf(),
            false => std::env::current_dir().unwrap_or_default().join(at),
        };
        let Some(here) = full.is_dir().then_some(full) else { return };
        self.working_dir = Some(std::fs::canonicalize(&here).unwrap_or(here));
    }

    pub fn project_root(&self) -> PathBuf {
        self.project_root_from(&self.working_dir())
    }

    /// The same, told where 「here」 is.
    ///
    /// Warning: **不看打開了哪幾個檔**（2026-10-01 改，照 helix 的
    /// `find_workspace`）。從前它當前緩衝區排第一、再數別的緩衝區，於是 `gd`
    /// 跳進 rustup 的源碼之後整個「項目」跟着跑。
    fn project_root_from(&self, here: &Path) -> PathBuf {
        book_root(here)
    }

    /// This book's log as it stands on disk. Missing is empty, not an error.
    fn progress_log(&self, path: &Path) -> crate::progress::Log {
        std::fs::read_to_string(path)
            .map(|text| crate::progress::Log::from_text(&text))
            .unwrap_or_default()
    }

    /// Write the log back, answering with what went wrong.
    ///
    /// Warning: **Atomically, like every other write to a reader's file.** This one
    /// rewrites the *whole* ledger on every successful save, and `fs::write`
    /// truncates before it writes: a full disk, a power cut or a `kill` inside
    /// that window leaves months of `:count-progress` history as an empty file
    /// or half a line — silently, because the caller drops the error on
    /// purpose. `write_file_atomically` writes a temporary beside it and
    /// renames, so the old ledger survives every failure intact.
    fn write_progress_log(&self, path: &Path, log: &crate::progress::Log) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        crate::buffer::write_file_atomically(path, &log.to_text())
    }

    /// Record what this file holds now, after a save (Feature #244).
    ///
    /// **It is silent, and it makes nothing.** A save must not fail, or even
    /// say anything, because a progress log could not be written — and a
    /// manuscript is not the only thing an editor saves. A ledger appears when
    /// the writer asks for one (`:count-target`, `:count-progress`), never because a
    /// config file was edited in a directory that had never heard of yumete;
    /// after that every save keeps it up to date.
    pub(super) fn note_progress(&mut self) {
        let Some(path) = self.progress_path() else {
            return;
        };
        if !path.is_file() {
            return;
        }
        let Some(name) = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
        else {
            return;
        };
        let full = self.current_buffer().path().map(Path::to_path_buf);
        // Warning: **只記這本書自己的檔**（2026-10-02 查出來的）。從前它只問「有沒有
        // 這本賬」，不問「這一份在不在這本書裏」——於是**在書開着的時候隨手存一
        // 個別處的檔，那個檔名就進了寫稿人的寫作進度**。
        //
        // 這個倉自己就中着：`cargo test` 在 `$TMPDIR` 裏存臨時檔，而測試進程的
        // cwd 在倉裏，`root()` 於是算成這個倉——`.yumete/progress.tsv` 裏攢了一百
        // 多行 `yumete-editor-write-97129.md`，混在真的章節中間，而「目標 2000」
        // 那個數就是照這本賬算的。順帶：每跑一次測試工作區就髒一次。
        //
        // 上面那段註釋早就擔心過這件事（「a config file was edited in a directory
        // that had never heard of yumete」），可它防的是「這本賬不存在」，不是
        // 「這一份不屬於這本書」。和搜索那邊同一條規矩（`the_open_one_in_scope`）。
        let here = full.as_deref().map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()));
        let book = std::fs::canonicalize(self.root()).unwrap_or_else(|_| self.root());
        if !here.is_some_and(|p| p.starts_with(&book)) {
            return;
        }
        let now = self.han_in_rope(self.current_buffer().rope());
        let opened = full
            .as_ref()
            .and_then(|p| self.opened_with.get(p).copied())
            .unwrap_or(now);
        let date = self.today();
        let mut log = self.progress_log(&path);
        log.note(&date, &name, opened, now);
        let _ = self.write_progress_log(&path, &log);
    }

    /// `:count-progress` — 寫作進度: today against the target, and every day before.
    pub(super) fn progress_report(&mut self) {
        let Some(path) = self.progress_path() else {
            self.status = say!("progress.no-file");
            return;
        };
        let mut log = self.progress_log(&path);
        // Asking is enough to open the ledger: `:count-progress` on a book that has
        // never been counted answers 「還沒有記錄」 *and* starts today's row, so
        // that the next save has somewhere to go. Nothing else in this editor
        // asks a writer to say 「yes, really」 twice.
        let name = self
            .current_buffer()
            .path()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned());
        let here = self.han_in_rope(self.current_buffer().rope());
        let date = self.today();
        if let Some(name) = name {
            let full = self.current_buffer().path().map(Path::to_path_buf);
            let opened = full
                .as_ref()
                .and_then(|p| self.opened_with.get(p).copied())
                .unwrap_or(here);
            log.note(&date, &name, opened, here);
            if let Err(why) = self.write_progress_log(&path, &log) {
                self.status = say!("progress.cannot-write", path.display(), why);
                return;
            }
        }
        let days = log.days();
        let today = log.written_on(&date);
        let streak = log.streak(&date);
        if days.iter().all(|(_, written)| *written == 0) && days.len() <= 1 {
            self.status = say!("progress.nothing-yet");
            return;
        }
        // The bar is measured against the target when there is one, and against
        // the best day there has been when there is not — a writer without a
        // target still wants to see Tuesday next to Wednesday.
        let scale = log
            .target
            .unwrap_or_else(|| days.iter().map(|(_, w)| *w).max().unwrap_or(0).max(1) as usize);
        let mut listing = String::new();
        for (day, written) in &days {
            let row = say!(
                "progress.day",
                day,
                written,
                crate::progress::bar(*written, scale)
            );
            // A day with no bar yet would otherwise end in the 全角 space the
            // row is spelled with, and a results buffer full of trailing
            // whitespace is one `:w` away from being a diff.
            listing.push_str(row.trim_end());
            listing.push('\n');
        }
        // 本書 comes from the **ledger**, not from the buffer in front of the
        // reader: `:count-progress` opens a listing, and a second one read from
        // inside that listing used to report the listing's own length.
        let book = log.book();
        self.show_listing(listing, say!("progress.results"));
        self.status = match log.target {
            Some(target) => say!(
                "progress.report-target",
                today,
                target,
                today.max(0) * 100 / target.max(1) as i64,
                book,
                streak
            ),
            None => say!("progress.report", today, book, streak),
        };
    }

    /// `:count-target <字>` — how many 字 a day, or `off`.
    pub(super) fn set_target(&mut self, target: Option<usize>) {
        let Some(path) = self.progress_path() else {
            self.status = say!("progress.no-file");
            return;
        };
        let mut log = self.progress_log(&path);
        log.target = target;
        if let Err(why) = self.write_progress_log(&path, &log) {
            self.status = say!("progress.cannot-write", path.display(), why);
            return;
        }
        self.status = match target {
            Some(target) => say!("progress.target-set", target, path.display()),
            None => say!("progress.target-cleared"),
        };
    }

    /// How many buffers are open, and which one is showing (both 1-based, for
    /// the status line).
    pub fn buffer_position(&self) -> (usize, usize) {
        (self.current + 1, self.buffers.len())
    }

    /// Show the next buffer, wrapping (Helix `gn`, `:buffer-next`).
    pub fn next_buffer(&mut self) {
        if self.only_one_buffer() {
            return;
        }
        let next = (self.current + 1) % self.buffers.len();
        self.show_buffer(next);
    }

    /// Say so when there is nowhere to switch to, rather than swallowing the
    /// key: a `gn` that does nothing silently reads as a broken keymap.
    fn only_one_buffer(&mut self) -> bool {
        if self.buffers.len() == 1 {
            self.status = say!("buffer.only-one-open");
            return true;
        }
        false
    }

    /// **切回剛纔那一份**（`ga`，helix 的 `goto_last_accessed_file`）。
    ///
    /// Warning: **`gn`/`gp` 答不了這件事。** 它們按順序走一圈，而正文和筆記、這一章和上一章
    /// 之間來回切是最常做的一件事——開着五個檔的時候 `gp` 未必回得到剛纔那一個。
    ///
    /// Warning: **只記 [`Self::show_buffer`] 那一條路。** 換面板（`switch_pane`）和關檔都是直接
    /// 動 `current` 的，它們不算「看過」：前者兩半同時在屏幕上，後者那一份已經不在了。
    pub fn goto_last_file(&mut self) {
        let Some(id) = self.last_file else {
            self.status = say!("goto.no-last-file");
            return;
        };
        let Some(index) = self.buffer_with(id) else {
            self.last_file = None;
            self.status = say!("goto.last-file-was-closed");
            return;
        };
        self.show_buffer(index);
    }

    /// Show the previous buffer, wrapping (Helix `gp`, `:buffer-previous`).
    pub fn prev_buffer(&mut self) {
        if self.only_one_buffer() {
            return;
        }
        let count = self.buffers.len();
        let previous = (self.current + count - 1) % count;
        self.show_buffer(previous);
    }

    /// Switch to buffer `index`, putting the cursor back where it was left.
    pub(super) fn show_buffer(&mut self, index: usize) {
        if index == self.current || index >= self.buffers.len() {
            return;
        }
        let at = self.sel.head();
        self.buffers[self.current].save_cursor(at);
        // **剛纔看的是哪一份**（`ga`，helix 的 `goto_last_accessed_file`，2026-09-28）。
        // Warning: **記的是 id，不是下標**：關掉一個檔會把後面每一個的下標往前挪，記下標的話
        // `ga` 會帶你去另一章。`Pane` 為同一個理由早就記 id 了。
        self.last_file = Some(self.current_buffer().id());
        self.current = index;
        // **翻到一份還沒決定過的草稿，就在那一刻問**（2026-10-02 定）。開檔
        // 是一條路，換檔是另一條——而「第一次看見這一份」在兩條路上是同一件事。
        self.ask_about_the_draft();
        let restored = self.current_buffer().saved_cursor();
        self.set_cursor(restored);
        self.extend = false;
        // Everything that was about the *other* document goes — one list, in
        // one place. Whether this file is a grid is asked again there, so a
        // chapter opened next to a table cannot inherit the table's columns.
        // How you were reading it, though, is a fact about you: coming back to
        // a table you were walking by character should not silently put you
        // back on cells.
        let grain = self.table.as_ref().map(|v| v.grain);
        self.forget_the_document();
        if let (Some(grain), Some(view)) = (grain, self.table.as_mut()) {
            view.grain = grain;
        }
        // The `[n/total]` indicator is already on the status line; repeating it
        // here would print it twice on every switch.
        self.status = self.current_buffer().display_name().to_string();
        // The buffer list and the outline are both about *this* file.
        self.refresh_sidebar();
        self.mark_visited();
    }

    /// **Remember that this is the file being written now** — the head of the
    /// list `空格 f` offers.
    ///
    /// Newest first, and one entry per file: coming back to a chapter moves it
    /// up rather than adding it again. Bounded, because a long afternoon would
    /// otherwise grow it without end and only the first handful is ever read.
    pub(super) fn mark_visited(&mut self) {
        const KEPT: usize = 32;
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            return;
        };
        self.visited.retain(|seen| seen != &path);
        self.visited.insert(0, path);
        self.visited.truncate(KEPT);
    }

    /// The files this session has been in, newest first.
    pub(super) fn visited(&self) -> &[PathBuf] {
        &self.visited
    }

    /// The active buffer — or, while the other half of a split is being drawn,
    /// the buffer *that* half is showing (#281).
    pub fn current_buffer(&self) -> &Buffer {
        &self.buffers[self.viewing.get().map_or(self.current, |v| v.buffer)]
    }

    /// The active buffer, mutably.
    ///
    /// **Never the peeked one.** [`Self::view_pane`] is a reading override and
    /// this is the one door that ignores it: the keys belong to the live half,
    /// and an edit that landed in the half you were only looking at would be a
    /// worse bug than the misdraw the override is there to fix.
    pub fn current_buffer_mut(&mut self) -> &mut Buffer {
        &mut self.buffers[self.current]
    }

    /// Answer every question about `pane`'s file and place until the guard is
    /// dropped (#281).
    ///
    /// The other half of a split keeps a buffer id and a cursor of its own, and
    /// the divider above it prints that buffer's name — but the renderer reads
    /// the *current* buffer, so the half captioned 「第二章」 drew whatever
    /// chapter the keys were in. One scope around the draw puts all of it —
    /// text, folds, markup, tables, the caret the wrap is measured from — on
    /// the file the caption names.
    ///
    /// Reading only: see [`Self::current_buffer_mut`]. A pane whose buffer has
    /// been closed overrides nothing and the live half is drawn twice, which is
    /// what the page did before this existed.
    pub fn view_pane(&self, pane: &Pane) -> Viewed<'_> {
        let previous = self.viewing.get();
        if let Some(buffer) = self.buffers.iter().position(|b| b.id() == pane.buffer) {
            // Clamped **here**, once: the live half can have deleted the text
            // the pane was left standing in, and a stale offset walked into a
            // rope is a panic, not a misdraw.
            let at = pane.cursor().min(self.buffers[buffer].rope().len_chars());
            self.viewing.set(Some(Viewing { buffer, at }));
        }
        Viewed { editor: self, previous }
    }

    /// Where the caret is for the purpose of *drawing* — the pane's own place
    /// while [`Self::view_pane`] is open, and the live cursor otherwise.
    ///
    /// Every immutable reader of the caret goes through this. The mutable ones
    /// read the field, because an override is only ever open during a draw.
    pub(super) fn caret(&self) -> usize {
        self.viewing.get().map_or(self.sel.head(), |v| v.at)
    }

    /// The other end of the selection, by the same rule. A peeked pane has no
    /// selection of its own — what it marks is the hit it was opened to show —
    /// so both ends are its caret and [`Self::has_selection`] is false there.
    pub(super) fn mark(&self) -> usize {
        self.viewing.get().map_or(self.sel.anchor(), |v| v.at)
    }

    /// The number of open buffers.
    pub fn buffer_count(&self) -> usize {
        self.buffers.len()
    }

    /// Every open buffer's name and whether it has unsaved changes — what the
    /// tab bar draws (Feature #95).
    pub fn buffer_tabs(&self) -> Vec<(String, bool)> {
        self.buffers
            .iter()
            .map(|b| (b.display_name(), b.is_modified()))
            .collect()
    }

    /// **Every open buffer's path**, for whoever has to know when one closed.
    ///
    /// The language server does: it keeps analysing a file it was told about
    /// and keeps pushing diagnostics for it, and `Problems` are kept by path —
    /// so `:diagnostics-all` goes on listing a file that was closed an hour ago.
    /// `textDocument/didClose` is what ends that, and this is how the front
    /// end works out which one to send it for. 2026-09-23 補。
    pub fn buffer_paths(&self) -> Vec<PathBuf> {
        self.buffers.iter().filter_map(|b| b.path().map(Path::to_path_buf)).collect()
    }

    /// Show the `index`th buffer, for a front end that can point at one.
    pub fn show_buffer_at(&mut self, index: usize) {
        self.show_buffer(index);
    }

    /// Open `path` as a new buffer and make it active.
    pub fn open_file<P: AsRef<Path>>(&mut self, path: P) -> io::Result<()> {
        // A file already open is *shown*, not opened again. Two buffers over
        // one file means two undo histories, two dirty flags, and two claims on
        // one recovery copy — a way to lose work, not a way to open a file.
        let path = path.as_ref();
        if let Some(i) = self.buffer_showing(path) {
            self.show_buffer(i);
            return Ok(());
        }
        let mut buffer = Buffer::open(path)?;
        // `--readonly` is about the *session*, so it locks what the session
        // opens — not only the file named on the command line. The disk's own
        // answer is already in there and is never overruled by this.
        if self.readonly_default {
            buffer.set_readonly(true);
        }
        // A file whose name does not say what it is takes the project's word
        // for it — by extension, or by that exact name.
        if buffer.syntax_was_guessed() {
            let name = buffer.display_name();
            if let Some(syntax) = self.configured_syntax(&name) {
                buffer.set_syntax(syntax);
            }
        }
        self.add_buffer(buffer);
        // **回到上次停的那一行**（2026-10-05 定）。不知道就不動。
        //
        // Warning: **擺在這裏，所以誰都蓋得過它**：會話還原、`:open 檔:30`、`:replace`
        // 跳到命中——那些都在開檔之後纔挪光標，後說的算。
        let went = self.current_buffer().path().map(Path::to_path_buf);
        if let Some(path) = went {
            self.go_back_to_where_i_was(&path);
        }
        self.mark_visited();
        self.table_on_open();
        // 改動條：開檔是喊 git 的三個時刻之一（#55）。關着的話這一句是個空操作。
        self.refresh_vcs(true);
        // **開文件就順手認一遍這本書自己的詞** (#448). Nothing happens here —
        // the scan is three hundred milliseconds of counting and it belongs on
        // a thread — so this only leaves the request where the front end will
        // find it, and the front end decides how often it is worth honouring.
        self.ask_for_detection();
        Ok(())
    }

    /// **一個路徑的真名**，問過一次就記住——見 [`Editor::canonical`]。
    ///
    /// Warning: **問不出來就回 `None`，而且不記。** 盤上還沒有那個檔是常事
    /// （`:w` 寫一個新檔），記下來就永遠當它沒有。
    pub(super) fn real_path(&self, path: &Path) -> Option<std::path::PathBuf> {
        if let Some(found) = self.canonical.borrow().get(path) {
            return Some(found.clone());
        }
        let real = std::fs::canonicalize(path).ok()?;
        self.canonical.borrow_mut().insert(path.to_path_buf(), real.clone());
        Some(real)
    }

    /// **這個文件開着沒有**，按真實路徑比，不按拼法。
    pub(super) fn buffer_showing(&self, path: &Path) -> Option<usize> {
        let same = self.real_path(path);
        self.buffers.iter().position(|b| match (b.path(), &same) {
            (Some(open), Some(want)) => self.real_path(open).as_ref() == Some(want),
            (Some(open), None) => open == path,
            _ => false,
        })
    }

    /// **把搜索面板為「看一眼」開出來的那一份還回去。**
    ///
    /// 2026-09-27 定，原話：「真打开，但标成预览」。在結果名單裏 `jk` 走一遍一本
    /// 書，走一步開一份，走完緩衝區列表就滿了——所以同一時間只留一份，走到下一處
    /// 就把上一處那一份收走。`Enter` 把它釘住（`search_preview` 清空），從此它就
    /// 是一份普通的緩衝。
    ///
    /// Warning: **改過的不收。** 那是人幹的活，不許無聲無息地關掉；它就此不再是預覽。
    /// `keep` 是剛剛開出來的那一份，同一份就什麼都不做。
    pub(super) fn let_go_of_the_search_preview(&mut self, keep: u64) {
        let Some(id) = self.search_preview else { return };
        if id == keep || self.buffers.len() <= 1 {
            return;
        }
        let Some(i) = self.buffer_with(id) else {
            self.search_preview = None;
            return;
        };
        if i == self.current || self.buffers[i].is_modified() {
            self.search_preview = None;
            return;
        }
        self.buffers[i].clear_swap();
        self.buffers.remove(i);
        if self.current > i {
            self.current -= 1;
        }
        // A pane naming a buffer that is gone is not a pane.
        if self.other.as_ref().is_some_and(|pane| pane.buffer == id) {
            self.other = None;
            self.live_pane = 0;
        }
        self.search_preview = None;
    }

    /// Open a file this one *pulls in* — a `#include`d chapter.
    ///
    /// It inherits the syntax, because a chapter included into a Typst book is
    /// Typst whatever its name says and whatever is in it: a chapter that is
    /// nothing but writing has no Typst in it to find, and reading it as
    /// Markdown would make `*很早*` mean nothing. Only files reached *through*
    /// an include inherit — opening an unrelated file is not a claim about it.
    pub(super) fn open_included_file(&mut self, path: &Path) -> io::Result<()> {
        let from = self.current_buffer().syntax();
        self.open_file(path)?;
        if self.current_buffer().syntax_was_guessed() && from == crate::syntax::Syntax::Typst {
            self.current_buffer_mut().set_syntax(from);
            self.markup_memo.forget();
            *self.block_cache.borrow_mut() = None;
        }
        Ok(())
    }

    /// Close the active buffer (`:bd`), refusing while it has unsaved changes.
    ///
    /// The last buffer is not closed but emptied: an editor with no buffer has
    /// nowhere to put the cursor.
    pub(super) fn close_buffer(&mut self, force: bool) -> Result<CommandOutcome, EditorError> {
        if !force && self.current_buffer().is_modified() {
            return Err(EditorError::UnsavedChanges);
        }
        self.current_buffer_mut().clear_swap();
        if self.buffers.len() == 1 {
            self.buffers[0] = Buffer::scratch();
            self.set_cursor(0);
            self.forget_the_document();
            // Warning: **關掉一個緩衝區也要重畫那張單子**（2026-10-07 審出來的）。換
            // 緩衝區那條路（[`Self::show_buffer`]）一直有這一句，關掉這一條沒
            // 有——而緩衝區那一扇面板的行是推進去存着的，不是每幀現算。於是
            // `:bd` 之後單子上那一條還在，上面還標着「你在這裏」；按下去打開
            // 的是一個已經不在的號碼。
            self.refresh_sidebar();
            self.status = say!("buffer.closed");
            return Ok(CommandOutcome::Continue);
        }
        let closed = self.buffers.remove(self.current).display_name();
        self.current = self.current.min(self.buffers.len() - 1);
        let restored = self.current_buffer().saved_cursor();
        self.set_cursor(restored);
        self.forget_the_document();
        self.refresh_sidebar();
        let (n, total) = self.buffer_position();
        self.status = say!("buffer.closed-now-showing", closed, self.buffer_name(), n, total);
        Ok(CommandOutcome::Continue)
    }

    /// Let go of everything that was about the document you were just in.
    ///
    /// **One list, called from every place the document changes** — closing a
    /// buffer, switching to another. Three sibling functions used to clear
    /// three different subsets of this, which is how a grid stayed on after
    /// its table was closed and a hit list went on answering `n` in a file it
    /// had never seen.
    pub(super) fn forget_the_document(&mut self) {
        // What was said about the last document's file, and when its disk was
        // last looked at, are not facts about this one: a warning latched on
        // buffer A must not silence the warning buffer B has coming (#214).
        self.last_disk_check = None;
        self.reload_warned = false;
        self.forget_what_was_asked();
        self.forget_the_text();
        // The hits are *not* thrown away: they name their own buffer and
        // revision now, so they are simply not an answer while you are
        // elsewhere — and they are one again when you come back to the file
        // and the text they were found in.
        self.table_on_open();
        // A pane naming a buffer that is gone is not a pane.
        if self
            .other
            .as_ref()
            .is_some_and(|pane| self.buffer_with(pane.buffer).is_none())
        {
            self.other = None;
            self.live_pane = 0;
        }
    }

    /// **問服務器的那幾句，和它答的那一則**（2026-10-09 補）。
    ///
    /// 它們記的是**字元下標**，而下標只在它自己那一份文檔裏有意思：`signature_on`
    /// 指着上一份裏那個左括號，換了檔之後拿它到這一份的 rope 上算坐標
    /// （[`Editor::a_hover_on_the_callee`]），問出去的就是一句問在別處的話。
    ///
    /// Warning: **開檔與換檔是兩條路，所以這是一張單子而不是幾行**。
    /// [`Self::forget_the_document`] 走換檔與關檔，`Editor::add_buffer` 走開新檔，
    /// 而那一支清的從來是另一個子集——`forget_the_document` 自己的註釋記着這個形狀
    /// 怎麽出過事（「三支姊妹函數各清一個子集」）。兩條路都叫這一支。
    ///
    /// 那張補全單子不在裏面：`Offering::stands_here` 自己認緩衝區與下標，同搜索的
    /// 命中表——不在眼前就不算答案，回來還算。
    pub(super) fn forget_what_was_asked(&mut self) {
        self.signature = None;
        self.signature_query = None;
        self.signature_asked_at = None;
        self.signature_on = None;
        self.signature_doc_query = None;
        self.completion_query = None;
        self.completion_at = None;
    }

    /// Let go of everything derived from **the text**, the document staying the
    /// document.
    ///
    /// The half of [`Self::forget_the_document`] that an edit too big for the
    /// ordinary revision check needs — a sort rebuilds the file out of its own
    /// lines — and **only** that half. Calling the whole thing after a sort put
    /// the grid away: `forget_the_document` ends by asking the file what table
    /// it is, and a `.csv` with no schema beside it is not one, so `t1s` sorted
    /// the rows and dropped the reader back into the source
    /// (2026-09-05：「表格排序 t1s 會直接回到源碼視圖」).
    pub(super) fn forget_the_text(&mut self) {
        self.segment_memo.forget();
        self.markup_memo.forget();
        *self.md_cache.borrow_mut() = None;
        *self.md_tables.borrow_mut() = None;
        *self.block_cache.borrow_mut() = None;
        *self.fold_cache.borrow_mut() = None;
        *self.key_index.borrow_mut() = None;
    }

    /// The active buffer's short name.
    pub(super) fn buffer_name(&self) -> String {
        self.current_buffer().display_name()
    }

    /// Work on the `index`th buffer as though it were the active one, and put
    /// the editor back where it was (#350).
    ///
    /// `self.current` is what every verb reads, so a job that goes through the
    /// buffers has to move it — and [`Self::show_buffer`] is far too much
    /// machinery to run per file: it re-reads the outline, the grid and the
    /// sidebar for a document nobody is looking at. What is dangerous is not
    /// the switch but **the way out of it**. A loop that breaks in the middle
    /// leaves `current` on a buffer whose cursor, whose caches and whose grid
    /// all still belong to another document, and the next keystroke indexes
    /// this file's rope with the other file's offset — `:wa` stopping on the
    /// oversize question did exactly that, and `x` afterwards took the process
    /// down. So the switch and its undoing are one call, and landing somewhere
    /// else **on purpose** is `show_buffer`, through the front door.
    pub(super) fn with_buffer<T>(&mut self, index: usize, work: impl FnOnce(&mut Self) -> T) -> T {
        let was = self.current;
        self.current = index;
        let out = work(self);
        self.current = was.min(self.buffers.len().saturating_sub(1));
        out
    }

    /// Save every buffer that has changed (`:write-all`).
    pub(super) fn write_all(&mut self) -> Result<CommandOutcome, EditorError> {
        let mut saved = 0usize;
        let mut asked = None;
        let mut failed: Vec<String> = Vec::new();
        for i in 0..self.buffers.len() {
            if !self.buffers[i].is_modified() {
                continue;
            }
            match self.with_buffer(i, |e| e.write_current(None)) {
                Ok(Wrote::Asked) => {
                    asked = Some(i);
                    break;
                }
                Ok(_) => saved += 1,
                Err(err) => failed.push(err.to_string()),
            }
        }
        if let Some(i) = asked {
            // **Stop on the file that asked, standing on it.** The question
            // names one buffer, so the reader has to be looking at that one;
            // carrying on through the rest would put the answer against
            // whichever file the loop reached. Through `show_buffer`, so the
            // cursor and the caches come with it.
            self.show_buffer(i);
            return Ok(CommandOutcome::Continue);
        }
        self.status = if failed.is_empty() {
            say!("buffer.saved-many", saved)
        } else {
            say!("buffer.saved-some-not-all", saved, failed.len(), listed(&failed))
        };
        Ok(CommandOutcome::Continue)
    }

    /// Open the `path:line:` named on the cursor's line (`gf`).
    ///
    /// The shape a grep result has, and the shape every compiler and every
    /// other grep prints — so it also works on a line pasted in from a shell.
    pub(super) fn goto_file_under_cursor(&mut self) {
        // A `[yumete] 檔名` line names a file too (#287), and it is the one
        // line in a wiki where `gf` has something to open.
        if self.open_wiki_include() {
            return;
        }
        // **And so does a 百科 name** (2026-09-19: 「wiki 詞條 gf 跳轉定義
        // 文件失效了」). `gd` has always opened the file an entry is written in
        // — but 「open the file this names」 is exactly what `gf` is for, and a
        // name on the page names one. Standing on 滕子京 and pressing `gf` used
        // to answer 「這一行沒寫文件名」, which is true of the line and false of
        // the word under the cursor. `gd` is unchanged; this is the same door
        // with the other handle.
        if self.follow_wiki() {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.line(rope.char_to_line(self.sel.head())).to_string();
        let text = line.trim();

        // `#import "ch01.typ": chapter` / `#include "ch01.typ"` — a main file
        // that pulls its chapters in *is* the table of contents, so `gf` on one
        // of those lines opens the chapter.
        if let Some(quoted) = quoted_path(text) {
            let here = self
                .current_buffer()
                .path()
                .and_then(|p| p.parent().map(Path::to_path_buf));
            let full = match here {
                Some(dir) => dir.join(&quoted),
                None => PathBuf::from(&quoted),
            };
            if let Err(err) = self.open_included_file(&full) {
                self.status = say!("buffer.cannot-open", quoted, err);
            }
            return;
        }

        let Some((path, rest)) = text.split_once(':') else {
            self.status = say!("buffer.no-file-named-on-this-line");
            return;
        };
        let at = rest
            .split_once(':')
            .and_then(|(n, _)| n.trim().parse::<usize>().ok());
        // Relative to the directory the results were gathered from, which is
        // the one yumete was started in.
        let path = Path::new(path.trim());
        let full = match (&self.listing_root, path.is_absolute()) {
            (Some(root), false) => root.join(path),
            _ => path.to_path_buf(),
        };
        if let Err(err) = self.open_file(&full) {
            self.status = say!("buffer.cannot-open", path.display(), err);
            return;
        }
        if let Some(n) = at {
            self.goto_line(n);
        }
    }

    /// The link the cursor is standing in, if it is standing in one.
    ///
    /// Only Markdown writes links this way; Typst spells them `#link(…)`,
    /// which is code and is read as code.
    pub(super) fn link_under_cursor(&self) -> Option<crate::markdown::Link> {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.sel.head());
        let at = self.sel.head() - rope.line_to_char(line);
        let text = rope.line(line).to_string();
        let text = text.trim_end_matches(['\n', '\r']);
        match self.current_buffer().syntax() {
            crate::syntax::Syntax::Markdown => crate::markdown::link_at(text, at),
            _ => None,
        }
    }

    /// Follow the link under the cursor (`gx`) — Feature #285.
    ///
    /// A link in a manuscript points at one of three things, and they are not
    /// opened the same way:
    ///
    /// - **A page on the web** goes to whatever the reader browses with. Only
    ///   `http` and `https` do. The row this was built from said 「a scheme we
    ///   do not handle goes to the OS」, and that is the rule this deliberately
    ///   does **not** follow: handing an unknown scheme to `open` hands a file
    ///   in the manuscript the power to start any program registered for any
    ///   scheme on the machine, and a manuscript is a file that arrives by
    ///   email. What is not `http` or `https` is named and refused.
    /// - **Another file of the book** — `[附錄](fu.md)`, `[[第三章]]` — opens
    ///   as a buffer, which is 「用窗口打开」 answered by the window already
    ///   here. Read where the link is written from: a chapter names its
    ///   neighbours the way it sits beside them on the disk. An absolute path
    ///   is refused for the same reason as an unknown scheme — `/etc/…` is not
    ///   the name of a chapter.
    /// - **A place in a page** — the `#雪` half — is a heading, looked up in
    ///   the outline after the file it belongs to is open.
    ///
    /// Nothing here goes through a shell. `open`/`xdg-open` are handed the URL
    /// as one argument by the front end (see `yumete_tui::show`), and this side
    /// never builds a command line at all.
    ///
    /// Warning: **`gd` follows one too** (#454), by the same route: a link is a
    /// definition, and the one a manuscript has most of.
    pub(super) fn follow_link(&mut self) {
        let Some(link) = self.link_under_cursor() else {
            self.status = say!("link.none-here");
            return;
        };
        match link_scheme(&link.target).as_deref() {
            Some("http") | Some("https") => {
                // The fragment is the page's own business, so it goes back on.
                let url = match &link.anchor {
                    Some(a) => format!("{}#{a}", link.target),
                    None => link.target.clone(),
                };
                self.status = say!("link.opening", url);
                self.open_request = Some(url);
                return;
            }
            Some(other) => {
                self.status = say!("link.scheme-refused", other.to_string());
                return;
            }
            None => {}
        }
        // `[雪](#雪)` — a place in this same file, so nothing is opened.
        if link.target.is_empty() {
            let Some(anchor) = link.anchor else {
                self.status = say!("link.none-here");
                return;
            };
            return self.goto_heading_named(&anchor);
        }
        let named = Path::new(&link.target);
        if named.is_absolute() || link.target.starts_with('~') {
            self.status = say!("link.not-a-chapter", link.target);
            return;
        }
        let here = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        // A `[[wiki]]` names a page, it does not spell a file: the suffix is
        // this manuscript's, not the writer's to type again.
        let mut tries = vec![here.join(named)];
        if link.wiki {
            let suffix = self
                .current_buffer()
                .path()
                .and_then(|p| p.extension().map(|e| e.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "md".to_string());
            tries.insert(0, here.join(format!("{}.{suffix}", link.target)));
            tries.insert(1, here.join(format!("{}.md", link.target)));
        }
        let Some(full) = tries.into_iter().find(|p| p.is_file()) else {
            self.status = say!("link.no-such-file", link.target);
            return;
        };
        if let Err(err) = self.open_included_file(&full) {
            self.status = say!("buffer.cannot-open", link.target, err);
            return;
        }
        match link.anchor {
            Some(anchor) => self.goto_heading_named(&anchor),
            None => self.status = say!("link.opened", link.target),
        }
    }

    /// Follow the link the cursor is on, for a front end that has just put the
    /// cursor there — Ctrl-click (Feature #285).
    ///
    /// The same answer `gx` gives, because it is the same question: the mouse
    /// only decides *where*, and where is already the cursor by the time this
    /// is called.
    pub fn follow_link_here(&mut self) {
        self.follow_link();
    }

    /// Go to the heading a link's `#雪` names, in the file now shown.
    fn goto_heading_named(&mut self, anchor: &str) {
        // An anchor is written two ways and means one thing: the web spells
        // 「The Snow」 as `the-snow`, and a manuscript in 漢字 spells 雪 as 雪.
        // Comparing what is left after the punctuation an anchor drops reads
        // both without having to know which one this file was written for.
        let key = |title: &str| -> String {
            title.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
        };
        let want = key(anchor);
        match self.outline().into_iter().find(|(_, _, t)| key(t) == want) {
            Some((line, _, _)) => self.goto_line(line + 1),
            None => self.status = say!("link.no-such-heading", anchor.to_string()),
        }
    }

    /// Which open buffer a write to `target` would land in, if any.
    ///
    /// **Is this file open in any buffer?** (2026-09-22)
    ///
    /// Asked by the front end about the file a preview server is serving: a
    /// preview of a chapter nobody has open any more is a program holding a
    /// port and a few hundred megabytes for a document that is not on the
    /// screen. Same identity rule as [`Self::buffer_holding`] — a hard link is
    /// one file under two names.
    pub fn holds_file(&self, path: &Path) -> bool {
        // Warning: **便宜的那一問先答。** 這是前端**每按一鍵**都要問的（預覽開着的時
        // 候），而 `buffer_holding` 每個緩衝區要 `canonicalize` 一次——一次系統
        // 調用。拼寫一模一樣就是它，不必問磁碟；只有拼寫對不上纔值得去問「是不
        // 是同一個檔的另一個名字」，而那一步只在快要殺掉服務器之前跑一次。
        if self.buffers.iter().any(|b| b.path() == Some(path)) {
            return true;
        }
        self.buffer_holding(path).is_some()
    }

    /// **Identity, not spelling** — see [`crate::buffer::write_target`]. Every
    /// writer that is handed a path by the reader asks this before it writes:
    /// a file that is open in this editor may only be replaced by the buffer
    /// that is bound to it, stamps and all. Anything else replaces text the
    /// editor is still holding, and the buffer goes on saying it is clean.
    ///
    /// Returns the buffer's index, so the caller can name it in the message.
    pub(super) fn buffer_holding(&self, target: &Path) -> Option<usize> {
        let resolved = crate::buffer::write_target(target);
        self.buffers.iter().position(|b| {
            let Some(path) = b.path() else {
                return false;
            };
            crate::buffer::write_target(path) == resolved
                // …**or the same file under another name**: a hard link is one
                // file with two directory entries, and canonicalizing tells
                // them apart because there is nothing to tell.
                || crate::buffer::same_file(path, target)
        })
    }

    /// Whether writing `target` is refused, having said why if it is.
    ///
    /// **Never onto a manuscript.** `:export typst` on a `.typ` chapter used
    /// to name its own source — an export keeps 標題、段落、注音 and nothing
    /// else, so the figures, the tables and the raw Typst were gone from the
    /// file on disk, and the message that followed pointed at `:e!`, which
    /// throws the good copy in memory away too.
    ///
    /// The guard used to compare the two paths as *strings*, while the writer
    /// resolves them: `main.typ` against `/…/main.typ`, or a symlink against
    /// what it points at, walked straight past it. And the chapter in danger is
    /// not only this one — any file open in this editor is being held in memory
    /// and will be saved from there.
    ///
    /// **An existing file is replaced only when you say so**, which is the rule
    /// `:w` keeps. The exporter is the other writer, and it did not.
    ///
    /// Every writer that is not `:w` asks this: `:export`, `:export csv`, and
    /// `:shot`. They used to ask it in three byte-identical copies (#227 made
    /// the third), which is three places for the next fix to miss two of.
    fn refuse_to_overwrite(&mut self, target: &Path, force: bool) -> bool {
        if let Some(which) = self.buffer_holding(target) {
            self.status = match which == self.current {
                true => say!("export.same-as-the-manuscript"),
                false => say!("export.target-is-open", self.buffers[which].display_name()),
            };
            return true;
        }
        if !force && target.exists() {
            self.status = say!("export.target-exists", target.display());
            return true;
        }
        false
    }

    /// Write the manuscript out for somebody else to typeset (`:export`).
    ///
    /// The default name is the document's own with the extension swapped, which
    /// is what a writer means by "export this chapter"; a path given explicitly
    /// wins. A scratch buffer has no name to derive one from and must be told.
    pub(super) fn export(
        &mut self,
        format: &str,
        path: Option<&str>,
        force: bool,
    ) -> Result<CommandOutcome, EditorError> {
        // Warning: **鎖住的那一份一個檔都不寫**（2026-10-02 定「鎖住就不導出」）。
        // `--help` 上寫着「Open locked: nothing this run opens can be typed
        // into」，而 `:export!` 從前在 `--readonly` 的會話裏照樣把一個**不相干**
        // 的檔整個蓋掉。保護的是「這一趟開着的檔」還是「這一趟寫出去的字」，是
        // 一個範圍問題；定下來的是後者——鎖住就什麼都不寫。
        if self.refuse_readonly() {
            return Ok(CommandOutcome::Continue);
        }
        // **`csv` is the one export that is a region, not the document.** A
        // manuscript has no rows; the table under the cursor does. So it is
        // answered here, from the same machinery `:table-csv` uses, rather than
        // by `export::export`, which is handed whole texts and answers with
        // whole texts (Feature #227).
        if let Some(delimiter) = crate::export::delimiter_of(format) {
            return self.export_delimited(delimiter, format, path, force);
        }
        let Some(format) = crate::export::Format::parse(format) else {
            self.status = say!("export.no-such-format", format);
            return Ok(CommandOutcome::Continue);
        };
        let target = match path {
            Some(path) => self.here_path(path),
            None => match self.current_buffer().path() {
                Some(source) => source.with_extension(format.extension()),
                None => return Err(EditorError::NoFileName),
            },
        };
        let style = crate::export::Style {
            vertical: self.layout() == Layout::Vertical,
            hanging: self.hanging,
            zong_len: self.zong_length,
            dialects: self.ruby,
            title: self.current_buffer().display_name(),
            paper: self.paper,
            // Warning: **讀的是這一份稿子的語法**，不是要寫成的格式（2026-09-28）。
            source: self.current_buffer().syntax(),
        };
        if self.refuse_to_overwrite(&target, force) {
            return Ok(CommandOutcome::Continue);
        }
        let written = crate::export::export(&self.current_buffer().text(), format, &style);
        // Written the way a save is written: whole, or not at all.
        crate::buffer::write_file_atomically(&target, &written).map_err(EditorError::Io)?;
        self.status = say!("export.wrote", target.display());
        Ok(CommandOutcome::Continue)
    }

    /// `:shot` — a picture of the page or of the screen (#189).
    ///
    /// The whole of the decision is made here, a frame early: which file, and
    /// what draws it. What is left is the cells, which only the front end
    /// holds — so the answer is parked in `screenshot_request` and
    /// [`Editor::take_screenshot_request`] hands it over **after** the next
    /// frame is drawn, which is the one with no command line across it.
    ///
    /// **The word says which format**, not the extension: `:shot txt` is a
    /// picture you can ask for without spelling out a path, and
    /// `:shot html 給編輯.md` writes the coloured one under the name it was
    /// given rather than silently changing what was asked for.
    ///
    /// A name that is not given is [`downloads_dir`] plus the document's own
    /// stem and the second it was taken —
    /// `驚蟄_20260906143012.png`. Two reasons, both from the writer
    /// (2026-09-06): a picture is a thing you send someone and then forget, so
    /// it has no business landing in the folder the manuscript lives in; and a
    /// dated name never collides, which is a better answer than asking about
    /// overwriting. The bang is kept for the name you spell out yourself,
    /// where a collision is still possible and still yours.
    pub(super) fn take_a_picture(
        &mut self,
        shot: crate::command::Shot,
        force: bool,
    ) -> Result<CommandOutcome, EditorError> {
        let (how, path) = match shot {
            crate::command::Shot::Screen => {
                // Nothing is written, so there is nothing for the bang to
                // force — and a bang that quietly does nothing is how a person
                // comes to believe it did something.
                if force {
                    self.status = say!("shot.the-bang-is-for-a-file");
                    return Ok(CommandOutcome::Continue);
                }
                self.screenshot_request = Some(ShotJob::Screen);
                return Ok(CommandOutcome::Continue);
            }
            crate::command::Shot::File { how, path } => (how, path),
        };
        let target = match path {
            Some(path) => self.here_path(&path),
            None => match self.current_buffer().path() {
                Some(source) => {
                    let stem = source
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let when = crate::clock::stamp();
                    downloads_dir().join(format!("{stem}_{when}.{}", how.extension()))
                }
                None => return Err(EditorError::NoFileName),
            },
        };
        if self.refuse_to_overwrite(&target, force) {
            return Ok(CommandOutcome::Continue);
        }
        self.screenshot_request = Some(match how {
            crate::command::ShotFormat::Png => ShotJob::Png { target },
            crate::command::ShotFormat::Html => ShotJob::Page {
                target,
                text: false,
            },
            crate::command::ShotFormat::Text => ShotJob::Page { target, text: true },
        });
        Ok(CommandOutcome::Continue)
    }

    /// `:export csv` / `:export tsv` — the table under the cursor, as a file.
    ///
    /// The buffer is not touched: this is the difference between `:table-csv`,
    /// which converts the table in place because that is what the writer wants
    /// to go on editing, and this, which hands a copy to whatever else is going
    /// to read it.
    fn export_delimited(
        &mut self,
        delimiter: char,
        format: &str,
        path: Option<&str>,
        force: bool,
    ) -> Result<CommandOutcome, EditorError> {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        let region = match self.md_row_in_a_fence() {
            true => None,
            false => crate::mdtable::region(|i| self.line_text(i), line),
        };
        // A file that is already a grid exports as itself — reading a `.csv`
        // and writing a `.tsv` is a conversion, and it is the same one.
        let lines = match region.as_ref() {
            Some(region) => {
                Ok::<_, (usize, usize)>(crate::mdtable::to_delimited(&self.md_lines(region), delimiter))
            }
            None if self.table.as_ref().is_some_and(|v| v.bounds == Bounds::WholeFile) => {
                let from = self.table.as_ref().map(|v| v.schema.delimiter).unwrap_or(',');
                let text = self.current_buffer().text();
                let mut out = Vec::new();
                for source in text.lines() {
                    let row: Vec<String> = crate::table::cells(source, from)
                        .into_iter()
                        // **Read it out of the file's quoting and back into
                        // the target's.** A comma inside a field needs quotes
                        // in a `.csv` and none in a `.tsv`; carrying the
                        // quotes across would hand the next program a value
                        // with quotation marks in its name.
                        .map(|span| {
                            let value = crate::table::unquote(&crate::table::cell_text(source, span));
                            crate::table::quote_for(&value, delimiter)
                        })
                        .collect();
                    out.push(row.join(&delimiter.to_string()));
                }
                Ok(out)
            }
            None => {
                self.status = say!("table.not-in-a-pipe-table");
                                return Ok(CommandOutcome::Continue);
            }
        };
        let lines = match lines {
            Ok(lines) => lines,
            Err((row, column)) => {
                self.status = say!(
                    "table.cell-holds-the-delimiter",
                    row + 1,
                    column + 1,
                    delimiter
                );
                return Ok(CommandOutcome::Continue);
            }
        };
        let target = match path {
            Some(path) => self.here_path(path),
            None => match self.current_buffer().path() {
                Some(source) => source.with_extension(format.trim().to_ascii_lowercase()),
                None => return Err(EditorError::NoFileName),
            },
        };
        if self.refuse_to_overwrite(&target, force) {
            return Ok(CommandOutcome::Continue);
        }
        // **導出的那一份跟着源文件走**（2026-10-07 定）：它帶 BOM 就寫 BOM，它是
        // CRLF 就寫 CRLF。從前這裏寫死了 `\n`、也不寫 BOM——而一份 BOM ＋ CRLF 的
        // `.csv` 導出來兩樣都沒了，Excel 打開就是亂碼。那三個字節正是
        // `tests/byte_fidelity.rs` 存在的理由之一，它的註釋寫着「the thing Excel
        // reads to decide a `.csv` is UTF-8」。
        let ending = self.current_buffer().ending();
        let mut written = match self.current_buffer().marked() {
            true => String::from("\u{feff}"),
            false => String::new(),
        };
        written.push_str(&lines.join(ending));
        written.push_str(ending);
        crate::buffer::write_file_atomically(&target, &written).map_err(EditorError::Io)?;
        self.status = say!("export.wrote", target.display());
        Ok(CommandOutcome::Continue)
    }
}

/// **多大**，給人看的那一種：`812 B`、`12.4 KB`、`3.1 MB`。
///
/// 一千零二十四進一位，小數點後一位——`:info` 那一頁上這個數是拿來估量的，不是
/// 拿來對賬的（要準的那個數在 `ls -l` 那裏）。
fn how_big(bytes: u64) -> String {
    const STEP: f64 = 1024.0;
    let names = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut which = 0;
    while size >= STEP && which + 1 < names.len() {
        size /= STEP;
        which += 1;
    }
    match which {
        0 => format!("{bytes} B"),
        _ => format!("{size:.1} {}", names[which]),
    }
}

/// **什麼時候改的**，`2026-10-08 13:02`。
///
/// 自己從 Unix 紀元算起，不拉一個日期庫進來：這一處是整個倉裏唯一要把時刻寫成
/// 字的地方（`:yume-where` 那個版本號是編譯時戳好的字串）。閏年認，閏秒不認——
/// 一份稿子的修改時間差一秒沒有人會發現，而多一條依賴是要養的。
fn when_was(when: std::time::SystemTime) -> String {
    let secs = match when.duration_since(std::time::UNIX_EPOCH) {
        Ok(it) => it.as_secs() as i64,
        Err(_) => return String::new(),
    };
    // 本地時區：拿 `localtime` 那一套要麼拉庫、要麼走 libc，所以這裏報的是 UTC
    // 偏移之前的那個數——⚠ 它是 **UTC**，不是本地時間。
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    let (hour, minute) = (rest / 3600, (rest % 3600) / 60);
    let (mut year, mut left) = (1970, days);
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let in_year = if leap { 366 } else { 365 };
        if left < in_year {
            break;
        }
        left -= in_year;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let lengths = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 0;
    while month < 12 && left >= lengths[month] {
        left -= lengths[month];
        month += 1;
    }
    format!("{year:04}-{:02}-{:02} {hour:02}:{minute:02} UTC", month + 1, left + 1)
}
