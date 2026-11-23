//! **Which book a Typst file belongs to.**
//!
//! A Typst document is a book: one main file that pulls its chapters in with
//! `#include` and its setup with `#import`. A chapter typeset alone lacks what
//! its book sets up, so a preview of a chapter typesets the book. The book is
//! found by reading the Typst files in the chapter's folder and the folders
//! above it, up to the project's root, and following who names whom up to
//! the file nobody names. The open buffers are read as they stand.
//!
//! The answers are kept until a Typst file is saved or opened: working them
//! out reads the disk, and the focus moves every turn. Until then each
//! folder is listed, and each file read, once, so the books of several open
//! files worked out after one save share one search.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How many folders the search climbs from a file's own, at most.
const CLIMB: usize = 8;
/// How many Typst files of one folder are read, at most.
const PER_FOLDER: usize = 400;
/// The largest Typst file read: anything larger is not a book's skeleton.
const LARGEST: u64 = 4 << 20;

/// What a file read from the disk names, by the size and time it had.
type Named = (Option<(u64, SystemTime)>, Vec<PathBuf>);

/// **The books worked out so far**, and what each file read names.
#[derive(Debug, Default)]
pub(crate) struct Books {
    /// What the open Typst buffers were when these answers were worked out.
    fingerprint: u64,
    /// The topmost file above each file asked about.
    top: HashMap<PathBuf, PathBuf>,
    /// Every file each main file pulls in, itself included.
    within: HashMap<PathBuf, HashSet<PathBuf>>,
    /// What each file read from the disk names, kept across fingerprints:
    /// read again only when its size or time changes.
    named: HashMap<PathBuf, Named>,
    /// What each file names, as read under this fingerprint: neither the
    /// buffer nor the disk is asked twice.
    read: HashMap<PathBuf, Vec<PathBuf>>,
    /// The Typst files of each folder listed under this fingerprint.
    listed: HashMap<PathBuf, Vec<PathBuf>>,
    /// The folders climbed from each folder under this fingerprint.
    climbed: HashMap<PathBuf, Vec<PathBuf>>,
}

/// An open Typst buffer: its file, as the disk spells it, and its text,
/// shared with the buffer.
pub(crate) struct Open {
    pub path: PathBuf,
    pub text: ropey::Rope,
}

impl Books {
    /// Drop the answers when the open Typst buffers changed since: a save
    /// or a file opened.
    pub(crate) fn note(&mut self, fingerprint: u64) {
        if self.fingerprint != fingerprint {
            self.fingerprint = fingerprint;
            self.top.clear();
            self.within.clear();
            self.read.clear();
            self.listed.clear();
            self.climbed.clear();
        }
    }

    /// The answer kept for `file`, if there is one.
    pub(crate) fn kept_top(&self, file: &Path) -> Option<PathBuf> {
        self.top.get(file).cloned()
    }

    /// Whether the answer for what `main` pulls in is kept.
    pub(crate) fn has_within(&self, main: &Path) -> bool {
        self.within.contains_key(main)
    }

    /// Whether `main`'s book, as kept, holds `file`.
    pub(crate) fn kept_within(&self, main: &Path, file: &Path) -> bool {
        self.within.get(main).is_some_and(|all| all.contains(file))
    }

    /// **The topmost file above `file`**: who names it, who names that, up
    /// to the file nobody names; `file` itself when nobody does. Every path
    /// as the disk spells it.
    pub(crate) fn top_of(&mut self, file: &Path, open: &[Open], home: Option<&Path>) -> PathBuf {
        if let Some(top) = self.top.get(file) {
            return top.clone();
        }
        let mut candidates: Vec<PathBuf> = Vec::new();
        let here = file.parent().unwrap_or(Path::new("")).to_path_buf();
        let folders = self.climbed.entry(here).or_insert_with(|| folders_up(file, home)).clone();
        for folder in folders {
            let files = self.listed.entry(folder).or_insert_with_key(|folder| typst_files_in(folder));
            candidates.extend(files.iter().cloned());
        }
        candidates.extend(open.iter().map(|o| o.path.clone()));
        candidates.sort();
        candidates.dedup();
        let mut namers: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        for c in &candidates {
            for name in self.names_of(c, open) {
                if name != *c {
                    namers.entry(name).or_default().push(c.clone());
                }
            }
        }
        let mut here = file.to_path_buf();
        let mut seen = HashSet::from([here.clone()]);
        while let Some(up) = namers.get(&here).and_then(|ups| ups.iter().find(|up| !seen.contains(*up))) {
            here = up.clone();
            seen.insert(here.clone());
        }
        self.top.insert(file.to_path_buf(), here.clone());
        here
    }

    /// **Every file `main` pulls in**, itself and the files those pull in.
    pub(crate) fn within(&mut self, main: &Path, open: &[Open]) -> &HashSet<PathBuf> {
        if !self.within.contains_key(main) {
            let mut all = HashSet::from([main.to_path_buf()]);
            let mut todo = vec![main.to_path_buf()];
            while let Some(next) = todo.pop() {
                for name in self.names_of(&next, open) {
                    if all.insert(name.clone()) {
                        todo.push(name);
                    }
                }
            }
            self.within.insert(main.to_path_buf(), all);
        }
        &self.within[main]
    }

    /// What `file` names: the open buffer's text when it is open, else the
    /// disk's, read again only when the file changed; once a fingerprint.
    fn names_of(&mut self, file: &Path, open: &[Open]) -> Vec<PathBuf> {
        if let Some(names) = self.read.get(file) {
            return names.clone();
        }
        let names = self.names_read(file, open);
        self.read.insert(file.to_path_buf(), names.clone());
        names
    }

    /// [`Self::names_of`], asked of the buffer or the disk.
    fn names_read(&mut self, file: &Path, open: &[Open]) -> Vec<PathBuf> {
        let dir = file.parent().unwrap_or(Path::new(""));
        if let Some(o) = open.iter().find(|o| o.path == file) {
            return named_on(o.text.lines().map(Cow::from), dir);
        }
        let Ok(meta) = std::fs::metadata(file) else { return Vec::new() };
        let stamp = meta.modified().ok().map(|t| (meta.len(), t));
        if let Some((kept, names)) = self.named.get(file) {
            if *kept == stamp && stamp.is_some() {
                return names.clone();
            }
        }
        let names = match meta.len() <= LARGEST {
            true => std::fs::read_to_string(file).map(|text| named_in(&text, dir)).unwrap_or_default(),
            false => Vec::new(),
        };
        self.named.insert(file.to_path_buf(), (stamp, names.clone()));
        names
    }
}

/// **The folders a file's book is looked for in**: its own and the ones
/// above it, up to the project's root — the first folder holding `.git` or
/// `typst.toml` — and never above the home folder or past [`CLIMB`].
pub(crate) fn folders_up(file: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut folder = file.parent().filter(|f| !f.as_os_str().is_empty());
    while let Some(here) = folder {
        out.push(here.to_path_buf());
        let root = here.join(".git").exists() || here.join("typst.toml").is_file();
        if root || out.len() > CLIMB || Some(here) == home {
            break;
        }
        folder = here.parent().filter(|up| !up.as_os_str().is_empty() && Some(*up) != home);
    }
    out
}

/// The Typst files of `folder`, as the disk spells them, in name order.
fn typst_files_in(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "typ") && p.is_file())
        .take(PER_FOLDER)
        .collect();
    out.sort();
    out
}

/// **The files a Typst text names** with `#include` or `#import`, read from
/// `dir`, as the disk spells them; a name that is no file is left out.
pub(crate) fn named_in(text: &str, dir: &Path) -> Vec<PathBuf> {
    named_on(text.lines().map(Cow::Borrowed), dir)
}

/// [`named_in`], of a text's lines.
fn named_on<'a>(lines: impl Iterator<Item = Cow<'a, str>>, dir: &Path) -> Vec<PathBuf> {
    lines
        .filter_map(|line| super::quoted_path(&line))
        .filter_map(|name| std::fs::canonicalize(dir.join(name)).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn place(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yumete-books-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::canonicalize(&dir).unwrap()
    }

    /// **A chapter's book is the file nobody names**, followed up through a
    /// part that names it, from a folder below; a file nobody names is its
    /// own book; a cycle ends.
    #[test]
    fn a_chapter_climbs_to_its_book() {
        let dir = place("climb");
        std::fs::create_dir_all(dir.join("parts")).unwrap();
        std::fs::write(dir.join(".git"), "").unwrap();
        std::fs::write(dir.join("book.typ"), "#import \"setup.typ\": *\n#include \"parts/one.typ\"\n").unwrap();
        std::fs::write(dir.join("setup.typ"), "#let x = 1\n").unwrap();
        std::fs::write(dir.join("parts/one.typ"), "#include \"ch.typ\"\n").unwrap();
        std::fs::write(dir.join("parts/ch.typ"), "= 一\n").unwrap();
        std::fs::write(dir.join("letter.typ"), "= 信\n").unwrap();
        std::fs::write(dir.join("a.typ"), "#include \"b.typ\"\n").unwrap();
        std::fs::write(dir.join("b.typ"), "#include \"a.typ\"\n").unwrap();
        let mut books = Books::default();
        assert_eq!(books.top_of(&dir.join("parts/ch.typ"), &[], None), dir.join("book.typ"));
        assert_eq!(books.top_of(&dir.join("setup.typ"), &[], None), dir.join("book.typ"), "what it imports");
        assert_eq!(books.top_of(&dir.join("letter.typ"), &[], None), dir.join("letter.typ"), "nobody names it");
        let top = books.top_of(&dir.join("a.typ"), &[], None);
        assert!(top == dir.join("b.typ") || top == dir.join("a.typ"), "a cycle ends: {}", top.display());
        let within = books.within(&dir.join("book.typ"), &[]).clone();
        assert!(within.contains(&dir.join("parts/ch.typ")) && within.contains(&dir.join("setup.typ")));
        assert!(!within.contains(&dir.join("letter.typ")));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// **The open buffer is read as it stands**, and the answers are kept
    /// until the fingerprint changes.
    #[test]
    fn an_open_buffer_is_read_and_answers_are_kept() {
        let dir = place("open");
        std::fs::write(dir.join("typst.toml"), "").unwrap();
        std::fs::write(dir.join("main.typ"), "= 書\n").unwrap();
        std::fs::write(dir.join("ch.typ"), "= 章\n").unwrap();
        let mut books = Books::default();
        books.note(1);
        assert_eq!(books.top_of(&dir.join("ch.typ"), &[], None), dir.join("ch.typ"));
        let open = [Open { path: dir.join("main.typ"), text: ropey::Rope::from_str("#include \"ch.typ\"\n") }];
        assert_eq!(books.top_of(&dir.join("ch.typ"), &open, None), dir.join("ch.typ"), "kept");
        books.note(2);
        assert_eq!(books.top_of(&dir.join("ch.typ"), &open, None), dir.join("main.typ"), "worked out again");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// **The search stops at the project's root and below home.**
    #[test]
    fn the_search_stops_at_the_root_and_home() {
        let dir = place("roots");
        std::fs::create_dir_all(dir.join("proj/a/b")).unwrap();
        std::fs::write(dir.join("proj/typst.toml"), "").unwrap();
        let file = dir.join("proj/a/b/x.typ");
        assert_eq!(folders_up(&file, None), vec![dir.join("proj/a/b"), dir.join("proj/a"), dir.join("proj")]);
        let home = dir.join("proj/a");
        assert_eq!(folders_up(&file, Some(&home)), vec![dir.join("proj/a/b")], "not into home");
        assert_eq!(folders_up(&home.join("y.typ"), Some(&home)), vec![home.clone()], "a file in home reads home");
        std::fs::remove_dir_all(&dir).ok();
    }
}
