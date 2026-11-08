//! **The preview tab** (2026-11-08): the typeset page of a Typst file, or a
//! Markdown file rendered, as VS Code, Zed and Emacs show a preview, in a
//! tab of its own.
//!
//! A preview is a tab like an open file — in the tab line, `:b`, the buffer
//! picker, `gn` and `gp` — whose buffer holds no text: it names the Typst
//! file it typesets (its *main* file), and a window showing it draws that
//! file's page (`yumete_tui::page_window`). It is **bound to that file**: it
//! follows the cursor while the keys are in a window on that file or on a
//! chapter of its book (a Typst file under the main file's folder), and
//! otherwise keeps its place. Several may be open, one a document.
//!
//! `:bc` on it, and `:view-preview off`, close it; `:q` in a window showing
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

/// **How big the page is drawn** (`:view-preview-fit`, `[editor] preview_fit`).
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
    /// evil-collection turn them (2026-11-07).
    Pages(i32),
    /// The page of that number, from 1: `5G`, `5gg`.
    Goto(usize),
    /// Columns sideways, leftwards when positive: `h` and `l`, which move
    /// along a vertical page the way it reads (2026-11-08).
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

    /// What kind of preview tab `id` is.
    pub fn preview_kind_of(&self, id: u64) -> Option<crate::buffer::PreviewKind> {
        self.buffer_with(id).and_then(|i| self.buffers[i].preview_kind())
    }

    /// A path as the disk spells it, for telling two spellings of one file
    /// apart from two files: `/var/…` and `/private/var/…` on macOS.
    fn real(&self, path: &Path) -> PathBuf {
        self.real_path(path).unwrap_or_else(|| absolute(path))
    }

    /// The name a preview of `main` goes by in the tab line.
    fn preview_label(main: &Path) -> String {
        let name = main.file_name().map_or_else(|| main.display().to_string(), |n| n.to_string_lossy().into_owned());
        say!("preview.tab", name)
    }

    /// **Open the preview tab of `main`** without showing it, or find the
    /// one open. Its id.
    pub(super) fn add_preview_tab(&mut self, main: PathBuf) -> u64 {
        let kind = crate::buffer::PreviewKind::for_path(&main);
        self.add_preview_tab_of(main, kind)
    }

    /// [`Self::add_preview_tab`], of `kind`.
    pub(super) fn add_preview_tab_of(&mut self, main: PathBuf, kind: crate::buffer::PreviewKind) -> u64 {
        if let Some(id) = self.preview_of(&main) {
            return id;
        }
        // As the disk spells it, which is how the buffers spell their files
        // and so how tinymist is told where the cursor is: a task started on
        // `/var/…` finds nothing at `/private/var/…`.
        let main = self.real(&main);
        let label = Self::preview_label(&main);
        let buffer = Buffer::preview_of(kind, main, &label);
        let id = buffer.id();
        self.buffers.push(buffer);
        self.refresh_sidebar();
        id
    }

    /// **The preview of the document the keys are in**, by its index: the
    /// preview itself when the keys are in one; else the one typesetting
    /// this file, or, failing that, the book it is a chapter of.
    fn preview_here(&self) -> Option<usize> {
        let here = &self.buffers[self.current];
        if here.is_preview() {
            return Some(self.current);
        }
        let path = self.real(here.path()?);
        let syntax = here.syntax();
        let mains: Vec<(usize, PathBuf)> =
            self.buffers.iter().enumerate().filter_map(|(i, b)| Some((i, self.real(b.preview_main()?)))).collect();
        mains.iter().find(|(_, main)| *main == path).map(|(i, _)| *i).or_else(|| {
            mains
                .iter()
                .filter(|(_, main)| in_the_book(main, &path, syntax))
                .max_by_key(|(_, main)| main.components().count())
                .map(|(i, _)| *i)
        })
    }

    /// **The open Typst file that pulls this one in** (2026-11-08), by
    /// `#include` or `#import`: a chapter typeset on its own lacks what its
    /// book sets up, so the book is what a chapter's new preview typesets.
    /// Only open files are read, and only one step up; `None` when no open
    /// file names this one.
    fn includer_of_here(&self) -> Option<PathBuf> {
        let here = &self.buffers[self.current];
        if here.syntax() != crate::syntax::Syntax::Typst {
            return None;
        }
        let me = self.real(here.path()?);
        self.buffers.iter().enumerate().filter(|(i, _)| *i != self.current).find_map(|(_, b)| {
            let path = b.path().filter(|_| b.syntax() == crate::syntax::Syntax::Typst)?;
            let dir = path.parent()?;
            let names = b.rope().lines().filter_map(|line| quoted_path(&line.to_string()));
            names.into_iter().any(|name| self.real(&dir.join(name)) == me).then(|| path.to_path_buf())
        })
    }

    /// Whether the preview of this document is on the screen.
    pub(super) fn preview_here_on_screen(&self) -> bool {
        self.preview_here().is_some_and(|i| self.preview_on_screen(self.buffers[i].id()))
    }

    /// **Open the preview of `main` beside the live window**, to its right,
    /// opening its tab first if need be; the keys stay where they are, as a
    /// preview opened to the side leaves them in VS Code. A preview already
    /// on the screen is said to be open instead. The window, or `None` when
    /// there is no room.
    pub fn open_page_window(&mut self, main: PathBuf) -> Option<WindowId> {
        let kind = crate::buffer::PreviewKind::for_path(&main);
        self.open_page_window_of(main, kind)
    }

    /// [`Self::open_page_window`], a preview of `kind`: Markdown's own for a
    /// Markdown file, the typeset page for a Typst one.
    pub fn open_page_window_of(&mut self, main: PathBuf, kind: crate::buffer::PreviewKind) -> Option<WindowId> {
        let fresh_tab = self.preview_of(&main).is_none();
        let preview = self.add_preview_tab_of(main, kind);
        if let Some((id, _)) = self.page_windows().into_iter().find(|(_, page)| page.preview == preview) {
            self.status = say!("preview.window-open");
            return Some(id);
        }
        let from = self.live;
        if let Some(area) = self.text_area.get() {
            let mut tried = self.windows.clone();
            let room = tried.split_on(from, crate::window::Split::Beside, false).is_some_and(|fresh| {
                let (placed, _) = tried.place(area, self.splits_right_to_left());
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
        let fresh = self.windows.split_on(from, crate::window::Split::Beside, false)?;
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
        });
        Some(fresh)
    }

    /// **Hand the front end a preview to open** for this document: the
    /// window (with the browser's as the `fallback` when the window cannot
    /// be drawn), or the browser's. The window of a document with a preview
    /// tab is that tab's; a buffer never saved has no file to typeset.
    pub(super) fn ask_for_a_preview(&mut self, window: bool, fallback: bool) {
        let tab = self.preview_here().and_then(|i| {
            let b = &self.buffers[i];
            Some((b.preview_main()?.to_path_buf(), b.preview_kind()?.syntax()))
        });
        let (path, syntax) = match (tab, self.current_buffer().path()) {
            (Some((main, syntax)), _) if window || self.buffers[self.current].is_preview() => (main, syntax),
            (_, Some(path)) => match self.includer_of_here().filter(|_| window) {
                Some(book) => (book, crate::syntax::Syntax::Typst),
                None => (path.to_path_buf(), self.current_buffer().syntax()),
            },
            _ => {
                self.status = say!("preview.save-first");
                return;
            }
        };
        self.preview_request = Some(match window {
            true => Preview::Window { path, syntax, fallback },
            false => Preview::Start { path, syntax },
        });
    }

    /// Whether any preview is on the screen.
    pub fn has_a_page_window(&self) -> bool {
        !self.page_windows().is_empty()
    }

    /// **Close the preview of this document**: its tab, and every window
    /// that showed nothing else. `false` when there was none.
    pub(super) fn close_preview_here(&mut self) -> bool {
        match self.preview_here() {
            Some(i) => {
                let id = self.buffers[i].id();
                self.close_preview(id)
            }
            None => false,
        }
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
    /// printed (`:view-preview-theme`).
    pub fn preview_theme(&self) -> bool {
        self.preview_theme
    }

    pub fn set_preview_theme(&mut self, on: bool) {
        self.preview_theme = on;
    }

    /// Whether the Markdown preview follows the text window into vertical
    /// (`:view-preview-vertical`).
    pub fn preview_vertical(&self) -> bool {
        self.preview_vertical
    }

    pub fn set_preview_vertical(&mut self, on: bool) {
        self.preview_vertical = on;
    }

    /// **Whether the Markdown preview is set vertically now** (2026-11-08):
    /// `:view-preview-vertical` is on and the page the writer chose is
    /// vertical. Only the direction is read from the editing window; how the
    /// page is set is the export's.
    pub fn markdown_preview_vertical(&self) -> bool {
        self.preview_vertical && self.layout_chosen() == crate::zong::Layout::Vertical
    }

    /// **A key in a preview**: the scroll keys move the page, `:`, `C-w`,
    /// `Space`, `C-o` and `C-i` go on to the command line, the key groups
    /// and the jump list, and nothing else does anything — there is no text
    /// here to move in or edit. `true` when the key was taken here.
    ///
    /// The keys are a PDF reader's under vim (2026-11-07, as Zathura and
    /// evil-collection have them): `gg` and `G` the top and bottom, `J` and
    /// `K` a whole page, and a count before `G` or `gg` the page of that
    /// number. `gn`, `gp`, `ga`, `gt` and `gT` go to another tab; under vim,
    /// `ZZ` and `ZQ` close the window as anywhere.
    pub(super) fn on_page_window_key(&mut self, key: Key) -> bool {
        let mut count = self.page_count.take();
        if std::mem::take(&mut self.page_g) {
            match key {
                Key::Char('g') => self.page_steps.push(count.map_or(PageStep::Top, PageStep::Goto)),
                // **The tabs from a preview** (2026-11-08): helix's `gn`,
                // `gp` and `ga`, and vim's `gt` and `gT`, whatever the keymap
                // — a preview is a tab, and this is how a tab is left.
                Key::Char('n' | 't') => self.next_buffer(),
                Key::Char('p' | 'T') => self.prev_buffer(),
                Key::Char('a') => self.goto_last_file(),
                // A `g` that is nothing else here takes its count.
                _ => {}
            }
            if matches!(key, Key::Char('g' | 'n' | 't' | 'p' | 'T' | 'a')) {
                return true;
            }
            // Any other key after a `g` is its own key, without the count.
            count = None;
        }
        // **vim's way out** (2026-11-08): `ZZ` is `:x` and `ZQ` is `:q!`,
        // which in a preview close its window, or leave from the last one.
        if std::mem::take(&mut self.page_z) {
            match key {
                Key::Char('Z') => self.run_bound_command("x"),
                Key::Char('Q') => self.run_bound_command("quit!"),
                _ => {}
            }
            return true;
        }
        if key == Key::Char('Z') && self.vim_keymap() {
            self.page_z = true;
            return true;
        }
        let times = count.unwrap_or(1) as i32;
        let step = match key {
            Key::Char(':') | Key::Ctrl('w') | Key::Char(' ') | Key::Ctrl('o') | Key::Ctrl('i') => return false,
            Key::Char(digit @ '0'..='9') if digit != '0' || count.is_some() => {
                let digit = digit.to_digit(10).unwrap_or(0) as usize;
                self.page_count = Some(count.unwrap_or(0).saturating_mul(10).saturating_add(digit));
                return true;
            }
            Key::Char('g') => {
                self.page_g = true;
                self.page_count = count;
                return true;
            }
            Key::Char('j') | Key::Down | Key::Enter => PageStep::Rows(times),
            Key::Char('k') | Key::Up => PageStep::Rows(-times),
            Key::Char('h') | Key::Left => PageStep::Across(times),
            Key::Char('l') | Key::Right => PageStep::Across(-times),
            Key::Ctrl('d') => PageStep::Halves(times),
            Key::Ctrl('u') => PageStep::Halves(-times),
            Key::Ctrl('f') | Key::PageDown => PageStep::Screens(times),
            Key::Ctrl('b') | Key::PageUp => PageStep::Screens(-times),
            Key::Char('J') => PageStep::Pages(times),
            Key::Char('K') => PageStep::Pages(-times),
            Key::Home => PageStep::Top,
            Key::Char('G') => count.map_or(PageStep::Bottom, PageStep::Goto),
            Key::End => PageStep::Bottom,
            _ => return true,
        };
        self.page_steps.push(step);
        true
    }
}
