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
//! - **a stderr thread**, whose only job is to keep reading. ⚠️ Leaving stderr
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
//! ⚠️ **Nothing here ever blocks.** `try_recv`, `try_wait`, and a channel for
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

/// How long after the last keystroke a changed file is sent (milliseconds).
///
/// The same 300 ms the 改動條 waits (`Editor::VCS_IDLE`), and for the same
/// reason: any key restarts the wait, so this is 「停手 300 毫秒」 rather than a
/// clock of its own. Sending on every keystroke would have `rust-analyzer`
/// re-analysing a crate per character.
const SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

/// How often the loop looks in while an answer is owed.
///
/// ⚠️ **Only while one is owed.** The event loop blocks on the terminal, so
/// something has to give it a deadline or a diagnostic that arrives while
/// nobody is typing waits for the next keypress to be drawn. `due_in` returns
/// `None` the moment nothing is outstanding, and an idle editor goes back to
/// blocking forever — the same bargain `vcs_due_in` makes.
const LOOK_IN: std::time::Duration = std::time::Duration::from_millis(120);

/// The `id` of the `initialize` request. One per server, and always this.
const HELLO: i64 = 1;

/// Where the ids for everything else start.
///
/// ⚠️ **Not 1, and not shared with the handshake.** An answer is matched by
/// its id and nothing else, so two requests must never wear the same one —
/// and [`HELLO`] is the one id whose meaning is fixed.
const FIRST_ASK: i64 = 2;

/// One running server.
struct Server {
    /// ⚠️ **`None` in a test, and only there.** Everything interesting about
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
    /// ⚠️ **Only diagnostics.** A request's answer is owed as long as its id
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
    /// ⚠️ **Its own slot, not a second use of `asked_where`.** Both questions
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
    /// When the current buffer last changed, for the settle above.
    touched: Option<std::time::Instant>,
    /// **Which files have been told to a server but never answered about.**
    ///
    /// That gap is the cold start: `rust-analyzer` reads the whole crate
    /// before it says anything, which on a real project is ten or twenty
    /// seconds of an editor that looks broken. 2026-09-21：「在等的這段時間能
    /// 不能在狀態欄出現個提示？」
    ///
    /// ⚠️ **Only until the first answer**, and per file. After that a re-read
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
            sent: HashMap::new(),
            saved: HashMap::new(),
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
    /// ⚠️ **An empty command is 「not this language」**, which is how a reader
    /// turns off one of the ones that come filled in — so it is not the same
    /// as a missing entry.
    fn named<'a>(
        config: &'a yumete_config::Config,
        language: &str,
    ) -> Option<&'a yumete_config::Server> {
        config
            .lsp
            .get(language)?
            .iter()
            .find(|s| !s.command.is_empty() && on_the_path(&s.command))
    }

    /// The language a buffer is, as LSP spells it — `None` for prose.
    ///
    /// ⚠️ **Only real code.** Markdown, Typst and 文本 have language servers in
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
    pub fn follow(&mut self, editor: &Editor, config: &yumete_config::Config) {
        let Some(language) = Self::language_of(editor) else { return };
        let Some(named) = Self::named(config, language) else { return };
        let Some(path) = editor.current_buffer().path().map(Path::to_path_buf) else {
            // An unsaved buffer has no URI, and a server answers about files.
            return;
        };
        if !self.running.contains_key(language) && !self.failed.contains(language) {
            match start(named, editor) {
                Ok(server) => {
                    self.running.insert(language.to_string(), server);
                }
                Err(why) => {
                    // ⚠️ **Said once, and never again.** A missing
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
        self.sent.insert(path, revision);
    }

    /// **Send the `gd` question, if the editor has one waiting** (#53 ②).
    ///
    /// Separate from [`Self::follow`] because it is a *request*: it wants an
    /// answer, and the answer has to be told from every other one the server
    /// sends. The id is remembered here; [`Self::collect`] matches on it.
    ///
    /// ⚠️ **The file has to have been sent first.** A server asked about a
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
        // ⚠️ **問的是行列號，答的是服務器手上那份正文。** 打完字還沒過
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

    /// **Send the 「what is this?」 question** (`空格 k`, #53 ③).
    ///
    /// The same shape as [`Self::ask`], and for the same reasons — a request
    /// wants an answer, and only this side knows which id it sent for what.
    pub fn ask_what(&mut self, editor: &mut Editor, config: &yumete_config::Config) {
        let _ = config;
        let Some(language) = Self::language_of(editor) else { return };
        if !self.running.contains_key(language) {
            // 服務器没起來，就照實說——而不是讓那句問話一直掛着。
            if editor.take_hover_query().is_some() {
                editor.no_hover();
            }
            return;
        }
        // ⚠️ **問的是行列號，答的是服務器手上那份正文。** 打完字還沒過
        // settle（300 毫秒）時 `follow` 一個字都還沒發出去，這時候問，服務器
        // 按**上一版**正文去數第幾行第幾列——指到的是別的東西，或者乾脆說
        // 「哪兒都沒寫」。所以問題**留着不取**，下一輪正文發出去了再問，同
        // [`Self::ask_next`] 一個道理。2026-09-23 補。
        if !self.told_the_latest(editor) {
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
    /// ⚠️ **Not until the server has the text this is a question about.** The
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
                    // ⚠️ An unanswered request can stall a server for good.
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
                                // ⚠️ **The first one, and only the first.**
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
    /// ⚠️ **Its complaints go with it.** Leaving them on the page would show
    /// errors from a program that is no longer watching, and every one of them
    /// would stay until the session ended.
    fn lost(&mut self, language: &str, editor: &mut Editor) {
        let ran = self.running.remove(language).is_some_and(|s| s.ready);
        for path in self.sent.keys().cloned().collect::<Vec<_>>() {
            editor.forget_problems(&path);
        }
        self.sent.clear();
        self.saved.clear();
        self.says = Some(match ran {
            // A crash after it was working is not 「this machine has no
            // rust-analyzer」: the next file starts a fresh one, which is the
            // whole of the 「restart without taking the editor down」 rule.
            true => say!("lsp.stopped", language),
            // ⚠️ **One that never答上話 is written off**, or the editor spawns
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
            server.say_now(lsp::shutdown(2));
            server.say_now(lsp::exit());
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
/// ⚠️ An absolute path is asked about directly; anything else is looked for on
/// `PATH`, the way a shell would.
fn on_the_path(command: &str) -> bool {
    let named = Path::new(command);
    if named.is_absolute() || command.contains(std::path::MAIN_SEPARATOR) {
        return named.is_file();
    }
    let Some(path) = std::env::var_os("PATH") else { return false };
    std::env::split_paths(&path).any(|dir| dir.join(command).is_file())
}

/// Start one server and get its threads going.
fn start(named: &yumete_config::Server, editor: &Editor) -> std::io::Result<Server> {
    let mut child = Command::new(&named.command)
        .args(&named.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // ⚠️ **The project root, not the editor's cwd.** A server asked to
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

    // ⚠️ **stderr must be read or the server wedges** when its pipe fills.
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

        servers.follow(&editor, &config);
        assert_eq!(method(&heard.try_recv().unwrap()), "textDocument/didOpen");
        // ⚠️ **Nothing changed, so nothing is said.** Otherwise every turn of
        // the event loop — every cursor move — would re-send the file.
        servers.follow(&editor, &config);
        assert!(heard.try_recv().is_err(), "一個字沒改就不再說");

        editor.on_key(yumete_core::input::Key::Char('i'));
        editor.on_key(yumete_core::input::Key::Char('x'));
        // The first pass after a change starts the settle; the second sends.
        servers.follow(&editor, &config);
        servers.follow(&editor, &config);
        assert_eq!(method(&heard.try_recv().unwrap()), "textDocument/didChange");
    }

    /// ⚠️ **Prose has no language server here.** Markdown has one in the
    /// world; this editor *is* the tool for a manuscript, and a second opinion
    /// about a chapter is not what anybody asked for.
    #[test]
    fn a_manuscript_starts_nothing() {
        let config = yumete_config::Config { lsp: yumete_config::factory_servers(), ..Default::default() };
        let (editor, _path) = editor_on("第一章.md", "那年冬天。\n");
        let mut servers = Servers::default();
        servers.follow(&editor, &config);
        assert!(servers.running.is_empty(), "散文不起服務器");
        assert!(servers.says.is_none(), "也不說任何話");
    }

    #[test]
    fn what_a_server_says_lands_on_the_page() {
        let config = yumete_config::Config { lsp: yumete_config::factory_servers(), ..Default::default() };
        let (mut editor, path) = editor_on("b.rs", "fn main() {}\n");
        let (mut servers, _heard, tell) = Servers::pretend("rust");
        servers.follow(&editor, &config);

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

        // ⚠️ **A server that dies takes its complaints with it.** Leaving them
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
        assert_eq!(answer["id"], 7, "⚠️ 不答，服務器可能就在那兒等着");
    }

    /// A reader turning one of the built-in servers off.
    #[test]
    fn an_empty_command_means_no_server_for_that_language() {
        let mut lsp = yumete_config::factory_servers();
        lsp.insert("rust".into(), vec![yumete_config::Server::default()]);
        let config = yumete_config::Config { lsp, ..Default::default() };
        let (editor, _path) = editor_on("d.rs", "fn main() {}\n");
        let mut servers = Servers::default();
        servers.follow(&editor, &config);
        assert!(servers.running.is_empty(), "關掉了就不起");
    }

    /// **一串候選，用 PATH 上第一個裝了的**（2026-09-21）。
    ///
    /// ⚠️ **一串名字表達不了**：helix 給 python 列的五個各有各的參數
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
            Servers::named(&config, "rust").map(|s| s.command.as_str()),
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
        assert!(Servers::named(&none, "rust").is_none());
    }

    /// ⚠️ **One that never got up is written off** — 2026-09-20, found against
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
        servers.follow(&editor, &config);
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
