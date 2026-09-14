//! Put 靈明's 碼表 into the binary at build time — the installed one when this
//! machine has it, and 靈明精華版 from this repository when it does not.
//!
//! **Why the full table is not in the repository.** It is 3.7 MB of compiled
//! binary, and a binary blob does not delta: committing it would take the
//! repository from one megabyte to four, and another four with every refresh,
//! for a file that is *generated* from the 宇浩 source tree and is not source.
//! So the full table is read from wherever it is already installed, at the
//! moment the binary is built, and never stored here.
//!
//! **What stands in when it is not there.** 靈明精華版 —
//! `schemes/lingming_essential.ytab`, 0.25 MB: every character in CJK 基本區
//! and 擴展A plus the 字根區 and the seven 字集, all sources, the 簡碼 — but no
//! 詞 (recipe and reasoning in `scripts/make_jinghua.py`). ⚠️ **It is not in
//! this repository either**: pure data tables are a build input, never a
//! tracked file. A release build downloads it from `forfudan/yume-release`
//! (public, no token); `scripts/build.sh` installs the *full* table instead,
//! which is better and is what a developer's build carries.
//!
//! It is the floor, never the ceiling: the full table wins when both are
//! there, and at run time an installed 靈明 wins over whichever one is in the
//! binary (`ImeSession::new`). **A machine with neither builds anyway**, and
//! says so honestly rather than failing.
//!
//! `YUMETE_BUILTIN_DIR` says to look **only** there, for a release build that
//! wants the tables from somewhere specific — an empty directory therefore
//! forces the bundled 精華版, which is how the fallback is tested.

use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=YUMETE_BUILTIN_DIR");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    // Where `yume_core::data_manifest` says they live: a scheme's own table
    // under `schemes/`, the shared 符號表 under `data/`. The old flat names are
    // still looked for, so a machine whose data directory predates yume's
    // split still builds with 靈明 in it.
    let full = find("schemes/ling.ytab").or_else(|| find("ling.ytab"));
    let jinghua = full.is_none();
    let table = match &full {
        Some(path) => Some(path.clone()),
        None => find("schemes/lingming_essential.ytab"),
    };
    // The 符號表 is the same file either way — 精華版's is cut from the same
    // 15,716 rows — so there is no mixture to worry about.
    let symbols = find("data/symbols.ytab").or_else(|| find("symbols.ytab"));
    let mut body = String::new();
    body.push_str(&declare("BUILTIN_TABLE", table.as_deref()));
    body.push_str(&declare("BUILTIN_SYMBOLS", symbols.as_deref()));
    // Say *which* table, not just how old. `:yume` prints this, and 「出廠自帶
    // 2026-09-12」 beside a candidate list with no 詞 in it is an answer that
    // sends the reader looking for a bug in 宇浩.
    let version = table.as_deref().and_then(stamp).map(|when| match jinghua {
        true => format!("精華版 {when}"),
        false => when,
    });
    body.push_str(&format!(
        "pub const BUILTIN_VERSION: Option<&str> = {version:?};\n"
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
    // Beside the tables, which since yume's `data/` + `schemes/` split is one
    // directory up from the table itself.
    let dir = table.parent()?;
    let version = [dir.join("VERSION"), dir.join("../VERSION")]
        .into_iter()
        .find(|p| p.is_file())
        .unwrap_or_else(|| dir.join("VERSION"));
    if let Ok(text) = std::fs::read_to_string(&version) {
        println!("cargo:rerun-if-changed={}", version.display());
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.trim().strip_prefix(key))
                .map(str::to_string)
        };
        // The build stamp alone. A version number says which release this
        // came from; what a reader actually wants to know is *how old is it*,
        // and the date answers that without them having to remember what
        // 3.12.0 was.
        if let Some(build) = field("build=") {
            return Some(readable(&build));
        }
    }
    // No VERSION: say when the table was made. A date, not a count of seconds
    // — the question is "how old is this", and 1788384005 does not answer it.
    let when = std::fs::metadata(table).ok()?.modified().ok()?;
    let secs = when.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some(date(secs))
}

/// Yume's own `20260828130838` as `2026-08-28 13:08`.
fn readable(stamp: &str) -> String {
    let digits: String = stamp.chars().filter(char::is_ascii_digit).collect();
    if digits.len() < 12 {
        return stamp.to_string();
    }
    format!(
        "{}-{}-{} {}:{}",
        &digits[0..4],
        &digits[4..6],
        &digits[6..8],
        &digits[8..10],
        &digits[10..12]
    )
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
/// ⚠️ **Every candidate is watched, not just the one that answered.** Cargo
/// treats a `rerun-if-changed` path that did not exist and now does as a
/// change, and that is exactly the case that matters: `scripts/build.sh`
/// builds the binary *before* it installs the data, so the first run embeds
/// 靈明精華版 — and without this the second run would not rebuild
/// `yumete-ime`, because nothing it watched had changed. The binary would keep
/// saying 「出廠自帶 精華版」 on a machine with the full tables installed,
/// until someone ran `cargo clean`.
fn find(file: &str) -> Option<PathBuf> {
    // Set, and it is the whole list: a release build that names a directory
    // means *that* directory, and silently reaching past it to whatever the
    // build machine happens to have installed is how a package ends up
    // carrying a table nobody chose.
    if let Ok(dir) = std::env::var("YUMETE_BUILTIN_DIR") {
        let path = PathBuf::from(dir).join(file);
        println!("cargo:rerun-if-changed={}", path.display());
        return path.is_file().then_some(path);
    }
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(data) = std::env::var("XDG_DATA_HOME") {
        dirs.push(PathBuf::from(data).join("yumete"));
    }
    if let Ok(home) = std::env::var("HOME") {
        dirs.push(PathBuf::from(&home).join(".local/share/yumete"));
        dirs.push(
            PathBuf::from(&home).join("Library/Application Support/yumete"),
        );
    }
    let mut found = None;
    for path in dirs.into_iter().map(|dir| dir.join(file)) {
        println!("cargo:rerun-if-changed={}", path.display());
        if found.is_none() && path.is_file() {
            found = Some(path);
        }
    }
    found
}
