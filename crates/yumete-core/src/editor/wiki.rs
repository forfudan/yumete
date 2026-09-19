//! The editor's side of 作品百科 (#287, `development.md` §5.8): when the wiki
//! is read, what its names do to the segmenter, and `:wiki`.

use super::*;
use crate::wiki::{Source, Wiki, WIKI_MD};
use std::collections::BTreeMap;

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
            // ⚠️ **Whichever spelling is really there** (2026-09-19, caught in
            // review). This looked for both and then always answered `wiki.md`
            // — so on a book whose wiki is `wiki.txt`, `:wiki edit` opened an
            // **empty** `wiki.md`, and the first save made the loader prefer
            // that one: the writer's entries went dark in one keystroke.
            if there.join(WIKI_MD).is_file() {
                return there.join(WIKI_MD);
            }
            if there.join("wiki.txt").is_file() {
                return there.join("wiki.txt");
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
        self.wiki.came_from(path)
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
                Source::Unreadable { path, why } => say!("wiki.unreadable", shown(path), why),
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
        let missed = self.wiki_missed_here();
        if !missed.is_empty() {
            out.push_str(&format!("\n## {}\n\n", say!("wiki.missed")));
            for (name, lines) in missed {
                // The first few; a name the chapter is about is on every page,
                // and the point is 「it is not marked」, not a concordance.
                let where_ = lines
                    .iter()
                    .take(5)
                    .map(usize::to_string)
                    .collect::<Vec<_>>()
                    .join("、");
                let where_ = match lines.len() > 5 {
                    true => format!("{where_}…"),
                    false => where_,
                };
                out.push_str(&format!("- {}\n", say!("wiki.missed-at", name, where_)));
            }
        }
        self.show_listing(out, say!("wiki.title"));
    }
}

/// One entry, laid out to be read (#287, §5.8.5): the breadcrumb, then its
/// body with its own sub-headings re-levelled so the entry reads as `#`.
///
/// ⚠️ **It borrows the entry rather than copying it** (2026-09-19). This is
/// built **every frame the cursor stands on a name**, and an entry may be a
/// chapter in its own right: copying every line of it cost 10 ms a frame at
/// 5000 lines and 30 ms at 20000, measured — the editor going sticky while you
/// read. Nothing here outlives the frame it is drawn in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiPart<'a> {
    /// From the global wiki rather than this book's.
    pub global: bool,
    /// The headings above it, outermost first.
    pub trail: &'a [String],
    pub lines: Vec<WikiLine<'a>>,
    /// Where the heading is written.
    pub source: &'a Path,
    pub line: usize,
}

/// A line of an entry's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WikiLine<'a> {
    /// A sub-heading, at its depth **within the entry** (2 is the first level
    /// under the entry itself).
    Heading(usize, &'a str),
    Text(&'a str),
}

/// Every entry of the name the cursor is standing on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiView<'a> {
    pub name: String,
    /// Book first, then global; `(depth, order)` within each.
    pub parts: Vec<WikiPart<'a>>,
}

impl WikiView<'_> {
    /// **The 章節 line** — 「辭典 › 真境」 — when the view is one entry
    /// (作者 2026-09-18: 「章節那一行能不能用灰一些的顏色」).
    ///
    /// It is not part of what the entry *says*: it is where the entry was
    /// written down. So the panel draws it quietly, under the name and above
    /// the body, rather than as the body's first paragraph. Several entries of
    /// one name keep their breadcrumbs inline — each one belongs to its own
    /// entry, and there is no single line to lift out.
    pub fn lede(&self) -> Option<String> {
        match self.parts.as_slice() {
            [one] if !one.trail.is_empty() => Some(one.trail.join(" › ")),
            _ => None,
        }
    }

    /// The entry without its 章節 line — what [`Self::lede`] lifted out —
    /// and **no more of it than a panel could draw**.
    ///
    /// ⚠️ `upto` is not a preference, it is what keeps this off the critical
    /// path (2026-09-19). A float is at most a third of the page deep, so
    /// `upto` lines is already more of the entry than it can use; and since
    /// every line kept is at least one row (or one 縱) drawn, a cut here can
    /// never take away a line the panel would have shown. The panel's own
    /// 「…」 still says it was cut, because it still has more than it can fit.
    /// Without it the whole entry was built **and then wrapped** every frame.
    pub fn body_prose(&self, upto: usize) -> String {
        match self.lede() {
            Some(lede) => self
                .prose(upto)
                .strip_prefix(&lede)
                .map(|rest| rest.trim_start_matches('\n').to_string())
                .unwrap_or_else(|| self.prose(upto)),
            None => self.prose(upto),
        }
    }

    /// The whole view as prose: one entry after another, a rule between, and
    /// the global ones under 「全局」.
    pub fn as_prose(&self) -> String {
        self.prose(usize::MAX)
    }

    /// …with at most `upto` lines of **body** (the furniture is never cut).
    fn prose(&self, upto: usize) -> String {
        let mut out: Vec<std::borrow::Cow<'_, str>> = Vec::new();
        let mixed = self.parts.iter().any(|p| p.global) && self.parts.iter().any(|p| !p.global);
        let mut global_said = false;
        // Blank lines are not counted: the panel drops empty paragraphs, so
        // they cost no row and cutting on them would shorten what is shown.
        let mut kept = 0usize;
        'parts: for (i, part) in self.parts.iter().enumerate() {
            if i > 0 {
                out.push("".into());
            }
            if mixed && part.global && !global_said {
                out.push(format!("── {} ──", say!("wiki.global")).into());
                global_said = true;
            } else if i > 0 {
                out.push("──".into());
            }
            if !part.trail.is_empty() {
                out.push(part.trail.join(" › ").into());
            }
            for line in &part.lines {
                if kept >= upto {
                    break 'parts;
                }
                match line {
                    WikiLine::Heading(depth, title) => {
                        out.push(format!("{} {title}", "#".repeat(*depth)).into());
                        kept += 1;
                    }
                    WikiLine::Text(text) => {
                        out.push((*text).into());
                        kept += usize::from(!text.trim().is_empty());
                    }
                }
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
    pub fn wiki_here(&self) -> Option<WikiView<'_>> {
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
                            WikiLine::Heading(depth - entry.depth + 1, trimmed[depth..].trim())
                        } else {
                            WikiLine::Text(text.as_str())
                        }
                    })
                    .collect();
                WikiPart {
                    global: entry.global,
                    trail: entry.ancestors.as_slice(),
                    lines,
                    source: entry.source.as_path(),
                    line: entry.line,
                }
            })
            .collect();
        Some(WikiView { name, parts })
    }

    /// The entry for the **floating** panel — only while nothing the writer
    /// typed answers first (a row, a footnote, a comment), and only while the
    /// sidebar's 百科 page is not open: one place at a time.
    pub fn wiki_floating(&self) -> Option<WikiView<'_>> {
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
        // Taken out of the view before anything moves: the view borrows the
        // wiki, and opening a file is what re-reads it.
        let Some((path, line)) = self
            .wiki_here()
            .and_then(|view| view.parts.first().map(|p| (p.source.to_path_buf(), p.line)))
        else {
            return false;
        };
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
        match self.wiki_marks_visible() {
            true => self.wiki_names_on_line(line),
            false => Vec::new(),
        }
    }

    /// Where the names are, whether or not the marks are being drawn — so that
    /// `:wiki` can say what it could not mark even with `:wiki hide` on.
    fn wiki_names_on_line(&self, line: usize) -> Vec<(usize, usize)> {
        if self.line_is_literal(line) {
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

    /// A fence or a comment — where a name is quoted rather than used.
    fn line_is_literal(&self, line: usize) -> bool {
        let block = self.block_of(line);
        block.is_literal() || matches!(block, crate::markdown::Block::Comment { .. })
    }

    /// **The names this chapter has and does not show** (2026-09-19).
    ///
    /// A name joins the segmenter and the mark goes where the segmenter *cut*,
    /// which is the right rule — 中國人 inside 中國人民 is not the name — and
    /// also means a name can be in the wiki, be right there in the sentence,
    /// and never light up: 「有身體」 loses to 這裏有／身體, and a name with a
    /// space or a Latin letter is not one token at all. That used to happen
    /// **in silence**: the entry was written, nothing appeared, and `:wiki`
    /// reported nothing wrong. Now it is asked the only way it can be answered
    /// honestly — **by looking, not by predicting**: every name is searched for
    /// in the chapter that is open, and an occurrence carrying no mark is
    /// named with the lines it is on. Only this file, because only this file is
    /// known; the heading says so.
    fn wiki_missed_here(&self) -> Vec<(String, Vec<usize>)> {
        // ⚠️ **Only a chapter.** A listing has no path, and `:wiki` twice in a
        // row would otherwise read its own report back — 「有身體：第 12 行」
        // says nothing about the book. The wiki itself is out for the same
        // reason: every entry begins `## 名字`, so it would report all of them.
        let Some(here) = self.current_buffer().path().map(Path::to_path_buf) else {
            return Vec::new();
        };
        if self.is_wiki_file(&here) {
            return Vec::new();
        }
        // Grouped by first character so that most lines cost one scan of their
        // own characters and no search at all.
        let mut by_first: HashMap<char, Vec<&str>> = HashMap::new();
        for name in self.wiki.by_name.keys() {
            let mut cs = name.chars();
            match (cs.next(), cs.next()) {
                // One-character names are reported already, as never-markable.
                (Some(first), Some(_)) => by_first.entry(first).or_default().push(name),
                _ => {}
            }
        }
        if by_first.is_empty() {
            return Vec::new();
        }
        let rope = self.current_buffer().rope();
        let mut missed: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for line in 0..rope.len_lines() {
            if self.line_is_literal(line) {
                continue;
            }
            let chars = crate::zong::line_chars(rope, line);
            if !chars.iter().any(|c| by_first.contains_key(c)) {
                continue;
            }
            // **Every hit first, and only then the segmenter** — cutting a line
            // into words is the expensive half, and a first character in common
            // (人, 有) is not a hit. On a chapter where the names really are
            // everywhere this saves nothing; on a real one it skips most lines.
            let mut hits: Vec<(usize, &str)> = Vec::new();
            for (at, c) in chars.iter().enumerate() {
                let Some(names) = by_first.get(c) else { continue };
                for name in names {
                    let long = name.chars().count();
                    if at + long <= chars.len()
                        && chars[at..at + long].iter().collect::<String>() == **name
                    {
                        hits.push((at, name));
                    }
                }
            }
            if hits.is_empty() {
                continue;
            }
            let marked = self.wiki_names_on_line(line);
            for (at, name) in hits {
                let long = name.chars().count();
                if marked.iter().any(|&(a, b)| a == at && b == at + long) {
                    continue;
                }
                // Six is one more than the report prints: enough to say 「and
                // more」 without keeping a concordance of a name on every page.
                let seen = missed.entry(name).or_default();
                if seen.len() < 6 {
                    seen.push(line + 1);
                }
            }
        }
        missed.into_iter().map(|(name, lines)| (name.to_string(), lines)).collect()
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
            crate::wiki::Source::Unreadable { path: p, .. } => same(p) || p == &path,
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
