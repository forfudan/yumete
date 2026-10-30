//! 作品百科 — a book's own encyclopaedia (#287, `development.md` §5.8).
//!
//! `.yumete/wiki.md` is a Markdown file whose `##`-and-deeper headings are
//! entries. A line **inside a comment** that begins `[yumete]` names another
//! file, read as if pasted where the line stands — so a wiki grows into
//! 人物.md, 地理.md, 門派.md without a new idea. Everything here is a function
//! of files and text; the editor decides when to read and what to do with it.
//!
//! **A folder of entries** (2026-10-30, the Obsidian shape): `.yumete/wiki/`
//! is read when it is there, every file in it one entry named by the file;
//! a `[yumete] 百科/` line (trailing slash) or `wiki: 百科/` in `wiki.md`'s
//! front matter names another such folder — `..` from `.yumete/wiki.md` is
//! the project itself, which turns every file name into a marked name. A
//! file's own headings are its sub-entries, and `aliases:` in its front
//! matter are more names for it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// At most this many files, and this deep, however the includes are written:
/// a bad edit must not stall the editor walking a tree.
const MAX_FILES: usize = 64;
const MAX_DEPTH: usize = 8;
/// A folder of entries is listed, not followed, so it may hold more: a project
/// named as its own wiki has every chapter in it.
const MAX_FOLDER_FILES: usize = 4096;
/// The folder read without being named — beside `wiki.md`.
pub const WIKI_DIR: &str = "wiki";

/// The file `:wiki-edit` makes, and the spelling a writer may have used instead.
pub const WIKI_MD: &str = "wiki.md";
const WIKI_TXT: &str = "wiki.txt";

/// How a wiki name is marked on the page (`:wiki hide|color|line`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mark {
    /// Not at all.
    Off,
    /// 金 ink, in both layouts — the factory answer (2026-09-17).
    #[default]
    Color,
    /// A dotted underline across; a ground (第 80 檔) down a 縱, where an
    /// underline would be a stack of dashes between the glyphs.
    Line,
}

/// One entry: a heading of depth two or more, and everything under it up to
/// the next heading that is not deeper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub depth: usize,
    /// The headings above it, outermost first — a `#` divider included, which
    /// is the divider's one job.
    pub ancestors: Vec<String>,
    /// The lines under the heading, sub-headings and all, as written.
    pub body: Vec<String>,
    /// Where the heading is, so a jump opens the file it is actually in.
    pub source: PathBuf,
    pub line: usize,
    /// Read from the global wiki rather than this book's.
    pub global: bool,
}

/// What happened to one file the wiki reached for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Read, and gave this many entries.
    Read { path: PathBuf, entries: usize },
    /// Named by a directive and not there.
    Missing { path: PathBuf },
    /// There, and unreadable — no permission, or not UTF-8. Warning: **Not the same
    /// as missing** (2026-09-19, caught in review): `read_to_string` folds
    /// ENOENT, EACCES and invalid UTF-8 into one `Err`, so a `wiki.md` in
    /// GB18030 — plausible for this audience — was reported as 「there is no
    /// such file」 about a file plainly sitting there. This is the feature's
    /// error channel; it has to name what really happened.
    Unreadable { path: PathBuf, why: String },
    /// Outside the book, absolute, or through a link that leaves it.
    Refused { named: String, from: PathBuf },
    /// Already read once; the second directive is ignored.
    Again { path: PathBuf },
    /// Past the file or depth cap.
    TooMany { named: String },
    /// A folder of entries: how many files it gave, and how many entries.
    Folder { path: PathBuf, files: Vec<PathBuf>, entries: usize },
}

/// Everything the wiki holds.
#[derive(Debug, Clone, Default)]
pub struct Wiki {
    pub entries: Vec<Entry>,
    /// Every entry of each name, book before global, then `(depth, order)`.
    pub by_name: HashMap<String, Vec<usize>>,
    pub sources: Vec<Source>,
    /// [`Self::files`], canonicalised — worked out **once, here**, because
    /// [`Self::came_from`] is asked every frame (2026-09-19). It used to
    /// canonicalise both sides of every comparison on the spot: two syscalls
    /// per wiki file per frame, ≈2.8 ms a frame on a book with sixty includes.
    canonical: Vec<PathBuf>,
}

impl Wiki {
    /// Read this book's wiki (if `book` is there) and the global one (if
    /// `global` is), book first.
    pub fn load(book: Option<&Path>, global: Option<&Path>) -> Wiki {
        let mut wiki = Wiki::default();
        for (root, is_global) in [(book, false), (global, true)] {
            let Some(root) = root else { continue };
            let Some(path) = found(root) else { continue };
            // Includes stay under the directory holding `.yumete/` — or, for
            // the global wiki, the directory it is kept in.
            // Warning: **Canonicalise the wiki's own path first** (2026-09-19,
            // caught in review). The bound is 「the directory holding
            // `.yumete/`」, taken as two parents up — and when the book was
            // opened by a *relative* path (`yumete ch.md`, which is how one is
            // usually opened) that is the empty path, `canonicalize("")` fails,
            // and `Path::starts_with("")` is true of everything. The same file
            // opened by an absolute path refused an include pointing out of the
            // book; opened by a relative one it read it. One of those two
            // answers had to be wrong, and it was the permissive one.
            //
            // An empty bound now **refuses everything** rather than admitting
            // it: a sandbox that cannot say where its wall is has no wall.
            let here = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            let bound = match is_global {
                false => here.parent().and_then(Path::parent),
                true => here.parent(),
            }
            .map(Path::to_path_buf)
            .unwrap_or_default();
            let mut reader = Reader {
                bound,
                seen: HashSet::new(),
                sources: Vec::new(),
                lines: Vec::new(),
                folders: Vec::new(),
                aliases: Vec::new(),
                groups: HashMap::new(),
            };
            if path.is_file() {
                reader.read(&path, 0);
            }
            // The folder beside `wiki.md` is read unasked; a `wiki.md` is not
            // needed for it (the Obsidian user has only the folder).
            let beside = path.with_file_name(WIKI_DIR);
            if beside.is_dir() {
                reader.folder(&path, WIKI_DIR, &beside);
            }
            let before = wiki.entries.len();
            wiki.entries.extend(entries(&reader.lines, is_global));
            // An entry file's group is its folders, not whatever `#` the lines
            // before it left in force; the divider it starts with only resets
            // that and is dropped here.
            for entry in &mut wiki.entries[before..] {
                if let Some(group) = reader.groups.get(&entry.source) {
                    let own = entry.ancestors.iter().skip(1).cloned();
                    entry.ancestors = group.iter().cloned().chain(own).collect();
                }
            }
            // An alias is the same entry under another name — a copy, so the
            // file's own sections are not entries twice.
            for (file, alias) in &reader.aliases {
                let Some(own) = wiki.entries[before..]
                    .iter()
                    .find(|e| e.depth == 2 && e.line == 0 && &e.source == file)
                    .cloned()
                else {
                    continue;
                };
                wiki.entries.push(Entry { name: alias.clone(), ..own });
            }
            let mut counts: HashMap<PathBuf, usize> = HashMap::new();
            for entry in &wiki.entries[before..] {
                *counts.entry(entry.source.clone()).or_default() += 1;
            }
            for source in &mut reader.sources {
                if let Source::Read { path, entries } = source {
                    *entries = counts.get(path).copied().unwrap_or(0);
                }
            }
            for (folder, files) in reader.folders {
                let entries = files.iter().map(|f| counts.get(f).copied().unwrap_or(0)).sum();
                wiki.sources.push(Source::Folder { path: folder, files, entries });
            }
            wiki.sources.extend(reader.sources);
        }
        for (i, entry) in wiki.entries.iter().enumerate() {
            wiki.by_name.entry(entry.name.clone()).or_default().push(i);
        }
        let entries = &wiki.entries;
        for list in wiki.by_name.values_mut() {
            list.sort_by_key(|&i| (entries[i].global, entries[i].depth, i));
        }
        wiki.canonical =
            wiki.files().iter().filter_map(|p| std::fs::canonicalize(p).ok()).collect();
        wiki
    }

    /// Whether `path` is one of the files this was read from — **the same
    /// file**, however it is spelled (`./x` and an absolute path are one).
    ///
    /// Warning: Both sides have to canonicalise for the canonical answer to count.
    /// Comparing `canonicalize(a).ok() == canonicalize(b).ok()` calls two
    /// missing files equal — `None == None` — so an unsaved buffer and a wiki
    /// that had since been deleted read as the same file.
    pub fn came_from(&self, path: &Path) -> bool {
        self.files().contains(&path)
            || std::fs::canonicalize(path)
                .is_ok_and(|real| self.canonical.contains(&real))
    }

    /// Every file the wiki was read from — a save of any of them reloads it.
    pub fn files(&self) -> Vec<&Path> {
        self.sources
            .iter()
            .flat_map(|s| match s {
                Source::Read { path, .. } => vec![path.as_path()],
                Source::Folder { files, .. } => files.iter().map(PathBuf::as_path).collect(),
                _ => Vec::new(),
            })
            .collect()
    }

    /// The folders of entries — a file **saved into** one is a new entry, so
    /// a save under any of them reloads the wiki, listed or not yet.
    pub fn folders(&self) -> Vec<&Path> {
        self.sources
            .iter()
            .filter_map(|s| match s {
                Source::Folder { path, .. } => Some(path.as_path()),
                _ => None,
            })
            .collect()
    }

    /// Whether `path` lies in one of the folders of entries.
    pub fn in_a_folder(&self, path: &Path) -> bool {
        // The file may not be there yet (an unsaved new entry): its folder is.
        let real = std::fs::canonicalize(path).unwrap_or_else(|_| {
            match (path.parent().map(std::fs::canonicalize), path.file_name()) {
                (Some(Ok(parent)), Some(name)) => parent.join(name),
                _ => path.to_path_buf(),
            }
        });
        self.folders().iter().any(|f| {
            let folder = std::fs::canonicalize(f).unwrap_or_else(|_| f.to_path_buf());
            real.starts_with(&folder) && is_entry_file(&real)
        })
    }

    /// The names that can join the segmenter — two characters or more.
    pub fn words(&self) -> impl Iterator<Item = &str> {
        self.by_name.keys().map(String::as_str).filter(|n| n.chars().count() >= 2)
    }

    /// Names that are entries but can never be marked in the prose.
    pub fn unmarkable(&self) -> Vec<&str> {
        let mut out: Vec<&str> =
            self.by_name.keys().map(String::as_str).filter(|n| n.chars().count() < 2).collect();
        out.sort_unstable();
        out
    }
}

/// `wiki.md`, or `wiki.txt` when that is what the writer named it.
fn found(root: &Path) -> Option<PathBuf> {
    if root.is_file() {
        return Some(root.to_path_buf());
    }
    let dir = root.parent()?;
    [WIKI_MD, WIKI_TXT].iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

/// The lines of the whole wiki, includes spliced in, each with where it came
/// from.
struct Reader {
    bound: PathBuf,
    seen: HashSet<PathBuf>,
    sources: Vec<Source>,
    lines: Vec<(String, PathBuf, usize)>,
    /// Each folder of entries and the files it gave, in the order read.
    folders: Vec<(PathBuf, Vec<PathBuf>)>,
    /// `(file, alias)`: more names for a file's entry, from its front matter.
    aliases: Vec<(PathBuf, String)>,
    /// Each entry file's group: the folders between its folder of entries and
    /// it, outermost first.
    groups: HashMap<PathBuf, Vec<String>>,
}

impl Reader {
    fn read(&mut self, path: &Path, depth: usize) {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if !self.seen.insert(canonical.clone()) {
            self.sources.push(Source::Again { path: path.to_path_buf() });
            return;
        }
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(why) if why.kind() == std::io::ErrorKind::NotFound => {
                self.sources.push(Source::Missing { path: path.to_path_buf() });
                return;
            }
            Err(why) => {
                self.sources
                    .push(Source::Unreadable { path: path.to_path_buf(), why: why.to_string() });
                return;
            }
        };
        self.sources.push(Source::Read { path: path.to_path_buf(), entries: 0 });
        let lines: Vec<&str> = text.lines().collect();
        // `wiki: 百科/` in the front matter names a folder of entries; the
        // front matter itself is metadata, not an entry's body.
        let (front, body_from) = front_matter(&lines);
        for folder in front.iter().filter_map(|(k, v)| (*k == "wiki").then_some(*v)) {
            self.include(path, folder, depth);
        }
        for (n, line, directive) in scan(&lines[body_from..]) {
            let n = n + body_from;
            match directive {
                None => self.lines.push((line.to_string(), path.to_path_buf(), n)),
                Some(named) => self.include(path, named, depth),
            }
        }
    }

    /// Every entry file under `dir`, one entry each, named by the file; a
    /// sub-folder's name is the group heading above its files.
    fn folder(&mut self, from: &Path, named: &str, dir: &Path) {
        let canonical = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        if self.folders.iter().any(|(f, _)| std::fs::canonicalize(f).ok().as_deref() == Some(&canonical)) {
            self.sources.push(Source::Again { path: dir.to_path_buf() });
            return;
        }
        let mut files: Vec<PathBuf> = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(here) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&here) else { continue };
            let mut found: Vec<PathBuf> = read.filter_map(|e| e.ok().map(|e| e.path())).collect();
            found.sort();
            for path in found {
                // Hidden directories hold the project's own machinery
                // (`.yumete/`, `.git/`), not entries — so a project named as
                // its own wiki does not read its wiki.md twice.
                let hidden = path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.'));
                if path.is_dir() && !hidden {
                    stack.push(path);
                } else if path.is_file() && is_entry_file(&path) {
                    if files.len() >= MAX_FOLDER_FILES {
                        self.sources.push(Source::TooMany { named: named.to_string() });
                        break;
                    }
                    files.push(tidy(&path));
                }
            }
        }
        files.sort();
        for file in &files {
            let real = std::fs::canonicalize(file).unwrap_or_else(|_| file.clone());
            if !self.seen.insert(real) {
                continue;
            }
            let text = match std::fs::read_to_string(file) {
                Ok(text) => text,
                Err(why) => {
                    self.sources.push(Source::Unreadable { path: file.clone(), why: why.to_string() });
                    continue;
                }
            };
            let group: Vec<String> = file
                .parent()
                .and_then(|p| p.strip_prefix(tidy(dir)).ok())
                .into_iter()
                .flat_map(Path::components)
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let (lines, aliases) = entry_file_lines(dir, file, &text);
            self.lines.extend(lines);
            self.aliases.extend(aliases.into_iter().map(|a| (file.clone(), a)));
            self.groups.insert(file.clone(), group);
        }
        let _ = from;
        self.folders.push((tidy(dir), files));
    }

    fn include(&mut self, from: &Path, named: &str, depth: usize) {
        if depth + 1 > MAX_DEPTH || self.seen.len() >= MAX_FILES {
            self.sources.push(Source::TooMany { named: named.to_string() });
            return;
        }
        let refused = || Source::Refused { named: named.to_string(), from: from.to_path_buf() };
        let wanted = Path::new(named);
        if wanted.is_absolute() {
            self.sources.push(refused());
            return;
        }
        let target = from.parent().unwrap_or(Path::new(".")).join(wanted);
        // Warning: `canonicalize` fails on a path that does not exist, so a typo must
        // not come out as a security refusal: the parent is resolved and the
        // name put back on it.
        let resolved = match std::fs::canonicalize(&target) {
            Ok(p) => p,
            Err(_) => match (target.parent().map(std::fs::canonicalize), target.file_name()) {
                (Some(Ok(parent)), Some(name)) => parent.join(name),
                _ => {
                    self.sources.push(Source::Missing { path: target });
                    return;
                }
            },
        };
        if self.bound.as_os_str().is_empty() || !resolved.starts_with(&self.bound) {
            self.sources.push(refused());
            return;
        }
        // A trailing slash names a folder of entries; so does a name that is
        // a directory on disk.
        if named.ends_with('/') || target.is_dir() {
            match target.is_dir() {
                true => self.folder(from, named, &target),
                false => self.sources.push(Source::Missing { path: target }),
            }
            return;
        }
        self.read(&target, depth + 1);
    }
}

/// `a/.yumete/../b` as `a/b` — for the report, not for the disk.
fn tidy(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir if out.file_name().is_some() => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// `.md` or `.txt`: what an entry file is, in a folder of them.
fn is_entry_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "md" || e == "txt" || e == "markdown")
}

/// `key: value` pairs of a leading `---` block, and the line the body starts
/// on. A list value (`aliases:` then `- 小名` lines, or `[a, b]`) comes back
/// as one pair per item.
fn front_matter<'a>(lines: &[&'a str]) -> (Vec<(&'a str, &'a str)>, usize) {
    if lines.first().map(|l| l.trim()) != Some("---") {
        return (Vec::new(), 0);
    }
    let Some(end) = lines.iter().skip(1).position(|l| l.trim() == "---") else { return (Vec::new(), 0) };
    let mut out = Vec::new();
    let mut list_key: Option<&str> = None;
    for line in &lines[1..=end] {
        if let Some(item) = line.strip_prefix(' ').or(Some(line)).and_then(|l| l.trim_start().strip_prefix("- ")) {
            if let Some(key) = list_key {
                let item = item.trim().trim_matches(['"', '\'']);
                if !item.is_empty() {
                    out.push((key, item));
                }
                continue;
            }
        }
        list_key = None;
        let Some((key, value)) = line.split_once(':') else { continue };
        let key = key.trim();
        let value = value.trim().trim_matches(['"', '\'']);
        if key.is_empty() || !key.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
            continue;
        }
        if value.is_empty() {
            list_key = Some(key);
        } else if let Some(list) = value.strip_prefix('[').and_then(|v| v.strip_suffix(']')) {
            out.extend(list.split(',').map(|v| v.trim().trim_matches(['"', '\''])).filter(|v| !v.is_empty()).map(|v| (key, v)));
        } else {
            out.push((key, value));
        }
    }
    (out, end + 2)
}

/// The lines one entry file contributes: a `#` divider (so no heading of the
/// file before it stays in force), a `##` heading named by the file, then its
/// text with its own headings pushed under — so `w`, the gold mark and `gd`
/// know the file by its name, and its sections stay its own. With them, the
/// aliases its front matter gives.
fn entry_file_lines(dir: &Path, file: &Path, text: &str) -> (Vec<(String, PathBuf, usize)>, Vec<String>) {
    let mut out = Vec::new();
    let name = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    if name.is_empty() {
        return (out, Vec::new());
    }
    let divider = dir.file_name().map(|d| d.to_string_lossy().into_owned()).unwrap_or_else(|| "/".to_string());
    out.push((format!("# {divider}"), file.to_path_buf(), 0));
    let lines: Vec<&str> = text.lines().collect();
    let (front, body_from) = front_matter(&lines);
    let aliases: Vec<String> =
        front.iter().filter(|(k, _)| *k == "aliases" || *k == "alias").map(|(_, v)| v.to_string()).collect();
    let body: Vec<&str> = lines[body_from..].to_vec();
    let shallowest = body
        .iter()
        .filter_map(|l| {
            let t = l.trim_start();
            let d = t.chars().take_while(|&c| c == '#').count();
            ((1..=6).contains(&d) && t[d..].starts_with(' ')).then_some(d)
        })
        .min()
        .unwrap_or(3);
    // Headings sit under the file's own: the shallowest becomes `###`.
    let push = 3usize.saturating_sub(shallowest);
    let shifted: Vec<(String, usize)> = body
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let t = l.trim_start();
            let d = t.chars().take_while(|&c| c == '#').count();
            let heading = (1..=6).contains(&d) && t[d..].starts_with(' ');
            match heading {
                true => (format!("{}{}", "#".repeat((d + push).min(6)), &t[d..]), body_from + i),
                false => (l.to_string(), body_from + i),
            }
        })
        .collect();
    out.push((format!("## {name}"), file.to_path_buf(), 0));
    out.extend(shifted.into_iter().map(|(l, n)| (l, file.to_path_buf(), n)));
    (out, aliases)
}

/// Each line with its number, and — for a `[yumete] 檔名` line inside a
/// comment — the name it asks for instead of its text.
///
/// Inside a fence nothing is a directive; inside a comment only `[yumete]`
/// lines are, and the rest of the note is dropped with its markers, so a note
/// beside a directive neither becomes an entry's body nor a heading.
fn scan<'a>(lines: &[&'a str]) -> Vec<(usize, &'a str, Option<&'a str>)> {
    let mut out = Vec::new();
    let mut fence: Option<char> = None;
    let mut note: Option<&'static str> = None;
    for (n, &line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if note.is_none() {
            let mark = ['`', '~'].into_iter().find(|&c| trimmed.starts_with(&c.to_string().repeat(3)));
            match (fence, mark) {
                (None, Some(m)) => fence = Some(m),
                (Some(f), Some(m)) if f == m => fence = None,
                _ => {}
            }
            if fence.is_some() || mark.is_some() {
                out.push((n, line, None));
                continue;
            }
        }
        // The parts of this line that are inside a note, and whether one is
        // left open at its end.
        let mut inside: Vec<&str> = Vec::new();
        let mut outside = true;
        if let Some(closer) = note {
            outside = false;
            match crate::markdown::comment_closes(line, closer) {
                Some(at) => {
                    let byte = line.char_indices().nth(at).map_or(line.len(), |(b, _)| b);
                    inside.push(&line[..byte]);
                    note = crate::markdown::comment_left_open(&line[byte + closer.len()..]);
                }
                None => inside.push(line),
            }
        } else {
            for span in crate::markdown::spans(line) {
                if span.kind == crate::markdown::Kind::Comment {
                    let from = line.char_indices().nth(span.start).map_or(line.len(), |(b, _)| b);
                    let to = line.char_indices().nth(span.end).map_or(line.len(), |(b, _)| b);
                    inside.push(&line[from..to]);
                }
            }
            note = crate::markdown::comment_left_open(line);
        }
        let directives: Vec<&str> = inside
            .iter()
            .filter_map(|t| t.trim().strip_prefix("[yumete]"))
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect();
        if directives.is_empty() {
            // A line that is nothing but a note says nothing to the wiki.
            if outside && !line.trim_start().starts_with("<!--") && !line.trim_start().starts_with("%%") {
                out.push((n, line, None));
            }
        } else {
            for d in directives {
                out.push((n, line, Some(d)));
            }
        }
    }
    out
}

/// Split the spliced lines into entries.
fn entries(lines: &[(String, PathBuf, usize)], global: bool) -> Vec<Entry> {
    let mut out: Vec<Entry> = Vec::new();
    // The headings in force, by depth: `trail[d-1]` is the heading at depth d.
    let mut trail: Vec<String> = Vec::new();
    // The entries still collecting body lines, innermost last.
    let mut open: Vec<usize> = Vec::new();
    let mut fence = false;
    for (text, source, line) in lines {
        let trimmed = text.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fence = !fence;
        }
        let depth = trimmed.chars().take_while(|&c| c == '#').count();
        let heading = !fence
            && (1..=6).contains(&depth)
            && trimmed[depth..].starts_with(' ')
            && !trimmed[depth..].trim().is_empty();
        if heading {
            let name = trimmed[depth..].trim().to_string();
            open.retain(|&i| out[i].depth < depth);
            trail.truncate(depth - 1);
            let ancestors = trail.clone();
            while trail.len() < depth - 1 {
                trail.push(String::new());
            }
            trail.push(name.clone());
            for &i in &open {
                out[i].body.push(text.clone());
            }
            if depth >= 2 {
                open.push(out.len());
                out.push(Entry {
                    name,
                    depth,
                    ancestors: ancestors.into_iter().filter(|a| !a.is_empty()).collect(),
                    body: Vec::new(),
                    source: source.clone(),
                    line: *line,
                    global,
                });
            }
            continue;
        }
        for &i in &open {
            out[i].body.push(text.clone());
        }
    }
    for entry in &mut out {
        while entry.body.last().is_some_and(|l| l.trim().is_empty()) {
            entry.body.pop();
        }
        while entry.body.first().is_some_and(|l| l.trim().is_empty()) {
            entry.body.remove(0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(files: &[(&str, &str)]) -> PathBuf {
        static NTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "yumete-wiki-{}-{}",
            std::process::id(),
            NTH.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        for (name, text) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    #[test]
    fn headings_of_depth_two_are_entries_and_carry_their_children() {
        let dir = book(&[(
            ".yumete/wiki.md",
            "# 人類\n\n## 亞洲人\n亞洲在太平洋西邊。\n### 中國人\n中國人是東亞的一個人羣。\n## 歐洲人\n別處\n",
        )]);
        let wiki = Wiki::load(Some(&dir.join(".yumete/wiki.md")), None);
        let names: Vec<&str> = wiki.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["亞洲人", "中國人", "歐洲人"], "# is a divider, not an entry");
        let asia = &wiki.entries[0];
        assert_eq!(asia.ancestors, ["人類"]);
        assert_eq!(asia.body, ["亞洲在太平洋西邊。", "### 中國人", "中國人是東亞的一個人羣。"]);
        assert_eq!(wiki.entries[1].ancestors, ["人類", "亞洲人"]);
    }

    #[test]
    fn a_directive_in_a_note_pastes_a_file_where_it_stands() {
        let dir = book(&[
            (".yumete/wiki.md", "# 人物\n<!-- [yumete] 人物.md -->\n<!--\n[yumete] 地理.md\n只是備註\n-->\n"),
            (".yumete/人物.md", "## 阿寧\n主角。\n"),
            (".yumete/地理.md", "## 落霞鎮\n小鎮。\n# 不是詞條\n"),
        ]);
        let wiki = Wiki::load(Some(&dir.join(".yumete/wiki.md")), None);
        let ning = &wiki.entries[wiki.by_name["阿寧"][0]];
        assert_eq!(ning.ancestors, ["人物"], "the breadcrumb comes from where it was pasted");
        assert!(ning.source.ends_with("人物.md"));
        assert!(wiki.by_name.contains_key("落霞鎮"), "a directive inside a note of several lines");
        assert!(!wiki.entries.iter().any(|e| e.body.iter().any(|l| l.contains("只是備註"))));
        let read: Vec<usize> = wiki
            .sources
            .iter()
            .filter_map(|s| match s {
                Source::Read { entries, .. } => Some(*entries),
                _ => None,
            })
            .collect();
        assert_eq!(read, [0, 1, 1], "wiki.md itself gave none; each include gave one");
    }

    #[test]
    fn an_include_that_leaves_the_book_is_refused_and_a_typo_is_only_missing() {
        let dir = book(&[(".yumete/wiki.md", "<!-- [yumete] ../../外面.md -->\n<!-- [yumete] 沒有.md -->\n<!-- [yumete] wiki.md -->\n")]);
        let wiki = Wiki::load(Some(&dir.join(".yumete/wiki.md")), None);
        assert!(matches!(wiki.sources[1], Source::Refused { .. }), "{:?}", wiki.sources);
        assert!(matches!(wiki.sources[2], Source::Missing { .. }), "{:?}", wiki.sources);
        assert!(matches!(wiki.sources[3], Source::Again { .. }), "{:?}", wiki.sources);
    }

    #[test]
    fn the_book_comes_before_the_global_wiki_and_a_one_character_name_is_listed() {
        let dir = book(&[
            ("書/.yumete/wiki.md", "## 阿寧\n書裏的。\n## 墨\n一個字。\n"),
            ("全局/wiki.md", "## 阿寧\n系列的。\n"),
        ]);
        let wiki = Wiki::load(Some(&dir.join("書/.yumete/wiki.md")), Some(&dir.join("全局/wiki.md")));
        let both: Vec<bool> = wiki.by_name["阿寧"].iter().map(|&i| wiki.entries[i].global).collect();
        assert_eq!(both, [false, true]);
        assert_eq!(wiki.unmarkable(), ["墨"]);
        assert!(!wiki.words().any(|w| w == "墨"));
    }
    /// **An entry three levels down an included file still knows its name.**
    ///
    /// The include is written in the book's own `wiki.md` and points out of the
    /// `.yumete` directory; the file it names has its own `#` title, so the
    /// entries under it sit at depth 2 and 3. This began as a scratch test that
    /// printed what it found and asserted nothing — which is a test that passes
    /// whatever happens (tidied 2026-09-19).
    #[test]
    fn a_folder_of_files_is_one_entry_each_with_aliases_and_groups() {
        let dir = book(&[
            (".yumete/wiki.md", "---\nwiki: ../百科/\n---\n# 雜\n<!-- [yumete] ../地理/ -->\n## 雜條\n雜\n"),
            ("百科/人物/安寧.md", "---\naliases: [小寧, 寧兒]\n---\n主角。生於洛霞。\n\n# 幼年\n小時候。\n"),
            ("百科/門派.md", "## 天門\n門\n"),
            ("地理/洛霞.txt", "一座城。\n"),
            (".yumete/wiki/洞庭.md", "湖。\n"),
        ]);
        let wiki = Wiki::load(Some(&dir.join(".yumete/wiki.md")), None);
        let mut names: Vec<&str> = wiki.entries.iter().map(|e| e.name.as_str()).collect();
        names.sort_unstable();
        assert_eq!(names, ["天門", "安寧", "寧兒", "小寧", "幼年", "洛霞", "洞庭", "門派", "雜條"]);
        let alias = &wiki.entries[wiki.by_name["小寧"][0]];
        assert_eq!(alias.body, wiki.entries[wiki.by_name["安寧"][0]].body);
        let an = &wiki.entries[wiki.by_name["安寧"][0]];
        assert_eq!(an.ancestors, ["人物"], "the sub-folder is the group");
        assert_eq!(an.body[0], "主角。生於洛霞。");
        assert_eq!(an.body[2], "### 幼年", "the file's own heading sits under it");
        assert_eq!(an.line, 0);
        let young = &wiki.entries[wiki.by_name["幼年"][0]];
        assert_eq!(young.line, 5, "its own line in its own file");
        assert_eq!(young.ancestors, ["人物", "安寧"]);
        let folders = wiki.folders();
        assert_eq!(folders.len(), 3, "{folders:?}");
        assert!(wiki.in_a_folder(&dir.join("百科/人物/新人.md")), "a file saved there is an entry");
        assert!(!wiki.in_a_folder(&dir.join("第一章.md")));
        assert!(wiki.came_from(&dir.join("地理/洛霞.txt")));
        let folder_sources = wiki.sources.iter().filter(|s| matches!(s, Source::Folder { .. })).count();
        assert_eq!(folder_sources, 3);
    }

    #[test]
    fn the_project_itself_can_be_the_wiki() {
        let dir = book(&[
            (".yumete/wiki.md", "<!-- [yumete] ../ -->\n"),
            ("第一章.md", "# 第一章\n安寧出場。\n"),
            ("人物/安寧.md", "主角。\n"),
        ]);
        let wiki = Wiki::load(Some(&dir.join(".yumete/wiki.md")), None);
        let mut names: Vec<&str> = wiki.by_name.keys().map(String::as_str).collect();
        names.sort_unstable();
        assert_eq!(names, ["安寧", "第一章"], ".yumete/ itself is skipped: {names:?}");
    }

    #[test]
    fn an_entry_under_a_sub_heading_of_an_included_file_is_found() {
        let dir = std::env::temp_dir().join("yumete-wiki-an-entry-under-a-sub-heading");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("書/.yumete")).unwrap();
        std::fs::create_dir_all(dir.join("書/辭典")).unwrap();
        std::fs::write(
            dir.join("書/.yumete/wiki.md"),
            "# 辭典\n<!-- [yumete] ../辭典/洞庭湖辭典.md -->\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("書/辭典/洞庭湖辭典.md"),
            "# 洞庭湖辭典\n\n## 書齋\n山口。\n\n## 君山\n境界。\n\n### 小島\n更深。\n",
        )
        .unwrap();
        let wiki = crate::wiki::Wiki::load(Some(&dir.join("書/.yumete/wiki.md")), None);
        let found: Vec<(&str, usize)> =
            wiki.entries.iter().map(|e| (e.name.as_str(), e.depth)).collect();
        assert!(found.contains(&("書齋", 2)), "{found:?}");
        assert!(found.contains(&("君山", 2)), "{found:?}");
        assert!(found.contains(&("小島", 3)), "the third level too: {found:?}");
        let deepest = wiki.entries.iter().find(|e| e.name == "小島").expect("小島");
        assert_eq!(deepest.ancestors.last().map(String::as_str), Some("君山"), "{:?}", deepest.ancestors);
        assert!(deepest.source.ends_with("洞庭湖辭典.md"), "{:?}", deepest.source);
        std::fs::remove_dir_all(&dir).ok();
    }
}
