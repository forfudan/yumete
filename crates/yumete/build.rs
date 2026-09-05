//! The version string, stamped at build time.
//!
//! `0.1.0` alone cannot answer 「我跑的是哪一份」, and during a week of twenty
//! commits a day that is the first question every bug report has to answer.
//! So the binary carries three things:
//!
//! ```text
//! yumete 0.1.0-dev.20260905123000+bbc1485.dirty
//!        │     │   │              │       └ the tree had uncommitted changes
//!        │     │   │              └ the commit it was built from
//!        │     │   └ when it was built, local time
//!        │     └ not a release
//!        └ the version in Cargo.toml
//! ```
//!
//! The shape is SemVer: `-dev.…` is a pre-release (so `0.1.0-dev.x` sorts
//! *before* `0.1.0`, which is what an unreleased build should do), and `+…` is
//! build metadata. A release build — `YUMETE_RELEASE=1` — is the bare version
//! with nothing appended.
//!
//! **`.dirty` matters more than it looks.** Without it the hash is a lie
//! whenever the tree is edited but not committed, which here is most of the
//! time; with it, the hash is honestly a *starting point*.

use std::process::Command;

fn main() {
    // Any source anywhere, and the two files git touches on a commit. This
    // crate is a leaf — nothing depends on it — so re-running the script and
    // recompiling `main.rs` costs a relink that was happening anyway.
    println!("cargo:rerun-if-changed=../");
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
    println!("cargo:rerun-if-env-changed=YUMETE_RELEASE");

    let base = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".to_string());
    if std::env::var("YUMETE_RELEASE").is_ok_and(|v| v != "0" && !v.is_empty()) {
        println!("cargo:rustc-env=YUMETE_VERSION={base}");
        return;
    }

    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".to_string());
    let git = |args: &[&str]| -> Option<String> {
        let out = Command::new("git").arg("-C").arg(&root).args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
    };
    // Local time, not UTC: this number is read next to a wall clock and quoted
    // back in a sentence, not sorted across timezones.
    let stamp = Command::new("date")
        .arg("+%Y%m%d%H%M%S")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default();

    let mut version = base;
    if !stamp.is_empty() {
        version.push_str("-dev.");
        version.push_str(&stamp);
    }
    if let Some(hash) = git(&["rev-parse", "--short=7", "HEAD"]).filter(|h| !h.is_empty()) {
        version.push('+');
        version.push_str(&hash);
        if git(&["status", "--porcelain"]).is_some_and(|s| !s.is_empty()) {
            version.push_str(".dirty");
        }
    }
    println!("cargo:rustc-env=YUMETE_VERSION={version}");
}
