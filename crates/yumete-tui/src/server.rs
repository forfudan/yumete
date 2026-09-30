//! **Running a language server** — the process half of #53／#54 (L2,
//! 2026-09-20).
//!
//! [`yumete_core::lsp`] is the wire: pure functions over strings, every one of
//! them tested without a server. This is everything that is *not* pure — a
//! child process, two threads, and the rule about when a file is told about.
//!
//! ## The shape
//!
//! One server per language, started the first time a file of that language is
//! really opened. Each one owns:
//!
//! - **a writer thread**, because a pipe can block, and blocking the editor on
//!   a server that has stopped reading would freeze the page under the cursor;
//! - **a reader thread**, which frames what comes back and hands over
//!   [`Notice`]s;
//! - **a stderr thread**, whose only job is to keep reading. Warning: Leaving stderr
//!   unread wedges the server the moment its pipe fills — and `rust-analyzer`
//!   is chatty. This is the failure that looks like 「it worked for a minute
//!   and then stopped」.
//!
//! ## What the editor does about it
//!
//! Two calls, both cheap, both on the event loop:
//!
//! - [`Servers::follow`] after the page has settled: start what is needed,
//!   `didOpen` a file the server has not heard of, `didChange` one that moved.
//! - [`Servers::collect`] on the way round: drain what came back into
//!   [`yumete_core::editor::Editor::set_problems`].
//!
//! Warning: **Nothing here ever blocks.** `try_recv`, `try_wait`, and a channel for
//! everything that could wait. An editor that stops because another program
//! stopped is the one outcome not worth any number of diagnostics.

use std::collections::{HashMap, HashSet};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};

use yumete_core::editor::Editor;
use yumete_core::say;
use yumete_core::lsp::{self, Notice};

/// How long after a change a changed file is sent (milliseconds).
///
/// The same 300 ms the 改動條 waits (`Editor::VCS_IDLE`), and for the same
/// reason: sending on every keystroke would have `rust-analyzer` re-analysing
/// a crate per character.
///
/// Warning: **這是節流，不是防抖。** 計時從「發現正文變了」那一刻起算，按鍵不重置它
/// （全樹寫 `touched` 的只有清和設兩處）——所以連續打字是**每 300 毫秒發一次，
/// 發在詞的中間**，不是「停手 300 毫秒」。註釋從前寫的是後者
/// （2026-09-23 審出來的）。這樣更好而不是更差：補全那一半正是靠打字中間發出去
/// 的那一條 `didChange` 纔問得出東西；真正的語言服務器客戶端多半連節流都沒有，
/// 每一次改動都發。
const SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

/// How often the loop looks in while an answer is owed.
///
/// Warning: **Only while one is owed.** The event loop blocks on the terminal, so
/// something has to give it a deadline or a diagnostic that arrives while
/// nobody is typing waits for the next keypress to be drawn. `due_in` returns
/// `None` the moment nothing is outstanding, and an idle editor goes back to
/// blocking forever — the same bargain `vcs_due_in` makes.
const LOOK_IN: std::time::Duration = std::time::Duration::from_millis(120);

/// The `id` of the `initialize` request. One per server, and always this.
const HELLO: i64 = 1;

/// The `id` of the `shutdown` request, and always this.
///
/// Warning: **Not [`FIRST_ASK`].** It used to be a literal `2`, which is the id the
/// first `gd`／hover／completion of the session wears — 「two requests must
/// never wear the same one」, and this one broke it (2026-09-23 審出來的).
/// Nobody reads the answer at exit, so it never showed; the rule is the point.
const GOODBYE: i64 = 0;

/// Where the ids for everything else start.
///
/// Warning: **Not 1, and not shared with the handshake.** An answer is matched by
/// its id and nothing else, so two requests must never wear the same one —
/// and [`HELLO`] is the one id whose meaning is fixed.
const FIRST_ASK: i64 = 2;

/// One running server.
struct Server {
    /// Warning: **`None` in a test, and only there.** Everything interesting about
    /// this module is the rule 「when is a file told about」, and that rule has
    /// nothing to do with a process — so the tests hand over a pair of plain
    /// channels and read what would have gone down the pipe. A test that
    /// needed a real `rust-analyzer` would be a test nobody runs.
    child: Option<Child>,
    /// Messages out. The writer thread owns the pipe.
    to: Sender<String>,
    /// Notices in.
    from: Receiver<Notice>,
    /// Has the handshake finished? Until it has, everything else waits.
    ready: bool,
    /// What was said before the handshake finished.
    queued: Vec<String>,
    /// Which files this server has been told about.
    open: HashSet<PathBuf>,
    /// The version number `didChange` counts up.
    version: i64,
    /// **Whether a fresh round of diagnostics is still owed.**
    ///
    /// Warning: **Only diagnostics.** A request's answer is owed as long as its id
    /// is in one of the three slots below — that is what [`Server::owed`]
    /// reads. Putting both in this one flag is what made `gd` wait for the
    /// next keypress: diagnostics arrive unasked, all the time, and the first
    /// one to land after the question cleared the flag, so [`Servers::due_in`]
    /// said 「nothing outstanding」 and the loop went back to blocking while
    /// the answer sat in the channel. 2026-09-23 審出來的。
    waiting: bool,
    /// The next id to put on a request.
    next_ask: i64,
    /// **Which id was `gd`**, so its answer is told apart from every other.
    asked_where: Option<i64>,
    /// The id of the 「what comes next?」 now out, if one is (#53 ④).
    asked_next: Option<i64>,
    /// The id of the 「what is this?」 now out, if one is (#53 ③).
    ///
    /// Warning: **Its own slot, not a second use of `asked_where`.** Both questions
    /// are about the same spot and can be asked one after the other, and an
    /// answer carries only an id — one slot would make a hover answer look
    /// like a definition that had somehow lost its place.
    asked_what: Option<i64>,
}

/// Every server, and the one rule about when to talk to them.
pub struct Servers {
    /// Language name → the server answering for it.
    running: HashMap<String, Server>,
    /// Languages that could not be started, so the editor says so once.
    failed: HashSet<String>,
    /// **哪幾個語言已經說過「這臺機器上沒有」了**——一個語言只說一次。
    told_about: HashSet<String>,
    /// 每個起來了的服務器，那個可執行檔到底在哪——`:diagnostics-all` 頂上那一行要說。
    found: HashMap<String, PathBuf>,
    /// The buffer revision each open file was last sent at, so a file is not
    /// re-sent for a keystroke that changed nothing.
    sent: HashMap<PathBuf, u64>,
    /// **How many times each open file had been saved when we last said so.**
    ///
    /// A save is news the text alone does not carry: rust-analyzer answers out
    /// of two mouths, its own analysis (which follows every keystroke) and
    /// `cargo check` (which runs on `didSave` and nothing else). 2026-09-21
    /// 報的「我把錯的行刪了，錯誤信息還在」就是第二張嘴從來沒被叫醒——刪掉的
    /// 那一行的 `(rustc)` 診斷一直是上一次 check 的舊帳。
    saved: HashMap<PathBuf, u64>,
    /// **Which server each file was told to.**
    ///
    /// Warning: `sent`／`saved`／`waiting_on` 是全局的，而 `running` 是按語言分的——
    /// 少了這一格，rust-analyzer 一崩就會把 `.go` 的診斷也從頁面上抹掉，把
    /// gopls 的 `sent` 也清空（於是下一趟白發一條 `didSave`，讓它整跑一次
    /// 檢查），而 `waiting_on` 裏那個死掉的服務器的路徑永遠清不掉——「分析中」
    /// 這一整場就再也收不回去了。2026-09-23 審出來的。
    whose: HashMap<PathBuf, &'static str>,
    /// When the current buffer last changed, for the settle above.
    touched: Option<std::time::Instant>,
    /// **Which files have been told to a server but never answered about.**
    ///
    /// That gap is the cold start: `rust-analyzer` reads the whole crate
    /// before it says anything, which on a real project is ten or twenty
    /// seconds of an editor that looks broken. 2026-09-21：「在等的這段時間能
    /// 不能在狀態欄出現個提示？」
    ///
    /// Warning: **Only until the first answer**, and per file. After that a re-read
    /// is milliseconds, and a line that flickered 「分析中」 on every keystroke
    /// would be noise where a status line is the scarcest thing on the page.
    waiting_on: HashSet<PathBuf>,
    /// How long that settle is. A field rather than the constant so a test can
    /// take it to zero instead of sleeping.
    settle: std::time::Duration,
    /// What to put on the status line, once.
    pub says: Option<String>,
}

impl Default for Servers {
    fn default() -> Servers {
        Servers {
            running: HashMap::new(),
            failed: HashSet::new(),
            told_about: HashSet::new(),
            found: HashMap::new(),
            sent: HashMap::new(),
            saved: HashMap::new(),
            whose: HashMap::new(),
            touched: None,
            waiting_on: HashSet::new(),
            settle: SETTLE,
            says: None,
        }
    }
}

impl Servers {
    /// **Which program answers for this language**, or `None` for 「none」.
    ///
    /// A language may name several candidates and **the first one installed
    /// wins** — python has no single obvious answer (helix lists five), so
    /// filling one in would be guessing on the reader's behalf and making them
    /// pay for the guess every time they open a `.py`.
    ///
    /// Warning: **An empty command is 「not this language」**, which is how a reader
    /// turns off one of the ones that come filled in — so it is not the same
    /// as a missing entry.
    fn named<'a>(
        config: &'a yumete_config::Config,
        language: &str,
        from: Option<&Path>,
    ) -> Option<&'a yumete_config::Server> {
        config
            .lsp
            .get(language)?
            .iter()
            .find(|s| !s.command.is_empty() && on_the_path(&s.command, from))
    }

    /// **從哪個目錄開始往上爬**——被編輯的那個檔所在的地方。
    ///
    /// 沒有檔（還沒存盤）就從項目根起：那時服務器本來也起不來（見
    /// [`Servers::why_it_cannot_ask`] 的第三條），但別的問話還要一個答案。
    fn look_from(editor: &Editor) -> PathBuf {
        editor
            .current_buffer()
            .path()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf)
            .unwrap_or_else(|| editor.project_root())
    }

    /// The language a buffer is, as LSP spells it — `None` for prose.
    ///
    /// Warning: **Only real code.** Markdown, Typst and 文本 have language servers in
    /// the world, but this editor *is* the tool for those, and starting a
    /// second opinion about a manuscript is not what anybody asked for.
    fn language_of(editor: &Editor) -> Option<&'static str> {
        match editor.current_buffer().syntax() {
            yumete_core::syntax::Syntax::Code(language) => Some(language.name()),
            _ => None,
        }
    }

    /// Start, open, and update — everything that depends on where the cursor
    /// is. Called once a turn, after the keys have been handled.
    pub fn follow(&mut self, editor: &mut Editor, config: &yumete_config::Config) {
        let Some(language) = Self::language_of(editor) else { return };
        let Some(named) = Self::named(config, language, Some(&Self::look_from(editor))) else {
            // **打開的那一刻就說，別等人問**（2026-09-29 報的：「为什么不是在打开
            // 文件的时候就检查 LSP 并且显示消息」）。從前這裏是默默 `return`，於是
            // 讀者打開 `.py` 什麽都不知道，一直到按 `空格 k` 纔撞上。
            //
            // Warning: **每個語言只說一次**（作者定）。這是一件關於這臺機器的事實，
            // 不是一個事件——一個項目裏切五個 `.py` 就罵五遍，會把狀態欄上別的話
            // 全蓋掉。同 `lsp.cannot-start` 那一條的理由。
            if self.told_about.insert(language.to_string()) {
                self.says = Some(match config.lsp.get(language).filter(|v| !v.is_empty()) {
                    Some(wanted) => say!("lsp.not-installed", Self::the_names(wanted)),
                    None => say!("lsp.no-server-configured", language),
                });
            }
            return;
        };
        let Some(path) = editor.current_buffer().path().map(Path::to_path_buf) else {
            // An unsaved buffer has no URI, and a server answers about files.
            return;
        };
        if !self.running.contains_key(language) && !self.failed.contains(language) {
            let here = Self::look_from(editor);
            let Some(at) = found_here(&named.command, Some(&here)) else { return };
            // **用的是項目裏那一個就說一句**（2026-09-29 作者定）。Warning: 不寫目錄
            // ——狀態欄那一行很貴，完整路徑在 `:diagnostics-all` 頂上那一行。
            let mine = !std::env::var_os("PATH")
                .map(|path| {
                    std::env::split_paths(&path).any(|dir| dir.join(&named.command) == at)
                })
                .unwrap_or(false);
            match start(named, editor, &at) {
                Ok(server) => {
                    self.running.insert(language.to_string(), server);
                    // `:diagnostics-all` 頂上那一行：名字　路徑　狀態。路徑相對項目根，
                    // 同那張單子上檔名的規矩——絕對路徑九十個字符，讀不了。
                    let root = editor.project_root();
                    let short = at.strip_prefix(&root).unwrap_or(&at);
                    editor.note_the_server(Some(say!(
                        "lsp.where-it-is",
                        named.command,
                        short.display(),
                        say!("lsp.ready")
                    )));
                    self.found.insert(language.to_string(), at);
                    if mine {
                        self.says = Some(say!("lsp.from-the-project", named.command));
                    }
                }
                Err(why) => {
                    // Warning: **Said once, and never again.** A missing
                    // `rust-analyzer` is a fact about the machine, not an
                    // event, and repeating it every keystroke would bury every
                    // other thing the status line has to say.
                    self.failed.insert(language.to_string());
                    self.says = Some(say!("lsp.cannot-start", named.command, why));
                }
            }
        }
        let Some(server) = self.running.get_mut(language) else { return };
        let revision = editor.current_buffer().revision();
        let known = self.sent.get(&path).copied();
        let saves = editor.current_buffer().saves();
        if known == Some(revision) {
            // The text is as told. **A save still has to be told**, and told
            // *after* the text it saved — so it waits here, one turn behind
            // the `didChange` above, rather than racing it.
            if self.saved.get(&path).copied() != Some(saves) {
                self.saved.insert(path.clone(), saves);
                server.say(lsp::did_save(&path));
                server.waiting = true;
            }
            return;
        }
        // **Wait for the typing to stop.** `touched` is reset by every change,
        // so this fires once, 300 ms after the last one.
        match known {
            None => {}
            Some(_) => {
                let now = std::time::Instant::now();
                match self.touched {
                    Some(at) if now.duration_since(at) >= self.settle => self.touched = None,
                    Some(_) => return,
                    None => {
                        self.touched = Some(now);
                        return;
                    }
                }
            }
        }
        let text = editor.current_buffer().rope().to_string();
        server.version += 1;
        let message = match server.open.contains(&path) {
            false => {
                server.open.insert(path.clone());
                // Opened, not saved by us — so the first save the reader makes
                // is the first one that counts.
                self.saved.insert(path.clone(), saves);
                lsp::did_open(&path, language, server.version, &text)
            }
            true => lsp::did_change(&path, server.version, &text),
        };
        server.say(message);
        server.waiting = true;
        // 第一次告訴它這個檔，就開始等；答過一次之後不再說（見 `waiting_on`）。
        if known.is_none() {
            self.waiting_on.insert(path.clone());
            self.says = Some(say!("lsp.reading", named.command));
        }
        self.whose.insert(path.clone(), language);
        self.sent.insert(path, revision);
    }

    /// **A file that is not open any more is not our business any more.**
    ///
    /// Warning: 從前 `lsp::did_close` 全樹一處都没調用（2026-09-23 審出來的）。關掉
    /// 一個檔，服務器照舊分析它、照舊推它的診斷，而 `Problems` 是按路徑存
    /// 的——`:diagnostics-all` 會一直列着一個早就關掉的檔。
    ///
    /// 走在 [`Self::follow`] 之後：那一支剛把當前這個檔說出去，這一支再看還有
    /// 誰不在了。
    pub fn forget_closed_files(&mut self, editor: &mut Editor) {
        let open: HashSet<PathBuf> = editor.buffer_paths().into_iter().collect();
        let gone: Vec<PathBuf> = self
            .whose
            .keys()
            .filter(|path| !open.contains(*path))
            .cloned()
            .collect();
        for path in gone {
            if let Some(language) = self.whose.remove(&path) {
                if let Some(server) = self.running.get_mut(language) {
                    server.open.remove(&path);
                    server.say(lsp::did_close(&path));
                }
            }
            editor.forget_problems(&path);
            self.sent.remove(&path);
            self.saved.remove(&path);
            self.waiting_on.remove(&path);
        }
    }

    /// **Send the `gd` question, if the editor has one waiting** (#53 ②).
    ///
    /// Separate from [`Self::follow`] because it is a *request*: it wants an
    /// answer, and the answer has to be told from every other one the server
    /// sends. The id is remembered here; [`Self::collect`] matches on it.
    ///
    /// Warning: **The file has to have been sent first.** A server asked about a
    /// position in a document it has never been told about answers `null` —
    /// which reads exactly like 「this is written nowhere」. `follow` runs
    /// first on the same turn, so by the time this asks, `didOpen` is out.
    pub fn ask(&mut self, editor: &mut Editor, config: &yumete_config::Config) {
        let _ = config;
        let Some(language) = Self::language_of(editor) else { return };
        if !self.running.contains_key(language) {
            // 服務器没起來，就照實說——而不是讓那句問話一直掛着。
            if editor.take_definition_query().is_some() {
                editor.no_definition();
            }
            return;
        }
        // Warning: **問的是行列號，答的是服務器手上那份正文。** 打完字還沒過
        // settle（300 毫秒）時 `follow` 一個字都還沒發出去，這時候問，服務器
        // 按**上一版**正文去數第幾行第幾列——指到的是別的東西，或者乾脆說
        // 「哪兒都沒寫」。所以問題**留着不取**，下一輪正文發出去了再問，同
        // [`Self::ask_next`] 一個道理。2026-09-23 補。
        if !self.told_the_latest(editor) {
            return;
        }
        let Some((path, line, column)) = editor.take_definition_query() else { return };
        let Some(server) = self.running.get_mut(language) else { return };
        let id = server.next_ask;
        server.next_ask += 1;
        server.asked_where = Some(id);
        server.say(lsp::definition(id, &path, line, column));
    }

    /// 這個語言配了哪幾個候選，頓號隔開——找不到的時候要全說出來。
    fn the_names(wanted: &[yumete_config::Server]) -> String {
        wanted
            .iter()
            .map(|s| s.command.as_str())
            .filter(|c| !c.is_empty())
            .collect::<Vec<_>>()
            .join("、")
    }

    /// **為什麽這一問發不出去**，`None` ＝ 發得出去（2026-09-29）。
    ///
    /// 五種，一種一句話。判準就是啓動那條路上的五道閘（見 [`Servers::follow`]），
    /// 按同樣的次序問一遍——所以屏幕上說的和真正卡住的地方一定是同一處。
    fn why_it_cannot_ask(
        &self,
        editor: &Editor,
        config: &yumete_config::Config,
        language: &str,
    ) -> Option<String> {
        // 一、設置裏這個語言根本沒寫服務器。
        let Some(wanted) = config.lsp.get(language).filter(|v| !v.is_empty()) else {
            return Some(say!("lsp.no-server-configured", language));
        };
        // 二、寫了，可這臺機器上一個都找不到。
        //
        // Warning: **把找過的全列出來**（2026-09-29 報的）。python 出廠配了四個
        // 候選（`ty`／`ruff`／`pylsp`／`jedi-language-server`，抄的 helix），而
        // 從前這句話只說第一個——讀者於是只會去裝 `ty`，其實裝哪一個都行。
        if Self::named(config, language, Some(&Self::look_from(editor))).is_none() {
            return Some(say!("lsp.not-installed", Self::the_names(wanted)));
        }
        // 三、稿子還沒存盤——服務器答的是檔案，沒有路徑就沒得問。
        if editor.current_buffer().path().is_none() {
            return Some(say!("lsp.buffer-unsaved"));
        }
        // 四、試過了，起不來，此後不再試（`failed`）。
        if self.failed.contains(language) {
            return Some(say!("lsp.start-failed"));
        }
        // 五、正在起，或者剛握上手還沒說 ready。
        match self.running.get(language) {
            Some(server) if server.ready => None,
            _ => Some(say!("lsp.starting")),
        }
    }

    /// **Send the 「what is this?」 question** (`空格 k`, #53 ③).
    ///
    /// The same shape as [`Self::ask`], and for the same reasons — a request
    /// wants an answer, and only this side knows which id it sent for what.
    pub fn ask_what(&mut self, editor: &mut Editor, config: &yumete_config::Config) {
        let Some(language) = Self::language_of(editor) else { return };
        // **問不出去的話，說清楚是哪一種問不出去**（2026-09-29 報的：「这句话的
        // 意思是服务器没有开启，还是服务器开启了但是没有找到相关文档？」）。
        //
        // Warning: **從前這五種只有一句話**：`no_hover()` 的「服務器對這個沒什麽
        // 可說的」。那句話字面的意思是「它答了，說沒有」，而服務器根本沒起來的
        // 時候說的也是它——隔壁註釋還寫着「照實說」。作者原話：「现在太含混了。」
        if let Some(word) = self.why_it_cannot_ask(editor, config, language) {
            // Warning: **只有人按了鍵纔說**。跟着光標走的那一問（`:docs on`）是編
            // 輯器自己發的，每走到一個標點就罵一句不是人要的。
            if editor.take_hover_query().is_some() {
                self.says = Some(word);
            }
            return;
        }
        // Warning: **問的是行列號，答的是服務器手上那份正文。** 打完字還沒過
        // settle（300 毫秒）時 `follow` 一個字都還沒發出去，這時候問，服務器
        // 按**上一版**正文去數第幾行第幾列——指到的是別的東西，或者乾脆說
        // 「哪兒都沒寫」。所以問題**留着不取**，下一輪正文發出去了再問，同
        // [`Self::ask_next`] 一個道理。2026-09-23 補。
        if !self.told_the_latest(editor) {
            return;
        }
        // **跟着光標的那一問**（2026-09-29）：`空格 K` 開着的時候編輯器自己發，
        // 走的是同一條路——它只在光標停穩了三百毫秒之後纔交得出一個問題。
        if let Some(query) = editor.docs_owed() {
            let Some(server) = self.running.get_mut(language) else { return };
            let id = server.next_ask;
            server.next_ask += 1;
            server.asked_what = Some(id);
            server.say(lsp::hover(id, &query.0, query.1, query.2));
            return;
        }
        let Some((path, line, column)) = editor.take_hover_query() else { return };
        let Some(server) = self.running.get_mut(language) else { return };
        let id = server.next_ask;
        server.next_ask += 1;
        server.asked_what = Some(id);
        server.say(lsp::hover(id, &path, line, column));
    }

    /// **Send the 「what comes next?」 question** (`C-n` and every letter typed,
    /// #53 ④).
    ///
    /// Warning: **Not until the server has the text this is a question about.** The
    /// `didChange` that carries the letter just typed waits out the settle
    /// (300 ms), and a `completion` sent before it arrives is answered against
    /// the *previous* version of the file — which is a list of the things that
    /// could follow the word as it was one keystroke ago. So the question is
    /// **left standing** rather than taken, and goes out on the turn after the
    /// text does. This is the same ordering `didSave` needs, for the same
    /// reason, and it is the whole of what makes the automatic half work.
    pub fn ask_next(&mut self, editor: &mut Editor, config: &yumete_config::Config) {
        let _ = config;
        let Some(language) = Self::language_of(editor) else { return };
        if !self.running.contains_key(language) {
            if editor.take_completion_query().is_some() {
                editor.no_offers();
            }
            return;
        }
        if !self.told_the_latest(editor) {
            return;
        }
        let Some((path, line, column)) = editor.take_completion_query() else { return };
        let Some(server) = self.running.get_mut(language) else { return };
        let id = server.next_ask;
        server.next_ask += 1;
        server.asked_next = Some(id);
        server.say(lsp::completion(id, &path, line, column));
    }

    /// **Does the server hold the text the buffer holds?** See [`Self::ask_next`].
    fn told_the_latest(&self, editor: &Editor) -> bool {
        let Some(path) = editor.current_buffer().path() else { return false };
        self.sent.get(path) == Some(&editor.current_buffer().revision())
    }

    /// Take everything the servers have said and give it to the editor.
    ///
    /// **Never waits.** What has arrived, arrives; what has not will be here
    /// next time round.
    pub fn collect(&mut self, editor: &mut Editor) -> bool {
        let mut anything = false;
        let mut gone: Vec<String> = Vec::new();
        // 借出來給下面那個迴圈用——它同時要借 `self.running`。
        let mut answered = std::mem::take(&mut self.waiting_on);
        let mut said_so = false;
        for (language, server) in self.running.iter_mut() {
            loop {
                match server.from.try_recv() {
                    Ok(Notice::Ready) => {
                        server.ready = true;
                        server.say_now(lsp::initialized());
                        for message in std::mem::take(&mut server.queued) {
                            server.say_now(message);
                        }
                    }
                    Ok(Notice::Said { path, said }) => {
                        // 第一次回話——冷啓動結束了。
                        if answered.remove(&path) {
                            said_so = true;
                        }
                        editor.set_problems(path, said);
                        server.waiting = false;
                        anything = true;
                    }
                    // Warning: An unanswered request can stall a server for good.
                    Ok(Notice::Asked { id }) => server.say(lsp::empty_answer(id)),
                    // **The answer to `gd`** — anything else with an id is an
                    // answer nobody is waiting for any more.
                    Ok(Notice::Answer { id, places, told, offers }) => {
                        // **「接下來能打什麽」的答案**（#53 ④）。
                        if server.asked_next == Some(id) {
                            server.asked_next = None;
                            anything = true;
                            editor.show_offers(offers);
                        } else
                        // **「這是什麽」的答案**（#53 ③）。
                        if server.asked_what == Some(id) {
                            server.asked_what = None;
                            anything = true;
                            match told {
                                Some(text) => editor.show_hover(text),
                                None => editor.no_hover(),
                            }
                        } else if server.asked_where == Some(id) {
                            server.asked_where = None;
                            anything = true;
                            match places.first() {
                                // Warning: **The first one, and only the first.**
                                // A definition can have several answers (a
                                // trait and its impls), and a caret can only
                                // be in one of them; a picker over the rest is
                                // its own feature, not this one's half-done
                                // corner.
                                Some(place) => editor.go_to_definition(place),
                                None => editor.no_definition(),
                            }
                        }
                    }
                    Ok(Notice::Nothing) => {}
                    Err(TryRecvError::Empty) => break,
                    // The reader thread is gone, which means the pipe closed,
                    // which means the server did.
                    Err(TryRecvError::Disconnected) => {
                        gone.push(language.clone());
                        break;
                    }
                }
            }
            // A server that exited on its own is gone even if the channel has
            // not noticed yet.
            let ended = server.child.as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(Some(_))));
            if ended && !gone.contains(language) {
                gone.push(language.clone());
            }
        }
        self.waiting_on = answered;
        // 等完了就把那句話收走——留着它會蓋住下一句真要說的話。
        if said_so && self.waiting_on.is_empty() {
            self.says = Some(String::new());
        }
        for language in gone {
            self.lost(&language, editor);
            anything = true;
        }
        anything
    }

    /// A server died. Forget what it said and say so — **once**.
    ///
    /// Warning: **Its complaints go with it.** Leaving them on the page would show
    /// errors from a program that is no longer watching, and every one of them
    /// would stay until the session ended.
    fn lost(&mut self, language: &str, editor: &mut Editor) {
        let ran = self.running.remove(language).is_some_and(|s| s.ready);
        self.found.remove(language);
        // 它不在聽了，`:diagnostics-all` 頂上那一行也不該再說它在。
        editor.note_the_server(None);
        // **只忘這一個服務器說過的那些檔**（見 [`Servers::whose`]）。
        let its: Vec<PathBuf> = self
            .whose
            .iter()
            .filter(|(_, &lang)| lang == language)
            .map(|(path, _)| path.clone())
            .collect();
        for path in &its {
            editor.forget_problems(path);
            self.sent.remove(path);
            self.saved.remove(path);
            self.whose.remove(path);
            // 它不會再答了，所以那句「分析中」也不該再等它。
            self.waiting_on.remove(path);
        }
        // （那句「分析中」不必單獨收——下面這一句無論如何都要蓋上去。）
        self.says = Some(match ran {
            // A crash after it was working is not 「this machine has no
            // rust-analyzer」: the next file starts a fresh one, which is the
            // whole of the 「restart without taking the editor down」 rule.
            true => say!("lsp.stopped", language),
            // Warning: **One that never答上話 is written off**, or the editor spawns
            // one per turn of the loop, for ever. This machine 2026-09-20:
            // `~/.cargo/bin/rust-analyzer` is a rustup shim whose component is
            // not installed — it starts, prints one line to stderr and exits,
            // so `spawn()` succeeds and `try_wait` immediately says it is
            // gone. That looked exactly like a working server that had just
            // stopped, and the retry made it a fork bomb in slow motion.
            false => {
                self.failed.insert(language.to_string());
                say!("lsp.never-started", language)
            }
        });
    }
}

impl Server {
    /// **Is this server owing anything?** — diagnostics, or an answer to one
    /// of the three questions. See [`Server::waiting`].
    fn owed(&self) -> bool {
        self.waiting
            || !self.ready
            || self.asked_where.is_some()
            || self.asked_what.is_some()
            || self.asked_next.is_some()
    }
}

impl Servers {
    /// How long the event loop may sleep, or `None` to block.
    pub fn due_in(&self) -> Option<std::time::Duration> {
        let waiting = self.running.values().any(Server::owed);
        let settling = self.touched.is_some();
        (waiting || settling).then_some(LOOK_IN)
    }

    /// Say goodbye to every server. Called when the editor is leaving.
    pub fn stop(&mut self) {
        for server in self.running.values_mut() {
            server.say_now(lsp::shutdown(GOODBYE));
            server.say_now(lsp::exit());
            // Warning: **這兩句多半來不及出門。** `say_now` 只是塞進 channel，而下面
            // 立刻就 `kill`——中間沒有任何同步，writer 綫程搶不過。留着它們是
            // 因為一條走得出去的路比沒有好（關得慢的終端就走得出去），但**不要
            // 讀成「先禮後兵」**：實際發生的幾乎一律是兵。2026-09-23 記。
            //
            // A server that will not go is killed: this runs while the
            // terminal is being handed back, and a lingering child would hold
            // the pipe open behind it.
            if let Some(child) = server.child.as_mut() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
        self.running.clear();
    }
}

impl Server {
    /// Say something, or hold it until the handshake is done.
    fn say(&mut self, message: String) {
        match self.ready {
            true => self.say_now(message),
            false => self.queued.push(message),
        }
    }

    /// Say something now, handshake or not — `initialized` itself, and the
    /// goodbye.
    fn say_now(&self, message: String) {
        trace(">>", &message);
        // A closed channel is a dead server, which `collect` will notice on
        // its own round; there is nothing useful to do here.
        let _ = self.to.send(message);
    }
}

/// **Is this program on the machine?**
///
/// Asked before spawning rather than after, because a language with several
/// candidates has to skip the ones that are not installed — and a failed
/// `spawn` is indistinguishable from a program that started and died, which is
/// a different thing with a different answer ([`Servers::lost`]).
///
/// Warning: An absolute path is asked about directly; anything else is looked for
/// **in the project first**, then on `PATH` — see [`found_here`].
fn on_the_path(command: &str, from: Option<&Path>) -> bool {
    found_here(command, from).is_some()
}

/// **項目自己帶的那幾個 `bin`**，按這個次序看（2026-09-29）。
///
/// 一個包管理器裝的工具躺在項目裏，不在 `PATH` 上——`pixi add python-lsp-server`
/// 之後 `pylsp` 只有 `pixi run` 進得去。作者原話：「我们很多时候会使用包管理且装
/// 在项目文件夹中。比如 pixi uv 还有 .env 这种文件夹。都值得搜索。」
const BINS: &[&str] = &[".venv/bin", "venv/bin", "node_modules/.bin"];

/// 這幾個底下還隔着一層（環境名／版本），所以要展開一級。
const NESTED: &[&str] = &[".pixi/envs", ".direnv"];

/// **這個命令在哪**——項目裏找得到就回它的絕對路徑，否則回 `PATH` 上那一個。
///
/// 從 `from`（被編輯的那個檔所在的目錄）起一級一級往上，每一級看 [`BINS`] 與
/// [`NESTED`]，都沒有纔看 `PATH`。
///
/// Warning: **爬到最外層那個 `.git` 為止**，不是碰到第一個就停（2026-09-29 當天
/// 改的）。第一版停在第一個，而作者的樹是 `yuhao-ime/yumete/scripts/x.py`——
/// `yumete` 自己是一個倉，pixi 的環境在**上一級**的 `yuhao-ime`，於是那一停正好
/// 把答案關在門外。嵌套的倉全穿過去，停在最外面那一層倉上：再往上就真是別人的
/// 項目了。
///
/// Warning: **helix 不做這件事。** 它只 `which`（`helix-lsp/src/client.rs:228`），
/// 文檔明說「Binaries must be in `$PATH`」；vim 根本沒有內建 LSP。這一條是我們自
/// 己加的，理由是包管理器把工具裝在項目裏已經是常態。代價是起服務器那一次多幾十
/// 個 `stat`——只在啓動那一次，不是每一幀。
///
/// Warning: **項目裏的優先於 `PATH`**（2026-09-29 作者定）。項目裏那一個跟這個項
/// 目的解釋器、依賴對得上，全局那一個不一定。
fn found_here(command: &str, from: Option<&Path>) -> Option<PathBuf> {
    let named = Path::new(command);
    if named.is_absolute() || command.contains(std::path::MAIN_SEPARATOR) {
        return named.is_file().then(|| named.to_path_buf());
    }
    // 先量出邊界：最外層那個帶 `.git` 的祖先。一個都沒有就只爬到 `from` 自己。
    let outermost = from
        .into_iter()
        .flat_map(|d| d.ancestors())
        .filter(|step| step.join(".git").exists())
        .last();
    for step in from.into_iter().flat_map(|d| d.ancestors()) {
        for bin in BINS {
            let found = step.join(bin).join(command);
            if found.is_file() {
                return Some(found);
            }
        }
        for under in NESTED {
            let Ok(entries) = std::fs::read_dir(step.join(under)) else { continue };
            // 同一個 `.pixi/envs` 底下可能有幾個環境，次序要穩，不然兩次啓動
            // 挑到不同的那一個。
            let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
            names.sort();
            for env in names {
                let found = env.join("bin").join(command);
                if found.is_file() {
                    return Some(found);
                }
            }
        }
        if Some(step) == outermost {
            break;
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|dir| dir.join(command)).find(|p| p.is_file())
}

/// Start one server and get its threads going.
fn start(named: &yumete_config::Server, editor: &Editor, at: &Path) -> std::io::Result<Server> {
    // Warning: **起的是找到的那一個絕對路徑**，不是配置裏那個名字——不然 `Command`
    // 自己又去問一遍 `PATH`，項目裏那一個白找了（2026-09-29）。
    let mut child = Command::new(at)
        .args(&named.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Warning: **The project root, not the editor's cwd.** A server asked to
        // analyse a crate from somewhere else finds no `Cargo.toml` and
        // answers about nothing at all, silently.
        .current_dir(editor.project_root())
        .spawn()?;

    let (to, outgoing) = std::sync::mpsc::channel::<String>();
    let (incoming, from) = std::sync::mpsc::channel::<Notice>();

    // The writer: one pipe, one owner.
    let mut stdin = child.stdin.take().expect("piped");
    std::thread::spawn(move || {
        for message in outgoing {
            if stdin.write_all(&lsp::frame(&message)).is_err() || stdin.flush().is_err() {
                break;
            }
        }
    });

    // The reader: bytes in, whole messages out.
    let stdout = child.stdout.take().expect("piped");
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut buf: Vec<u8> = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
            }
            while let Some(message) = lsp::take_frame(&mut buf) {
                trace("<<", &message);
                if incoming.send(lsp::read(&message, HELLO)).is_err() {
                    return;
                }
            }
        }
    });

    // Warning: **stderr must be read or the server wedges** when its pipe fills.
    // Nothing is done with it: a server's log is its own business, and the one
    // thing that matters is that somebody is emptying the bucket.
    if let Some(mut stderr) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut sink = [0u8; 4096];
            while matches!(stderr.read(&mut sink), Ok(n) if n > 0) {}
        });
    }

    let server = Server {
        child: Some(child),
        to,
        from,
        ready: false,
        queued: Vec::new(),
        open: HashSet::new(),
        version: 0,
        waiting: true,
        next_ask: FIRST_ASK,
        asked_where: None,
        asked_what: None,
        asked_next: None,
    };
    server.say_now(lsp::initialize(HELLO, &editor.project_root()));
    Ok(server)
}


/// **The whole conversation, when `YUMETE_LSP_TRACE` names a file.**
///
/// A language server bug looks exactly like a bug here, and the only thing
/// that tells them apart is the bytes that crossed. Off unless asked for: the
/// file grows by the size of the document on every keystroke.
pub(crate) fn trace(way: &str, message: &str) {
    let Some(path) = std::env::var_os("YUMETE_LSP_TRACE") else { return };
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let short: String = message.chars().take(400).collect();
        let _ = writeln!(file, "{way} {short}");
    }
}

// Warning: **測試模組一律擺在檔尾。** `yumete-core/tests/messages.rs` 那張「每個標籤都
// 有條目」的網把源碼切在**第一個**頂格的 `#[cfg(test)]\nmod ` 處——擺在檔案中間，
// 它後面的生產代碼就整段從網裏消失，於是那裏加一則文案，面板上直接印標籤而測試
// 全綠。這一支從前擺在中間，後面壓着 15 行（2026-09-24 審出來的）。
#[cfg(test)]
mod tests {
    use super::*;
    use yumete_core::problem::{Problem, Severity};

    impl Servers {
        /// A server with no process behind it: a channel each way, and the
        /// handshake already done.
        ///
        /// What this buys is the ability to test **the rule** — one `didOpen`
        /// per file, a `didChange` only after the typing stops, a dead server
        /// taking its complaints with it — without a `rust-analyzer` on the
        /// machine, and in no time at all.
        fn pretend(language: &str) -> (Servers, Receiver<String>, Sender<Notice>) {
            let (to, heard) = std::sync::mpsc::channel::<String>();
            let (tell, from) = std::sync::mpsc::channel::<Notice>();
            let mut servers = Servers { settle: std::time::Duration::ZERO, ..Default::default() };
            servers.running.insert(
                language.to_string(),
                Server {
                    child: None,
                    to,
                    from,
                    ready: true,
                    queued: Vec::new(),
                    open: HashSet::new(),
                    version: 0,
                    waiting: false,
                    next_ask: FIRST_ASK,
                    asked_where: None,
                    asked_what: None,
                    asked_next: None,
                },
            );
            (servers, heard, tell)
        }
    }

    /// A real file on disk, opened — a server answers about files, and an
    /// unnamed buffer has no URI to answer about.
    fn editor_on(name: &str, text: &str) -> (Editor, PathBuf) {
        let dir = std::env::temp_dir().join(format!("yumete-lsp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let mut editor = Editor::new();
        editor.open_file(&path).unwrap();
        (editor, path)
    }

    fn method(message: &str) -> String {
        let value: serde_json::Value = serde_json::from_str(message).expect(message);
        value["method"].as_str().unwrap_or_default().to_string()
    }

    #[test]
    fn a_file_is_opened_once_and_changed_after_that() {
        let config = yumete_config::Config { lsp: yumete_config::factory_servers(), ..Default::default() };
        let (mut editor, _path) = editor_on("a.rs", "fn main() {}\n");
        let (mut servers, heard, _tell) = Servers::pretend("rust");

        servers.follow(&mut editor, &config);
        assert_eq!(method(&heard.try_recv().unwrap()), "textDocument/didOpen");
        // Warning: **Nothing changed, so nothing is said.** Otherwise every turn of
        // the event loop — every cursor move — would re-send the file.
        servers.follow(&mut editor, &config);
        assert!(heard.try_recv().is_err(), "一個字沒改就不再說");

        editor.on_key(yumete_core::input::Key::Char('i'));
        editor.on_key(yumete_core::input::Key::Char('x'));
        // The first pass after a change starts the settle; the second sends.
        servers.follow(&mut editor, &config);
        servers.follow(&mut editor, &config);
        assert_eq!(method(&heard.try_recv().unwrap()), "textDocument/didChange");
    }

    /// **問不出去的五種，一種一句話**（2026-09-29 報的：「这句话的意思是服务器
    /// 没有开启，还是服务器开启了但是没有找到相关文档？」）。
    ///
    /// Warning: **從前這五種只有一句話。** `no_hover()` 的「服務器對這個沒什麽可
    /// 說的」字面上是「它答了，說沒有」，而服務器根本沒起來的時候說的也是它。
    #[test]
    fn a_question_that_cannot_be_asked_says_which_of_the_five_it_is() {
        let none = yumete_config::Config::default();
        let stocked =
            yumete_config::Config { lsp: yumete_config::factory_servers(), ..Default::default() };
        let (mut editor, _path) = editor_on("c.rs", "fn main() {}\n");

        // 一、設置裏這個語言根本沒寫服務器。
        let bare = Servers::default();
        assert_eq!(
            bare.why_it_cannot_ask(&editor, &none, "rust"),
            Some(say!("lsp.no-server-configured", "rust")),
        );

        // 二、寫了，可這臺機器上一個都找不到。說的是它找的那個名字。
        let missing = yumete_config::Config {
            lsp: std::iter::once((
                "rust".to_string(),
                vec![yumete_config::Server {
                    command: "no-such-analyzer".into(),
                    ..Default::default()
                }],
            ))
            .collect(),
            ..Default::default()
        };
        assert_eq!(
            bare.why_it_cannot_ask(&editor, &missing, "rust"),
            Some(say!("lsp.not-installed", "no-such-analyzer")),
        );

        // 三、稿子還沒存盤——服務器答的是檔案。
        let (fresh, _heard, _tell) = Servers::pretend("rust");
        let blank = Editor::new();
        assert_eq!(
            fresh.why_it_cannot_ask(&blank, &stocked, "rust"),
            Some(say!("lsp.buffer-unsaved")),
        );

        // 四、試過了，起不來，此後不再試。
        let mut written_off = Servers::default();
        written_off.failed.insert("rust".to_string());
        if yumete_config::factory_servers().contains_key("rust") {
            let told = written_off.why_it_cannot_ask(&editor, &stocked, "rust");
            // 這臺機器上那個命令可能真的不在，那時二先答——兩句都對，不是這一條要釘的。
            assert!(
                told == Some(say!("lsp.start-failed")) || matches!(&told, Some(t) if t.contains("找不到")),
                "{told:?}"
            );
        }

        // 五、起來了、也 ready 了：問得出去，不說話。
        let (ready, _heard, _tell) = Servers::pretend("rust");
        assert_eq!(ready.why_it_cannot_ask(&editor, &stocked, "rust"), None, "問得出去");
    }

    /// **項目自己帶的那個可執行檔找得到，而且贏過 `PATH`**（2026-09-29 定）。
    ///
    /// 作者原話：「我们很多时候会使用包管理且装在项目文件夹中。比如 pixi uv 还有
    /// .env 这种文件夹。都值得搜索。而且我们应该从文件所在或者项目所在的位置搜索，
    /// 没有就想上提级直到出现合适的。不行就用 PATH 的。」
    ///
    /// Warning: **helix 不做這件事**（只 `which`，`helix-lsp/src/client.rs:228`），
    /// vim 沒有內建 LSP。這一條是我們自己加的。
    #[test]
    fn a_server_that_lives_in_the_project_is_found_before_the_one_on_the_path() {
        let dir = std::env::temp_dir().join(format!("yumete-bins-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // 一棵假的項目樹：根上有 `.git` 和 pixi 的環境，稿子埋在兩層底下。
        let deep = dir.join("crates/one/src");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::create_dir_all(dir.join(".git")).unwrap();
        let bin = dir.join(".pixi/envs/default/bin");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join("pylsp"), "#!/bin/sh\n").unwrap();

        // 從稿子那一層起往上爬，爬到根上找着。
        let found = found_here("pylsp", Some(&deep)).expect("項目裏那一個");
        assert_eq!(found, bin.join("pylsp"), "用的是項目裏那一個");

        // `.venv/bin` 也認，而且它排在 `.pixi` 前面（同一級的次序寫死在 `BINS`）。
        let venv = dir.join("crates/one/.venv/bin");
        std::fs::create_dir_all(&venv).unwrap();
        std::fs::write(venv.join("pylsp"), "#!/bin/sh\n").unwrap();
        assert_eq!(
            found_here("pylsp", Some(&deep)),
            Some(venv.join("pylsp")),
            "更近的那一級先答",
        );

        // Warning: **嵌套的倉要穿過去，停在最外那一層**（2026-09-29 當天改的）。
        // 作者的樹是 `yuhao-ime/yumete/scripts/x.py`：`yumete` 自己是一個倉，而
        // pixi 的環境在上一級的 `yuhao-ime`。第一版停在第一個 `.git` 上，正好把
        // 答案關在門外。
        let inner = dir.join("crates/one");
        std::fs::create_dir_all(inner.join(".git")).unwrap();
        std::fs::remove_dir_all(&venv).unwrap();
        assert_eq!(
            found_here("pylsp", Some(&deep)),
            Some(bin.join("pylsp")),
            "裏面那一層倉不擋路，外面那個 pixi 找得到",
        );

        // 可再往上就是別人的項目了：最外那一層之外的不許找。
        let above = dir.parent().unwrap().join(format!("yumete-above-{}", std::process::id()));
        std::fs::create_dir_all(above.join(".venv/bin")).unwrap();
        std::fs::write(above.join(".venv/bin/lonely-9x"), "#!/bin/sh\n").unwrap();
        assert_eq!(found_here("lonely-9x", Some(&deep)), None, "爬不出最外那一層倉");

        // 找不到就回 `PATH` 上那一個：`sh` 哪臺機器都有。
        assert!(found_here("sh", Some(&deep)).is_some(), "退回 PATH");

        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&above);
    }

    /// **打開程序文件的那一刻就說沒有，而且每個語言只說一次**（2026-09-29 報的：
    /// 「为什么不是在打开文件的时候就检查 LSP 并且显示消息」）。
    #[test]
    fn opening_a_file_says_once_that_this_machine_has_no_server_for_it() {
        // 出廠給 python 配了四個候選，而這臺跑測試的機器上多半一個都沒有。
        let config =
            yumete_config::Config { lsp: yumete_config::factory_servers(), ..Default::default() };
        let (mut editor, _path) = editor_on("d.py", "import decimal\n");
        let mut servers = Servers::default();
        servers.follow(&mut editor, &config);
        let said = servers.says.take();
        let Some(said) = said else {
            // 這臺機器上真的裝了一個：那就該起得來，也就沒話說。
            assert!(servers.running.contains_key("python"), "沒說話就得是起來了");
            return;
        };
        // Warning: **四個全列出來**，不是只說第一個——讀者於是知道裝哪一個都行。
        for command in ["ty", "ruff", "pylsp", "jedi-language-server"] {
            assert!(said.contains(command), "要列全：{said:?} 少了 {command}");
        }

        // 每個語言只說一次：再開一份 `.py` 一個字都不說。
        let (mut again, _path) = editor_on("e.py", "import json\n");
        servers.follow(&mut again, &config);
        assert!(servers.says.is_none(), "說過就不再說：{:?}", servers.says);
    }

    /// Warning: **Prose has no language server here.** Markdown has one in the
    /// world; this editor *is* the tool for a manuscript, and a second opinion
    /// about a chapter is not what anybody asked for.
    #[test]
    fn a_manuscript_starts_nothing() {
        let config = yumete_config::Config { lsp: yumete_config::factory_servers(), ..Default::default() };
        let (mut editor, _path) = editor_on("第一章.md", "那年冬天。\n");
        let mut servers = Servers::default();
        servers.follow(&mut editor, &config);
        assert!(servers.running.is_empty(), "散文不起服務器");
        assert!(servers.says.is_none(), "也不說任何話");
    }

    #[test]
    fn what_a_server_says_lands_on_the_page() {
        let config = yumete_config::Config { lsp: yumete_config::factory_servers(), ..Default::default() };
        let (mut editor, path) = editor_on("b.rs", "fn main() {}\n");
        let (mut servers, _heard, tell) = Servers::pretend("rust");
        servers.follow(&mut editor, &config);

        tell.send(Notice::Said {
            path: path.clone(),
            said: vec![Problem {
                line: 0,
                utf16_column: 3,
                severity: Severity::Error,
                message: "說不通".into(),
                source: Some("rustc".into()),
            }],
        })
        .unwrap();
        assert!(servers.collect(&mut editor), "有東西來了");
        assert_eq!(editor.problem_on_line(0), Some(Severity::Error));
        assert_eq!(editor.problem_count(), 1);

        // Warning: **A server that dies takes its complaints with it.** Leaving them
        // would show errors from a program that is no longer looking, for as
        // long as the session lasted.
        drop(tell);
        assert!(servers.collect(&mut editor), "死了也是一件事");
        assert_eq!(editor.problem_count(), 0, "話跟着走");
        assert!(servers.says.is_some(), "而且說一聲");
        // …and one that **had been working** is not written off: the next file
        // starts a fresh one, which is the whole of 「崩了要能自己重起」.
        assert!(servers.failed.is_empty(), "崩了不等於這臺機器沒有它");
    }

    #[test]
    fn a_request_from_the_server_is_answered_rather_than_left_hanging() {
        let (mut editor, _path) = editor_on("c.rs", "fn main() {}\n");
        let (mut servers, heard, tell) = Servers::pretend("rust");
        tell.send(Notice::Asked { id: 7 }).unwrap();
        servers.collect(&mut editor);
        let answer: serde_json::Value =
            serde_json::from_str(&heard.try_recv().expect("答了")).unwrap();
        assert_eq!(answer["id"], 7, "Warning: 不答，服務器可能就在那兒等着");
    }

    /// A reader turning one of the built-in servers off.
    #[test]
    fn an_empty_command_means_no_server_for_that_language() {
        let mut lsp = yumete_config::factory_servers();
        lsp.insert("rust".into(), vec![yumete_config::Server::default()]);
        let config = yumete_config::Config { lsp, ..Default::default() };
        let (mut editor, _path) = editor_on("d.rs", "fn main() {}\n");
        let mut servers = Servers::default();
        servers.follow(&mut editor, &config);
        assert!(servers.running.is_empty(), "關掉了就不起");
    }

    /// **一串候選，用 PATH 上第一個裝了的**（2026-09-21）。
    ///
    /// Warning: **一串名字表達不了**：helix 給 python 列的五個各有各的參數
    /// （`ruff server`、`ty server`，而 `jedi-language-server` 不帶參數），所以
    /// 這是一串**表**，不是一串字符串。
    #[test]
    fn the_first_candidate_that_is_installed_is_the_one_that_answers() {
        let here = std::env::current_exe().expect("這個測試自己");
        let me = here.to_string_lossy().into_owned();
        let one = |command: &str| yumete_config::Server {
            command: command.to_string(),
            args: Vec::new(),
        };
        let config = yumete_config::Config {
            lsp: std::collections::HashMap::from([(
                "rust".to_string(),
                // 前兩個機器上没有，第三個是這個測試自己的執行檔——一定在。
                vec![one("nothing-called-this-exists"), one("nor-this-one"), one(&me)],
            )]),
            ..Default::default()
        };
        assert_eq!(
            Servers::named(&config, "rust", None).map(|s| s.command.as_str()),
            Some(me.as_str()),
            "跳過没裝的，停在第一個裝了的"
        );

        // 一個都没裝，就當這種語言没有服務器——而不是硬起一個起不來的。
        let none = yumete_config::Config {
            lsp: std::collections::HashMap::from([(
                "rust".to_string(),
                vec![one("nothing-called-this-exists")],
            )]),
            ..Default::default()
        };
        assert!(Servers::named(&none, "rust", None).is_none());
    }

    /// Warning: **One that never got up is written off** — 2026-09-20, found against
    /// a real `rust-analyzer`: `~/.cargo/bin/rust-analyzer` on this machine is
    /// a rustup shim whose component is not installed, so it starts, prints
    /// one line to stderr and exits. `spawn()` succeeds, `try_wait` says it is
    /// gone, and「a working server that has just stopped」is the wrong reading:
    /// retrying made one process per turn of the event loop.
    #[test]
    fn a_server_that_dies_before_it_ever_answers_is_not_started_again() {
        let (mut editor, _path) = editor_on("f.rs", "fn main() {}\n");
        let (mut servers, _heard, tell) = Servers::pretend("rust");
        servers.running.get_mut("rust").unwrap().ready = false;
        drop(tell);
        servers.collect(&mut editor);
        assert!(servers.failed.contains("rust"), "不再試了");
        assert!(servers.says.is_some(), "說一聲為什麼");

        // …and a later `follow` really does not start another one.
        let config = yumete_config::Config { lsp: yumete_config::factory_servers(), ..Default::default() };
        servers.follow(&mut editor, &config);
        assert!(servers.running.is_empty());
    }

    /// The loop blocks on the terminal, so something has to ask it to look in
    /// — but **only** while an answer is owed.
    #[test]
    fn an_idle_editor_gives_the_loop_no_deadline_at_all() {
        let (mut servers, _heard, tell) = Servers::pretend("rust");
        assert_eq!(servers.due_in(), None, "閒着就一直等，不燒電");
        servers.running.get_mut("rust").unwrap().waiting = true;
        assert_eq!(servers.due_in(), Some(LOOK_IN));
        // The answer comes, and the deadline goes with it.
        let (mut editor, path) = editor_on("e.rs", "fn main() {}\n");
        tell.send(Notice::Said { path, said: Vec::new() }).unwrap();
        servers.collect(&mut editor);
        assert_eq!(servers.due_in(), None);
    }

    /// 2026-09-23 審出來的：診斷是**不請自來**的，一秒好幾條。從前它落地就
    /// 把「還欠着」那一格抹掉，於是 `gd` 的答案還在管子裏，事件迴圈已經回去
    /// 阻塞在鍵盤上了——答案要等下一次按鍵纔畫出來。
    #[test]
    fn a_question_still_out_keeps_the_loop_awake_when_a_diagnostic_lands() {
        let (mut servers, _heard, tell) = Servers::pretend("rust");
        let (mut editor, path) = editor_on("e.rs", "fn main() {}\n");
        servers.running.get_mut("rust").unwrap().asked_where = Some(FIRST_ASK);
        tell.send(Notice::Said { path, said: Vec::new() }).unwrap();
        servers.collect(&mut editor);
        assert_eq!(servers.due_in(), Some(LOOK_IN), "問出去的還沒答，就不許睡死");
    }
}
