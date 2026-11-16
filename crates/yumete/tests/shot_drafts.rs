//! **`--shot` draws the same frame whatever a crash left beside the file.**
//!
//! A recovery copy another session left (`.a.md.<pid>.yumete-swp`) makes the
//! editor ask about it on opening; a picture of the keys pressed must not
//! carry that question on one machine and not on another. The copy itself is
//! left where it is, for the next session that does read it.
//!
//! Driven through the real binary, like [`inert_notice`](../inert_notice.rs).

use std::process::Command;

fn frame(path: &std::path::Path, home: &std::path::Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_yumete"))
        .arg(path)
        .arg("--lang=en")
        .arg("--shot=80x12")
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .output()
        .expect("the binary to run");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn a_left_recovery_copy_is_not_on_the_frame() {
    let dir = std::env::temp_dir().join(format!("yumete-shot-drafts-{}", std::process::id()));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).expect("a directory");
    let path = dir.join("a.md");
    std::fs::write(&path, "one\ntwo\n").expect("a file to open");
    let clean = frame(&path, &home);

    let swap = dir.join(".a.md.999991.yumete-swp");
    std::fs::write(&swap, "one\ntwo\nunsaved\n").expect("a recovery copy");
    let unnamed = home.join("data").join("yumete").join("drafts");
    std::fs::create_dir_all(&unnamed).expect("a drafts directory");
    std::fs::write(unnamed.join("scratch-999991-1.yumete-swp"), "lost\n").expect("an unnamed draft");
    let left = frame(&path, &home);

    assert!(clean.contains("one"), "{clean}");
    assert!(!left.contains("draft"), "the left copies reached the frame:\n{left}");
    assert_eq!(left, clean, "the same keys drew another frame");
    assert!(swap.exists(), "the copy is left for a session that reads it");
    let _ = std::fs::remove_dir_all(&dir);
}
