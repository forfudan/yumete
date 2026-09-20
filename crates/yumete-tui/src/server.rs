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
    /// Whether an answer is still owed — what [`Servers::due_in`] reads.
    waiting: bool,
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
    /// When the current buffer last changed, for the settle above.
    touched: Option<std::time::Instant>,
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
            touched: None,
            settle: SETTLE,
            says: None,
        }
    }
}

impl Servers {
    /// **What this language's server is called**, or `None` for 「no server」.
    ///
    /// An empty command is a reader turning one of the built-in two off, which
    /// is why it is not simply a missing entry.
    fn named<'a>(
        config: &'a yumete_config::Config,
        language: &str,
    ) -> Option<&'a yumete_config::Server> {
        config.lsp.get(language).filter(|s| !s.command.is_empty())
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
        if known == Some(revision) {
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
                lsp::did_open(&path, language, server.version, &text)
            }
            true => lsp::did_change(&path, server.version, &text),
        };
        server.say(message);
        server.waiting = true;
        self.sent.insert(path, revision);
    }

    /// Take everything the servers have said and give it to the editor.
    ///
    /// **Never waits.** What has arrived, arrives; what has not will be here
    /// next time round.
    pub fn collect(&mut self, editor: &mut Editor) -> bool {
        let mut anything = false;
        let mut gone: Vec<String> = Vec::new();
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
                        editor.set_problems(path, said);
                        server.waiting = false;
                        anything = true;
                    }
                    // ⚠️ An unanswered request can stall a server for good.
                    Ok(Notice::Asked { id }) => server.say(lsp::empty_answer(id)),
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

    /// How long the event loop may sleep, or `None` to block.
    pub fn due_in(&self) -> Option<std::time::Duration> {
        let waiting = self.running.values().any(|s| s.waiting || !s.ready);
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
        // A closed channel is a dead server, which `collect` will notice on
        // its own round; there is nothing useful to do here.
        let _ = self.to.send(message);
    }
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

    /// A reader turning one of the two built-in servers off.
    #[test]
    fn an_empty_command_means_no_server_for_that_language() {
        let mut lsp = yumete_config::factory_servers();
        lsp.insert("rust".into(), yumete_config::Server::default());
        let config = yumete_config::Config { lsp, ..Default::default() };
        let (editor, _path) = editor_on("d.rs", "fn main() {}\n");
        let mut servers = Servers::default();
        servers.follow(&editor, &config);
        assert!(servers.running.is_empty(), "關掉了就不起");
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
}
