//! **The preview tab**: the typeset page of a Typst file, or a
//! Markdown file rendered, as VS Code, Zed and Emacs show a preview, in a
//! tab of its own.
//!
//! A preview is a tab like an open file — in the tab line, `:b`, the buffer
//! picker, `gn` and `gp` — whose buffer holds no text: it names the
//! document it shows (its *main* file: a Markdown file, or the main file of
//! a Typst book), and a window showing it draws that document's page
//! (`yumete_tui::page_window`, `yumete_tui::md_window`).
//!
//! **Each document has at most one preview, following or locked.** The
//! following preview — at most one of each kind — shows the document the
//! keys are in, and keeps the last one while they are in a file of another
//! kind, in a file with nothing to preview, or in a document with a locked
//! preview of its own: no document is shown twice. A locked preview stays
//! on its document (`:preview-lock`). When a document's last buffer
//! closes, its locked preview closes with it and the following preview
//! moves to another open document of its kind, or closes.
//!
//! `:bc` on it, and `:preview off`, close it; `:q` in a window showing
//! it closes the window and leaves the tab. **Windows showing a preview do
//! not count as windows when the last one goes**: `:q` in the last text
//! window leaves the editor, as it would with no preview open.

use super::*;
use crate::window::WindowId;

/// **A window showing a preview**: the preview's tab (its buffer's id) and
/// the file it typesets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageWindow {
    pub preview: u64,
    pub main: PathBuf,
}

/// **Whether `path` belongs to the book `main` typesets**: the file itself,
/// or a Typst file under its folder — a book is one main file that includes
/// its chapters, and the cursor spends the day in a chapter.
pub fn in_the_book(main: &Path, path: &Path, syntax: crate::syntax::Syntax) -> bool {
    // A Markdown file is its own document: nothing is a chapter of it.
    let book = crate::buffer::PreviewKind::for_path(main) == crate::buffer::PreviewKind::Typst;
    path == main || (book && syntax == crate::syntax::Syntax::Typst && main.parent().is_some_and(|book| path.starts_with(book)))
}

/// **Which preview a document has**, as the preview commands read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewState {
    /// No preview shows it.
    None,
    /// The following preview of its kind shows it.
    Following,
    /// It has a preview locked to it.
    Locked,
}

/// A document: what one preview shows — its kind, and its main file as the
/// disk spells it.
pub type Document = (crate::buffer::PreviewKind, PathBuf);

/// **How big the page is drawn** (`:preview-fit`, `[editor] preview_fit`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewFit {
    /// The page as wide as the window, scrolled to the cursor.
    #[default]
    Width,
    /// The whole page in the window.
    Page,
}

impl PreviewFit {
    pub fn parse(word: &str) -> Option<PreviewFit> {
        match word.trim() {
            "width" => Some(PreviewFit::Width),
            "page" => Some(PreviewFit::Page),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            PreviewFit::Width => "width",
            PreviewFit::Page => "page",
        }
    }
}

/// A scroll key pressed in a preview, for the front end, which knows how
/// tall the page is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageStep {
    /// Rows of cells, down when positive.
    Rows(i32),
    /// Half the window's height.
    Halves(i32),
    /// The window's height.
    Screens(i32),
    Top,
    Bottom,
    /// Whole pages, on when positive: `J` and `K`, as Zathura and Emacs's
    /// evil-collection turn them.
    Pages(i32),
    /// The page of that number, from 1: `5G`, `5gg`.
    Goto(usize),
    /// Columns sideways, leftwards when positive: `h` and `l`, which move
    /// along a vertical page the way it reads.
    Across(i32),
}

/// **Where a preview's cursor is**: the file the keys are in (absolute, as
/// tinymist knows it), what kind of file it is, and the cursor's line and
/// character on it, from 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageTarget {
    pub path: Option<PathBuf>,
    pub syntax: crate::syntax::Syntax,
    pub line: usize,
    pub character: usize,
    /// How many characters the line has, its line break not counted.
    pub line_length: usize,
    /// The buffer and its revision, so a new edit is told from an old one.
    pub buffer: u64,
    pub revision: u64,
    /// **The place tinymist is asked to show**, line and character, from 0:
    /// see [`follow_place`].
    pub follow: (usize, usize),
}

/// How many lines from a blank one [`follow_place`] looks for text.
const FOLLOW_REACH: usize = 1000;

/// **The place a page is asked to follow** for the cursor at `line`,
/// `character` of `rope`.
///
/// **The place after the character the cursor stands on**: tinymist looks at
/// what comes before the place it is given, and at the start of a line that
/// is the line break, which is on no page (0.15.12 gives no jump at character
/// 0). **On a blank line, the nearest line with text**: a blank line is on no
/// page either, and `:N`, `{` and `}` land on them all the time. The line
/// above is taken at its end, the place on the page just before the blank;
/// the one below at its first character; the nearer wins, the one above when
/// they are as near.
pub fn follow_place(rope: &ropey::Rope, line: usize, character: usize) -> (usize, usize) {
    let length = |k: usize| rope.line(k).chars().take_while(|c| *c != '\n' && *c != '\r').count();
    let blank = |k: usize| rope.line(k).chars().all(char::is_whitespace);
    let lines = rope.len_lines();
    if line >= lines || !blank(line) {
        return (line, (character + 1).min(length(line.min(lines.saturating_sub(1)))));
    }
    for d in 1..=FOLLOW_REACH {
        if let Some(above) = line.checked_sub(d).filter(|&k| !blank(k)) {
            return (above, length(above));
        }
        if let Some(below) = Some(line + d).filter(|&k| k < lines && !blank(k)) {
            return (below, 1.min(length(below)));
        }
        if d > line && line + d >= lines {
            break;
        }
    }
    (line, 0)
}

/// A path as the language server was handed it, made absolute.
pub(super) fn absolute(path: &Path) -> PathBuf {
    match path.is_absolute() {
        true => path.to_path_buf(),
        false => std::env::current_dir().unwrap_or_default().join(path),
    }
}

impl Editor {
    /// The buffer window `id` shows: the current one for the window with the
    /// keys, the pane's for any other.
    fn buffer_in_window(&self, id: WindowId) -> Option<&Buffer> {
        match id == self.live {
            true => Some(&self.buffers[self.current]),
            false => self.panes.get(&id).and_then(|pane| self.buffer_with(pane.buffer)).map(|i| &self.buffers[i]),
        }
    }

    /// **The windows showing a preview**, in screen order.
    pub fn page_windows(&self) -> Vec<(WindowId, PageWindow)> {
        self.windows
            .leaves()
            .into_iter()
            .filter_map(|id| {
                let buffer = self.buffer_in_window(id)?;
                let main = buffer.preview_main()?.to_path_buf();
                Some((id, PageWindow { preview: buffer.id(), main }))
            })
            .collect()
    }

    /// Whether window `id` shows a page rather than text.
    pub fn is_page_window(&self, id: WindowId) -> bool {
        self.windows.has(id) && self.buffer_in_window(id).is_some_and(Buffer::is_preview)
    }

    /// Whether the keys are in a window showing a preview.
    pub fn page_window_has_the_keys(&self) -> bool {
        self.panel_focus.is_none() && self.buffers[self.current].is_preview()
    }

    /// The preview the keys are in, by its tab's id.
    pub fn preview_with_the_keys(&self) -> Option<u64> {
        self.page_window_has_the_keys().then(|| self.buffers[self.current].id())
    }

    /// **Every preview tab**: its id and the file it typesets, in tab order.
    pub fn previews(&self) -> Vec<(u64, PathBuf)> {
        self.buffers.iter().filter_map(|b| Some((b.id(), b.preview_main()?.to_path_buf()))).collect()
    }

    /// The preview window `id` shows, by its tab's id.
    pub fn preview_in(&self, id: WindowId) -> Option<u64> {
        self.buffer_in_window(id).filter(|b| b.is_preview()).map(Buffer::id)
    }

    /// Whether preview `id` is on the screen, in some window.
    pub fn preview_on_screen(&self, id: u64) -> bool {
        self.page_windows().iter().any(|(_, page)| page.preview == id)
    }

    /// The preview tab of `main`, by its id, of whichever kind.
    pub fn preview_of(&self, main: &Path) -> Option<u64> {
        let main = self.real(main);
        self.buffers.iter().find(|b| b.preview_main().is_some_and(|m| self.real(m) == main)).map(Buffer::id)
    }

    /// **The text of the open buffer holding `path`**, with the buffer's id
    /// and revision so a new edit is told from an old one; `None` when no
    /// buffer holds the file. A Markdown preview renders this, unsaved
    /// edits and all.
    pub fn text_of(&self, path: &Path) -> Option<(u64, u64, String)> {
        let b = &self.buffers[self.buffer_of_file(path)?];
        Some((b.id(), b.revision(), b.rope().to_string()))
    }

    /// [`Self::text_of`] without the text: asked every turn, to know when
    /// the text is worth taking.
    pub fn revision_of(&self, path: &Path) -> Option<(u64, u64)> {
        let b = &self.buffers[self.buffer_of_file(path)?];
        Some((b.id(), b.revision()))
    }

    /// The buffer holding `path`, by its spelling first, by the disk's after.
    fn buffer_of_file(&self, path: &Path) -> Option<usize> {
        self.buffers.iter().position(|b| b.path() == Some(path)).or_else(|| {
            let want = self.real(path);
            self.buffers.iter().position(|b| !b.is_preview() && b.path().is_some_and(|p| self.real(p) == want))
        })
    }

    /// Whether preview `id` is locked to its document.
    pub fn preview_is_locked(&self, id: u64) -> bool {
        self.buffer_with(id).is_some_and(|i| self.buffers[i].preview_locked())
    }

    /// **The HTML export of the Markdown file `path`**, as `:export html`
    /// writes it: the open buffer's text, unsaved edits and all, else the
    /// disk's. The browser's Markdown preview shows it.
    pub fn html_of(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        let (text, title, source) = match self.buffer_of_file(path) {
            Some(i) => {
                let b = &self.buffers[i];
                (b.text().to_string(), b.display_name(), b.syntax())
            }
            None => {
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                (std::fs::read_to_string(path)?, name, crate::syntax::Syntax::Markdown)
            }
        };
        let style = crate::export::Style {
            settings: self.export.clone(),
            title,
            source,
            base: path.parent().map(Path::to_path_buf),
            painter: self.formula_painter,
            look: None,
        };
        Ok(crate::export::export_file(&text, crate::export::Format::Html, &style))
    }

    /// What kind of preview tab `id` is.
    pub fn preview_kind_of(&self, id: u64) -> Option<crate::buffer::PreviewKind> {
        self.buffer_with(id).and_then(|i| self.buffers[i].preview_kind())
    }

    /// A path as the disk spells it, for telling two spellings of one file
    /// apart from two files: `/var/…` and `/private/var/…` on macOS.
    pub(super) fn real(&self, path: &Path) -> PathBuf {
        self.real_path(path).unwrap_or_else(|| absolute(path))
    }

    /// The name a preview of `main` goes by in the tab line: the file's
    /// name, and whether it is locked to it.
    fn preview_label(main: &Path, locked: bool) -> String {
        let name = main.file_name().map_or_else(|| main.display().to_string(), |n| n.to_string_lossy().into_owned());
        match locked {
            true => say!("preview.tab-locked", name),
            false => say!("preview.tab", name),
        }
    }

    /// **Open the following preview of `main`** without showing it, or find
    /// the preview it has. Its id.
    pub(super) fn add_preview_tab(&mut self, main: PathBuf) -> u64 {
        let kind = crate::buffer::PreviewKind::for_path(&main);
        self.add_preview_tab_of(main, kind, false)
    }

    /// **The preview of `main`**, of `kind`, without showing it: the one it
    /// has; else, unless `locked`, the following preview of its kind moved
    /// to it; else a new tab. Its id.
    pub(super) fn add_preview_tab_of(&mut self, main: PathBuf, kind: crate::buffer::PreviewKind, locked: bool) -> u64 {
        if let Some(id) = self.preview_of(&main) {
            return id;
        }
        // As the disk spells it, which is how the buffers spell their files
        // and so how tinymist is told where the cursor is: a task started on
        // `/var/…` finds nothing at `/private/var/…`.
        let main = self.real(&main);
        if let Some(i) = self.following(kind).filter(|_| !locked) {
            self.repoint(i, main, false);
            return self.buffers[i].id();
        }
        let label = Self::preview_label(&main, locked);
        let buffer = Buffer::preview_of(kind, main, locked, &label);
        let id = buffer.id();
        self.buffers.push(buffer);
        self.refresh_sidebar();
        id
    }

    /// **Point preview `i` at the document `main`**, locked to it or
    /// following: its name in the tab line and in the windows showing it
    /// goes with it.
    fn repoint(&mut self, i: usize, main: PathBuf, locked: bool) {
        let label = Self::preview_label(&main, locked);
        self.buffers[i].repoint_preview(main, locked, &label);
        let id = self.buffers[i].id();
        for pane in self.panes.values_mut().filter(|pane| pane.buffer == id) {
            pane.caption = label.clone();
        }
        self.refresh_sidebar();
    }

    /// The following preview of `kind`, by its index.
    fn following(&self, kind: crate::buffer::PreviewKind) -> Option<usize> {
        self.buffers.iter().position(|b| b.preview_kind() == Some(kind) && !b.preview_locked())
    }

    /// The preview locked to the document `main`, by its index.
    fn locked_to(&self, kind: crate::buffer::PreviewKind, main: &Path) -> Option<usize> {
        self.buffers.iter().position(|b| {
            b.preview_kind() == Some(kind) && b.preview_locked() && b.preview_main().is_some_and(|m| self.real(m) == main)
        })
    }

    /// **Which preview the document `main` has**, and that preview's index.
    pub(super) fn preview_state(&self, kind: crate::buffer::PreviewKind, main: &Path) -> (PreviewState, Option<usize>) {
        if let Some(i) = self.locked_to(kind, main) {
            return (PreviewState::Locked, Some(i));
        }
        match self.following(kind).filter(|&i| self.buffers[i].preview_main().is_some_and(|m| self.real(m) == main)) {
            Some(i) => (PreviewState::Following, Some(i)),
            None => (PreviewState::None, None),
        }
    }

    /// **What the open Typst buffers are**: their files and the stamps
    /// their files had when last read or written. A save, or a file opened,
    /// changes it, and with it the books worked out.
    fn note_books(&self) {
        let mut hasher = DefaultHasher::new();
        for b in self.buffers.iter().filter(|b| !b.is_preview() && b.syntax() == crate::syntax::Syntax::Typst) {
            b.path().hash(&mut hasher);
            b.disk_stamp().hash(&mut hasher);
        }
        self.books.borrow_mut().note(hasher.finish());
    }

    /// The open Typst buffers, for the book search.
    fn open_typst(&self) -> Vec<super::books::Open> {
        self.buffers
            .iter()
            .filter(|b| !b.is_preview() && b.syntax() == crate::syntax::Syntax::Typst)
            .filter_map(|b| Some(super::books::Open { path: self.real(b.path()?), text: b.rope().to_string() }))
            .collect()
    }

    /// **The book the Typst file `path` belongs to**, its main file: the
    /// document of an existing preview whose book holds it; else the file
    /// nobody names above it, the open buffers and the disk read (see
    /// [`super::books`]). [`Self::note_books`] first.
    fn book_of(&self, path: &Path) -> PathBuf {
        let mains: Vec<PathBuf> = self
            .buffers
            .iter()
            .filter(|b| b.preview_kind() == Some(crate::buffer::PreviewKind::Typst))
            .filter_map(|b| b.preview_main().map(|m| self.real(m)))
            .collect();
        if let Some(main) = mains.iter().find(|m| m.as_path() == path) {
            return main.clone();
        }
        let mut open = None;
        let mut books = self.books.borrow_mut();
        for main in &mains {
            if !books.has_within(main) {
                books.within(main, open.get_or_insert_with(|| self.open_typst()));
            }
            if books.kept_within(main, path) {
                return main.clone();
            }
        }
        if let Some(top) = books.kept_top(path) {
            return top;
        }
        let home = Self::home_dir().map(|h| self.real(&h));
        books.top_of(path, open.get_or_insert_with(|| self.open_typst()), home.as_deref())
    }

    /// **The document buffer `i` is in**: the preview's own for a preview;
    /// the file for a Markdown file; the book for a Typst file. `None` for a
    /// buffer never saved and for a file with nothing to preview.
    fn document_of(&self, i: usize) -> Option<Document> {
        use crate::buffer::PreviewKind;
        let b = &self.buffers[i];
        if let (Some(kind), Some(main)) = (b.preview_kind(), b.preview_main()) {
            return Some((kind, main.to_path_buf()));
        }
        let path = self.real(b.path()?);
        match b.syntax() {
            crate::syntax::Syntax::Markdown => Some((PreviewKind::Markdown, path)),
            crate::syntax::Syntax::Typst => Some((PreviewKind::Typst, self.book_of(&path))),
            _ => None,
        }
    }

    /// **The document the keys are in** — see [`Self::document_of`].
    pub fn document_here(&self) -> Option<Document> {
        self.note_books();
        self.document_of(self.current)
    }

    /// **Move the previews to where they belong**, once a turn: the
    /// following preview to the document the keys are in, unless that
    /// document has a locked preview; and the previews of a document whose
    /// last buffer closed — the locked one closed, the following one moved
    /// to another open document of its kind, or closed when there is none.
    pub fn steer_previews(&mut self) {
        self.note_books();
        if !self.buffers[self.current].is_preview() {
            if let Some((kind, main)) = self.document_of(self.current) {
                if self.locked_to(kind, &main).is_none() {
                    if let Some(i) = self.following(kind) {
                        if self.buffers[i].preview_main() != Some(main.as_path()) {
                            self.repoint(i, main, false);
                        }
                    }
                }
            }
        }
        let previews: Vec<(u64, crate::buffer::PreviewKind, PathBuf, bool)> = self
            .buffers
            .iter()
            .filter_map(|b| Some((b.id(), b.preview_kind()?, b.preview_main()?.to_path_buf(), b.preview_locked())))
            .collect();
        for (id, kind, main, locked) in previews {
            if self.document_is_open(kind, &main) {
                continue;
            }
            let next = match locked {
                true => None,
                false => self.another_document(kind),
            };
            match (next, self.buffer_with(id)) {
                (Some(main), Some(i)) => self.repoint(i, main, false),
                _ => {
                    self.close_preview(id);
                }
            }
        }
    }

    /// Whether a buffer is open on the document `main`.
    fn document_is_open(&self, kind: crate::buffer::PreviewKind, main: &Path) -> bool {
        (0..self.buffers.len())
            .filter(|&i| !self.buffers[i].is_preview())
            .any(|i| self.document_of(i).is_some_and(|(k, m)| k == kind && m == main))
    }

    /// **An open document of `kind` without a locked preview**, for the
    /// following preview to move to: the one the keys are in first, then in
    /// tab order.
    fn another_document(&self, kind: crate::buffer::PreviewKind) -> Option<PathBuf> {
        std::iter::once(self.current)
            .chain(0..self.buffers.len())
            .filter(|&i| !self.buffers[i].is_preview())
            .filter_map(|i| self.document_of(i))
            .find(|(k, main)| *k == kind && self.locked_to(kind, main).is_none())
            .map(|(_, main)| main)
    }

    /// **The Typst file the preview of this document typesets**, for
    /// `:export-as-preview`: the book this file belongs to, or the one the
    /// preview the keys are in shows. `None` for a buffer never saved.
    pub(super) fn typst_main_here(&self) -> Option<PathBuf> {
        match self.document_here() {
            Some((crate::buffer::PreviewKind::Typst, main)) => Some(main),
            _ => self.current_buffer().path().map(|p| self.real(p)),
        }
    }

    /// **The open files of the book `main` typesets with unsaved changes**:
    /// the `typst` compiler reads the files on disk, where the
    /// preview reads the buffers.
    pub(super) fn unsaved_in_the_book(&self, main: &Path) -> usize {
        self.buffers
            .iter()
            .filter(|b| b.is_modified() && b.path().is_some_and(|p| in_the_book(main, &self.real(p), b.syntax())))
            .count()
    }

    /// Whether `:redraw` asked for the whole terminal to be drawn again
    /// since the front end last asked; asking takes the request.
    pub fn take_redraw(&mut self) -> bool {
        std::mem::take(&mut self.redraw)
    }

    /// **Open the following preview of `main` beside the live window** —
    /// see [`Self::open_page_window_of`].
    pub fn open_page_window(&mut self, main: PathBuf) -> Option<WindowId> {
        let kind = crate::buffer::PreviewKind::for_path(&main);
        self.open_page_window_of(main, kind, false)
    }

    /// **Open the preview of `main` beside the live window**, to its right:
    /// the preview it has, else one of `kind`, following or `locked` (see
    /// [`Self::add_preview_tab_of`]). The keys stay where they are, as a
    /// preview opened to the side leaves them in VS Code. A preview already
    /// on the screen is said to be open instead. The window, or `None` when
    /// there is no room.
    pub fn open_page_window_of(&mut self, main: PathBuf, kind: crate::buffer::PreviewKind, locked: bool) -> Option<WindowId> {
        let tabs = self.buffers.len();
        let preview = self.add_preview_tab_of(main, kind, locked);
        let fresh_tab = self.buffers.len() > tabs;
        if let Some((id, _)) = self.page_windows().into_iter().find(|(_, page)| page.preview == preview) {
            self.status = say!("preview.window-open");
            return Some(id);
        }
        let from = self.live;
        // **Below a 縱書 page, beside a horizontal one**: vertical columns
        // fill the width, so the page takes the height left under them.
        let how = match self.layout() {
            Layout::Vertical => crate::window::Split::Below,
            Layout::Horizontal => crate::window::Split::Beside,
        };
        if let Some(area) = self.text_area.get() {
            let mut tried = self.windows.clone();
            let room = tried.split_on(from, how, false, false).is_some_and(|fresh| {
                let (placed, _) = tried.place(area);
                placed
                    .iter()
                    .filter(|p| p.id == fresh || p.id == from)
                    .all(|p| p.area.height >= Self::WINDOW_MIN_ROWS && p.area.width >= Self::WINDOW_MIN_COLUMNS)
            });
            if !room {
                // Nothing was opened, so neither is the tab.
                if fresh_tab {
                    self.drop_preview_tab(preview);
                }
                self.status = say!("pane.no-room");
                return None;
            }
        }
        let fresh = self.windows.split_on(from, how, false, false)?;
        let caption = self.buffer_with(preview).map(|i| self.buffers[i].display_name()).unwrap_or_default();
        self.panes.insert(fresh, Pane {
            buffer: preview,
            cursor: 0,
            anchor: 0,
            goal_column: None,
            goal_slot: None,
            extend: false,
            highlight: None,
            caption,
            jumps: Vec::new(),
            jump_at: 0,
            seen: vec![preview],
            preview: false,
            half_page: None,
            table: super::PaneTable::Kept { view: None, turned: false },
        });
        Some(fresh)
    }

    /// **The document a preview command is about**: the one the keys are
    /// in. `None`, and said why, for a buffer never saved or a file with
    /// nothing to preview.
    fn preview_document_here(&mut self) -> Option<Document> {
        let found = self.document_here();
        if found.is_none() {
            self.status = match self.current_buffer().path() {
                None => say!("preview.save-first"),
                Some(_) => say!("preview.nothing-to-preview"),
            };
        }
        found
    }

    /// **`:preview` and `:preview-window`**, by the state of this
    /// document's preview: on, a following preview opened beside the text,
    /// or the one it has brought up; off, its preview closed — a locked one
    /// hands the document to the following preview of its kind. Bare is on
    /// when it has none. `fallback`: the browser when a window cannot be
    /// drawn.
    pub(super) fn preview_switch(&mut self, how: crate::command::Switch, fallback: bool) {
        self.steer_previews();
        let Some((kind, main)) = self.preview_document_here() else { return };
        let (state, at) = self.preview_state(kind, &main);
        let on = match how {
            crate::command::Switch::Flip => state == PreviewState::None,
            crate::command::Switch::To(on) => on,
        };
        let id = at.map(|i| self.buffers[i].id());
        match (on, id) {
            (true, Some(id)) if self.preview_on_screen(id) => self.status = say!("preview.window-open"),
            (true, _) => {
                let locked = state == PreviewState::Locked;
                self.preview_request = Some(Preview::Window { path: main, syntax: kind.syntax(), fallback, locked });
            }
            (false, None) => self.status = say!("preview.not-open"),
            (false, Some(id)) => {
                self.close_preview(id);
                if state == PreviewState::Locked {
                    if let Some(i) = self.following(kind) {
                        self.repoint(i, main, false);
                    }
                }
                self.status = say!("preview.window-closed");
            }
        }
    }

    /// **`:preview-lock`**, by the state of this document's preview: on, a
    /// locked preview opened beside the text, or the following one locked
    /// where it stands; off, the locked one following again, in place of
    /// the following preview of its kind, which closes. Bare is off when it
    /// is locked.
    pub(super) fn preview_lock(&mut self, how: crate::command::Switch) {
        self.steer_previews();
        let Some((kind, main)) = self.preview_document_here() else { return };
        let (state, at) = self.preview_state(kind, &main);
        let id = at.map(|i| self.buffers[i].id());
        match (how.over(state == PreviewState::Locked), state, at) {
            (true, PreviewState::None, _) => {
                self.preview_request = Some(Preview::Window { path: main, syntax: kind.syntax(), fallback: true, locked: true });
            }
            (true, PreviewState::Following, Some(i)) => {
                self.repoint(i, main, true);
                self.status = say!("preview.locked");
            }
            (true, _, _) => self.status = say!("preview.already-locked"),
            (false, PreviewState::Locked, Some(_)) => {
                if let Some(old) = self.following(kind).map(|i| self.buffers[i].id()) {
                    self.close_preview(old);
                }
                if let Some(i) = id.and_then(|id| self.buffer_with(id)) {
                    self.repoint(i, main, false);
                }
                self.status = say!("preview.unlocked");
            }
            (false, _, _) => self.status = say!("preview.not-locked"),
        }
    }

    /// **`:preview-browser`**: this document's preview shown in the
    /// browser — the preview it has, or a following one opened for it. The
    /// browser shows one preview: on moves it here; off stops it, whatever
    /// it shows. Bare is on unless it shows this document's.
    pub(super) fn preview_browser(&mut self, how: crate::command::Switch) {
        self.steer_previews();
        let Some((kind, main)) = self.preview_document_here() else { return };
        let (_, at) = self.preview_state(kind, &main);
        let id = at.map(|i| self.buffers[i].id());
        let shown = id.is_some() && self.browser == id;
        match how.over(shown) {
            true if shown => self.preview_request = Some(Preview::Show),
            true => {
                let id = match id {
                    Some(id) => id,
                    None => self.add_preview_tab_of(main, kind, false),
                };
                self.browser = Some(id);
            }
            false => self.browser = None,
        }
    }

    /// **Show the preview of `main` in the browser** in place of a window
    /// that cannot be drawn: the preview it has, else one of `kind`,
    /// following or `locked`, opened without a window. Its id.
    pub fn show_in_the_browser(&mut self, main: PathBuf, kind: crate::buffer::PreviewKind, locked: bool) -> u64 {
        let id = self.add_preview_tab_of(main, kind, locked);
        self.browser = Some(id);
        id
    }

    /// **What the browser is to show**: the preview's id, its kind and its
    /// document; `None` when it shows nothing.
    pub fn browser_preview(&self) -> Option<(u64, crate::buffer::PreviewKind, PathBuf)> {
        let b = &self.buffers[self.buffer_with(self.browser?)?];
        Some((b.id(), b.preview_kind()?, b.preview_main()?.to_path_buf()))
    }

    /// The browser shows nothing any more: what served it could not start.
    pub fn stop_the_browser(&mut self) {
        self.browser = None;
    }

    /// Whether the preview of the document the keys are in is the one the
    /// browser shows.
    pub(super) fn browser_shows_here(&self) -> bool {
        let Some((kind, main)) = self.document_here() else { return false };
        let (_, at) = self.preview_state(kind, &main);
        at.is_some_and(|i| self.browser == Some(self.buffers[i].id()))
    }

    /// The state of the preview of the document the keys are in.
    pub(super) fn preview_state_here(&self) -> PreviewState {
        match self.document_here() {
            Some((kind, main)) => self.preview_state(kind, &main).0,
            None => PreviewState::None,
        }
    }

    /// **Carry the previews of `from` to `to`**: a file renamed or saved
    /// under another name takes the preview it is the document of along.
    /// `from` as the disk spelt it before.
    pub(super) fn carry_previews(&mut self, from: &Path, to: &Path) {
        let to = self.real(to);
        for i in 0..self.buffers.len() {
            if self.buffers[i].preview_main() == Some(from) {
                let locked = self.buffers[i].preview_locked();
                self.repoint(i, to.clone(), locked);
            }
        }
    }

    /// Whether any preview is on the screen.
    pub fn has_a_page_window(&self) -> bool {
        !self.page_windows().is_empty()
    }

    /// **Close preview `id`** — its tab, as `:bc` closes a file's. A window
    /// that showed it goes back to the tab it showed before, and closes
    /// when it showed nothing else; the window with the keys likewise, the
    /// keys going to the window they were in last. `false` when there is no
    /// such preview.
    pub fn close_preview(&mut self, id: u64) -> bool {
        let Some(index) = self.buffer_with(id).filter(|&i| self.buffers[i].is_preview()) else {
            return false;
        };
        if index == self.current {
            let back = self.seen_here.iter().rev().copied().find(|&b| b != id && self.buffer_with(b).is_some());
            match back.and_then(|b| self.buffer_with(b)) {
                Some(i) => self.show_buffer(i),
                None => {
                    if let Some(target) = self.window_after_closing() {
                        let leaving = self.live;
                        if self.switch_to(target) {
                            self.close_window(leaving);
                        }
                    }
                }
            }
        }
        self.drop_preview_tab(id);
        true
    }

    /// Take tab `id` out of the list. When the keys are still in it — the
    /// last window, with no other tab seen in it — the one beside it shows,
    /// as `:bc` leaves it, or an empty scratch when it was the only tab.
    fn drop_preview_tab(&mut self, id: u64) {
        let Some(index) = self.buffer_with(id) else { return };
        if self.buffers.len() == 1 {
            self.buffers[0] = Buffer::scratch();
            self.set_cursor(0);
        } else {
            let was_here = index == self.current;
            self.buffers.remove(index);
            if self.current > index {
                self.current -= 1;
            }
            self.current = self.current.min(self.buffers.len() - 1);
            if was_here {
                let restored = self.current_buffer().saved_cursor();
                self.set_cursor(restored);
            }
        }
        if self.last_file == Some(id) {
            self.last_file = None;
        }
        if self.browser == Some(id) {
            self.browser = None;
        }
        self.forget_the_document();
        self.refresh_sidebar();
    }

    /// **Where preview `id`'s cursor is** — see [`PageTarget`]: the cursor
    /// of the window with the keys, while that window shows the preview's
    /// file or a chapter of its book; `None` otherwise, and the preview
    /// keeps its place.
    pub fn page_target(&self, id: u64) -> Option<PageTarget> {
        let main = self.buffer_with(id).and_then(|i| self.buffers[i].preview_main())?;
        if self.panel_focus.is_some() {
            return None;
        }
        let buffer = &self.buffers[self.current];
        let path = absolute(buffer.path()?);
        if !in_the_book(&self.real(main), &self.real(&path), buffer.syntax()) {
            return None;
        }
        let rope = buffer.rope();
        let at = self.sel.head().min(rope.len_chars());
        let line = rope.char_to_line(at);
        let text = rope.line(line);
        let line_length = text.chars().take_while(|c| *c != '\n' && *c != '\r').count();
        Some(PageTarget {
            path: Some(path),
            syntax: buffer.syntax(),
            line,
            character: at - rope.line_to_char(line),
            line_length,
            buffer: buffer.id(),
            revision: buffer.revision(),
            follow: follow_place(rope, line, at - rope.line_to_char(line)),
        })
    }

    /// The scroll keys pressed in a preview since the last look, taken: they
    /// move [`Self::preview_with_the_keys`].
    pub fn take_page_steps(&mut self) -> Vec<PageStep> {
        std::mem::take(&mut self.page_steps)
    }

    /// How big the page is drawn.
    pub fn preview_fit(&self) -> PreviewFit {
        self.preview_fit
    }

    pub fn set_preview_fit(&mut self, fit: PreviewFit) {
        self.preview_fit = fit;
    }

    /// Whether the page is drawn in the theme's colours rather than as
    /// printed (`:preview-theme`).
    pub fn preview_theme(&self) -> bool {
        self.preview_theme
    }

    pub fn set_preview_theme(&mut self, on: bool) {
        self.preview_theme = on;
    }

    /// Whether the Markdown preview follows the text window into vertical
    /// (`:preview-vertical`).
    pub fn preview_vertical(&self) -> bool {
        self.preview_vertical
    }

    pub fn set_preview_vertical(&mut self, on: bool) {
        self.preview_vertical = on;
    }

    /// **The fonts the Markdown preview is set in**: `[editor]
    /// preview_font_family` when it names any, else the export's
    /// (`[export] font_family`). `:export-as-preview` writes its page in
    /// these; `:export` keeps the export's own.
    pub fn preview_fonts(&self) -> &str {
        match self.preview_fonts.trim().is_empty() {
            true => &self.export.fonts,
            false => &self.preview_fonts,
        }
    }

    pub fn set_preview_fonts(&mut self, fonts: &str) {
        self.preview_fonts = fonts.trim().to_string();
    }

    /// **Whether the Markdown preview is set vertically now**:
    /// `:preview-vertical` is on and the page the writer chose is
    /// vertical. Only the direction is read from the editing window; how the
    /// page is set is the export's.
    pub fn markdown_preview_vertical(&self) -> bool {
        self.preview_vertical && self.layout_chosen() == crate::zong::Layout::Vertical
    }

    /// **A key in a preview**, by the preview's table ([`Place::Preview`]):
    /// the reader's `[keys.preview]` line for it, then a one-key
    /// `[keys.normal]` line naming a command a preview has (`[keys]
    /// inherit`), then the default (`places.rs`, `PREVIEW_KEYS`). A key
    /// nothing names does nothing: there is no text here to move in or
    /// edit, so the preset's `x` is not a preview's. `true` when the key is
    /// taken here; `false` hands it on to Normal mode.
    ///
    /// A count before a key goes to the page's own commands (`5G`, the page
    /// of that number). Under vim, `ZZ` and `ZQ` close the window as
    /// anywhere.
    ///
    /// [`Place::Preview`]: yumete_cjk::actions::Place::Preview
    pub(super) fn on_page_window_key(&mut self, key: Key) -> bool {
        // **A key a sequence is waiting for is the sequence's, not the
        // page's.** `C-w` arrives with `pending` set (the caller asks that
        // already, `on_key`); a flat line of the reader's (`"<space>ff"`)
        // arrives with `alias_held` set instead, and read here its second
        // key would be eaten, the line never finished and the held prefix
        // left to eat the keys after it. `Esc` goes on too, and lets go of
        // what is held.
        if !self.alias_held.is_empty() {
            return false;
        }
        let mut count = self.page_count.take();
        if std::mem::take(&mut self.page_g) {
            let after = match key {
                Key::Char(c) => super::places::PREVIEW_G_KEYS.iter().find(|(k, _)| *k == c).map(|(_, name)| *name),
                _ => None,
            };
            match after {
                Some("goto_file_start") => self.page_steps.push(count.map_or(PageStep::Top, PageStep::Goto)),
                Some("goto_next_buffer") => self.next_buffer(),
                Some("goto_previous_buffer") => self.prev_buffer(),
                Some("goto_last_accessed_file") => self.goto_last_file(),
                _ => {}
            }
            if after.is_some() {
                return true;
            }
            // Any other key after a `g` is its own key, without the count.
            count = None;
        }
        // **vim's way out**: `ZZ` is `:x` and `ZQ` is `:quit!`,
        // which in a preview close its window, or leave from the last one.
        if std::mem::take(&mut self.page_z) {
            match key {
                Key::Char('Z') => self.run_bound_command("x"),
                Key::Char('Q') => self.run_bound_command("quit!"),
                _ => {}
            }
            return true;
        }
        let place = yumete_cjk::actions::Place::Preview;
        let typed = super::keys::key_internal(key);
        let theirs = !self.expanding_alias
            && !typed.is_empty()
            && (self.place_bindings[place as usize].contains_key(&typed) || self.inherited(place, &typed).is_some());
        if key == Key::Char('Z') && self.vim_keymap() && !theirs {
            self.page_z = true;
            return true;
        }
        // A count is the page's, typed before its command.
        if let Key::Char(digit @ '0'..='9') = key {
            if !theirs && (digit != '0' || count.is_some()) {
                let digit = digit.to_digit(10).unwrap_or(0) as usize;
                self.page_count = Some(count.unwrap_or(0).saturating_mul(10).saturating_add(digit));
                return true;
            }
        }
        let Some(commands) = self.commands_in(place, key) else {
            return true;
        };
        // **A default that is a Normal command is Normal mode's key**: `:`,
        // `Space`, `C-w`, `C-o` and `C-i` reach there what they reach in the
        // text, the reader's lines that begin with them included. A line of
        // theirs that names the default's own command is the default.
        let default = super::places::PREVIEW_KEYS.iter().find(|(k, _)| *k == key).map(|(_, cmd)| vec![cmd.name().to_string()]);
        let as_default = !theirs || default.as_ref() == Some(&commands);
        if as_default && !commands.iter().all(|one| Self::a_page_command(one).is_some()) {
            return false;
        }
        for one in &commands {
            match Self::a_page_command(one) {
                Some(cmd) => self.page_command(cmd, count),
                None => self.run_binding(std::slice::from_ref(one), theirs),
            }
            if self.quit_from_binding {
                break;
            }
        }
        true
    }

    /// The preview's own command a name is, if it is one: the scroll keys,
    /// `g` and `Esc`. The rest of a preview's names are Normal commands.
    fn a_page_command(name: &str) -> Option<yumete_cjk::actions::PlaceCmd> {
        use yumete_cjk::actions::PlaceCmd::*;
        yumete_cjk::actions::PlaceCmd::from_name(name).filter(|cmd| {
            matches!(
                cmd,
                MoveLineDown
                    | MoveLineUp
                    | MoveCharLeft
                    | MoveCharRight
                    | HalfPageDown
                    | HalfPageUp
                    | PageDown
                    | PageUp
                    | NextPage
                    | PrevPage
                    | GotoFileStart
                    | GotoLastLine
                    | GotoMode
                    | NormalMode
            )
        })
    }

    /// **One of the preview's own commands**, `count` times where it moves.
    fn page_command(&mut self, cmd: yumete_cjk::actions::PlaceCmd, count: Option<usize>) {
        use yumete_cjk::actions::PlaceCmd;
        let times = count.unwrap_or(1) as i32;
        let step = match cmd {
            PlaceCmd::MoveLineDown => PageStep::Rows(times),
            PlaceCmd::MoveLineUp => PageStep::Rows(-times),
            PlaceCmd::MoveCharLeft => PageStep::Across(times),
            PlaceCmd::MoveCharRight => PageStep::Across(-times),
            PlaceCmd::HalfPageDown => PageStep::Halves(times),
            PlaceCmd::HalfPageUp => PageStep::Halves(-times),
            PlaceCmd::PageDown => PageStep::Screens(times),
            PlaceCmd::PageUp => PageStep::Screens(-times),
            PlaceCmd::NextPage => PageStep::Pages(times),
            PlaceCmd::PrevPage => PageStep::Pages(-times),
            PlaceCmd::GotoFileStart => count.map_or(PageStep::Top, PageStep::Goto),
            PlaceCmd::GotoLastLine => count.map_or(PageStep::Bottom, PageStep::Goto),
            // `g` waits for the key after it, the count kept for `gg`.
            PlaceCmd::GotoMode => {
                self.page_g = true;
                self.page_count = count;
                return;
            }
            // **Esc asks the system's input method aside again**, as it does
            // in Normal mode in the text: the keys here are commands too.
            PlaceCmd::NormalMode => {
                self.say_it_again = true;
                return;
            }
            _ => return,
        };
        self.page_steps.push(step);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The page follows the cursor onto a blank line**: the place after
    /// the cursor's character on a line with text; on a blank one, the
    /// nearest line with text, the one above at its end, the one below at
    /// its first character.
    #[test]
    fn a_blank_line_follows_the_nearest_text() {
        let rope = ropey::Rope::from_str("= 第一章\n\n第一段。\n\n\n\n第二段。\n   \n");
        assert_eq!(follow_place(&rope, 0, 0), (0, 1), "after the character");
        assert_eq!(follow_place(&rope, 2, 3), (2, 4), "at the end, the end");
        assert_eq!(follow_place(&rope, 1, 0), (0, 5), "the line above, at its end");
        assert_eq!(follow_place(&rope, 3, 0), (2, 4), "above, the nearer");
        assert_eq!(follow_place(&rope, 5, 0), (6, 1), "below, the nearer");
        assert_eq!(follow_place(&rope, 4, 0), (2, 4), "as near: the one above");
        assert_eq!(follow_place(&rope, 7, 2), (6, 4), "spaces are blank");
        assert_eq!(follow_place(&rope, 8, 0), (6, 4), "the empty last line");
        let rope = ropey::Rope::from_str("\n\n\n標題\n");
        assert_eq!(follow_place(&rope, 0, 0), (3, 1), "nothing above");
        assert_eq!(follow_place(&ropey::Rope::from_str("\n\n"), 1, 0), (1, 0), "no text at all");
    }
}
