//! **The Traditional Chinese of the interface is made here.**
//!
//! The message files under `messages/` are written in English (`en`) and
//! Simplified Chinese (`zhs`, alias `sc`). Traditional (`zht`, aliases `tc`
//! and `zh`) is written by hand only where the conversion gets it wrong;
//! everywhere else it is made from `zhs` the way `:convert s c` (`` ` `` `c`)
//! makes it: OpenCC's `s2t`, then the 大陸通規 glyph table `src/glyphs_c.txt`
//! walked one character at a time, then the few words s2t gets wrong in this
//! text (`src/terms_c.txt`: 碼錶 for 碼表, 隻讀 for 只讀).
//!
//! A hand-written `zht` is checked the other way: back through the glyph
//! table, then `t2s`. Traditional to Simplified has no ambiguity worth the
//! name, so when the result is not the `zhs` beside it, one of the two was
//! edited without the other, and the build says which message. A message
//! whose `zht` differs on purpose says so with a `# tc on purpose` comment.
//!
//! What comes out is a copy of each file in `OUT_DIR`, every alias spelled
//! `zhs`/`zht` and every message carrying a `zht`; `src/messages.rs` reads
//! those copies. Nothing generated is written back into the source tree.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use ferrous_opencc::config::BuiltinConfig;
use ferrous_opencc::OpenCC;

fn main() {
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets it")).join("messages");
    std::fs::create_dir_all(&out).expect("OUT_DIR is writable");
    let glyphs_path = root.join("src").join("glyphs_c.txt");
    println!("cargo:rerun-if-changed={}", glyphs_path.display());
    let terms_path = root.join("src").join("terms_c.txt");
    println!("cargo:rerun-if-changed={}", terms_path.display());
    let terms = terms(&std::fs::read_to_string(&terms_path).expect("terms_c.txt is there"));
    println!("cargo:rerun-if-changed=messages");

    let s2t = OpenCC::from_config(BuiltinConfig::S2t).expect("s2t is built in");
    let t2s = OpenCC::from_config(BuiltinConfig::T2s).expect("t2s is built in");
    let glyphs = Glyphs::parse(&std::fs::read_to_string(&glyphs_path).expect("glyphs_c.txt is there"));

    let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("messages"))
        .expect("messages/ is there")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .collect();
    files.sort();
    for path in files {
        println!("cargo:rerun-if-changed={}", path.display());
        let text = std::fs::read_to_string(&path).expect("a message file reads");
        let name = path.file_name().expect("a file name").to_string_lossy().to_string();
        let made = process(&name, &text, &|zhs| fix_terms(&glyphs.forward(&s2t.convert(zhs)), &terms), &|zht| {
            t2s.convert(&glyphs.back(zht))
        });
        write_if_changed(&out.join(&name), &made);
    }

    // The Chinese lesson (`:tutor`) is written in Simplified too, and its
    // Traditional made the same way.
    let lesson_path = root.join("src").join("tutor_zhs.md");
    println!("cargo:rerun-if-changed={}", lesson_path.display());
    let lesson = std::fs::read_to_string(&lesson_path).expect("tutor_zhs.md is there");
    let made = fix_terms(&glyphs.forward(&s2t.convert(&lesson)), &terms);
    write_if_changed(&out.parent().expect("OUT_DIR").join("tutor_zht.md"), &made);
}

/// One file: aliases spelled out, a `zht` made where none was written, and a
/// warning for every written `zht` that does not come back as its `zhs`.
fn process(name: &str, text: &str, to_trad: &dyn Fn(&str) -> String, to_simp: &dyn Fn(&str) -> String) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let mut block: Vec<String> = Vec::new();
    let flush = |block: &mut Vec<String>, out: &mut String| {
        if block.is_empty() {
            return;
        }
        let field = |b: &[String], name: &str| -> Option<String> {
            let prefix = format!("{name} = ");
            b.iter().find_map(|l| l.trim().strip_prefix(&prefix).and_then(unquote))
        };
        let key = field(block, "key").unwrap_or_default();
        let zhs = field(block, "zhs");
        let zht = field(block, "zht");
        match (&zhs, &zht) {
            (Some(zhs), None) if !zhs.is_empty() => {
                let made = to_trad(zhs);
                let at = block.iter().position(|l| l.trim().starts_with("zhs = ")).unwrap_or(block.len() - 1);
                block.insert(at + 1, format!("zht = \"{made}\""));
            }
            // `# tc on purpose` above the key: a written `zht` that differs on
            // purpose (it shows character forms side by side, or names the
            // scripts), checked once by hand.
            (Some(zhs), Some(zht))
                if !zhs.is_empty() && !zht.is_empty() && !block.iter().any(|l| l.trim().starts_with("# tc on purpose")) =>
            {
                let back = to_simp(zht);
                if &back != zhs {
                    println!(
                        "cargo:warning=messages/{name}: {key}: the written tc does not convert back to the sc: tc \u{300c}{zht}\u{300d} gives \u{300c}{back}\u{300d}, sc is \u{300c}{zhs}\u{300d}"
                    );
                }
            }
            _ => {}
        }
        for line in block.drain(..) {
            out.push_str(&line);
            out.push('\n');
        }
    };
    for line in text.lines() {
        if line.trim() == "[[message]]" {
            flush(&mut block, &mut out);
        }
        block.push(spell_out(line));
    }
    flush(&mut block, &mut out);
    out
}

/// `wrong<TAB>right` a line, longest first, so a longer fix is not cut short
/// by a shorter one inside it.
fn terms(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = text
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| l.split_once('\t'))
        .map(|(a, b)| (a.trim().to_string(), b.trim().to_string()))
        .collect();
    out.sort_by_key(|(a, _)| std::cmp::Reverse(a.chars().count()));
    out
}

fn fix_terms(text: &str, terms: &[(String, String)]) -> String {
    terms.iter().fold(text.to_string(), |text, (wrong, right)| text.replace(wrong.as_str(), right))
}

/// `sc = …` is `zhs = …`; `tc = …` and `zh = …` are `zht = …`.
fn spell_out(line: &str) -> String {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    for (alias, name) in [("sc = ", "zhs = "), ("tc = ", "zht = "), ("zh = ", "zht = ")] {
        if let Some(rest) = trimmed.strip_prefix(alias) {
            return format!("{indent}{name}{rest}");
        }
    }
    line.to_string()
}

/// The contents of a `"…"`: the messages hold no escapes.
fn unquote(value: &str) -> Option<String> {
    Some(value.trim().strip_prefix('"')?.strip_suffix('"')?.to_string())
}

/// Write only when the content changed, so an unchanged message file does not
/// make `messages.rs` compile again.
fn write_if_changed(path: &Path, text: &str) {
    if std::fs::read_to_string(path).is_ok_and(|was| was == text) {
        return;
    }
    std::fs::write(path, text).expect("OUT_DIR is writable");
}

/// The 大陸通規 glyph table, read as `src/convert.rs` reads it: `舊<TAB>新` a
/// row (the first candidate wins), a one-field row a glyph the standard merged
/// two characters into, which goes forward only.
struct Glyphs {
    forward: HashMap<char, char>,
    back: HashMap<char, char>,
}

impl Glyphs {
    fn parse(text: &str) -> Glyphs {
        let one = |field: &str| {
            let mut chars = field.chars();
            let first = chars.next()?;
            chars.next().is_none().then_some(first)
        };
        let mut forward = HashMap::new();
        let mut rows: Vec<(char, char)> = Vec::new();
        let mut merged: HashSet<char> = HashSet::new();
        for line in text.lines() {
            let line = line.trim_end_matches('\r').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let mut parts = line.split('\t');
            let Some(from) = parts.next() else { continue };
            let Some(to) = parts.next() else {
                if let Some(c) = one(from.trim()) {
                    merged.insert(c);
                }
                continue;
            };
            let Some(from) = one(from) else { continue };
            let Some(to) = to.split_whitespace().next().and_then(one) else { continue };
            if from != to {
                forward.insert(from, to);
                rows.push((from, to));
            }
        }
        let mut back = HashMap::new();
        for (from, to) in rows {
            if !merged.contains(&to) {
                back.insert(to, from);
            }
        }
        Glyphs { forward, back }
    }

    fn forward(&self, text: &str) -> String {
        text.chars().map(|c| *self.forward.get(&c).unwrap_or(&c)).collect()
    }

    fn back(&self, text: &str) -> String {
        text.chars().map(|c| *self.back.get(&c).unwrap_or(&c)).collect()
    }
}
