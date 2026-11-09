//! **The Markdown preview, drawn**: a Markdown file rendered by
//! `yumete-mdview` in proportional type, in a preview tab of its own, as VS
//! Code's Markdown preview sits beside the text.
//!
//! The editor keeps the tabs and which windows show them
//! (`editor/page_preview.rs`, the same as a Typst preview's); this draws
//! them. Two halves, as [`crate::page_window`] has:
//!
//! - [`MdPreviews`], owned by the loop, keeps **one worker thread a preview
//!   tab**, which holds that tab's [`yumete_mdview::Preview`]: it is handed
//!   the text once the typing has been still for [`SETTLE`], the window's
//!   size in cells and pixels, the theme, and where to stand — the cursor's
//!   line, which it puts a third of the way down, or a height kept by hand —
//!   and hands back one picture exactly the size of the window. Only previews
//!   on the screen are asked for anything.
//! - [`draw`], called by the frame, puts the latest picture into the window
//!   through the terminal's graphics ([`crate::graphics`]).
//!
//! **No feedback loop**: the picture's size comes from the window's cells and
//! the cell's pixels alone, never from a picture read back, so nothing it
//! draws can change what it is asked for next (the Typst window's page once
//! shook between two sizes that way).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ratatui::layout::{Alignment, Rect};
use ratatui::widgets::{Paragraph, Wrap};

use yumete_core::buffer::PreviewKind;
use yumete_core::editor::{Editor, PageStep};
use yumete_core::say;
use yumete_core::window::WindowId;
use yumete_mdview::{Options, Preview, Theme};

use crate::server::Doorbell;

/// How long the typing has to be still before the preview is laid out
/// again: a held key is not a layout a character.
pub const SETTLE: Duration = Duration::from_millis(200);

/// **Where the picture stands in the page.**
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Top {
    /// The source line's place a third of the way down the window.
    Line(usize),
    /// A height in the page, in pixels: kept by hand, or kept while the keys
    /// are elsewhere.
    At(i64),
}

/// **What a worker is asked to draw**: the window in cells, a cell in
/// pixels, the colours, and where to stand.
#[derive(Debug, Clone, PartialEq)]
pub struct Want {
    pub columns: u16,
    pub rows: u16,
    pub cell: (u16, u16),
    pub theme: Theme,
    pub top: Top,
    /// `[editor] preview_font_family`, else `[export] font_family`: the
    /// preview is set in the fonts the export asks for unless the writer
    /// names others for the screen.
    pub fonts: String,
    /// `[export] ruby_*`: the ruby spellings the export reads.
    pub ruby: yumete_core::ruby::Dialects,
    /// **Set vertically** while the text window is and
    /// `:preview-vertical` is on, as `[export] vertical_latin` and
    /// `[export] hanging` say.
    pub vertical: Option<yumete_mdview::Vertical>,
    /// The line of the file the cursor is on, for the marks on the frame.
    pub caret: Option<usize>,
}

impl Want {
    /// The picture's size in pixels.
    pub fn pixels(&self) -> (u32, u32) {
        (self.columns as u32 * self.cell.0.max(1) as u32, self.rows as u32 * self.cell.1.max(1) as u32)
    }

    /// How far the picture reaches along the page: its height, or its width
    /// down a vertical page, which goes on to the left.
    pub fn along(&self) -> f32 {
        let (w, h) = self.pixels();
        if self.vertical.is_some() { w as f32 } else { h as f32 }
    }

    /// Device pixels to a CSS pixel: a cell 20 pixels tall sets the body at
    /// 16 pixels, near the terminal's own type, and a Retina terminal's
    /// taller cell twice that.
    pub fn scale(&self) -> f32 {
        (self.cell.1.max(1) as f32 / 20.0).clamp(0.5, 4.0)
    }
}

/// A job for a worker: new text if there is any, where the file is, and
/// what to draw.
struct Job {
    text: Option<Arc<String>>,
    base_dir: PathBuf,
    want: Want,
}

/// **A picture, drawn**: the PNG, the cells it was made for, where it stands
/// in the page and how tall the page is.
#[derive(Debug)]
pub struct Shot {
    pub png: Vec<u8>,
    pub hash: u64,
    pub columns: u16,
    pub rows: u16,
    pub top: f32,
    pub page: f32,
    /// Where the cursor's line is along the page, for the marks on the frame.
    pub caret: Option<f32>,
    /// Set down the page, columns going on to the left.
    pub vertical: bool,
}

/// **What a Markdown preview's window draws**, shared by the loop and the
/// frame (which `--shot` and the tests draw without the loop).
#[derive(Debug, Default)]
struct Board {
    shot: Option<Arc<Shot>>,
    /// The window's size in cells, when last drawn.
    cells: Option<(u16, u16)>,
}

static BOARDS: Mutex<Option<HashMap<u64, Board>>> = Mutex::new(None);

fn with_board<T>(id: u64, work: impl FnOnce(&mut Board) -> T) -> T {
    let mut all = BOARDS.lock().unwrap_or_else(|e| e.into_inner());
    work(all.get_or_insert_with(HashMap::new).entry(id).or_default())
}

/// The window size preview `id` was last drawn at, in cells.
pub fn cells_of(id: u64) -> Option<(u16, u16)> {
    with_board(id, |b| b.cells)
}

/// The picture preview `id` shows now — for a test.
pub fn shot_of(id: u64) -> Option<Arc<Shot>> {
    with_board(id, |b| b.shot.clone())
}

/// Clear every board — a test's.
pub fn clear() {
    *BOARDS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

static BELL: OnceLock<Doorbell> = OnceLock::new();

/// Wake the loop this way when a picture is ready.
pub fn ring_on_news(bell: Doorbell) {
    let _ = BELL.set(bell);
}

/// Where a preview's text comes from: the open buffer at a revision, or the
/// file on disk when no buffer holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Buffer(u64, u64),
    Disk,
}

/// **One Markdown preview tab**: its worker and what it was last asked.
struct Tab {
    main: PathBuf,
    jobs: mpsc::Sender<Job>,
    shots: mpsc::Receiver<Shot>,
    /// The text handed over last, and when a newer one was first seen.
    source: Option<Source>,
    changed: Option<Instant>,
    sent: Option<Want>,
    /// Where the picture stands, as the latest request asks: what the scroll
    /// keys move from, and what is kept while the keys are elsewhere.
    top: f32,
    page: f32,
    shown: bool,
    /// How many jobs were sent — for the tests that count feedback.
    jobs_sent: usize,
}

impl Tab {
    fn start(main: PathBuf) -> Tab {
        let (jobs, inbox) = mpsc::channel::<Job>();
        let (out, shots) = mpsc::channel::<Shot>();
        std::thread::Builder::new()
            .name("md-preview".to_string())
            .spawn(move || work(inbox, out))
            .expect("a thread for the Markdown preview");
        Tab { main, jobs, shots, source: None, changed: None, sent: None, top: 0.0, page: 0.0, shown: false, jobs_sent: 0 }
    }
}

/// **The worker**: one `Preview`, kept between jobs so a block that did not
/// change keeps its layout; only the newest job is drawn.
fn work(inbox: mpsc::Receiver<Job>, out: mpsc::Sender<Shot>) {
    let mut preview: Option<(u32, Preview)> = None; // the scale's bits, the preview
    let mut theme: Option<Theme> = None;
    let mut fonts: Option<String> = None;
    let mut ruby: Option<yumete_core::ruby::Dialects> = None;
    let mut vertical: Option<Option<yumete_mdview::Vertical>> = None;
    let mut width = 0u32;
    let mut text: Arc<String> = Arc::new(String::new());
    let mut base_dir = PathBuf::new();
    while let Ok(mut job) = inbox.recv() {
        // Only the newest is drawn, with the newest text any of them carried.
        while let Ok(next) = inbox.try_recv() {
            let carried = next.text.clone().or(job.text.take());
            job = next;
            job.text = carried;
        }
        let mut relay = false;
        if let Some(t) = job.text.take() {
            text = t;
            relay = true;
        }
        let want = &job.want;
        let (w, h) = want.pixels();
        let scale = want.scale();
        let fresh = preview.as_ref().is_none_or(|(bits, _)| *bits != scale.to_bits()) || job.base_dir != base_dir;
        if fresh {
            base_dir = job.base_dir.clone();
            let options = Options { width: w, scale, theme: want.theme.clone(), base_dir: base_dir.clone() };
            preview = Some((scale.to_bits(), Preview::new(options, Box::new(crate::tex::Ratex))));
            theme = Some(want.theme.clone());
            fonts = None;
            ruby = None;
            vertical = None;
            width = w;
            relay = true;
        }
        let Some((_, p)) = preview.as_mut() else { continue };
        if theme.as_ref() != Some(&want.theme) {
            theme = Some(want.theme.clone());
            p.set_theme(want.theme.clone());
        }
        if ruby != Some(want.ruby) {
            ruby = Some(want.ruby);
            p.set_ruby(want.ruby);
            relay = true;
        }
        if fonts.as_ref() != Some(&want.fonts) {
            fonts = Some(want.fonts.clone());
            p.set_fonts(&want.fonts);
        }
        if width != w {
            width = w;
            p.set_width(w);
        }
        // A vertical page's columns are as long as the window is tall.
        p.set_height(h);
        if vertical != Some(want.vertical) {
            vertical = Some(want.vertical);
            p.set_vertical(want.vertical);
        }
        if relay {
            p.set_text(&text);
        }
        let page = p.size().1;
        let along = want.along();
        let top = match want.top {
            Top::Line(line) => p.y_for_line(line) - along / 3.0,
            Top::At(y) => y as f32,
        };
        let top = top.clamp(0.0, (page - along).max(0.0)).round();
        let caret = want.caret.map(|line| p.y_for_line(line));
        let mut picture = p.render(top, h);
        if let Some((from, to)) = want.caret.and_then(|line| p.block_span_of(line)) {
            paint_the_bar(&mut picture, (from - top, to - top), want.vertical.is_some(), want.scale(), p.caret_colour());
        }
        let png = picture.png();
        let hash = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            png.hash(&mut hasher);
            hasher.finish()
        };
        if out.send(Shot { png, hash, columns: want.columns, rows: want.rows, top, page, caret, vertical: want.vertical.is_some() }).is_err() {
            return;
        }
        if let Some(ring) = BELL.get() {
            ring();
        }
    }
}

/// **The Markdown previews**, owned by the loop: one a tab, by its id.
#[derive(Default)]
pub struct MdPreviews {
    tabs: HashMap<u64, Tab>,
    /// The palette last handed to the editor for `:export-as-preview`.
    looked: Option<Theme>,
}

impl MdPreviews {
    /// **One turn**: let the closed tabs go; take the pictures that came;
    /// move the preview the keys are in; for each preview on the screen,
    /// hand its worker what changed. Never waits. `true` when there is
    /// something new to draw.
    pub fn follow(&mut self, editor: &mut Editor, config: &yumete_config::Config) -> bool {
        let previews: Vec<(u64, PathBuf)> = editor
            .previews()
            .into_iter()
            .filter(|(id, _)| editor.preview_kind_of(*id) == Some(PreviewKind::Markdown))
            .collect();
        let mut redraw = false;
        let gone: Vec<u64> = self.tabs.keys().copied().filter(|id| !previews.iter().any(|(p, _)| p == id)).collect();
        for id in gone {
            self.tabs.remove(&id);
            crate::graphics::drop_pages_of(id);
            if let Some(all) = BOARDS.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                all.remove(&id);
            }
            redraw = true;
        }
        let keys = editor.preview_with_the_keys().filter(|id| previews.iter().any(|(p, _)| p == id));
        let steps = match keys {
            Some(_) => editor.take_page_steps(),
            None => Vec::new(),
        };
        let now = Instant::now();
        let cell = crate::graphics::cell_pixels();
        let theme = crate::preview_theme::theme(crate::theme::Palette::of(config));
        // **`:export-as-preview` writes the page in these colours**:
        // handed over whenever they change — a theme, or the
        // mood — so the page is the preview as it is drawn now.
        if self.looked.as_ref() != Some(&theme) {
            self.looked = Some(theme.clone());
            let palette = theme.clone();
            editor.set_preview_look(Arc::new(move |text: &str, fonts: &str| yumete_mdview::look::look(&palette, fonts, text)));
        }
        for (id, main) in previews {
            let shown = editor.preview_on_screen(id);
            let tab = self.tabs.entry(id).or_insert_with(|| Tab::start(main.clone()));
            tab.main = main;
            while let Ok(shot) = tab.shots.try_recv() {
                // The latest request's place stands unless a key moved it
                // since; the page's height is the picture's.
                tab.page = shot.page;
                if tab.sent.as_ref().is_some_and(|w| matches!(w.top, Top::Line(_))) {
                    tab.top = shot.top;
                }
                with_board(id, |b| b.shot = Some(Arc::new(shot)));
                redraw = true;
            }
            tab.shown = shown;
            if !shown {
                continue;
            }
            let Some((columns, rows)) = cells_of(id) else { continue };
            // The text: the open buffer's once the typing has settled, else
            // the file's.
            let mut text = None;
            match editor.revision_of(&tab.main) {
                Some((buffer, revision)) if tab.source != Some(Source::Buffer(buffer, revision)) => {
                    let first = !matches!(tab.source, Some(Source::Buffer(..)));
                    let since = *tab.changed.get_or_insert(now);
                    if first || now.duration_since(since) >= SETTLE {
                        text = editor.text_of(&tab.main).map(|(_, _, t)| Arc::new(t));
                        tab.source = Some(Source::Buffer(buffer, revision));
                        tab.changed = None;
                    }
                }
                Some(_) => {}
                None if tab.source != Some(Source::Disk) => {
                    text = Some(Arc::new(std::fs::read_to_string(&tab.main).unwrap_or_default()));
                    tab.source = Some(Source::Disk);
                    tab.changed = None;
                }
                None => {}
            }
            // Where to stand: the cursor's line while the keys are in this
            // file; a height moved by hand; else where it stood.
            let mut top = match (editor.page_target(id), tab.sent.as_ref().map(|w| w.top)) {
                (Some(target), _) => Top::Line(target.line),
                // The keys left the file: the picture stays where the cursor
                // last put it, without being drawn again to say so.
                (None, Some(line @ Top::Line(_))) => line,
                (None, _) => Top::At(tab.top as i64),
            };
            let vertical = match editor.markdown_preview_vertical() {
                true => Some(yumete_mdview::Vertical { latin: editor.export_settings().latin, hanging: editor.export_settings().hanging }),
                false => None,
            };
            if keys == Some(id) && !steps.is_empty() {
                // Down a vertical page a step is a column, about two cells
                // wide, and a screen the window's width, leftwards.
                let (row, screen) = match vertical {
                    Some(_) => (2.0 * cell.0.max(1) as f32, columns as f32 * cell.0.max(1) as f32),
                    None => (cell.1.max(1) as f32, rows as f32 * cell.1.max(1) as f32),
                };
                let mut y = tab.top;
                for step in &steps {
                    y = match *step {
                        PageStep::Rows(n) => y + n as f32 * row,
                        PageStep::Across(n) if vertical.is_some() => y + n as f32 * row,
                        PageStep::Across(_) => y,
                        PageStep::Halves(n) => y + n as f32 * (screen / 2.0).max(row),
                        PageStep::Screens(n) => y + n as f32 * screen,
                        PageStep::Top => 0.0,
                        PageStep::Bottom => tab.page,
                        // A Markdown page is one long page: `J`, `K` and
                        // `5G` have no page to turn to.
                        PageStep::Pages(_) | PageStep::Goto(_) => y,
                    };
                }
                tab.top = y.clamp(0.0, (tab.page - screen).max(0.0)).round();
                top = Top::At(tab.top as i64);
            }
            let want = Want {
                columns,
                rows,
                cell,
                theme: theme.clone(),
                top,
                fonts: editor.preview_fonts().to_string(),
                ruby: editor.export_settings().dialects(),
                vertical,
                caret: editor.page_target(id).map(|t| t.line).or_else(|| tab.sent.as_ref().and_then(|w| w.caret)),
            };
            if text.is_some() || tab.sent.as_ref() != Some(&want) {
                let base_dir = tab.main.parent().map(PathBuf::from).unwrap_or_default();
                if tab.jobs.send(Job { text, base_dir, want: want.clone() }).is_ok() {
                    tab.jobs_sent += 1;
                }
                tab.sent = Some(want);
            }
        }
        redraw
    }

    /// **How long the loop may sleep**: until the typing in a previewed file
    /// has been still for [`SETTLE`]. A picture that comes rings the bell.
    ///
    /// **At once when a window was drawn at another size** than its picture
    /// was asked for — the terminal resized, or the window moved to another
    /// side: the picture for the new size is asked for on the next turn,
    /// which no key has to bring.
    pub fn due_in(&self) -> Option<Duration> {
        let resized = self.tabs.iter().any(|(id, t)| {
            t.shown && t.sent.as_ref().is_some_and(|w| cells_of(*id).is_some_and(|c| c != (w.columns, w.rows)))
        });
        if resized {
            return Some(Duration::ZERO);
        }
        self.tabs
            .values()
            .filter(|t| t.shown)
            .filter_map(|t| t.changed.map(|since| SETTLE.saturating_sub(since.elapsed())))
            .min()
    }

    /// How many jobs preview `id`'s worker was sent — for a test.
    pub fn jobs_sent(&self, id: u64) -> usize {
        self.tabs.get(&id).map_or(0, |t| t.jobs_sent)
    }

    /// What preview `id` last asked its worker for — for a test.
    pub fn sent(&self, id: u64) -> Option<Want> {
        self.tabs.get(&id).and_then(|t| t.sent.clone())
    }
}

/// **Draw the Markdown preview window `id` shows into `area`**: the latest
/// picture, from its top-left corner; while the first is being made, a word.
/// `framed`: `area` has a frame round it, and the cursor's line is marked
/// on it.
pub fn draw(frame: &mut ratatui::Frame, editor: &Editor, config: &yumete_config::Config, area: Rect, id: WindowId, framed: bool) {
    let Some(preview) = editor.preview_in(id) else { return };
    let ink = crate::theme::Palette::of(config);
    frame.render_widget(Paragraph::new("").style(ink.page()), area);
    let shot = with_board(preview, |b| {
        b.cells = Some((area.width, area.height));
        b.shot.clone()
    });
    let can = crate::graphics::support() != crate::graphics::Support::None;
    match shot.filter(|_| can) {
        Some(shot) => {
            let picture = crate::graphics::show_page(preview, shot.hash, &shot.png, shot.columns, shot.rows);
            // A picture made for another size shows what fits until the new
            // one comes.
            let spot = Rect { width: shot.columns.min(area.width), height: shot.rows.min(area.height), ..area };
            crate::graphics::put_picture_from(frame.buffer_mut(), spot, picture, 0, 0);
            if framed {
                mark_the_caret(frame.buffer_mut(), area, &shot, ink.page().fg(ink.gold()));
            }
        }
        None => {
            let said = match can {
                false => say!("preview.no-pictures"),
                true => say!("preview.rendering"),
            };
            let quiet = ink.page().fg(ink.quiet());
            let lines = (said.chars().count() as u16 / area.width.max(1)) + 1;
            let middle = Rect { y: area.y + area.height.saturating_sub(lines) / 2, height: lines.min(area.height), ..area };
            frame.render_widget(Paragraph::new(said).style(quiet).alignment(Alignment::Center).wrap(Wrap { trim: true }), middle);
        }
    }
}

/// **A bar beside the paragraph the cursor is in**, painted into the
/// picture so it stands exactly beside its lines: in the left margin across
/// a page, along the top margin down a vertical one. `span` is the
/// paragraph's start and end along the page, measured from the picture's
/// own start.
fn paint_the_bar(picture: &mut yumete_mdview::Picture, span: (f32, f32), vertical: bool, scale: f32, colour: [u8; 4]) {
    let (w, h) = (picture.width as i64, picture.height as i64);
    let thick = (3.0 * scale).round().max(2.0) as i64;
    let inset = (4.0 * scale).round() as i64;
    let (from, to) = (span.0.round() as i64, span.1.round() as i64);
    // The rectangle, in the picture's pixels.
    let (x0, x1, y0, y1) = match vertical {
        false => (inset, inset + thick, from, to),
        true => (w - to, w - from, inset, inset + thick),
    };
    for y in y0.max(0)..y1.min(h) {
        for x in x0.max(0)..x1.min(w) {
            let at = ((y * w + x) * 4) as usize;
            if let Some(px) = picture.rgba.get_mut(at..at + 4) {
                px.copy_from_slice(&colour);
            }
        }
    }
}

/// **The way to the cursor's line, marked on the frame** while it is out
/// of view: `▶` `◀` on the side walls across a page, `▼` `▲` on the top and
/// bottom walls down a vertical one, at the edge nearest to it.
fn mark_the_caret(buf: &mut ratatui::buffer::Buffer, area: Rect, shot: &Shot, style: ratatui::style::Style) {
    let Some(caret) = shot.caret else { return };
    if area.width == 0 || area.height == 0 || area.x == 0 || area.y == 0 {
        return;
    }
    let cell = crate::graphics::cell_pixels();
    let along = caret - shot.top;
    // In view, the bar in the picture shows the paragraph; the arrows only
    // point the way to one out of view.
    let shown = match shot.vertical {
        false => area.height as f32 * cell.1.max(1) as f32,
        true => area.width as f32 * cell.0.max(1) as f32,
    };
    if (0.0..shown).contains(&along) {
        return;
    }
    let mut put = |x: u16, y: u16, mark: &str| {
        if let Some(c) = buf.cell_mut((x, y)) {
            c.set_symbol(mark).set_style(style);
        }
    };
    match shot.vertical {
        false => {
            let row = (along / cell.1.max(1) as f32).floor().clamp(0.0, (area.height - 1) as f32) as u16;
            put(area.x - 1, area.y + row, "▶");
            put(area.x + area.width, area.y + row, "◀");
        }
        true => {
            let column = (along / cell.0.max(1) as f32).floor().clamp(0.0, (area.width - 1) as f32) as u16;
            let x = area.x + area.width - 1 - column;
            put(x, area.y - 1, "▼");
            put(x, area.y + area.height, "▲");
        }
    }
}

/// **Why a Markdown preview window cannot be drawn**, or `None` when it can:
/// only a terminal that draws pictures can.
pub fn why_not() -> Option<String> {
    (crate::graphics::support() == crate::graphics::Support::None).then(|| say!("preview.no-pictures"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **The bar stands beside the paragraph, and only there**: in the left
    /// margin across a page, along the top margin down a vertical one.
    #[test]
    fn the_bar_is_painted_beside_the_paragraph() {
        let blank = || yumete_mdview::Picture { width: 100, height: 50, rgba: vec![0; 100 * 50 * 4] };
        let gold = [1, 2, 3, 255];
        let at = |p: &yumete_mdview::Picture, x: usize, y: usize| p.rgba[(y * 100 + x) * 4..][..4].to_vec();
        let mut across = blank();
        paint_the_bar(&mut across, (10.0, 30.0), false, 1.0, gold);
        assert_eq!(at(&across, 5, 15), gold);
        assert_eq!(at(&across, 5, 5), [0, 0, 0, 0], "above the paragraph");
        assert_eq!(at(&across, 50, 15), [0, 0, 0, 0], "not over the text");
        let mut down = blank();
        paint_the_bar(&mut down, (10.0, 30.0), true, 1.0, gold);
        assert_eq!(at(&down, 80, 5), gold, "its columns, counted from the right");
        assert_eq!(at(&down, 95, 5), [0, 0, 0, 0]);
        assert_eq!(at(&down, 80, 30), [0, 0, 0, 0], "not over the text");
    }
}
