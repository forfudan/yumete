//! **Running the reader's vim or Neovim, headless, on a copy of its
//! config**, for `--import vim`, `nvim` and `lazyvim`.
//!
//! The hard rule: **an import never writes to the reader's vim or
//! Neovim config, data, state or cache.** So the editor never sees them:
//!
//! - **It runs in a home of its own**, a new directory under the system's
//!   temporary one, with `HOME` and the four `XDG_*` directories pointed into
//!   it. The config directory is copied there, and so is the data directory
//!   (where lazy.nvim keeps its plugins), less `.git` and the big caches
//!   below. State and cache start empty. Whatever the config or its plugins
//!   write — `lazy-lock.json`, a plugin's build, a log — lands in the copy,
//!   and the copy is removed afterwards.
//! - **git stays inside the copy** (`GIT_CEILING_DIRECTORIES`). The copy has
//!   no `.git`, so a `git config` run in it would climb to whatever
//!   repository the temporary directory sits in, and write there.
//! - **The copy has bounds** ([`Sandbox::copy`]): 200,000 files and
//!   directories, 40 levels deep, and the same 10 s a run has. A link to a
//!   directory it is inside is made again as a link in the copy, and a link
//!   to a directory outside what is copied is left out and named.
//! - **No shada or viminfo, no swap file**: `-i NONE -n`.
//! - **It is killed after 10 s** (decision 9): a config that waits for input,
//!   or a plugin manager that starts downloading, would never end. What it
//!   started goes with it, then or when it quits: its process group
//!   ([`kill_group`]), and then every process that still carries the run's
//!   mark in its environment or works in a directory inside the copy, a job
//!   started with `detach` among them ([`kill_strays`]). A process that
//!   leaves the group, clears its environment and moves out of the copy
//!   escapes all three.
//! - **Ctrl-C, `kill` or a closed terminal** while it runs do the same and
//!   remove the copy before yumete goes ([`signals`]).
//!
//! Warning: **Windows has none of the last three.** It has no process group
//! to kill and no environment to read, and a console's Ctrl-C ends yumete
//! without removing the copy (a job object would close that gap, and needs a
//! crate this one does not have). At the time limit the editor and what runs
//! under it are killed there with `taskkill /T` ([`kill_tree`]); what it
//! started and left running when it quit by itself is not.
//!
//! What is read comes back as JSON from `dump.lua` or `dump.vim`, written
//! into the same directory.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub(super) const DUMP_LUA: &str = include_str!("dump.lua");
pub(super) const DUMP_VIM: &str = include_str!("dump.vim");
pub(super) const LAZYVIM_BASE: &str = include_str!("lazyvim_base.lua");

/// How long one run may take (decision 9: 10 s, no flag). Copying the config
/// may take as long again.
pub(super) const TIMEOUT: Duration = Duration::from_secs(10);

/// Directories of a data directory left out of the copy, at its top only:
/// mason's language servers are large and say nothing about keys, and the
/// rest are history an old Neovim kept there.
pub(super) const DATA_LEFT_OUT: &[&str] = &["mason", "shada", "swap", "undo", "backup", "view", "sessions"];

/// The most files and directories, together, a copy takes before the import
/// gives up.
const MOST_ENTRIES: usize = 200_000;

/// The deepest a copy goes.
const DEEPEST: usize = 40;

/// The variable every process of a run carries, naming the run's directory:
/// how [`kill_strays`] knows them.
const MARK: &str = "YUMETE_IMPORT_RUN";

/// Why a run gave nothing.
#[derive(Debug)]
pub(super) enum Failed {
    /// The program is not on `PATH`.
    NotInstalled,
    /// It did not finish in time, and was killed.
    TimedOut(Duration),
    /// It ended without writing what it read; what it said, if anything.
    NoOutput(Option<String>),
    /// Older than the dump can read; the version it gave.
    TooOld(String),
    /// The copy could not be made.
    Copy(String),
    /// yumete was asked to stop (Ctrl-C, `kill`, a closed terminal). Never
    /// shown: the sandbox ends the process once it has cleaned up.
    Stopped,
}

impl Failed {
    /// One sentence for the terminal and the file's header, of Vim's run
    /// or Neovim's (decisions 7 and 8: Neovim 0.9, Vim 9.0 at least).
    pub(super) fn said(&self, vim: bool) -> String {
        // Each on its own: the messages test reads a dotted literal after
        // `", "` as a message tag.
        let program = if vim { "vim" } else { "nvim" };
        let editor = if vim { "Vim" } else { "Neovim" };
        let least = if vim { "9.0" } else { "0.9" };
        match self {
            Failed::NotInstalled => format!("there is no `{program}` on PATH"),
            Failed::TimedOut(after) => format!(
                "{editor} did not finish in {} s: it may be waiting for input, or downloading plugins",
                after.as_secs()
            ),
            Failed::NoOutput(Some(said)) => format!("{editor} stopped before it could be read: {said}"),
            Failed::NoOutput(None) => format!("{editor} stopped before it could be read"),
            Failed::TooOld(found) => format!("needs {editor} {least} or later, found {found}"),
            Failed::Copy(why) => format!("could not copy the config to run it: {why}"),
            Failed::Stopped => format!("{editor} was stopped with the import"),
        }
    }
}

/// What a run read: the dump, and the first thing the editor said on
/// stderr while starting (a config's error), cut to 200 characters.
#[derive(Debug)]
pub(super) struct Dumped {
    pub json: serde_json::Value,
    pub said: Option<String>,
    /// Neovim said a Lua file or function stopped at an error (`E5108`,
    /// `E5112`, `E5113`): what it would set after that is not set.
    pub stopped: bool,
}

/// Why a copy stopped part way.
enum CopyStop {
    Io(std::io::Error),
    /// Past a bound; the sentence that says which.
    GaveUp(String),
    Stopped,
}

impl From<std::io::Error> for CopyStop {
    fn from(e: std::io::Error) -> CopyStop {
        CopyStop::Io(e)
    }
}

/// **A home of the import's own**, removed when dropped.
pub(super) struct Sandbox {
    pub root: PathBuf,
    /// `root` as it really is (`/private/var` for macOS's `/var`).
    real_root: PathBuf,
    pub home: PathBuf,
    /// Files and directories copied so far.
    entries: usize,
    runs: usize,
    /// When copying has to be done by: [`TIMEOUT`] from the sandbox's start.
    deadline: Instant,
    budget: Duration,
    /// Each link the copy left out, and why, for the terminal.
    pub left_out: Vec<String>,
    /// Ctrl-C and its kin, caught for as long as the sandbox lives.
    #[cfg(unix)]
    caught: Option<signals::Caught>,
}

impl Sandbox {
    /// A sandbox under the system's temporary directory, whose copies may
    /// take `budget` between them.
    pub(super) fn new(budget: Duration) -> Result<Sandbox, Failed> {
        Sandbox::new_in(&std::env::temp_dir(), budget)
    }

    /// The same, under `base`.
    pub(super) fn new_in(base: &Path, budget: Duration) -> Result<Sandbox, Failed> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = base.join(format!("yumete-run-{}-{stamp}", std::process::id()));
        // **The reader's alone, and new**: it holds a copy of their config
        // under the shared temporary directory, so it is made 0700, and a
        // directory already there by that name is refused rather than used
        // (and, refused before the sandbox exists, never removed either).
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut made = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut made, 0o700);
        made.create(&root).map_err(|e| Failed::Copy(format!("{}: {e}", root.display())))?;
        let real_root = std::fs::canonicalize(&root).map_err(|e| Failed::Copy(e.to_string()))?;
        let home = root.join("home");
        let sandbox = Sandbox {
            real_root,
            root,
            home,
            entries: 0,
            runs: 0,
            deadline: Instant::now() + budget,
            budget,
            left_out: Vec::new(),
            #[cfg(unix)]
            caught: Some(signals::Caught::install()),
        };
        for dir in [".config", ".local/share", ".local/state", ".cache"] {
            std::fs::create_dir_all(sandbox.home.join(dir)).map_err(|e| Failed::Copy(e.to_string()))?;
        }
        for dir in ["run", "tmp"] {
            std::fs::create_dir_all(sandbox.root.join(dir)).map_err(|e| Failed::Copy(e.to_string()))?;
        }
        Ok(sandbox)
    }

    pub(super) fn config(&self) -> PathBuf {
        self.home.join(".config")
    }

    pub(super) fn data(&self) -> PathBuf {
        self.home.join(".local").join("share")
    }

    /// **Copy `from` to `to`**, links followed, `.git` left out at every
    /// level and `left_out` at the top. Nothing is ever written to `from`.
    ///
    /// Warning: **a link can make a tree without end** — `lua/loop/a -> .`
    /// once made 357,928 directories in 30 s, and was still going. So:
    ///
    /// - a link to a directory the copy is inside is made again as a link to
    ///   that directory's copy (left out on Windows, where making a link
    ///   takes a privilege): the config reads what it read, and the copy is
    ///   one tree;
    /// - a link to a directory outside `from` is left out and named in
    ///   [`Sandbox::left_out`]: copied, it could be the whole home, and made
    ///   again as a link, the editor would write through it to the reader's
    ///   files. A link to a file is copied wherever it points (a stowed
    ///   `init.lua` is one);
    /// - and it gives up past [`MOST_ENTRIES`] files and directories,
    ///   [`DEEPEST`] levels, or the sandbox's time.
    ///
    /// Warning: **a copy only ever writes inside the sandbox.** A target
    /// outside it, or a config that holds the sandbox, is refused before
    /// anything is written, and a file is never copied onto itself, which
    /// would empty it: a target that is the reader's config by some other
    /// path (an absolute `NVIM_APPNAME`) stops here too.
    pub(super) fn copy(&mut self, from: &Path, to: &Path, left_out: &[&str]) -> Result<(), Failed> {
        // The top is followed: `~/.config/nvim` is often a link into dotfiles.
        let Ok(top) = std::fs::canonicalize(from) else { return Ok(()) };
        if !self.holds(to) {
            return Err(Failed::Copy(format!(
                "{} is not inside the import's own directory, so nothing was copied",
                to.display()
            )));
        }
        if self.real_root.starts_with(&top) {
            return Err(Failed::Copy(format!(
                "{} holds the import's own directory ({}), so nothing was copied",
                from.display(),
                self.root.display()
            )));
        }
        let mut inside = Vec::new();
        match self.copy_at(&top, from, to, left_out, &mut inside) {
            Ok(()) => Ok(()),
            Err(CopyStop::Io(e)) => Err(Failed::Copy(format!("{}: {e}", from.display()))),
            Err(CopyStop::GaveUp(why)) => Err(Failed::Copy(format!("gave up copying {}: {why}", from.display()))),
            Err(CopyStop::Stopped) => Err(Failed::Stopped),
        }
    }

    /// One entry of a copy. `inside` holds each directory the copy is in,
    /// as it really is and where its copy is.
    fn copy_at(
        &mut self,
        top: &Path,
        from: &Path,
        to: &Path,
        left_out: &[&str],
        inside: &mut Vec<(PathBuf, PathBuf)>,
    ) -> Result<(), CopyStop> {
        if signals::stopped() {
            return Err(CopyStop::Stopped);
        }
        if Instant::now() >= self.deadline {
            return Err(CopyStop::GaveUp(format!(
                "it took more than {} s ({} files and directories so far); it may be too big to run in a copy",
                self.budget.as_secs(),
                self.entries
            )));
        }
        if inside.len() > DEEPEST {
            return Err(CopyStop::GaveUp(format!("it is more than {DEEPEST} directories deep at {}", from.display())));
        }
        let (Ok(link), Ok(meta)) = (std::fs::symlink_metadata(from), std::fs::metadata(from)) else {
            // A link to nothing, or gone since it was listed.
            return Ok(());
        };
        if meta.is_file() {
            self.count()?;
            if !self.holds(to) || same_file(from, to) {
                return Err(CopyStop::GaveUp(format!("{} would be copied onto itself or out of the copy", from.display())));
            }
            std::fs::copy(from, to)?;
            return Ok(());
        }
        if !meta.is_dir() {
            return Ok(());
        }
        let Ok(real) = std::fs::canonicalize(from) else { return Ok(()) };
        if link.file_type().is_symlink() && !inside.is_empty() {
            if let Some((_, copied)) = inside.iter().find(|(dir, _)| *dir == real) {
                #[cfg(unix)]
                std::os::unix::fs::symlink(copied, to)?;
                #[cfg(not(unix))]
                let _ = copied;
                return Ok(());
            }
            if !real.starts_with(top) {
                self.left_out.push(format!("{}, a link to {}", from.display(), real.display()));
                return Ok(());
            }
        }
        self.count()?;
        std::fs::create_dir_all(to)?;
        inside.push((real, to.to_path_buf()));
        let depth = inside.len();
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let name = entry.file_name();
            let skip = name == ".git" || (depth == 1 && left_out.iter().any(|n| name == *n));
            if !skip {
                self.copy_at(top, &entry.path(), &to.join(&name), left_out, inside)?;
            }
        }
        inside.pop();
        Ok(())
    }

    /// **Whether `path` is inside the sandbox**, by its name (no `..`) and,
    /// through the deepest part of it that is there already, as it really
    /// is: a link on the way out of the sandbox does not count as inside.
    fn holds(&self, path: &Path) -> bool {
        use std::path::Component;
        let Ok(below) = path.strip_prefix(&self.root) else { return false };
        if !below.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir)) {
            return false;
        }
        let there = path.ancestors().find(|p| p.symlink_metadata().is_ok()).unwrap_or(&self.root);
        std::fs::canonicalize(there).is_ok_and(|real| real.starts_with(&self.real_root))
    }

    /// One more file or directory copied.
    fn count(&mut self) -> Result<(), CopyStop> {
        self.entries += 1;
        if self.entries > MOST_ENTRIES {
            return Err(CopyStop::GaveUp(format!(
                "it has more than {MOST_ENTRIES} files and directories; it may be too big to run in a copy"
            )));
        }
        Ok(())
    }

    /// **Run `program` with `args` in this home**, with `script` as the dump,
    /// and read what it wrote.
    pub(super) fn run(
        &mut self,
        program: &str,
        args: &[OsString],
        script: (&str, &str),
        timeout: Duration,
    ) -> Result<Dumped, Failed> {
        if signals::stopped() {
            return Err(Failed::Stopped);
        }
        self.runs += 1;
        let run = self.root.join("run");
        let script_path = run.join(script.0);
        std::fs::write(&script_path, script.1).map_err(|e| Failed::Copy(e.to_string()))?;
        let out = run.join(format!("dump-{}.json", self.runs));
        let err = run.join(format!("stderr-{}.txt", self.runs));
        let stderr = std::fs::File::create(&err).map_err(|e| Failed::Copy(e.to_string()))?;
        // git looks for a repository no higher than the home: the sandbox's
        // own directory and anything above it are out of bounds, under the
        // name it was given and the one it resolves to (`/var` is
        // `/private/var` on macOS).
        let mut ceilings = vec![self.root.clone()];
        ceilings.extend(std::fs::canonicalize(&self.root).ok().filter(|real| *real != self.root));
        #[cfg(windows)]
        let program = &on_path(program);
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(&self.home)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.config())
            .env("XDG_DATA_HOME", self.data())
            .env("XDG_STATE_HOME", self.home.join(".local").join("state"))
            .env("XDG_CACHE_HOME", self.home.join(".cache"))
            // Neovim's own temporary directory (`nvim.$USER`) and whatever a
            // plugin makes there go with the copy, a killed run's included,
            // rather than staying in `$TMPDIR`.
            .env("TMPDIR", self.root.join("tmp"))
            .env("YUMETE_DUMP_SCRIPT", &script_path)
            .env("YUMETE_DUMP", &out)
            .env("YUMETE_LOCK", run.join("lazy-lock.json"))
            .env(MARK, &self.root)
            .env("GIT_CEILING_DIRECTORIES", std::env::join_paths(&ceilings).unwrap_or_default())
            .env("GIT_CONFIG_NOSYSTEM", "1")
            // Each names a file of the reader's or a running Neovim.
            .env_remove("VIMINIT")
            .env_remove("MYVIMRC")
            .env_remove("NVIM_LOG_FILE")
            .env_remove("NVIM")
            .env_remove("NVIM_LISTEN_ADDRESS")
            // …or a repository, or git's config, of the reader's.
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_COMMON_DIR")
            .env_remove("GIT_CONFIG")
            .env_remove("GIT_CONFIG_GLOBAL")
            .env_remove("GIT_CONFIG_SYSTEM")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            // A file, not a pipe: a full pipe would stop the editor.
            .stderr(stderr);
        // A process group of its own, so that what it starts can be killed
        // with it.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        #[cfg(windows)]
        command.env("TMP", self.root.join("tmp")).env("TEMP", self.root.join("tmp"));
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Failed::NotInstalled),
            Err(e) => return Err(Failed::NoOutput(Some(e.to_string()))),
        };
        let watched = signals::Watched::new(&child);
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    kill_group(&child);
                    break;
                }
                Ok(None) if signals::stopped() => {
                    kill_tree(&child);
                    kill_group(&child);
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Failed::Stopped);
                }
                Ok(None) if start.elapsed() >= timeout => {
                    // Asked first: Neovim's own jobs run in sessions of their
                    // own, and only Neovim, leaving, stops them.
                    signal_group(&child, Signal::Term);
                    let asked = Instant::now();
                    while asked.elapsed() < Duration::from_secs(1) && matches!(child.try_wait(), Ok(None)) {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    kill_tree(&child);
                    kill_group(&child);
                    let _ = child.kill();
                    let _ = child.wait();
                    drop(watched);
                    kill_strays(&self.root);
                    return Err(Failed::TimedOut(timeout));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(e) => return Err(Failed::NoOutput(Some(e.to_string()))),
            }
        }
        drop(watched);
        kill_strays(&self.root);
        // The error's own line (`E5113: …`) before the line that leads to it,
        // with the copy's home written as the reader's `~`.
        let stderr = std::fs::read_to_string(&err).unwrap_or_default();
        // **An error part way through a Lua file stops the file**: a colour
        // scheme not installed in the copy takes every map after it, and the
        // file has to say they are missing.
        let stopped = ["E5108:", "E5112:", "E5113:"].iter().any(|e| stderr.contains(e));
        let said = Some(stderr).and_then(|text| {
            let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            let numbered = |l: &&&str| l.starts_with('E') && l[1..].starts_with(|c: char| c.is_ascii_digit());
            let line = lines.iter().find(numbered).or(lines.first())?;
            let line = line.replace(&self.home.display().to_string(), "~");
            Some(line.chars().take(200).collect::<String>())
        });
        // Read as UTF-8 where it is not: Neovim writes a description's bytes
        // as they are, and one byte that is not UTF-8 would make the whole
        // dump unreadable.
        let read = std::fs::read(&out).ok().map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let Some(json) = read.and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else {
            return Err(Failed::NoOutput(said));
        };
        if json.get("error").and_then(|e| e.as_str()) == Some("too old") {
            let found = json.get("nvim").or(json.get("vim")).and_then(|v| v.as_str()).unwrap_or("an older one");
            return Err(Failed::TooOld(found.to_string()));
        }
        Ok(Dumped { json, said, stopped })
    }
}

/// Whether `a` and `b` are one file (`b` there at all, and the same file
/// as `a`).
fn same_file(a: &Path, b: &Path) -> bool {
    let (Ok(x), Ok(y)) = (std::fs::metadata(a), std::fs::metadata(b)) else { return false };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        x.dev() == y.dev() && x.ino() == y.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = (x, y);
        std::fs::canonicalize(a).ok().is_some_and(|a| std::fs::canonicalize(b).ok() == Some(a))
    }
}

/// **Kill whatever the editor started** — a plugin manager's `git fetch`, a
/// language server — with it: the editor led a process group of its own.
/// Also once it has quit, as a job it left would write into a copy about to
/// be removed, or run on for ever.
///
/// Warning: **Windows has no process group to kill**; [`kill_tree`] stands
/// in while the editor runs, and what it left behind when it quit may run on
/// (a job object would close that gap, and needs a crate this one does not
/// have).
fn kill_group(child: &std::process::Child) {
    signal_group(child, Signal::Kill);
}

/// **Windows: the editor and every process under it**, while it still
/// runs, with the system's own `taskkill`: Windows has no process group to
/// kill. The `Child` holds the process open, so its id is not yet anyone
/// else's.
fn kill_tree(child: &std::process::Child) {
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    let _ = child;
}

/// **Windows: `vim.bat` when there is no `vim.exe`** on `PATH`: Vim's
/// installer puts the batch file there, and `Command` looks for an `.exe`
/// alone.
#[cfg(windows)]
fn on_path(program: &str) -> String {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let has = |name: &str| std::env::split_paths(&path).any(|dir| dir.join(name).is_file());
    match !has(&format!("{program}.exe")) && has(&format!("{program}.bat")) {
        true => format!("{program}.bat"),
        false => program.to_string(),
    }
}

/// The two signals a run is sent.
#[derive(Clone, Copy)]
enum Signal {
    /// Leave, stopping your own jobs.
    Term,
    Kill,
}

/// Send `signal` to the process group `child` leads.
fn signal_group(child: &std::process::Child, signal: Signal) {
    #[cfg(unix)]
    if let Ok(group) = libc::pid_t::try_from(child.id()) {
        let signal = match signal {
            Signal::Term => libc::SIGTERM,
            Signal::Kill => libc::SIGKILL,
        };
        // SAFETY: `kill` takes two integers and touches no memory of ours;
        // a negative pid names the group the child leads.
        unsafe {
            libc::kill(-group, signal);
        }
    }
    #[cfg(not(unix))]
    let _ = (child, signal);
}

/// **Kill every process the run left behind outside its group**: one that
/// carries the run's [`MARK`] in its environment, or works in a directory
/// inside the copy. Neovim's `jobstart(…, {detach = true})` runs its job in
/// a session of its own that nothing else reaches, and that job can write
/// into the copy seconds after the import has returned.
///
/// A process inherits its parent's environment and directory, so one or the
/// other is on everything the editor started, at any remove: the mark unless
/// it was started with its environment cleared (`clear_env`), the directory
/// unless it moved out. Warning: **macOS hides the environment of its own
/// programs** (`/bin/sh`, `/bin/sleep`: signed by Apple, they show only their
/// arguments), so there such a one is known by its directory alone.
///
/// Only the reader's own processes can be read, which are the only ones a
/// run can start. Again until none is found, as one may have started another
/// while it was being looked for.
fn kill_strays(root: &Path) {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let mut needle = Vec::from(MARK.as_bytes());
        needle.push(b'=');
        needle.extend_from_slice(root.as_os_str().as_bytes());
        needle.push(0);
        // A process's directory is told as it really is: `/private/var`
        // for macOS's `/var`.
        let mut roots = vec![root.to_path_buf()];
        roots.extend(std::fs::canonicalize(root).ok().filter(|real| real != root));
        let me = libc::pid_t::try_from(std::process::id()).unwrap_or(0);
        let mut buffer = Vec::new();
        for _ in 0..10 {
            let mut found = false;
            for pid in processes::all() {
                if pid <= 1 || pid == me {
                    continue;
                }
                let inside = processes::directory(pid).is_some_and(|dir| roots.iter().any(|r| dir.starts_with(r)));
                if inside || processes::environment(pid, &mut buffer).is_some_and(|env| carries(env, &needle)) {
                    // SAFETY: as in `signal_group`, two integers.
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                    found = true;
                }
            }
            if !found {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let _ = root;
}

/// Whether a NUL-separated environment holds `needle` (`NAME=value\0`) as
/// one of its strings.
#[cfg(any(target_os = "linux", target_os = "macos", test))]
pub(super) fn carries(env: &[u8], needle: &[u8]) -> bool {
    env.windows(needle.len()).enumerate().any(|(at, w)| w == needle && (at == 0 || env[at - 1] == 0))
}

/// The process table, as far as it is ours to read.
#[cfg(target_os = "linux")]
mod processes {
    pub(super) fn all() -> Vec<libc::pid_t> {
        let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
        dir.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).collect()
    }

    /// The directory it works in.
    pub(super) fn directory(pid: libc::pid_t) -> Option<std::path::PathBuf> {
        std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
    }

    /// Its environment, NUL-separated, read into `buffer`.
    pub(super) fn environment(pid: libc::pid_t, buffer: &mut Vec<u8>) -> Option<&[u8]> {
        *buffer = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
        Some(buffer)
    }
}

/// The process table, as far as it is ours to read.
#[cfg(target_os = "macos")]
mod processes {
    pub(super) fn all() -> Vec<libc::pid_t> {
        // SAFETY: asked for the count with no buffer, then into a buffer
        // with room for more than that; the kernel writes no further than
        // the size it is given.
        unsafe {
            let count = libc::proc_listallpids(std::ptr::null_mut(), 0);
            if count <= 0 {
                return Vec::new();
            }
            let mut pids: Vec<libc::pid_t> = vec![0; count as usize + 64];
            let bytes = libc::c_int::try_from(pids.len() * std::mem::size_of::<libc::pid_t>()).unwrap_or(0);
            let got = libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes);
            pids.truncate(got.max(0) as usize);
            pids
        }
    }

    /// The directory it works in (`PROC_PIDVNODEPATHINFO`).
    pub(super) fn directory(pid: libc::pid_t) -> Option<std::path::PathBuf> {
        use std::os::unix::ffi::OsStrExt;
        // SAFETY: `proc_pidinfo` writes at most the size it is given into
        // the struct, and says how much it wrote; the path in it ends in a
        // NUL within its `MAXPATHLEN` bytes.
        unsafe {
            let mut info: libc::proc_vnodepathinfo = std::mem::zeroed();
            let size = libc::c_int::try_from(std::mem::size_of::<libc::proc_vnodepathinfo>()).ok()?;
            if libc::proc_pidinfo(pid, libc::PROC_PIDVNODEPATHINFO, 0, (&raw mut info).cast(), size) != size {
                return None;
            }
            let path = std::slice::from_raw_parts(
                info.pvi_cdir.vip_path.as_ptr().cast::<u8>(),
                std::mem::size_of_val(&info.pvi_cdir.vip_path),
            );
            let path = &path[..path.iter().position(|&b| b == 0)?];
            Some(std::path::PathBuf::from(std::ffi::OsStr::from_bytes(path)))
        }
    }

    /// Its arguments and environment, NUL-separated (`KERN_PROCARGS2`),
    /// read into `buffer`.
    pub(super) fn environment(pid: libc::pid_t, buffer: &mut Vec<u8>) -> Option<&[u8]> {
        // SAFETY: `sysctl` writes at most `size` bytes into the space it is
        // given, and says how many it wrote in `size`.
        unsafe {
            if buffer.is_empty() {
                let mut most: libc::c_int = 0;
                let mut size = std::mem::size_of::<libc::c_int>();
                let mut name = [libc::CTL_KERN, libc::KERN_ARGMAX];
                if libc::sysctl(name.as_mut_ptr(), 2, (&raw mut most).cast(), &mut size, std::ptr::null_mut(), 0) != 0 {
                    return None;
                }
                buffer.resize(usize::try_from(most).ok()?, 0);
            }
            let mut size = buffer.len();
            let mut name = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
            if libc::sysctl(name.as_mut_ptr(), 3, buffer.as_mut_ptr().cast(), &mut size, std::ptr::null_mut(), 0) != 0 {
                return None;
            }
            Some(&buffer[..size])
        }
    }
}


/// **Ctrl-C, `kill` and a closed terminal, during an import**. The editor
/// leads a process group of its own, so the terminal's SIGINT reaches yumete
/// alone, and yumete dying of it would leave Neovim running and the copy of
/// the reader's config in the temporary directory. So for as long as a
/// [`Sandbox`] lives, SIGINT, SIGTERM and
/// SIGHUP kill the editor's group at once and only mark that the import is
/// to stop; the copy and the run notice, return, and the sandbox, dropped,
/// kills what is left, removes itself, and ends yumete by the same signal.
#[cfg(unix)]
mod signals {
    use std::sync::atomic::{AtomicI32, Ordering};

    /// The signal that asked yumete to stop, or 0.
    static STOP: AtomicI32 = AtomicI32::new(0);
    /// The group of the editor running now, or 0.
    static GROUP: AtomicI32 = AtomicI32::new(0);

    const CAUGHT: [libc::c_int; 3] = [libc::SIGINT, libc::SIGTERM, libc::SIGHUP];

    /// Only what is safe in a signal handler: two stores and a `kill`.
    extern "C" fn caught(signal: libc::c_int) {
        STOP.store(signal, Ordering::SeqCst);
        let group = GROUP.load(Ordering::SeqCst);
        if group > 0 {
            // SAFETY: `kill` is async-signal-safe, and takes two integers.
            unsafe {
                libc::kill(-group, libc::SIGKILL);
            }
        }
    }

    /// Whether yumete has been asked to stop.
    pub(super) fn stopped() -> bool {
        STOP.load(Ordering::SeqCst) != 0
    }

    /// The three signals caught, and what they did before.
    pub(super) struct Caught {
        was: Vec<(libc::c_int, libc::sigaction)>,
    }

    impl Caught {
        /// Catch the three. One a parent set to be ignored (`nohup`) stays
        /// ignored.
        pub(super) fn install() -> Caught {
            let mut was = Vec::new();
            // SAFETY: `sigaction` reads the action given and writes the old
            // one into the space given; the handler does only what a handler
            // may.
            unsafe {
                let mut now: libc::sigaction = std::mem::zeroed();
                now.sa_sigaction = caught as extern "C" fn(libc::c_int) as libc::sighandler_t;
                libc::sigemptyset(&mut now.sa_mask);
                now.sa_flags = libc::SA_RESTART;
                for signal in CAUGHT {
                    let mut old: libc::sigaction = std::mem::zeroed();
                    if libc::sigaction(signal, std::ptr::null(), &mut old) != 0 || old.sa_sigaction == libc::SIG_IGN {
                        continue;
                    }
                    if libc::sigaction(signal, &now, std::ptr::null_mut()) == 0 {
                        was.push((signal, old));
                    }
                }
            }
            Caught { was }
        }

        /// Put the three back, then, if one of them came, die of it as
        /// yumete would have without the sandbox.
        pub(super) fn release(self) {
            // SAFETY: putting back the actions read in `install`.
            unsafe {
                for (signal, old) in &self.was {
                    libc::sigaction(*signal, old, std::ptr::null_mut());
                }
            }
            let signal = STOP.swap(0, Ordering::SeqCst);
            if signal != 0 {
                // SAFETY: the default action for a signal yumete was sent.
                unsafe {
                    libc::signal(signal, libc::SIG_DFL);
                    libc::raise(signal);
                }
            }
        }
    }

    /// The editor's group, known to the handler while it runs.
    pub(super) struct Watched;

    impl Watched {
        pub(super) fn new(child: &std::process::Child) -> Watched {
            GROUP.store(i32::try_from(child.id()).unwrap_or(0), Ordering::SeqCst);
            Watched
        }
    }

    impl Drop for Watched {
        fn drop(&mut self) {
            GROUP.store(0, Ordering::SeqCst);
        }
    }
}

/// Windows: nothing caught (see the module's warning).
#[cfg(not(unix))]
mod signals {
    pub(super) fn stopped() -> bool {
        false
    }

    pub(super) struct Watched;

    impl Watched {
        pub(super) fn new(_: &std::process::Child) -> Watched {
            Watched
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        kill_strays(&self.root);
        let _ = std::fs::remove_dir_all(&self.root);
        #[cfg(unix)]
        if let Some(caught) = self.caught.take() {
            caught.release();
        }
    }
}

/// The arguments of a Neovim run: the reader's config, `--clean`, or
/// LazyVim alone (`-u` the base script).
pub(super) fn nvim_args(how: Nvim, sandbox: &Sandbox) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    match how {
        Nvim::Own => {}
        Nvim::Clean => args.push("--clean".into()),
        Nvim::LazyVimBase => {
            let base = sandbox.root.join("run").join("lazyvim_base.lua");
            let _ = std::fs::write(&base, LAZYVIM_BASE);
            args.push("-u".into());
            args.push(base.into_os_string());
        }
    }
    for a in ["--headless", "-i", "NONE", "-n", "-c", "lua dofile(vim.env.YUMETE_DUMP_SCRIPT)"] {
        args.push(a.into());
    }
    args
}

/// Which Neovim run.
#[derive(Clone, Copy)]
pub(super) enum Nvim {
    Own,
    Clean,
    LazyVimBase,
}

/// The arguments of a Vim run: with `vimrc`, or with none (`-u NONE`, the
/// baseline).
pub(super) fn vim_args(vimrc: Option<&Path>) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-N".into(), "-es".into(), "-u".into()];
    args.push(vimrc.map_or_else(|| "NONE".into(), |p| p.as_os_str().to_os_string()));
    for a in ["-i", "NONE", "-n", "-c", "execute 'source' fnameescape($YUMETE_DUMP_SCRIPT)"] {
        args.push(a.into());
    }
    args
}
