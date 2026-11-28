//! **`ye --import helix` opens the result, unsaved, where it would be
//! written**, and `:w` writes it there; the helix config it read is left as
//! it was.
//!
//! Driven through the real binary, with a home of the test's own: no
//! editor here ever sees the real `~/.config`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A directory of this test's own, emptied first so a rerun starts clean.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("yumete-import-opens-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `ye --import helix`, a frame drawn after `keys`, in `home`.
fn import(home: &Path, keys: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_yumete"))
        .args(["--import", "helix", "--lang=en", "--shot=80x16"])
        .arg(format!("--keys={keys}"))
        .env("HOME", home)
        .env("XDG_DATA_HOME", home.join("data"))
        .env("XDG_CONFIG_HOME", home.join("config"))
        .output()
        .expect("the binary to run");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

const HELIX: &str = "[editor]\nscrolloff = 7\n";

#[test]
fn an_import_opens_unsaved_and_w_writes_it() {
    let home = scratch("new");
    let helix = home.join("config").join("helix").join("config.toml");
    std::fs::create_dir_all(helix.parent().unwrap()).unwrap();
    std::fs::write(&helix, HELIX).unwrap();
    let ours = home.join("config").join("yumete").join("config.toml");

    let frame = import(&home, "");
    assert!(frame.contains("scrolloff = 7"), "the import is on the page:\n{frame}");
    assert!(frame.contains("[+]"), "changed, not written:\n{frame}");
    assert!(!ours.exists(), "nothing written before :w");

    import(&home, ":w\\n");
    let written = std::fs::read_to_string(&ours).expect(":w writes the config");
    assert!(written.contains("scrolloff = 7"), "{written}");
    assert!(written.starts_with("# Imported from helix"), "{written}");
    assert_eq!(std::fs::read_to_string(&helix).unwrap(), HELIX, "helix's config is only read");

    // **With a config there, the import goes beside it**, and :w writes
    // that file, never the config.
    let frame = import(&home, ":w\\n");
    let beside = home.join("config").join("yumete").join("config.imported.toml");
    assert!(frame.contains("scrolloff = 7"), "{frame}");
    assert_eq!(std::fs::read_to_string(&ours).unwrap(), written, "the config is left as it was");
    let second = std::fs::read_to_string(&beside).expect(":w writes the file beside it");
    assert!(second.contains("config.toml was already there"), "{second}");
    let _ = std::fs::remove_dir_all(&home);
}
