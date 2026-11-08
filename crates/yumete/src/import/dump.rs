//! **Running the reader's vim or Neovim, headless, on a copy of its
//! config** (2026-11-08), for `--import vim`, `nvim` and `lazyvim`.
//!
//! The author's hard rule: **an import never writes to the reader's vim or
//! Neovim config, data, state or cache.** So the editor never sees them:
//!
//! - **It runs in a home of its own**, a new directory under the system's
//!   temporary one, with `HOME` and the four `XDG_*` directories pointed into
//!   it. The config directory is copied there, and so is the data directory
//!   (where lazy.nvim keeps its plugins), less `.git` and the big caches
//!   below. State and cache start empty. Whatever the config or its plugins
//!   write — `lazy-lock.json`, a plugin's build, a log — lands in the copy,
//!   and the copy is removed afterwards.
//! - **No shada or viminfo, no swap file**: `-i NONE -n`.
//! - **It is killed after 10 s** (decision 9): a config that waits for input,
//!   or a plugin manager that starts downloading, would never end. What it
//!   started goes with it, then or when it quits ([`kill_group`]).
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

/// How long one run may take (decision 9: 10 s, no flag).
pub(super) const TIMEOUT: Duration = Duration::from_secs(10);

/// Directories of a data directory left out of the copy, at its top only:
/// mason's language servers are large and say nothing about keys, and the
/// rest are history an old Neovim kept there.
pub(super) const DATA_LEFT_OUT: &[&str] = &["mason", "shada", "swap", "undo", "backup", "view", "sessions"];

/// The most files a copy takes before the import gives up.
const MOST_FILES: usize = 200_000;

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
        }
    }
}

/// What a run read: the dump, and the first thing the editor said on
/// stderr while starting (a config's error), cut to 200 characters.
#[derive(Debug)]
pub(super) struct Dumped {
    pub json: serde_json::Value,
    pub said: Option<String>,
}

/// **A home of the import's own**, removed when dropped.
pub(super) struct Sandbox {
    pub root: PathBuf,
    pub home: PathBuf,
    files: usize,
    runs: usize,
}

impl Sandbox {
    pub(super) fn new() -> Result<Sandbox, Failed> {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("yumete-run-{}-{stamp}", std::process::id()));
        let home = root.join("home");
        for dir in [".config", ".local/share", ".local/state", ".cache"] {
            std::fs::create_dir_all(home.join(dir)).map_err(|e| Failed::Copy(e.to_string()))?;
        }
        std::fs::create_dir_all(root.join("run")).map_err(|e| Failed::Copy(e.to_string()))?;
        Ok(Sandbox { root, home, files: 0, runs: 0 })
    }

    pub(super) fn config(&self) -> PathBuf {
        self.home.join(".config")
    }

    pub(super) fn data(&self) -> PathBuf {
        self.home.join(".local").join("share")
    }

    /// **Copy `from` to `to`**, links followed, `.git` left out at every
    /// level and `left_out` at the top. Nothing is ever written to `from`.
    pub(super) fn copy(&mut self, from: &Path, to: &Path, left_out: &[&str]) -> Result<(), Failed> {
        self.copy_at(from, to, left_out, 0).map_err(|e| Failed::Copy(format!("{}: {e}", from.display())))
    }

    fn copy_at(&mut self, from: &Path, to: &Path, left_out: &[&str], depth: usize) -> std::io::Result<()> {
        let too_many = || std::io::Error::other("too many files");
        // A link to a directory above it would go round for ever.
        if depth > 40 {
            return Err(too_many());
        }
        let Ok(meta) = std::fs::metadata(from) else { return Ok(()) };
        if meta.is_file() {
            self.files += 1;
            if self.files > MOST_FILES {
                return Err(too_many());
            }
            std::fs::copy(from, to)?;
            return Ok(());
        }
        if !meta.is_dir() {
            return Ok(());
        }
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            let name = entry.file_name();
            let skip = name == ".git" || (depth == 0 && left_out.iter().any(|n| name == *n));
            if !skip {
                self.copy_at(&entry.path(), &to.join(&name), left_out, depth + 1)?;
            }
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
        self.runs += 1;
        let run = self.root.join("run");
        let script_path = run.join(script.0);
        std::fs::write(&script_path, script.1).map_err(|e| Failed::Copy(e.to_string()))?;
        let out = run.join(format!("dump-{}.json", self.runs));
        let err = run.join(format!("stderr-{}.txt", self.runs));
        let stderr = std::fs::File::create(&err).map_err(|e| Failed::Copy(e.to_string()))?;
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(&self.home)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.config())
            .env("XDG_DATA_HOME", self.data())
            .env("XDG_STATE_HOME", self.home.join(".local").join("state"))
            .env("XDG_CACHE_HOME", self.home.join(".cache"))
            .env("YUMETE_DUMP_SCRIPT", &script_path)
            .env("YUMETE_DUMP", &out)
            .env("YUMETE_LOCK", run.join("lazy-lock.json"))
            // Each names a file of the reader's or a running Neovim.
            .env_remove("VIMINIT")
            .env_remove("MYVIMRC")
            .env_remove("NVIM_LOG_FILE")
            .env_remove("NVIM")
            .env_remove("NVIM_LISTEN_ADDRESS")
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
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Failed::NotInstalled),
            Err(e) => return Err(Failed::NoOutput(Some(e.to_string()))),
        };
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => {
                    kill_group(&child);
                    break;
                }
                Ok(None) if start.elapsed() >= timeout => {
                    // Asked first: Neovim's own jobs run in sessions of their
                    // own, and only Neovim, leaving, stops them.
                    signal_group(&child, Signal::Term);
                    let asked = Instant::now();
                    while asked.elapsed() < Duration::from_secs(1) && matches!(child.try_wait(), Ok(None)) {
                        std::thread::sleep(Duration::from_millis(25));
                    }
                    kill_group(&child);
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(Failed::TimedOut(timeout));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
                Err(e) => return Err(Failed::NoOutput(Some(e.to_string()))),
            }
        }
        // The error's own line (`E5113: …`) before the line that leads to it,
        // with the copy's home written as the reader's `~`.
        let said = std::fs::read_to_string(&err).ok().and_then(|text| {
            let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            let numbered = |l: &&&str| l.starts_with('E') && l[1..].starts_with(|c: char| c.is_ascii_digit());
            let line = lines.iter().find(numbered).or(lines.first())?;
            let line = line.replace(&self.home.display().to_string(), "~");
            Some(line.chars().take(200).collect::<String>())
        });
        let Some(json) = std::fs::read_to_string(&out).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        else {
            return Err(Failed::NoOutput(said));
        };
        if json.get("error").and_then(|e| e.as_str()) == Some("too old") {
            let found = json.get("nvim").or(json.get("vim")).and_then(|v| v.as_str()).unwrap_or("an older one");
            return Err(Failed::TooOld(found.to_string()));
        }
        Ok(Dumped { json, said })
    }
}

/// **Kill whatever the editor started** — a plugin manager's `git fetch`, a
/// language server — with it: the editor led a process group of its own.
/// Also once it has quit, as a job it left would write into a copy about to
/// be removed, or run on for ever.
///
/// Warning: **Windows has no process group to kill**; only the editor itself
/// is killed there, and what it started may run on (a job object would close
/// that gap, and needs a crate this one does not have).
fn kill_group(child: &std::process::Child) {
    signal_group(child, Signal::Kill);
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

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
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
