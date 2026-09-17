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
