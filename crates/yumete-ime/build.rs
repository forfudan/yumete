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
    body.push_str(&format!(
        "pub const BUILTIN_VERSION: Option<&str> = {:?};\n",
        table.as_deref().and_then(stamp)
    ));
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

/// Which build of yume these tables came from.
///
/// Yume's own release packages carry a `VERSION` file beside the tables —
/// `version=3.12.0`, `build=20260828130838` — so the answer is authoritative
/// rather than guessed from a file name. A data directory assembled by
/// `scripts/build.sh` has no such file, and there the table's own timestamp is
/// the honest answer: it says *when*, which is what the question is really
/// asking.
fn stamp(table: &Path) -> Option<String> {
    let dir = table.parent()?;
    if let Ok(text) = std::fs::read_to_string(dir.join("VERSION")) {
        println!("cargo:rerun-if-changed={}", dir.join("VERSION").display());
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.trim().strip_prefix(key))
                .map(str::to_string)
        };
        if let Some(version) = field("version=") {
            return Some(match field("build=") {
                Some(build) => format!("{version} ({build})"),
                None => version,
            });
        }
    }
    // No VERSION: say when the table was made. A date, not a count of seconds
    // — the question is "how old is this", and 1788384005 does not answer it.
    let when = std::fs::metadata(table).ok()?.modified().ok()?;
    let secs = when.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(format!("自建 {}", date(secs)))
}

/// A Unix timestamp as `2026-09-02`.
///
/// Hinnant's civil-from-days, which is the whole of the calendar in a dozen
/// lines and saves a dependency for one string a year.
fn date(secs: u64) -> String {
    let days = (secs / 86_400) as i64 + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
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
