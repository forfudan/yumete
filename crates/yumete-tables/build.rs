//! Put the full 宇浩拼音, 靈明 and 星陳 tables into the binary, compressed.
//!
//! **Why the full tables.** Not a 精華版 to keep the binary small: 拼音 is the scheme most people
//! type with, and it needs its language layer, so the binary carries the full tables of three
//! schemes and 拼音 is the default. The sentence model goes in whole, and every file is
//! zstd-compressed (pinyin's four files measured 54.1 MB raw, 26.2 MB packed, 61 ms to unpack).
//!
//! **Where they come from: the macOS package of the current Yume release**
//! (`forfudan/yume-release`, tag `yuhao`), the one Yume itself ships. The
//! data in it is what the input method installs, so the editor and the input
//! method agree on every table.
//!
//! **The cache decides, not the day.** The files are fetched once into
//! `$YUMETE_BUILTIN_DIR` when that is set (a release build), else into
//! `~/.cache/yumete/tables`, and kept. Nothing expires on its own: a build
//! that quietly changes what it embeds is what this module is here to stop.
//! Delete the directory to take a newer release. What is installed on this
//! machine is never read, so the binary does not depend on the machine.
//!
//! **A machine that cannot fetch builds anyway**, without tables, and says
//! so, and the next build tries again; a release build (`YUMETE_RELEASE`)
//! fails instead, because a package that silently cannot type is worse than
//! no package.

use std::io::Read;
use std::path::{Path, PathBuf};

/// The files carried, by manifest name. The package holds them under
/// `Yume.app/Contents/Resources/` with the same names.
const FILES: &[&str] = &[
    "data/pinyin.yflb",
    "data/lang.ywtb",
    "data/lang.ywl",
    "data/lang.ygram",
    "data/symbols.ytab",
    "data/chaifen.ydiv",
    "data/charsets/common.ycs",
    "data/charsets/tonggui.ycs",
    "data/charsets/harmonic.ycs",
    "data/charsets/tongfan.ycs",
    "data/charsets/guji.ycs",
    "data/charsets/tai.ycs",
    "data/charsets/gang.ycs",
    "data/words_yuling.ywrd",
    "data/simptrad.txt",
    "schemes/pinyin.toml",
    "schemes/lingming.toml",
    "schemes/xingchen.toml",
    "schemes/lingming.ytab",
    "schemes/xingchen.ytab",
    "schemes/zigen_lingming.yzg",
    "schemes/zigen_xingchen.yzg",
];

const RELEASE: &str = "https://api.github.com/repos/forfudan/yume-release/releases/tags/yuhao";
const INSIDE: &str = "Yume.app/Contents/Resources/";

fn main() {
    for name in ["YUMETE_BUILTIN_DIR", "YUMETE_RELEASE", "XDG_CACHE_HOME", "HOME", "LOCALAPPDATA"] {
        println!("cargo:rerun-if-env-changed={name}");
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let release = std::env::var_os("YUMETE_RELEASE").is_some();
    let dir = match std::env::var_os("YUMETE_BUILTIN_DIR") {
        Some(dir) => Some(PathBuf::from(dir)),
        None => cache_dir(),
    };
    let ready = dir.as_deref().is_some_and(|dir| {
        if !complete(dir) {
            if let Err(why) = fetch(dir) {
                println!("cargo:warning=no 宇浩 tables in this binary: {why}");
            }
        }
        complete(dir)
    });
    if !ready && release {
        panic!("a release build needs the 宇浩 tables, and they could not be fetched");
    }
    // **Not ready, so asked again on the next build**: cargo reruns a build
    // script whose watched file does not exist, and the stamp is written
    // last. Without this an offline first build stayed without tables for
    // good.
    if let Some(dir) = dir.as_deref().filter(|_| !ready) {
        println!("cargo:rerun-if-changed={}", dir.join("VERSION").display());
    }
    let mut body = String::from("pub static FILES: &[(&str, &[u8])] = &[\n");
    let mut version = None;
    if let Some(dir) = dir.as_deref().filter(|_| ready) {
        for name in FILES {
            let packed = pack(dir, name);
            println!("cargo:rerun-if-changed={}", packed.display());
            body.push_str(&format!(
                "    ({name:?}, include_bytes!({:?})),\n",
                packed.display().to_string()
            ));
        }
        let stamp = dir.join("VERSION");
        println!("cargo:rerun-if-changed={}", stamp.display());
        version = std::fs::read_to_string(&stamp).ok().and_then(|text| {
            text.lines()
                .find_map(|l| l.trim().strip_prefix("build="))
                .map(readable)
        });
    }
    body.push_str("];\n");
    body.push_str(&format!("pub const VERSION: Option<&str> = {version:?};\n"));
    std::fs::write(out.join("tables.rs"), body).expect("write tables.rs");
}

/// Whether every file is in `dir`, with its stamp.
fn complete(dir: &Path) -> bool {
    dir.join("VERSION").is_file()
        && FILES
            .iter()
            .all(|name| at(dir, name).metadata().is_ok_and(|m| m.len() > 0))
}

fn at(dir: &Path, name: &str) -> PathBuf {
    name.split('/').fold(dir.to_path_buf(), |path, part| path.join(part))
}

/// `name`, zstd-compressed beside itself, made once.
fn pack(dir: &Path, name: &str) -> PathBuf {
    let raw = at(dir, name);
    let packed = raw.with_file_name(format!(
        "{}.zst",
        raw.file_name().unwrap().to_string_lossy()
    ));
    let fresh = |p: &Path| p.metadata().and_then(|m| m.modified()).ok();
    if packed.is_file() && fresh(&packed) >= fresh(&raw) {
        return packed;
    }
    let bytes = std::fs::read(&raw).expect("read a table");
    let squeezed = zstd::encode_all(&bytes[..], 19).expect("compress a table");
    write_whole(&packed, &squeezed).expect("write a compressed table");
    packed
}

/// Write `bytes` beside `path` and move them into place, so another build
/// sharing the cache never reads half a file.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let part = path.with_file_name(format!(".{name}.{}", std::process::id()));
    std::fs::write(&part, bytes)?;
    std::fs::rename(&part, path)
}

/// Fetch the current macOS package and take the tables out of it.
///
/// All or nothing: the files are written under their own names only after
/// every one of them was read out of the package, so a half-fetched set is
/// never mistaken for a whole one.
fn fetch(dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let listing = curl(RELEASE, None)?;
    let listing = String::from_utf8_lossy(&listing);
    // The asset is renamed with every release, so it is found by its shape:
    // `Yume-v<version>-<14-digit build>-macos-arm64.zip`.
    let url = listing
        .split('"')
        .find(|s| s.starts_with("https://") && s.ends_with("-macos-arm64.zip"))
        .ok_or("no macOS package on the yuhao release")?
        .to_string();
    let zip = dir.join(format!("package.{}.zip", std::process::id()));
    curl(&url, Some(&zip))?;
    let opened = std::fs::File::open(&zip).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(opened).map_err(|e| e.to_string())?;
    let mut taken = Vec::new();
    for name in FILES {
        let mut entry = archive
            .by_name(&format!("{INSIDE}{name}"))
            .map_err(|_| format!("{name} is not in the package"))?;
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.is_empty() {
            return Err(format!("{name} is empty in the package"));
        }
        taken.push((name, bytes));
    }
    let _ = std::fs::remove_file(&zip);
    for (name, bytes) in taken {
        let path = at(dir, name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        write_whole(&path, &bytes).map_err(|e| e.to_string())?;
    }
    // `Yume-v3.14.0-beta.20261029163103-macos-arm64.zip` → the version and the
    // build stamp, in the form Yume's own `VERSION` files take.
    let file = url.rsplit('/').next().unwrap_or_default();
    let middle = file
        .strip_prefix("Yume-v")
        .and_then(|s| s.strip_suffix("-macos-arm64.zip"))
        .unwrap_or_default();
    let (version, build) = middle.rsplit_once(['-', '.']).unwrap_or((middle, ""));
    let stamp = format!("version={version}\nbuild={build}\nfrom={file}\n");
    write_whole(&dir.join("VERSION"), stamp.as_bytes()).map_err(|e| e.to_string())
}

/// `curl` the URL, into `to` or back as bytes. A token in `GITHUB_TOKEN` is
/// passed to the API, so a CI runner sharing an address is not rate limited;
/// it goes in on standard input, never on the command line, where `ps`
/// would show it.
fn curl(url: &str, to: Option<&Path>) -> Result<Vec<u8>, String> {
    use std::io::Write;
    use std::process::Stdio;
    let mut command = std::process::Command::new("curl");
    command.args(["-fsSL", "--retry", "3", "--max-time", "600", "-K", "-"]);
    if let Some(to) = to {
        command.arg("-o").arg(to);
    }
    let mut config = String::new();
    if url.starts_with("https://api.github.com/") {
        if let Ok(token) = std::env::var("GITHUB_TOKEN") {
            config = format!("header = \"Authorization: Bearer {token}\"\n");
        }
    }
    let mut child = command
        .arg(url)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("curl: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(config.as_bytes()).map_err(|e| format!("curl: {e}"))?;
    }
    let out = child.wait_with_output().map_err(|e| format!("curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl {url}: {}", String::from_utf8_lossy(&out.stderr).trim()));
    }
    Ok(out.stdout)
}

/// `$XDG_CACHE_HOME/yumete/tables`, else `~/.cache/yumete/tables`. In the
/// home directory rather than `target/`, so `cargo clean` does not mean
/// fetching 54 MB again.
fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from))?;
    Some(base.join("yumete").join("tables"))
}

/// Yume's `20261029163103` as `2026-10-29 16:31`.
fn readable(stamp: &str) -> String {
    let d: String = stamp.chars().filter(char::is_ascii_digit).collect();
    if d.len() < 12 {
        return stamp.to_string();
    }
    format!("{}-{}-{} {}:{}", &d[0..4], &d[4..6], &d[6..8], &d[8..10], &d[10..12])
}
