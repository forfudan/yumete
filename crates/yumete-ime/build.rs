//! Put 靈明精華版 into the binary at build time — **always the same table**,
//! whatever this machine happens to have installed.
//!
//! **Why not the installed one.** It used to prefer the full `ling.ytab` when
//! the build machine had it, and fall back to 精華版 when it did not. That
//! made the binary a function of the machine: 2026-10-07 判詞——「編譯不是跟着
//! 電腦走的！！！編譯應該是穩定的！！！爲什麽必須帶靈明精華版是爲了壓縮二進制
//! 尺寸！！！」 On the author's own machine it embedded 3.7 MB that **nothing
//! ever read**: at run time an installed 靈明 wins over whichever table is in
//! the binary (`ImeSession::new`), so the full copy was pure weight. Now CI
//! (homebrew) and a developer's build carry the same 0.24 MB.
//!
//! **Where it comes from.** `yume` generates it and publishes it; this build
//! downloads it once and keeps it. 精華版 is every character in CJK 基本區 and
//! 擴展A plus the 字根區 and the seven 字集, all sources, the 簡碼 — but no 詞
//! (recipe in `scripts/make_jinghua.py`, which runs in the *yume* tree). Warning:
//! **It is not in this repository**: a pure data table is a build input, never
//! a tracked file — a binary blob does not delta, so committing it would make
//! the repository fatter with every refresh.
//!
//! **The cache is a `YUMETE_BUILTIN_DIR` this build fills in itself** — same
//! layout (`schemes/`, `data/`, `VERSION`), so there is one way to read these
//! files and not two. Delete it to take a newer 精華版; nothing expires on its
//! own, because a build that quietly changes what it embeds is the thing this
//! module is here to stop.
//!
//! `YUMETE_BUILTIN_DIR` still says to look **only** there, for a release build
//! that wants the tables from somewhere specific — and it is checked before
//! the cache, so it never reaches the network.
//!
//! **A machine with neither builds anyway**, and says so honestly rather than
//! failing: offline with an empty cache means no table in the binary, which is
//! what `cargo build` on a disconnected machine has always done.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

fn main() {
    println!("cargo:rerun-if-env-changed=YUMETE_BUILTIN_DIR");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    // **精華版，永遠**——裝好的那份完整表這裏不看，理由在模塊開頭。沒有就去取
    // 一次，取進一個自己填的 `YUMETE_BUILTIN_DIR`（同一套 `schemes/`＋`data/`
    // 的擺法），於是下面兩支 `find` 一個字都不用改。
    if std::env::var_os("YUMETE_BUILTIN_DIR").is_none() && find(LINGMING).is_none() {
        fetch_the_builtin();
    }
    let table = find(LINGMING);
    let xingchen = find(XINGCHEN);
    // The 符號表 is the same file either way — 精華版's is cut from the same
    // 15,716 rows — so there is no mixture to worry about.
    let symbols = find(SYMBOLS).or_else(|| find("symbols.ytab"));
    let mut body = String::new();
    body.push_str(&declare("BUILTIN_LINGMING", table.as_deref()));
    body.push_str(&declare("BUILTIN_XINGCHEN", xingchen.as_deref()));
    body.push_str(&declare("BUILTIN_SYMBOLS", symbols.as_deref()));
    // Say *which* table, not just how old. `:yume` prints this, and 「出廠自帶
    // 2026-09-12」 beside a candidate list with no 詞 in it is an answer that
    // sends the reader looking for a bug in 宇浩.
    // 永遠是精華版，所以這三個字無條件寫上去——從前這裏有個 `jinghua` 開關，是
    // 「裝了宇浩就嵌完整表」那個年代留下的（2026-10-07 撤了那條路）。
    let version = table.as_deref().and_then(stamp).map(|when| format!("精華版 {when}"));
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

/// **Watch `path` if it is there, or the `yumete` directory it would sit in.**
///
/// Warning: **Never hand cargo a path that is not there, and never walk above our
/// own directory.** Both mistakes make the build script rerun on **every**
/// cargo command, and everything downstream of this crate rebuild with it.
///
/// Measured 2026-09-27, with
/// `CARGO_LOG=cargo::core::compiler::fingerprint=info`:
///
/// * a `rerun-if-changed` on a **missing** file — cargo treats it as changed,
///   every time. That was the original;
/// * walking up to the nearest *existing* ancestor — which on macOS is
///   `~/Library/Application Support`, a directory **every app on the machine
///   writes into**. Cargo said it in one line:
///   `stale: changed "/Users/ZHU/Library/Application Support"`. That was the
///   first attempt at a fix, and it is no better.
///
/// Two `cargo test -p yumete-core --lib --no-run` in a row, nothing touched
/// between them, were **33 seconds each** — both recompiling `yumete-cjk` and
/// `yumete-core`. Every cargo command in the workspace paid it.
///
/// So: the file if it exists, else the `yumete` directory it belongs in (a
/// file appearing changes its directory's mtime, which is the case that
/// matters — `scripts/build.sh` installs the data *after* the first build),
/// else nothing at all. Warning: **With neither, a later install needs
/// `cargo clean -p {crate}`** — that is the honest price, and it is paid once
/// by whoever installs data onto a machine that had none.
fn watch(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
        return;
    }
    // `…/yumete/data/common_words.txt` → `…/yumete`. Never higher: one level
    // up from there is a directory the whole machine writes into.
    let ours = path.ancestors().find(|at| at.file_name() == Some(OsStr::new("yumete")));
    if let Some(dir) = ours.filter(|dir| dir.is_dir()) {
        println!("cargo:rerun-if-changed={}", dir.display());
    }
}

/// Where the installed Yume data lives, in the order the editor itself looks.
///
/// Warning: **Every candidate is watched, not just the one that answered**: the data
/// may be installed into any of them after this build, and then the next one
/// has to see it — `scripts/build.sh` builds the binary *before* it installs
/// the data, so the first run embeds 靈明精華版 and the second must notice the
/// full tables. Otherwise the binary keeps saying 「出廠自帶 精華版」 on a
/// machine that has them, until somebody runs `cargo clean`.
/// 出廠自帶那幾個檔，在 `yume-release` 上的名字與在這裏的擺法。
const LINGMING: &str = "schemes/lingming_essential.ytab";
const XINGCHEN: &str = "schemes/xingchen_essential.ytab";
const SYMBOLS: &str = "data/symbols.ytab";
const RELEASE: &str =
    "https://github.com/forfudan/yume-release/releases/download/yumete-data";

/// 取一次 精華版，放進快取。
///
/// **一次，而且三個檔要麼齊要麼不算**：`VERSION` 缺了 `:yume-where` 那一行就答
/// 不出版本，符號表缺了候選欄就沒有標點——半份比沒有更難查。取不到（離線、沒有
/// `curl`）就安安靜靜回去，二進制不帶表，照舊出聲。
fn fetch_the_builtin() {
    let Some(cache) = cache_dir() else { return };
    for dir in ["schemes", "data"] {
        if std::fs::create_dir_all(cache.join(dir)).is_err() {
            return;
        }
    }
    // **靈明那三個要麼齊要麼不算**，星陳是添頭：少了它星陳打不了字，少了那三個裏
    // 任何一個是連漢字都打不了。所以前者缺了收攤，後者缺了只說一聲。
    let mut got = Vec::new();
    for (asset, into, needed) in [
        ("lingming_essential.ytab", LINGMING, true),
        ("symbols.ytab", SYMBOLS, true),
        ("VERSION", "VERSION", true),
        ("xingchen_essential.ytab", XINGCHEN, false),
    ] {
        let to = cache.join(into);
        let ok = std::process::Command::new("curl")
            .args(["-fsSL", "--max-time", "60", "-o"])
            .arg(&to)
            .arg(format!("{RELEASE}/{asset}"))
            .status()
            .is_ok_and(|code| code.success())
            && to.metadata().is_ok_and(|m| m.len() > 0);
        if !ok {
            let _ = std::fs::remove_file(&to);
            if !needed {
                println!("cargo:warning=取不到 {asset}，這一份二進制不帶那個方案的出廠碼表");
                continue;
            }
            // 半份不留：下一趟編譯要麼乾淨地再取一次，要麼乾淨地沒有表。
            for one in got {
                let _ = std::fs::remove_file(cache.join(one));
            }
            println!("cargo:warning=取不到出廠自帶的碼表（{RELEASE}/{asset}），這一份二進制不帶表");
            return;
        }
        got.push(into);
    }
}

/// 快取擺在哪：`$XDG_CACHE_HOME/yumete/builtin`，沒有就 `~/.cache/…`。
///
/// 放在家目錄而不是 `target/`，因為 `cargo clean` 不該讓人重下一遍。
fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))?;
    Some(base.join("yumete").join("builtin"))
}

fn find(file: &str) -> Option<PathBuf> {
    // Set, and it is the whole list: a release build that names a directory
    // means *that* directory, and silently reaching past it to whatever the
    // build machine happens to have installed is how a package ends up
    // carrying a table nobody chose.
    if let Ok(dir) = std::env::var("YUMETE_BUILTIN_DIR") {
        let path = PathBuf::from(dir).join(file);
        watch(&path);
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
    // 自己填的那一份排在最後：裝好的和取回來的是同一個檔（`scripts/build.sh`
    // 印的那幾行 curl 就是往裝的地方放），誰先誰後都一樣，而排後面意味着這一支
    // 的老行為一個字都沒變。
    dirs.extend(cache_dir());
    let mut found = None;
    for path in dirs.into_iter().map(|dir| dir.join(file)) {
        watch(&path);
        if found.is_none() && path.is_file() {
            found = Some(path);
        }
    }
    found
}
