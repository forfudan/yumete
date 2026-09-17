//! The editor's side of 作品百科 (#287, `development.md` §5.8): when the wiki
//! is read, what its names do to the segmenter, and `:wiki`.

use super::*;
use crate::wiki::{Source, Wiki, WIKI_MD};

impl Editor {
    /// Read this book's wiki and the global one again, and put their names
    /// into the segmenter beside the book's own words.
    ///
    /// Read whenever the book's words are — opening a file of another book is
    /// what changes which wiki is this book's.
    pub(super) fn reload_wiki(&mut self) {
        let book = self.book_wiki_path();
        let global = self.global_wiki_path();
        self.wiki = Wiki::load(Some(&book), global.as_deref());
    }

    /// This book's `.yumete/wiki.md`, found the way `words.txt` is — up from
    /// the file being edited — whether or not it is there yet.
    fn book_wiki_path(&self) -> PathBuf {
        let from = self
            .current_buffer()
            .path()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut dir = Some(from.as_path());
        while let Some(d) = dir {
            let there = d.join(".yumete");
            if there.join(WIKI_MD).is_file() || there.join("wiki.txt").is_file() {
                return there.join(WIKI_MD);
            }
            dir = d.parent();
        }
        self.project_root().join(".yumete").join(WIKI_MD)
    }

    /// The global wiki, beside the global word list.
    fn global_wiki_path(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join(WIKI_MD))
    }

    /// The wiki's names, for the segmenter.
    pub(super) fn wiki_words(&self) -> yumete_cjk::WordList {
        let mut list = yumete_cjk::WordList::default();
        for name in self.wiki.words() {
            list.add(name);
        }
        list
    }

    /// Whether `path` is one of the files the wiki was read from — a save of
    /// any of them reads the whole graph again.
    pub(super) fn is_wiki_file(&self, path: &Path) -> bool {
        let same = |a: &Path| {
            std::fs::canonicalize(a).ok() == std::fs::canonicalize(path).ok() || a == path
        };
        self.wiki.files().into_iter().any(same)
            || (path.file_name().is_some_and(|n| n == WIKI_MD)
                && path.parent().is_some_and(|d| d.file_name().is_some_and(|n| n == ".yumete")))
    }

    /// `:wiki`, `:wiki edit`, `:wiki global`, `:wiki reload`.
    pub(super) fn wiki_command(&mut self, what: Option<&str>) -> Result<CommandOutcome, EditorError> {
        match what {
            Some("edit") => {
                let path = self.book_wiki_path();
                self.open_wiki_file(&path)?;
            }
            // **The keys stay in the writing** (2026-09-17). Every other view
            // is a list to walk; this page is drawn from where the cursor is,
            // so putting the keys in it would freeze what it shows.
            Some("panel") => self.show_wiki_panel(),
            Some("hide") => {
                self.wiki_mark = crate::wiki::Mark::Off;
                self.status = say!("wiki.marks-off");
            }
            Some("color") => {
                self.wiki_mark = crate::wiki::Mark::Color;
                self.status = say!("wiki.marks-color");
            }
            Some("line") => {
                self.wiki_mark = crate::wiki::Mark::Line;
                self.status = say!("wiki.marks-line");
            }
            Some("global") => match self.global_wiki_path() {
                Some(path) => self.open_wiki_file(&path)?,
                None => self.status = say!("word.no-data-directory"),
            },
            Some(_) => {
                self.reload_project_words();
                self.status = say!("wiki.reloaded", self.wiki.entries.len(), self.wiki.files().len());
            }
            None => self.wiki_report(),
        }
        Ok(CommandOutcome::Continue)
    }

    fn open_wiki_file(&mut self, path: &Path) -> Result<(), EditorError> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        self.open_file(path).map_err(EditorError::Io)?;
        // A `wiki.txt` is still Markdown: it is full of `##`.
        self.current_buffer_mut().set_syntax(crate::syntax::Syntax::Markdown);
        if self.current_buffer().text().trim().is_empty() {
            self.status = say!("wiki.opened-empty", path.display());
        }
        Ok(())
    }

    /// `:wiki` — every file the wiki reached for, what each gave, and what
    /// could not be marked. **The feature's error channel**: a wiki that
    /// quietly ignores half of itself is worse than one that fails.
    fn wiki_report(&mut self) {
        let root = self.project_root();
        let shown = |p: &Path| p.strip_prefix(&root).unwrap_or(p).display().to_string();
        let mut out = format!("# {}\n\n", say!("wiki.title"));
        if self.wiki.sources.is_empty() {
            out.push_str(&say!("wiki.none", shown(&self.book_wiki_path())));
            out.push('\n');
        }
        for source in &self.wiki.sources {
            let line = match source {
                Source::Read { path, entries } => say!("wiki.read", shown(path), entries),
                Source::Missing { path } => say!("wiki.missing", shown(path)),
                Source::Refused { named, from } => say!("wiki.refused", named, shown(from)),
                Source::Again { path } => say!("wiki.again", shown(path)),
                Source::TooMany { named } => say!("wiki.too-many", named),
            };
            out.push_str(&format!("- {line}\n"));
        }
        let unmarkable = self.wiki.unmarkable();
        if !unmarkable.is_empty() {
            out.push_str(&format!("\n## {}\n\n", say!("wiki.unmarkable")));
            for name in unmarkable {
                out.push_str(&format!("- {name}\n"));
            }
        }
        self.show_listing(out, say!("wiki.title"));
    }
}

/// One entry, laid out to be read (#287, §5.8.5): the breadcrumb, then its
/// body with its own sub-headings re-levelled so the entry reads as `#`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiPart {
    /// From the global wiki rather than this book's.
    pub global: bool,
    /// The headings above it, outermost first.
    pub trail: Vec<String>,
    pub lines: Vec<WikiLine>,
    /// Where the heading is written.
    pub source: PathBuf,
    pub line: usize,
}

/// A line of an entry's body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WikiLine {
    /// A sub-heading, at its depth **within the entry** (2 is the first level
    /// under the entry itself).
    Heading(usize, String),
    Text(String),
}

/// Every entry of the name the cursor is standing on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiView {
    pub name: String,
    /// Book first, then global; `(depth, order)` within each.
    pub parts: Vec<WikiPart>,
}

impl WikiView {
    /// The whole view as prose, for the floating panel: one entry after
    /// another, a rule between, and the global ones under 「全局」.
    pub fn as_prose(&self) -> String {
        let mut out: Vec<String> = Vec::new();
        let mixed = self.parts.iter().any(|p| p.global) && self.parts.iter().any(|p| !p.global);
        let mut global_said = false;
        for (i, part) in self.parts.iter().enumerate() {
            if i > 0 {
                out.push(String::new());
            }
            if mixed && part.global && !global_said {
                out.push(format!("── {} ──", say!("wiki.global")));
                global_said = true;
            } else if i > 0 {
                out.push("──".to_string());
            }
            if !part.trail.is_empty() {
                out.push(part.trail.join(" › "));
            }
            for line in &part.lines {
                out.push(match line {
                    WikiLine::Heading(depth, title) => format!("{} {title}", "#".repeat(*depth)),
                    WikiLine::Text(text) => text.clone(),
                });
            }
        }
        out.join("\n")
    }
}

impl Editor {
    /// `:wiki panel` — put the 百科 page up (or take it down), and leave the
    /// keys where they are.
    fn show_wiki_panel(&mut self) {
        match self.showing(crate::sidebar::View::Wiki) {
            Some(side) => self.close_panel(side),
            None => {
                self.show_sidebar(crate::sidebar::View::Wiki);
                self.panel_focus = None;
            }
        }
    }

    /// The entries of the word under the cursor, if it names any (#287).
    ///
    /// The word is the one the segmenter cut — which is exactly where a wiki
    /// name was merged in — so 中國人 inside 中國人民 is not asked about. A
    /// one-character entry is never answered here: it could not be marked,
    /// and a panel over every 墨 in a novel would be a panel over the novel.
    pub fn wiki_here(&self) -> Option<WikiView> {
        if self.wiki.by_name.is_empty() || self.mode == Mode::Insert {
            return None;
        }
        let line = self.cursor_line();
        let block = self.block_of(line);
        if block.is_literal() || matches!(block, crate::markdown::Block::Comment { .. }) {
            return None;
        }
        let rope = self.current_buffer().rope();
        let at = self.cursor - rope.line_to_char(line);
        let chars = crate::zong::line_chars(rope, line);
        let &(a, b) = self.segment_line(line).iter().find(|&&(a, b)| at >= a && at < b)?;
        if b - a < 2 {
            return None;
        }
        let name: String = chars[a..b.min(chars.len())].iter().collect();
        let found = self.wiki.by_name.get(&name)?;
        let parts = found
            .iter()
            .map(|&i| {
                let entry = &self.wiki.entries[i];
                let lines = entry
                    .body
                    .iter()
                    .map(|text| {
                        let trimmed = text.trim_start();
                        let depth = trimmed.chars().take_while(|&c| c == '#').count();
                        if depth > entry.depth && trimmed[depth..].starts_with(' ') {
                            WikiLine::Heading(depth - entry.depth + 1, trimmed[depth..].trim().to_string())
                        } else {
                            WikiLine::Text(text.clone())
                        }
                    })
                    .collect();
                WikiPart {
                    global: entry.global,
                    trail: entry.ancestors.clone(),
                    lines,
                    source: entry.source.clone(),
                    line: entry.line,
                }
            })
            .collect();
        Some(WikiView { name, parts })
    }

    /// The entry for the **floating** panel — only while nothing the writer
    /// typed answers first (a row, a footnote, a comment), and only while the
    /// sidebar's 百科 page is not open: one place at a time.
    pub fn wiki_floating(&self) -> Option<WikiView> {
        if self.wiki_include_here().is_some() {
            return None;
        }
        if self.showing(crate::sidebar::View::Wiki).is_some() || self.detail().is_some() {
            return None;
        }
        self.wiki_here()
    }

    /// `gd` on a wiki name: open the file the entry is written in, on its
    /// heading.
    pub(super) fn follow_wiki(&mut self) -> bool {
        let Some(view) = self.wiki_here() else {
            return false;
        };
        let Some(part) = view.parts.first() else {
            return false;
        };
        let (path, line) = (part.source.clone(), part.line);
        self.remember_jump();
        if self.open_file(&path).is_ok() {
            self.goto_line(line + 1);
        }
        true
    }
}

impl Editor {
    /// How wiki names are marked on the page (`:wiki hide|color|line`).
    pub fn wiki_mark(&self) -> crate::wiki::Mark {
        self.wiki_mark
    }

    /// Whether wiki names are marked on the page at all.
    pub fn wiki_marks_visible(&self) -> bool {
        self.wiki_mark != crate::wiki::Mark::Off
            && !self.wiki.by_name.is_empty()
            && self.markup_visible()
    }

    /// Where on `line` a wiki name is, as char ranges (#287, §5.8.3).
    ///
    /// **Exactly where the segmenter cut one out**, which is where the name
    /// was merged in: the segmentation this reads is the cached one, so a
    /// frame costs a hash lookup per word on the drawn rows and nothing walks
    /// the document. A one-character entry is never marked.
    pub fn wiki_marks_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        if !self.wiki_marks_visible() {
            return Vec::new();
        }
        let block = self.block_of(line);
        if block.is_literal() || matches!(block, crate::markdown::Block::Comment { .. }) {
            return Vec::new();
        }
        let chars = crate::zong::line_chars(self.current_buffer().rope(), line);
        self.segment_line(line)
            .into_iter()
            .filter(|&(a, b)| {
                b - a >= 2 && {
                    let word: String = chars[a..b.min(chars.len())].iter().collect();
                    self.wiki.by_name.contains_key(&word)
                }
            })
            .collect()
    }
}

/// What became of the file a `[yumete]` line names (#287).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiInclude {
    /// The name as written, for the title of the panel.
    pub named: String,
    /// Where it resolves to, whether or not it is there.
    pub path: PathBuf,
    /// What the last read made of it — `None` when this file is not one the
    /// wiki reached for at all (a directive in a chapter, say).
    pub state: Option<crate::wiki::Source>,
}

impl Editor {
    /// The `[yumete] 檔名` line the cursor is on, if it is on one.
    ///
    /// **Only in a wiki file**: a directive in a chapter is a note to self,
    /// and honouring it would make every file in the book a wiki source.
    pub fn wiki_include_here(&self) -> Option<WikiInclude> {
        let here = self.current_buffer().path()?.to_path_buf();
        if !self.is_wiki_file(&here) {
            return None;
        }
        let rope = self.current_buffer().rope();
        let line = rope.line(self.cursor_line()).to_string();
        // Inside a comment, and the first thing in it — the loader's own rule,
        // asked the same way so the two can never disagree.
        let named = line
            .split("[yumete]")
            .nth(1)?
            .trim_end_matches(['\n', '\r'])
            .trim_end_matches("-->")
            .trim_end_matches("%%")
            .trim()
            .to_string();
        if named.is_empty() {
            return None;
        }
        let path = here.parent().unwrap_or(Path::new(".")).join(&named);
        let same = |a: &Path| std::fs::canonicalize(a).ok() == std::fs::canonicalize(&path).ok();
        let state = self.wiki.sources.iter().find(|source| match source {
            crate::wiki::Source::Read { path: p, .. } => same(p),
            crate::wiki::Source::Missing { path: p } => same(p) || p == &path,
            crate::wiki::Source::Again { path: p } => same(p) || p == &path,
            crate::wiki::Source::Refused { named: n, .. } => *n == named,
            crate::wiki::Source::TooMany { named: n } => *n == named,
        });
        Some(WikiInclude { named, path, state: state.cloned() })
    }

    /// `gf` on a `[yumete]` line: open the file it names, existing or not —
    /// the way `gf` opens a `#include`\'s chapter.
    pub(super) fn open_wiki_include(&mut self) -> bool {
        let Some(include) = self.wiki_include_here() else {
            return false;
        };
        if let Err(err) = self.open_file(&include.path) {
            self.status = say!("buffer.cannot-open", include.named, err);
        }
        true
    }
}
