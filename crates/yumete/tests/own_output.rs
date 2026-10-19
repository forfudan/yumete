//! **`ye --grep word . > out.txt`, run inside the directory it searches.**
//!
//! The search found `out.txt`, read what it had just written into it, wrote
//! those lines again, and grew for as long as the disk lasted: 2026-10-19 one
//! such line reached **315 GB** and took a 926 GB disk down to 1.6 GB free,
//! in the background, without a word. GNU grep has refused this since forever
//! (「input file X is also the output」) and this one did not.
//!
//! Driven through the real binary, because what is being tested is the
//! redirection: the guard asks `fstat(1)` for the device and inode behind this
//! process's stdout, and `cargo test`'s own stdout is a pipe.
//!
//! Warning: **the child's file size is capped at 1 MB** (`RLIMIT_FSIZE`). A
//! regression here is a process writing without bound, and a test for it must
//! not be able to do the damage it is testing for — so a regression dies of
//! `SIGXFSZ` and fails the assertion instead.

#![cfg(unix)]

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

/// A directory of this test's own, emptied first so a rerun starts clean.
fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("yumete-own-output-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a directory to search");
    dir
}

/// One run of `--grep`, with stdout redirected into a file **inside** `dir`.
fn grep_into(dir: &std::path::Path, out: &str) -> (u64, String) {
    let path = dir.join(out);
    let sink = std::fs::File::create(&path).expect("somewhere to write");
    let mut child = Command::new(env!("CARGO_BIN_EXE_yumete"));
    child
        .args(["--grep", "bravo", "."])
        .current_dir(dir)
        .stdout(Stdio::from(sink))
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    // The safety net: one megabyte, set in the child between fork and exec.
    unsafe {
        child.pre_exec(|| {
            let lim = libc::rlimit { rlim_cur: 1 << 20, rlim_max: 1 << 20 };
            match libc::setrlimit(libc::RLIMIT_FSIZE, &lim) {
                0 => Ok(()),
                _ => Err(std::io::Error::last_os_error()),
            }
        });
    }
    let mut run = child.spawn().expect("the binary to start");
    let mut said = String::new();
    if let Some(mut err) = run.stderr.take() {
        let _ = err.read_to_string(&mut said);
    }
    run.wait().expect("the binary to finish");
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    (size, said)
}

#[test]
fn a_search_does_not_read_the_file_its_own_output_is_going_to() {
    let dir = scratch("plain");
    std::fs::write(dir.join("a.md"), "bravo body\nbravo again\n").unwrap();

    let (size, said) = grep_into(&dir, "g.txt");

    // Two hits from the one file that has them, and nothing else. Before the
    // guard this did not finish at all.
    let out = std::fs::read_to_string(dir.join("g.txt")).unwrap();
    assert_eq!(out.lines().count(), 2, "two hits, in a.md only: {out:?}");
    assert!(out.lines().all(|l| l.contains("a.md")), "{out:?}");
    assert!(size < 4096, "the output stayed small: {size} bytes");
    // **Said, not silent.** A search that skips a file and keeps quiet is the
    // one thing this tool may not do.
    assert!(said.contains("g.txt"), "stderr names the file it skipped: {said:?}");
    assert!(said.contains("where this output is going"), "{said:?}");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_file_is_known_by_its_identity_not_its_name() {
    let dir = scratch("symlink");
    std::fs::write(dir.join("a.md"), "bravo body\n").unwrap();
    // A second name for the output file. Matching on the name would miss it;
    // the device and inode do not.
    std::os::unix::fs::symlink("g.txt", dir.join("mirror.txt")).unwrap();

    let (size, said) = grep_into(&dir, "g.txt");

    let out = std::fs::read_to_string(dir.join("g.txt")).unwrap();
    assert_eq!(out.lines().count(), 1, "one hit, in a.md only: {out:?}");
    assert!(size < 4096, "{size} bytes");
    assert!(said.contains("where this output is going"), "{said:?}");

    let _ = std::fs::remove_dir_all(&dir);
}
