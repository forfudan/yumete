//! Put 靈明's 碼表 into the binary at build time, if it is on this machine.
//!
//! **Why at build time rather than in the repository.** The table is 3.7 MB of
//! compiled binary, and a binary blob does not delta: committing it would take
//! the repository from one megabyte to four, and another four with every
//! refresh, for a file that is *generated* from the 宇浩 source tree and is not
//! source. So it is read from wherever it is already installed, at the moment
//! the binary is built, and never stored here.
//!
//! **What that buys.** A yumete built by `scripts/build.sh` — which installs
//! the data first and then builds — carries 靈明 with it, so the binary that
//! ships can type 漢字 on a machine where nothing has been installed. A yumete
//! built by `cargo build` on a machine with no data simply has no built-in
//! table, and says so honestly rather than failing to build.
//!
//! `YUMETE_BUILTIN_DIR` overrides where to look, for a release build that
//! wants the tables from somewhere specific.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=YUMETE_BUILTIN_DIR");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let table = find("ling.ytab");
    let symbols = find("symbols.ytab");
    let mut body = String::new();
    body.push_str(&declare("BUILTIN_TABLE", table.as_deref()));
    body.push_str(&declare("BUILTIN_SYMBOLS", symbols.as_deref()));
    std::fs::write(out.join("builtin.rs"), body).expect("write builtin.rs");
}

/// One `Option<&[u8]>` constant, holding the file's bytes or nothing.
fn declare(name: &str, path: Option<&Path>) -> String {
    match path {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", path.display());
            format!(
                "pub const {name}: Option<&[u8]> = Some(include_bytes!({:?}));\n",
                path.display().to_string()
            )
        }
        None => format!("pub const {name}: Option<&[u8]> = None;\n"),
    }
}

/// Where the installed Yume data lives, in the order the editor itself looks.
fn find(file: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(dir) = std::env::var("YUMETE_BUILTIN_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Ok(data) = std::env::var("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(data).join("yumete"));
    }
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/yumete"));
        dirs.push(
            PathBuf::from(&home).join("Library/Application Support/yumete"),
        );
    }
    dirs.into_iter()
        .map(|dir| dir.join(file))
        .find(|path| path.is_file())
}
