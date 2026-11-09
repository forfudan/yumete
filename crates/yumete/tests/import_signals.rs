//! **Ctrl-C, `kill` or a closed terminal during `ye --import nvim`** (review,
//! 2026-11-09). Neovim runs in a process group of its own, so the signal
//! reached yumete alone: yumete died, Neovim ran on, the 10 s kill never came,
//! and the copy of the reader's config stayed in the temporary directory.
//!
//! Driven through the real binary, because what is tested is a signal and
//! the process dying of it. The home and the temporary directory are this
//! test's own: no editor here ever sees the real `~/.config/nvim`.

#![cfg(unix)]

use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// A directory of this test's own, emptied first so a rerun starts clean.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yumete-import-signal-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn alive(pid: libc::pid_t) -> bool {
    // SAFETY: signal 0 only asks whether the process is there.
    unsafe { libc::kill(pid, 0) == 0 }
}

/// What is left of the import's runs in `tmp`.
fn runs_left(tmp: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(tmp)
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.to_string_lossy().contains("yumete-run-")).collect())
        .unwrap_or_default()
}

#[test]
fn a_signal_during_an_import_takes_neovim_and_the_copy_with_it() {
    if Command::new("nvim").arg("--version").output().is_err() {
        eprintln!("skipped: no nvim");
        return;
    }
    for (name, signal) in [("int", libc::SIGINT), ("term", libc::SIGTERM), ("hup", libc::SIGHUP)] {
        let dir = scratch(name);
        let home = dir.join("home");
        let tmp = dir.join("tmp");
        let config = home.join(".config").join("nvim");
        std::fs::create_dir_all(&config).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();
        let pid_file = dir.join("nvim.pid");
        std::fs::write(
            config.join("init.lua"),
            format!(
                "local f = io.open(\"{}\", \"w\"); f:write(vim.fn.getpid()); f:close()\n\
                 vim.wait(60000, function() return false end)\n",
                pid_file.display()
            ),
        )
        .unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_yumete"));
        command
            .args(["--import", "nvim"])
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap_or_default())
            .env("HOME", &home)
            .env("TMPDIR", &tmp)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        // The three as a terminal would leave them, whatever this test was
        // started with (`nohup` ignores SIGHUP).
        // SAFETY: only `signal`, which is async-signal-safe, between fork
        // and exec.
        unsafe {
            command.pre_exec(|| {
                for s in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
                    libc::signal(s, libc::SIG_DFL);
                }
                Ok(())
            });
        }
        let mut yumete = command.spawn().unwrap();
        let started = Instant::now();
        let nvim = loop {
            if let Some(pid) = std::fs::read_to_string(&pid_file).ok().and_then(|t| t.trim().parse::<libc::pid_t>().ok()) {
                break pid;
            }
            assert!(started.elapsed() < Duration::from_secs(8), "{name}: Neovim never started");
            assert!(yumete.try_wait().unwrap().is_none(), "{name}: yumete ended before Neovim started");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(!runs_left(&tmp).is_empty(), "{name}: the copy is there while Neovim runs");
        let sent = Instant::now();
        // SAFETY: two integers.
        unsafe {
            libc::kill(yumete.id() as libc::pid_t, signal);
        }
        let status = loop {
            if let Some(status) = yumete.try_wait().unwrap() {
                break status;
            }
            if sent.elapsed() > Duration::from_secs(5) {
                let _ = yumete.kill();
                panic!("{name}: yumete did not end");
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(status.signal(), Some(signal), "{name}: yumete ends by the signal it was sent: {status:?}");
        assert!(sent.elapsed() < Duration::from_secs(3), "{name}: at once, not at the 10 s limit");
        let gone = (0..100).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            !alive(nvim)
        });
        if !gone {
            // SAFETY: it must not outlive the test either.
            unsafe { libc::kill(nvim, libc::SIGKILL) };
        }
        assert!(gone, "{name}: Neovim outlived yumete");
        assert_eq!(runs_left(&tmp), Vec::<PathBuf>::new(), "{name}: the copy is removed");
        assert!(!home.join(".config/yumete").exists(), "{name}: nothing was written");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
