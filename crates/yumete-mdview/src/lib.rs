//! **A Markdown file drawn as pictures** (2026-11-08), the way VS Code's
//! Markdown preview shows one: proportional type in the system's fonts, real
//! heading sizes, tables with lines, code on its ground, formulas and images
//! as pictures.
//!
//! Three steps, each its own module:
//!
//! - [`parse`] reads YFM (yumete flavored Markdown) into blocks that know
//!   their source lines;
//! - `layout` lays each top-level block out with parley, once: a block whose
//!   text did not change (only moved) keeps its layout, so an edit costs the
//!   blocks it touched;
//! - `paint` draws a viewport (a top and a height) with vello_cpu, so the
//!   front end asks for what is on screen, not for one giant picture.
//!
//! [`Preview`] holds the three together, with the map between source lines
//! and heights the preview needs to follow the cursor
//! ([`Preview::y_for_line`], [`Preview::line_for_y`]).
//!
//! **Formulas** are drawn by whoever owns a TeX renderer ([`Formulas`]):
//! yumete's is `yumete_tui::tex`, RaTeX, and this crate does not depend on it
//! so its tests and its other users need not carry it.

mod cache;
pub mod fonts;
pub mod parse;
pub mod theme;

mod layout;
mod paint;
mod vertical;

pub use vertical::Vertical;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

pub use theme::Theme;

use layout::{Engine, Laid};
use parse::{rebased, Block};

/// A formula, drawn: a PNG, and its baseline in pixels from its top.
pub struct Formula {
    pub png: Vec<u8>,
    pub baseline: f32,
}

/// **Who draws formulas**: TeX in, a picture out, `size` pixels to the em,
/// in `ink` (red, green, blue) on a clear ground. An `Err` sets the TeX as
/// code, in the theme's error colour.
pub trait Formulas: Send {
    fn draw(&self, tex: &str, display: bool, ink: [u8; 3], size: f32) -> Result<Formula, String>;
}

/// No formulas: every one is set as its TeX.
pub struct NoFormulas;

impl Formulas for NoFormulas {
    fn draw(&self, _: &str, _: bool, _: [u8; 3], _: f32) -> Result<Formula, String> {
        Err("no formula renderer".to_string())
    }
}

/// How a preview is drawn.
#[derive(Debug, Clone)]
pub struct Options {
    /// The page's width in device pixels.
    pub width: u32,
    /// Device pixels to a CSS pixel: 2 on a Retina screen.
    pub scale: f32,
    pub theme: Theme,
    /// Where an image's relative path starts: the Markdown file's folder.
    pub base_dir: PathBuf,
}

/// What a layout pass did.
#[derive(Debug, Clone, Copy, Default)]
pub struct Stats {
    pub blocks: usize,
    /// Blocks whose layout was kept.
    pub reused: usize,
    /// Blocks laid out anew.
    pub laid: usize,
    pub parse_ms: f64,
    pub layout_ms: f64,
}

/// A drawn viewport: straight RGBA, row by row.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Picture {
    /// The picture as a PNG file.
    pub fn png(&self) -> Vec<u8> {
        use image::ImageEncoder;
        let mut out = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new_with_quality(
            &mut out,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Adaptive,
        );
        let _ = encoder.write_image(&self.rgba, self.width, self.height, image::ExtendedColorType::Rgba8);
        out
    }
}

/// A placed top-level block: its top, its source lines, its layout.
struct Placed {
    y: f32,
    lines: (usize, usize),
    laid: Arc<Laid>,
}

/// **A rendered Markdown document.** Give it the text with
/// [`set_text`](Preview::set_text) after every edit; ask it for viewports
/// with [`render`](Preview::render).
pub struct Preview {
    engine: Engine,
    painter: paint::Painter,
    width: u32,
    blocks: Vec<Block>,
    /// A writing system the reader chose, over what the document says.
    han_override: Option<fonts::Han>,
    parse_cache: parse::Cache,
    cache: HashMap<u64, Arc<Laid>>,
    placed: Vec<Placed>,
    /// Every visual line's top and the source line it shows, top to bottom;
    /// down a vertical page, every column's distance from the page's right
    /// edge.
    marks: Vec<(f32, usize)>,
    /// How long the page is along the way it scrolls: down, or to the left
    /// when it is vertical.
    height: f32,
    /// The window's height, which a vertical page's columns fill.
    view_height: u32,
}

impl Preview {
    pub fn new(options: Options, formulas: Box<dyn Formulas>) -> Preview {
        Preview {
            engine: Engine::new(options.theme, options.scale, options.base_dir, formulas),
            painter: paint::Painter::new(),
            width: options.width.max(64),
            blocks: Vec::new(),
            han_override: None,
            parse_cache: parse::Cache::default(),
            cache: HashMap::new(),
            placed: Vec::new(),
            marks: Vec::new(),
            height: 0.0,
            view_height: 0,
        }
    }

    /// **Read `text` and lay it out**, keeping the layout of every block
    /// whose text did not change.
    pub fn set_text(&mut self, text: &str) -> Stats {
        let t = Instant::now();
        self.blocks = parse::parse_with(text, &mut self.parse_cache);
        // The writing system: the front matter's `lang` if it gives one,
        // else what the text itself is written in.
        let declared = self.blocks.first().and_then(|b| match &b.kind {
            parse::BlockKind::FrontMatter(lines) => lines.iter().find_map(|l| {
                let (key, value) = l.split_once(':')?;
                matches!(key.trim(), "lang" | "language" | "locale").then(|| fonts::Han::from_tag(value)).flatten()
            }),
            _ => None,
        });
        let han = self.han_override.or(declared).unwrap_or_else(|| fonts::han_of(text));
        if han != self.engine.han {
            self.engine.han = han;
            self.cache.clear();
        }
        let parse_ms = t.elapsed().as_secs_f64() * 1000.0;
        let mut stats = self.relayout();
        stats.parse_ms = parse_ms;
        stats
    }

    /// A new page width: every block is laid out again.
    pub fn set_width(&mut self, width: u32) -> Stats {
        self.width = width.max(64);
        self.relayout()
    }

    /// New colours: every block is laid out again.
    pub fn set_theme(&mut self, theme: Theme) -> Stats {
        self.engine.theme = theme;
        self.cache.clear();
        self.relayout()
    }

    /// The page's width, and how long it is along the way it scrolls, in
    /// device pixels: its height, or how far it reaches to the left when it
    /// is vertical.
    pub fn size(&self) -> (u32, f32) {
        (self.width, self.height)
    }

    /// **Set the page vertically** (2026-11-08), or across with `None`:
    /// columns read right to left, the page going on to the left. Every
    /// block is laid out again.
    pub fn set_vertical(&mut self, vertical: Option<Vertical>) -> Stats {
        if self.engine.vertical == vertical {
            return Stats::default();
        }
        self.engine.vertical = vertical;
        self.cache.clear();
        self.relayout()
    }

    /// Whether the page is set vertically.
    pub fn vertical(&self) -> bool {
        self.engine.vertical.is_some()
    }

    /// **The window's height** (2026-11-08), which a vertical page's columns
    /// fill: a new one lays a vertical page out again.
    pub fn set_height(&mut self, height: u32) -> Stats {
        if self.view_height == height {
            return Stats::default();
        }
        self.view_height = height;
        match self.vertical() {
            true => self.relayout(),
            false => Stats::default(),
        }
    }

    /// **The ruby spellings read** (2026-11-08): `[export] ruby_html`,
    /// `ruby_typst` and `ruby_markdown`; takes effect at the next
    /// [`set_text`](Preview::set_text).
    pub fn set_ruby(&mut self, ruby: yumete_core::ruby::Dialects) {
        self.parse_cache.set_ruby(ruby);
    }

    /// **The fonts the writing is set in** (2026-11-08): `[export]
    /// font_family`, the list the HTML and EPUB exports ask for, so the
    /// preview and the page agree. A new list lays every block out again.
    pub fn set_fonts(&mut self, list: &str) -> Stats {
        if self.engine.set_font_list(list) {
            self.cache.clear();
        }
        self.relayout()
    }

    /// Set every document in `han` (or, with `None`, in its own again);
    /// takes effect at the next [`set_text`](Preview::set_text).
    pub fn set_han(&mut self, han: Option<fonts::Han>) {
        self.han_override = han;
    }

    /// Whether adjacent full-width marks give up their blank halves (on
    /// unless turned off): every block is laid out again.
    pub fn set_squeeze(&mut self, on: bool) -> Stats {
        self.engine.squeeze = on;
        self.cache.clear();
        self.relayout()
    }

    /// How many formulas are kept drawn, and how many bytes of pictures:
    /// both kept to a budget, the least recently used going first.
    pub fn kept(&self) -> (usize, usize) {
        self.engine.kept()
    }

    /// The writing system the document is set in.
    pub fn han(&self) -> fonts::Han {
        self.engine.han
    }

    /// The blocks read last.
    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// The side margin, the top margin and the bottom margin.
    fn padding(&self) -> (f32, f32, f32) {
        let s = self.engine.scale;
        let side = if (self.width as f32) < 500.0 * s { 14.0 * s } else { 26.0 * s };
        (side, 16.0 * s, 32.0 * s)
    }

    fn relayout(&mut self) -> Stats {
        if self.vertical() {
            return self.relayout_vertical();
        }
        let t = Instant::now();
        let (side, top, bottom) = self.padding();
        let column = self.width as f32 - 2.0 * side;
        let mut fresh: HashMap<u64, Arc<Laid>> = HashMap::with_capacity(self.blocks.len());
        let mut stats = Stats { blocks: self.blocks.len(), ..Stats::default() };
        self.placed.clear();
        self.marks.clear();
        let mut y = top;
        let mut below = 0.0f32;
        for (k, block) in self.blocks.iter().enumerate() {
            let moved = rebased(block, block.lines.0);
            let mut h = std::collections::hash_map::DefaultHasher::new();
            moved.hash(&mut h);
            (column.to_bits(), self.engine.scale.to_bits()).hash(&mut h);
            // A picture changed on disk lays its block out again.
            for src in parse::images(&moved) {
                self.engine.image_stamp(src).hash(&mut h);
            }
            let key = h.finish();
            let laid = match self.cache.get(&key).or_else(|| fresh.get(&key)) {
                Some(l) => {
                    stats.reused += 1;
                    l.clone()
                }
                None => {
                    stats.laid += 1;
                    Arc::new(self.engine.top(&moved, side, column))
                }
            };
            if k > 0 {
                y += below.max(laid.top);
            }
            for &(my, line) in &laid.marks {
                self.marks.push((y + my, block.lines.0 + line));
            }
            self.placed.push(Placed { y, lines: block.lines, laid: laid.clone() });
            y += laid.height;
            below = laid.bottom;
            fresh.insert(key, laid);
        }
        self.marks.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.cache = fresh;
        self.height = y + bottom;
        stats.layout_ms = t.elapsed().as_secs_f64() * 1000.0;
        stats
    }

    /// **A vertical page laid out**: each top-level block from the right
    /// edge leftwards, its columns filling the window's height; a table's
    /// caption goes with the table.
    fn relayout_vertical(&mut self) -> Stats {
        let t = Instant::now();
        let s = self.engine.scale;
        let (side, top, bottom) = (26.0 * s, 20.0 * s, 24.0 * s);
        let height = if self.view_height == 0 { self.width * 7 / 5 } else { self.view_height } as f32;
        let page = vertical::Page { top, len: (height - top - bottom).max(4.0 * self.engine.base()), width: self.width as f32 };
        let key_of_page = (page.len.to_bits(), page.width.to_bits(), s.to_bits(), self.engine.vertical.map_or(0, |v| v.key()));
        let mut fresh: HashMap<u64, Arc<Laid>> = HashMap::with_capacity(self.blocks.len());
        let mut stats = Stats { blocks: self.blocks.len(), ..Stats::default() };
        self.placed.clear();
        self.marks.clear();
        let mut u = side;
        let mut after = 0.0f32;
        let mut k = 0;
        let mut first = true;
        while k < self.blocks.len() {
            // A table and the caption paragraph just before or after it.
            let block = &self.blocks[k];
            let is_table = |b: Option<&Block>| matches!(b.map(|b| &b.kind), Some(parse::BlockKind::Table { .. }));
            let (main, caption, span) = match vertical::caption_of(block) {
                Some(c) if is_table(self.blocks.get(k + 1)) => (k + 1, Some(c), (k, k + 1)),
                _ if is_table(Some(block)) => match self.blocks.get(k + 1).and_then(vertical::caption_of) {
                    Some(c) if !is_table(self.blocks.get(k + 2)) => (k, Some(c), (k, k + 1)),
                    _ => (k, None, (k, k)),
                },
                _ => (k, None, (k, k)),
            };
            let block = &self.blocks[main];
            let moved = rebased(block, block.lines.0);
            // The caption is counted as the table's first line, so a table
            // that only moved keeps its layout.
            let caption: Option<Vec<parse::Inline>> = caption.map(|c| c.into_iter().map(|r| parse::Inline { line: 0, ..r }).collect());
            let mut h = std::collections::hash_map::DefaultHasher::new();
            moved.hash(&mut h);
            caption.hash(&mut h);
            key_of_page.hash(&mut h);
            for src in parse::images(&moved) {
                self.engine.image_stamp(src).hash(&mut h);
            }
            let key = h.finish();
            let lines = (self.blocks[span.0].lines.0, self.blocks[span.1].lines.1);
            let laid = match self.cache.get(&key).or_else(|| fresh.get(&key)) {
                Some(l) => {
                    stats.reused += 1;
                    l.clone()
                }
                None => {
                    stats.laid += 1;
                    Arc::new(self.engine.vtop(&moved, caption.as_deref(), page))
                }
            };
            if !first {
                u += after.max(laid.top);
            }
            first = false;
            let shift = block.lines.0;
            for &(mu, line) in &laid.marks {
                self.marks.push((u + mu, shift + line));
            }
            self.placed.push(Placed { y: u, lines, laid: laid.clone() });
            u += laid.height;
            after = laid.bottom;
            fresh.insert(key, laid);
            k = span.1 + 1;
        }
        self.marks.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.cache = fresh;
        self.height = u + side;
        stats.layout_ms = t.elapsed().as_secs_f64() * 1000.0;
        stats
    }

    /// **The height at which source line `line` is shown**: the top of the
    /// visual line that holds it, or of the nearest line before it. Down a
    /// vertical page, how far from the page's right edge the column that
    /// holds it begins.
    pub fn y_for_line(&self, line: usize) -> f32 {
        let mut best: Option<(usize, f32)> = None;
        for &(y, l) in &self.marks {
            if l <= line && best.is_none_or(|(bl, by)| l > bl || (l == bl && y < by)) {
                best = Some((l, y));
            }
        }
        best.map_or(0.0, |b| b.1)
    }

    /// **The source line shown at height `y`**: the line the visual line
    /// over `y` starts on.
    pub fn line_for_y(&self, y: f32) -> usize {
        let k = self.marks.partition_point(|m| m.0 <= y);
        match k {
            0 => self.marks.first().map_or(0, |m| m.1),
            k => self.marks[k - 1].1,
        }
    }

    /// How far right the page's content reaches: more than the width when a
    /// table could not be made to fit.
    pub fn content_width(&self) -> f32 {
        self.placed.iter().map(|p| p.laid.right).fold(0.0, f32::max)
    }

    /// **The words drawn between `top` and `top + height`**, block by block:
    /// for a reader of the screen, a search in the preview, and the tests.
    /// Down a vertical page, `top` and `height` are along the page, from
    /// its right edge, and each column is a line.
    pub fn visible_text(&self, top: f32, height: f32) -> String {
        let mut out = String::new();
        if self.vertical() {
            for p in self.placed.iter().filter(|p| p.y + p.laid.height >= top && p.y <= top + height) {
                for (at, words) in &p.laid.cols {
                    if p.y + at >= top - 1.0 && p.y + at <= top + height {
                        out.push_str(words);
                        out.push('\n');
                    }
                }
            }
            return out;
        }
        for p in self.placed.iter().filter(|p| p.y + p.laid.height >= top && p.y <= top + height) {
            for item in &p.laid.items {
                if let layout::Item::Text { y, text, .. } = item {
                    let at = p.y + y;
                    if at + text.layout.height() >= top && at <= top + height {
                        out.push_str(&text.string);
                        out.push('\n');
                    }
                }
            }
        }
        out
    }

    /// The source lines of the top-level block shown at height `y`.
    pub fn block_at(&self, y: f32) -> Option<(usize, usize)> {
        let k = self.placed.partition_point(|p| p.y <= y);
        self.placed.get(k.checked_sub(1)?).map(|p| p.lines)
    }

    /// **Draw the viewport** from `top` down, `height` device pixels tall,
    /// the page's width wide. Down a vertical page, `top` is how far left
    /// of the page's right edge the window's right edge stands.
    pub fn render(&mut self, top: f32, height: u32) -> Picture {
        let h = height.clamp(1, u16::MAX as u32) as u16;
        let w = self.width.min(u16::MAX as u32) as u16;
        let along = if self.vertical() { w as f32 } else { h as f32 };
        let first = self.placed.partition_point(|p| p.y + p.laid.height < top - 64.0);
        let end = top + along + 64.0;
        let vertical = self.vertical();
        let shown: Vec<(vello_cpu::kurbo::Vec2, &Laid)> = self.placed[first..]
            .iter()
            .take_while(|p| p.y <= end)
            .map(|p| {
                let off = match vertical {
                    true => vello_cpu::kurbo::Vec2::new((w as f32 - p.y + top) as f64, 0.0),
                    false => vello_cpu::kurbo::Vec2::new(0.0, (p.y - top) as f64),
                };
                (off, p.laid.as_ref())
            })
            .collect();
        let rgba = self.painter.paint(&shown, self.engine.theme.background, w, h);
        Picture { width: w as u32, height: h as u32, rgba }
    }
}

#[cfg(test)]
mod tests;
