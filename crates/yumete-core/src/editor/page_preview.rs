//! **The page window** (2026-11-07): a window beside the text that shows the
//! page the cursor is on, typeset, as VS Code, Zed and Emacs show a preview.
//!
//! It is a window like any other — `C-w` walks into it, `:q` closes it — that
//! holds no text to edit. The editor keeps which windows are page windows,
//! which text window each follows and which file it typesets; the front end
//! asks tinymist for the page and draws it (`yumete_tui::page_window`).
//!
//! **Page windows do not count as windows when the last one goes**: `:q` in
//! the last text window leaves the editor, as it would with no preview open,
//! and the keys never land in a page window because a text window closed.

use super::*;
use crate::window::WindowId;

/// One page window: the text window it shows the page of, and the file it
/// typesets — the one it was opened on, for a book is one main file that
/// includes its chapters, and the cursor spends the day in a chapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageWindow {
    pub follows: WindowId,
    pub main: PathBuf,
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

/// A scroll key pressed in a page window, for the front end, which knows how
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
}

/// **What a page window is showing the page of**: the followed window's file
/// (absolute, as tinymist knows it), what kind of file it is, and the
/// cursor's line and character on it, from 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageTarget {
    pub path: Option<PathBuf>,
    pub syntax: crate::syntax::Syntax,
    pub line: usize,
    pub character: usize,
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
    /// **The page windows still standing**, in screen order.
    pub fn page_windows(&self) -> Vec<(WindowId, PageWindow)> {
        self.windows
            .leaves()
            .into_iter()
            .filter_map(|id| Some((id, self.page_windows.get(&id)?.clone())))
            .collect()
    }

    /// Whether window `id` shows a page rather than text.
    pub fn is_page_window(&self, id: WindowId) -> bool {
        self.page_windows.contains_key(&id) && self.windows.has(id)
    }

    /// Whether the keys are in a page window.
    pub fn page_window_has_the_keys(&self) -> bool {
        self.panel_focus.is_none() && self.is_page_window(self.live)
    }

    /// **Open a page window beside the live one**, to its right, showing the
    /// page of `main`; the keys stay where they are, as a preview opened to
    /// the side leaves them in VS Code. One that already follows this window
    /// is said to be open instead. The window, or `None` when there is no
    /// room.
    pub fn open_page_window(&mut self, main: PathBuf) -> Option<WindowId> {
        let open = self.page_windows().into_iter().find(|(id, page)| page.follows == self.live || *id == self.live);
        if let Some((id, _)) = open {
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
                self.status = say!("pane.no-room");
                return None;
            }
        }
        let fresh = self.windows.split_on(from, crate::window::Split::Beside, false)?;
        let at = self.sel.head();
        self.panes.insert(fresh, Pane {
            buffer: self.current_buffer().id(),
            cursor: at,
            anchor: at,
            goal_column: None,
            goal_slot: None,
            extend: false,
            highlight: None,
            caption: self.current_buffer().display_name().to_string(),
            jumps: Vec::new(),
            jump_at: 0,
            seen: Vec::new(),
            preview: false,
            half_page: None,
        });
        self.page_windows.insert(fresh, PageWindow { follows: from, main });
        Some(fresh)
    }

    /// **Hand the front end a preview to open** for this buffer: the window
    /// (with the browser's as the `fallback` when the window cannot be drawn),
    /// or the browser's. A buffer never saved has no file to typeset.
    pub(super) fn ask_for_a_preview(&mut self, window: bool, fallback: bool) {
        let Some(path) = self.current_buffer().path().map(Path::to_path_buf) else {
            self.status = say!("preview.save-first");
            return;
        };
        let syntax = self.current_buffer().syntax();
        self.preview_request = Some(match window {
            true => Preview::Window { path, syntax, fallback },
            false => Preview::Start { path, syntax },
        });
    }

    /// Whether any page window is open.
    pub fn has_a_page_window(&self) -> bool {
        !self.page_windows().is_empty()
    }

    /// **Close every page window**; the keys, if one had them, go back to
    /// the window it followed. `false` when there was none.
    pub fn close_page_windows(&mut self) -> bool {
        let pages = self.page_windows();
        if pages.is_empty() {
            self.page_windows.clear();
            return false;
        }
        if self.page_window_has_the_keys() {
            let back = self.page_windows.get(&self.live).map(|p| p.follows).filter(|&w| self.windows.has(w));
            if let Some(back) = back.or_else(|| self.window_after_closing()) {
                self.switch_to(back);
            }
        }
        for (id, _) in pages {
            self.close_window(id);
        }
        true
    }

    /// The file page window `id` typesets now: the front end moves it when
    /// the followed window has gone over to another book.
    pub fn set_page_main(&mut self, id: WindowId, main: PathBuf) {
        if let Some(page) = self.page_windows.get_mut(&id) {
            page.main = main;
        }
    }

    /// **What page window `id` shows the page of** — see [`PageTarget`].
    pub fn page_target(&self, id: WindowId) -> Option<PageTarget> {
        let page = self.page_windows.get(&id)?;
        let (index, cursor) = match page.follows == self.live {
            true => (self.current, self.sel.head()),
            false => {
                let pane = self.panes.get(&page.follows)?;
                (self.buffer_with(pane.buffer)?, pane.cursor)
            }
        };
        let buffer = &self.buffers[index];
        let rope = buffer.rope();
        let at = cursor.min(rope.len_chars());
        let line = rope.char_to_line(at);
        Some(PageTarget {
            path: buffer.path().map(absolute),
            syntax: buffer.syntax(),
            line,
            character: at - rope.line_to_char(line),
            buffer: buffer.id(),
            revision: buffer.revision(),
        })
    }

    /// The scroll keys pressed in a page window since the last look, taken.
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

    /// **A key in a page window**: the scroll keys move the page, `:` and
    /// `C-w` go on to the command line and the window keys, and nothing else
    /// does anything — there is no text here to move in or edit. `true` when
    /// the key was taken here.
    pub(super) fn on_page_window_key(&mut self, key: Key) -> bool {
        let step = match key {
            Key::Char(':') | Key::Ctrl('w') => return false,
            Key::Char('j') | Key::Down | Key::Enter => PageStep::Rows(1),
            Key::Char('k') | Key::Up => PageStep::Rows(-1),
            Key::Ctrl('d') => PageStep::Halves(1),
            Key::Ctrl('u') => PageStep::Halves(-1),
            Key::Ctrl('f') | Key::PageDown => PageStep::Screens(1),
            Key::Ctrl('b') | Key::PageUp => PageStep::Screens(-1),
            Key::Char('g') | Key::Home => PageStep::Top,
            Key::Char('G') | Key::End => PageStep::Bottom,
            _ => return true,
        };
        self.page_steps.push(step);
        true
    }

    /// **Point page window `id`'s own place at what it follows**, before the
    /// keys go into it: the editor's cursor lives in the buffer of the window
    /// with the keys, and this one shows the followed window's.
    pub(super) fn aim_the_page_pane(&mut self, id: WindowId) {
        let Some(follows) = self.page_windows.get(&id).map(|p| p.follows) else { return };
        let (buffer, at) = match follows == self.live {
            true => (self.current_buffer().id(), self.sel.head()),
            false => match self.panes.get(&follows) {
                Some(pane) => (pane.buffer, pane.cursor),
                None => return,
            },
        };
        if let Some(pane) = self.panes.get_mut(&id) {
            pane.buffer = buffer;
            pane.cursor = at;
            pane.anchor = at;
        }
    }

    /// **Window `closed` is gone**: a page window that followed it follows
    /// the window with the keys, or the first text window, and closes when
    /// there is none.
    pub(super) fn page_windows_lose(&mut self, closed: WindowId) {
        self.page_windows.remove(&closed);
        let orphans: Vec<WindowId> =
            self.page_windows.iter().filter(|(_, p)| p.follows == closed).map(|(&id, _)| id).collect();
        for id in orphans {
            let heir = Some(self.live)
                .filter(|&w| w != id && !self.page_windows.contains_key(&w))
                .or_else(|| self.windows.leaves().into_iter().find(|w| !self.page_windows.contains_key(w)));
            match heir {
                Some(heir) => {
                    if let Some(page) = self.page_windows.get_mut(&id) {
                        page.follows = heir;
                    }
                }
                None if id != self.live => {
                    self.close_window(id);
                }
                None => {}
            }
        }
    }

    /// **The windows without the page windows** — what a session keeps: a
    /// page is made again by asking, and a window of one with no tinymist
    /// to draw it would come back empty.
    pub(super) fn windows_kept(&self) -> crate::window::Windows {
        let mut kept = self.windows.clone();
        for id in self.page_windows.keys() {
            kept.close(*id);
        }
        kept
    }

    /// The window a session says the keys are in: a page window's own text
    /// window when they are in a page window.
    pub(super) fn keys_window_kept(&self) -> WindowId {
        match self.page_windows.get(&self.live) {
            Some(page) if self.windows.has(page.follows) => page.follows,
            _ => self.live,
        }
    }
}
