//! 作品百科 — a book's own encyclopaedia (#287, `development.md` §5.8).
//!
//! `.yumete/wiki.md` is a Markdown file whose `##`-and-deeper headings are
//! entries. A line **inside a comment** that begins `[yumete]` names another
//! file, read as if pasted where the line stands — so a wiki grows into
//! 人物.md, 地理.md, 門派.md without a new idea. Everything here is a function
//! of files and text; the editor decides when to read and what to do with it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// At most this many files, and this deep, however the includes are written:
/// a bad edit must not stall the editor walking a tree.
const MAX_FILES: usize = 64;
const MAX_DEPTH: usize = 8;

/// The file `:wiki-edit` makes, and the spelling a writer may have used instead.
pub const WIKI_MD: &str = "wiki.md";
const WIKI_TXT: &str = "wiki.txt";

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
    /// Outside the book, absolute, or through a link that leaves it.
    Refused { named: String, from: PathBuf },
    /// Already read once; the second directive is ignored.
    Again { path: PathBuf },
    /// Past the file or depth cap.
    TooMany { named: String },
}

/// Everything the wiki holds.
#[derive(Debug, Clone, Default)]
pub struct Wiki {
    pub entries: Vec<Entry>,
    /// Every entry of each name, book before global, then `(depth, order)`.
    pub by_name: HashMap<String, Vec<usize>>,
    pub sources: Vec<Source>,
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
            let bound = match is_global {
                false => path.parent().and_then(Path::parent),
                true => path.parent(),
            }
            .map(Path::to_path_buf)
            .unwrap_or_default();
            let bound = std::fs::canonicalize(&bound).unwrap_or(bound);
            let mut reader = Reader { bound, seen: HashSet::new(), sources: Vec::new(), lines: Vec::new() };
            reader.read(&path, 0);
            let before = wiki.entries.len();
            wiki.entries.extend(entries(&reader.lines, is_global));
            let mut counts: HashMap<PathBuf, usize> = HashMap::new();
            for entry in &wiki.entries[before..] {
                *counts.entry(entry.source.clone()).or_default() += 1;
            }
            for source in &mut reader.sources {
                if let Source::Read { path, entries } = source {
                    *entries = counts.get(path).copied().unwrap_or(0);
                }
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
        wiki
    }

    /// Every file the wiki was read from — a save of any of them reloads it.
    pub fn files(&self) -> Vec<&Path> {
        self.sources
            .iter()
            .filter_map(|s| match s {
                Source::Read { path, .. } => Some(path.as_path()),
                _ => None,
            })
            .collect()
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
}

impl Reader {
    fn read(&mut self, path: &Path, depth: usize) {
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if !self.seen.insert(canonical.clone()) {
            self.sources.push(Source::Again { path: path.to_path_buf() });
            return;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            self.sources.push(Source::Missing { path: path.to_path_buf() });
            return;
        };
        self.sources.push(Source::Read { path: path.to_path_buf(), entries: 0 });
        let lines: Vec<&str> = text.lines().collect();
        for (n, line, directive) in scan(&lines) {
            match directive {
                None => self.lines.push((line.to_string(), path.to_path_buf(), n)),
                Some(named) => self.include(path, named, depth),
            }
        }
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
        // ⚠️ `canonicalize` fails on a path that does not exist, so a typo must
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
        if !resolved.starts_with(&self.bound) {
            self.sources.push(refused());
            return;
        }
        self.read(&target, depth + 1);
    }
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
        let dir = std::env::temp_dir().join(format!(
            "yumete-wiki-{}-{}",
            std::process::id(),
            files.iter().map(|(n, t)| n.len() + t.len()).sum::<usize>()
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
}
