//! **The preview as a PDF**, `:export-as-preview pdf`: the very layout the
//! preview draws on the screen, written with krilla instead of painted with
//! vello_cpu. The same items ([`Item`]) are walked as [`crate::paint`]
//! walks them, so the page is the preview's, colour for colour and glyph for
//! glyph.
//!
//! - **Text is text**: every glyph goes out as a glyph of its font, the font
//!   embedded (subset), each with the characters it shows, so a reader can
//!   select and search it. Pictures (images, formulas) stay pictures.
//! - **Long pages**: the document is one tall page, as it scrolls in the
//!   preview, cut into several only where a page would be taller than
//!   [`MAX_PAGE_PT`] — the 200 inches many readers refuse to open past.
//!   Cuts fall between blocks, or between lines where one block alone is
//!   too long, and only at the limit itself when not even a line fits.
//! - **Down a vertical page** the same, turned: a page as tall as the
//!   preview's columns, as wide as the document, cut between blocks.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::Arc;

use krilla::color::rgb;
use krilla::geom::{Path, PathBuilder, Point, Size, Transform};
use krilla::num::NormalizedF32;
use krilla::paint::{Fill, FillRule, LineCap, LineJoin, Stroke};
use krilla::surface::Surface;
use krilla::text::{Font, GlyphId, KrillaGlyph};
use parley::{Layout, PositionedLayoutItem};
use vello_cpu::kurbo::{Affine, BezPath, PathEl, Rect, RoundedRect, Shape, Vec2};

use crate::layout::{Attach, Ink, Item, Laid, Pix, Run};
use crate::theme::Rgba;
use crate::Preview;

/// **The tallest page written**, in points: 200 inches, past which many
/// PDF readers will not open a page.
pub const MAX_PAGE_PT: f32 = 200.0 * 72.0;

/// Points to a CSS pixel: a CSS pixel is a 96th of an inch, a point a 72nd.
const PT_PER_PX: f32 = 72.0 / 96.0;

/// **Where one page begins and ends along the document**, in device
/// pixels, and the blank it adds before and after where it is cut.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Slice {
    pub from: f32,
    pub to: f32,
    pub before: f32,
    pub after: f32,
}

impl Slice {
    fn len(&self) -> f32 {
        self.before + (self.to - self.from) + self.after
    }
}

/// **Cut a document `total` long into pages** no longer than `limit`:
/// at the best of `blocks` (cuts between blocks) that fits, else of
/// `lines` (cuts between lines), else at the limit. A page cut off below
/// has `after` added, one that goes on from a cut `before`.
pub(crate) fn slices(total: f32, blocks: &[f32], lines: &[f32], limit: f32, before: f32, after: f32) -> Vec<Slice> {
    let mut out = Vec::new();
    let mut from = 0.0f32;
    let mut first = true;
    loop {
        let lead = if first { 0.0 } else { before };
        if lead + (total - from) <= limit {
            out.push(Slice { from, to: total, before: lead, after: 0.0 });
            return out;
        }
        let room = from + limit - lead - after;
        let best = |cuts: &[f32]| cuts.iter().copied().filter(|&c| c > from + 1.0 && c <= room).fold(None, |a: Option<f32>, c| Some(a.map_or(c, |a| a.max(c))));
        let to = best(blocks).or_else(|| best(lines)).unwrap_or(room.max(from + 1.0));
        out.push(Slice { from, to, before: lead, after });
        from = to;
        first = false;
    }
}

/// A face: its file's id, its index in the file, and its variation axes
/// (tag, value's bits).
type FaceKey = (u64, u32, Vec<(u32, u32)>);

/// The fonts and pictures of one document, each embedded once.
#[derive(Default)]
struct Store {
    fonts: HashMap<FaceKey, Option<Font>>,
    pictures: HashMap<usize, krilla::image::Image>,
}

impl Store {
    /// The PDF's font for parley's face at `settings` (its variation axes).
    fn font(&mut self, data: &parley::FontData, settings: &[(u32, f32)]) -> Option<Font> {
        let key = (data.data.id(), data.index, settings.iter().map(|&(t, v)| (t, v.to_bits())).collect());
        self.fonts
            .entry(key)
            .or_insert_with(|| {
                let bytes: Arc<dyn AsRef<[u8]> + Send + Sync> = Arc::new(data.data.clone());
                let axes: Vec<(krilla::text::Tag, f32)> = settings.iter().map(|&(t, v)| (krilla::text::Tag::new(&t.to_be_bytes()), v)).collect();
                Font::new_variable(bytes.into(), data.index, &axes)
            })
            .clone()
    }

    /// A picture as the PDF's, its pixels made straight from premultiplied.
    fn picture(&mut self, pix: &Pix) -> krilla::image::Image {
        self.pictures
            .entry(Arc::as_ptr(pix) as usize)
            .or_insert_with(|| {
                let mut rgba = pix.data_as_u8_slice().to_vec();
                for px in rgba.as_chunks_mut::<4>().0 {
                    let a = px[3] as u32;
                    if a > 0 && a < 255 {
                        for c in &mut px[..3] {
                            *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                        }
                    }
                }
                krilla::image::Image::from_rgba8(rgba, pix.width() as u32, pix.height() as u32)
            })
            .clone()
    }
}

impl Preview {
    /// **The document as a PDF** of long pages, titled `title`: what
    /// [`render`](Preview::render) would draw, scrolled from top to end, in
    /// vector type with its fonts embedded.
    pub fn pdf(&self, title: &str) -> Result<Vec<u8>, String> {
        let s = self.engine.scale;
        // Device pixels to points.
        let k = PT_PER_PX / s;
        let limit = MAX_PAGE_PT / k;
        let vertical = self.vertical();
        let blocks: Vec<f32> = self.placed.windows(2).map(|w| (w[0].y + w[0].laid.height + w[1].y) / 2.0).collect();
        let lines: Vec<f32> = self.marks.iter().map(|m| m.0).collect();
        let (before, after) = match vertical {
            true => (26.0 * s, 26.0 * s),
            false => self.padding_ends(),
        };
        let pages = slices(self.height.max(1.0), &blocks, &lines, limit, before, after);
        // Across the page: the preview's width, or more where a table could
        // not be made to fit; down a vertical page, its columns' height.
        let across = match vertical {
            true => (if self.view_height == 0 { self.width * 7 / 5 } else { self.view_height }) as f32,
            false => (self.width as f32).max(self.content_width() + self.padding().0),
        };
        let mut doc = krilla::Document::new();
        doc.set_metadata(krilla::metadata::Metadata::new().title(title.to_string()));
        let mut store = Store::default();
        for slice in &pages {
            let along = slice.len();
            let (w, h) = if vertical { (along, across) } else { (across, along) };
            let settings = krilla::page::PageSettings::from_wh(w * k, h * k).ok_or("a page with no size")?;
            let mut page = doc.start_page_with(settings);
            let mut surface = page.surface();
            surface.push_transform(&Transform::from_scale(k, k));
            fill(&mut surface, &rect_path(Rect::new(0.0, 0.0, w as f64, h as f64), 0.0), self.engine.theme.background);
            // Only what lies within the slice, the rest of a cut block
            // clipped away.
            let shown = match vertical {
                true => Rect::new((slice.after) as f64, 0.0, (along - slice.before) as f64, h as f64),
                false => Rect::new(0.0, slice.before as f64, w as f64, (along - slice.after) as f64),
            };
            let clip = rect_path(shown, 0.0);
            if let Some(clip) = &clip {
                surface.push_clip_path(clip, &FillRule::NonZero);
            }
            for p in self.placed.iter().filter(|p| p.y < slice.to && p.y + p.laid.height > slice.from) {
                let at = slice.before + p.y - slice.from;
                let off = match vertical {
                    true => Vec2::new((along - at) as f64, 0.0),
                    false => Vec2::new(0.0, at as f64),
                };
                draw_laid(&mut surface, &mut store, &p.laid, off, shown);
            }
            if clip.is_some() {
                surface.pop();
            }
            surface.pop();
            surface.finish();
            page.finish();
        }
        doc.finish().map_err(|e| format!("{e:?}"))
    }

    /// The top margin and the bottom margin of a page across.
    fn padding_ends(&self) -> (f32, f32) {
        let (_, top, bottom) = self.padding();
        (top, bottom)
    }
}

fn colour(c: Rgba) -> (rgb::Color, NormalizedF32) {
    (rgb::Color::new(c[0], c[1], c[2]), NormalizedF32::new(c[3] as f32 / 255.0).unwrap_or(NormalizedF32::ONE))
}

fn paint_of(c: Rgba) -> Fill {
    let (color, opacity) = colour(c);
    Fill { paint: color.into(), opacity, rule: FillRule::NonZero }
}

fn transform(a: Affine) -> Transform {
    let [sx, ky, kx, sy, tx, ty] = a.as_coeffs();
    Transform::from_row(sx as f32, ky as f32, kx as f32, sy as f32, tx as f32, ty as f32)
}

/// A kurbo path as the PDF's.
fn path_of(path: &BezPath) -> Option<Path> {
    let mut b = PathBuilder::new();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => b.move_to(p.x as f32, p.y as f32),
            PathEl::LineTo(p) => b.line_to(p.x as f32, p.y as f32),
            PathEl::QuadTo(a, p) => b.quad_to(a.x as f32, a.y as f32, p.x as f32, p.y as f32),
            PathEl::CurveTo(a, c, p) => b.cubic_to(a.x as f32, a.y as f32, c.x as f32, c.y as f32, p.x as f32, p.y as f32),
            PathEl::ClosePath => b.close(),
        }
    }
    b.finish()
}

fn rect_path(r: Rect, radius: f64) -> Option<Path> {
    match radius > 0.0 {
        true => path_of(&RoundedRect::from_rect(r, radius).to_path(0.1)),
        false => path_of(&r.to_path(0.1)),
    }
}

fn fill(surface: &mut Surface, path: &Option<Path>, c: Rgba) {
    let Some(path) = path else { return };
    surface.set_stroke(None);
    surface.set_fill(Some(paint_of(c)));
    surface.draw_path(path);
}

fn stroke(surface: &mut Surface, path: &Option<Path>, c: Rgba, width: f32, round: bool) {
    let Some(path) = path else { return };
    let (color, opacity) = colour(c);
    let (line_cap, line_join) = if round { (LineCap::Round, LineJoin::Round) } else { (LineCap::Butt, LineJoin::Miter) };
    surface.set_fill(None);
    surface.set_stroke(Some(Stroke { paint: color.into(), width, opacity, line_cap, line_join, ..Stroke::default() }));
    surface.draw_path(path);
    surface.set_stroke(None);
}

/// A picture scaled into a box, its corner on whole pixels, as the
/// preview places it.
fn draw_pic(surface: &mut Surface, store: &mut Store, pix: &Pix, place: Affine, w: f64, h: f64) {
    let Some(size) = Size::from_wh(w as f32, h as f32) else { return };
    surface.push_transform(&transform(place));
    surface.draw_image(store.picture(pix), size);
    surface.pop();
}

/// Whether a line of text belongs on the page showing `band`: its middle
/// is in it. A block cut between its lines writes each line on one page
/// only, so the text of the cut is never found twice.
fn ours(r: Rect, band: Rect) -> bool {
    let c = r.center();
    c.x >= band.x0 && c.x < band.x1 && c.y >= band.y0 && c.y < band.y1
}

/// Anywhere: a ruby's base and reading go with the line they stand in.
const EVERYWHERE: Rect = Rect::new(f64::MIN / 4.0, f64::MIN / 4.0, f64::MAX / 4.0, f64::MAX / 4.0);

/// **Draw a block** moved by `off`, its lines of text only where they
/// belong to `band` ([`ours`]); the rest is clipped to the page.
fn draw_laid(surface: &mut Surface, store: &mut Store, laid: &Laid, off: Vec2, band: Rect) {
    for item in &laid.items {
        match item {
            Item::Rect { r, fill: c, radius } => fill(surface, &rect_path(*r + off, *radius as f64), *c),
            Item::Frame { r, color, width, radius } => {
                let r = (*r + off).inset(-(*width as f64) / 2.0);
                stroke(surface, &rect_path(r, *radius as f64), *color, *width, false);
            }
            Item::Pic { r, pix } => {
                let r = *r + off;
                draw_pic(surface, store, pix, Affine::translate((r.x0.round(), r.y0.round())), r.width(), r.height());
            }
            Item::Turned { r, pix } => {
                // A quarter turn clockwise: the picture's top runs down the
                // box's right edge.
                let r = *r + off;
                let place = Affine::translate((r.x0.round() + r.width(), r.y0.round())) * Affine::rotate(std::f64::consts::FRAC_PI_2);
                draw_pic(surface, store, pix, place, r.height(), r.width());
            }
            Item::Lines { points, color, width } => {
                let mut path = BezPath::new();
                for (n, &(x, y)) in points.iter().enumerate() {
                    let p = (x as f64 + off.x, y as f64 + off.y);
                    if n == 0 {
                        path.move_to(p);
                    } else {
                        path.line_to(p);
                    }
                }
                stroke(surface, &path_of(&path), *color, *width, true);
            }
            Item::Glyphs(run) if ours(run.bounds + off, band) => draw_run(surface, store, run, off),
            Item::Glyphs(_) => {}
            Item::Text { x, y, text } => draw_layout(surface, store, &text.layout, &text.string, &text.attach, off.x + *x as f64, off.y + *y as f64, band),
        }
    }
}

/// **Glyphs placed by hand**, each at its own place in the run's frame.
fn draw_run(surface: &mut Surface, store: &mut Store, run: &Run, off: Vec2) {
    let Some(font) = store.font(&run.font, &[]) else { return };
    let placed: Vec<(u32, f32, f32, Range<usize>)> = run
        .glyphs
        .iter()
        .enumerate()
        .map(|(n, g)| (g.id, g.x, g.y, run.spans.get(n).cloned().unwrap_or(0..0)))
        .collect();
    surface.push_transform(&transform(Affine::translate(off) * run.transform));
    glyphs(surface, font, run.size, run.color, &placed, &run.text, 0.0);
    surface.pop();
}

/// **Glyphs at their places**: `(glyph, x, y, characters)`, `y` from the
/// baseline `base` of the frame drawn in. Each glyph's advance is the
/// distance to the next, so the PDF places them exactly where the layout
/// did.
fn glyphs(surface: &mut Surface, font: Font, size: f32, color: Rgba, placed: &[(u32, f32, f32, Range<usize>)], text: &str, base: f32) {
    let Some(first) = placed.first() else { return };
    let size = size.max(0.01);
    let out: Vec<KrillaGlyph> = placed
        .iter()
        .enumerate()
        .map(|(n, (id, x, y, range))| {
            let next = placed.get(n + 1).map_or(*x, |g| g.1);
            let range = if text.is_char_boundary(range.start) && text.is_char_boundary(range.end) && range.start <= range.end { range.clone() } else { 0..0 };
            KrillaGlyph::new(GlyphId::new(*id), (next - x) / size, 0.0, -(y - base) / size, 0.0, range, None)
        })
        .collect();
    surface.set_stroke(None);
    surface.set_fill(Some(paint_of(color)));
    surface.draw_glyphs(Point::from_xy(first.1, base), &out, font, text, size, false);
}

/// **Each glyph's characters** in a glyph run: the run's clusters walked
/// in the order its glyphs are, from where the line's earlier glyph runs of
/// the same shaped run left off (`taken`, by the run's text).
fn clusters_of(g: &parley::GlyphRun<'_, Ink>, count: usize, taken: &mut HashMap<(usize, usize), usize>) -> Vec<Range<usize>> {
    let run = g.run();
    let key = (run.text_range().start, run.text_range().end);
    let skip = taken.get(&key).copied().unwrap_or(0);
    let mut ranges: Vec<Range<usize>> = Vec::with_capacity(count);
    let mut used = 0;
    for cluster in run.visual_clusters().skip(skip) {
        let n = cluster.glyphs().count();
        if ranges.len() >= count && !(n == 0 && cluster.is_ligature_continuation()) {
            break;
        }
        used += 1;
        if n == 0 {
            // A ligature's later characters go with the glyph before.
            if let Some(last) = ranges.last_mut() {
                last.end = last.end.max(cluster.text_range().end);
            }
            continue;
        }
        ranges.extend(std::iter::repeat_n(cluster.text_range(), n));
    }
    taken.insert(key, skip + used);
    ranges.resize(count, 0..0);
    ranges
}

#[allow(clippy::too_many_arguments)]
fn draw_layout(surface: &mut Surface, store: &mut Store, layout: &Layout<Ink>, text: &str, attach: &[Attach], ox: f64, oy: f64, band: Rect) {
    for line in layout.lines() {
        let m = line.metrics();
        let reach = Rect::new(ox, oy + m.block_min_coord as f64, ox + layout.width().max(1.0) as f64, oy + m.block_max_coord as f64);
        if !ours(reach, band) {
            continue;
        }
        // Grounds first, so a run's ground never covers its neighbour's ink.
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(g) = item else { continue };
            let ink = g.style().brush;
            let Some(ground) = ink.ground else { continue };
            let size = g.run().font_size() as f64;
            let (x0, x1) = (ox + g.offset() as f64, ox + (g.offset() + g.advance()) as f64);
            let base = oy + g.baseline() as f64;
            let pad = match (ink.frame, ink.mono) {
                (true, _) => 0.3 * size,
                (false, true) => 0.2 * size,
                (false, false) => 0.05 * size,
            };
            let (up, down) = if ink.mono { (1.0, 0.32) } else { (1.0, 0.3) };
            let r = Rect::new(x0 - pad, base - up * size, x1 + pad, base + down * size);
            let radius = if ink.mono || ink.frame { 0.25 * size } else { 0.1 * size };
            fill(surface, &rect_path(r, radius), ground);
            if ink.frame {
                stroke(surface, &rect_path(r, radius), [ink.color[0], ink.color[1], ink.color[2], 0x50], (0.07 * size) as f32, false);
            }
        }
        let mut taken: HashMap<(usize, usize), usize> = HashMap::new();
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(g) => {
                    let run = g.run();
                    let ink = g.style().brush;
                    let rm = run.font_metrics();
                    let size = run.font_size();
                    let (x0, x1) = (ox + g.offset() as f64, ox + (g.offset() + g.advance()) as f64);
                    let base = g.baseline();
                    let synthesis = run.synthesis();
                    let axes: Vec<(u32, f32)> = synthesis.variation_settings().iter().map(|(t, v)| (u32::from_be_bytes(t.to_be_bytes()), *v)).collect();
                    let positioned: Vec<parley::Glyph> = g.positioned_glyphs().collect();
                    let ranges = clusters_of(&g, positioned.len(), &mut taken);
                    if let Some(font) = store.font(run.font(), &axes) {
                        let placed: Vec<(u32, f32, f32, Range<usize>)> = positioned.iter().zip(ranges).map(|(gl, r)| (gl.id, gl.x, gl.y, r)).collect();
                        // A slanted face made up where the family has none:
                        // the glyphs leaned about the baseline.
                        let lean = synthesis.skew().map_or(0.0, |a| a.to_radians().tan());
                        let frame = Affine::translate((ox, oy)) * Affine::new([1.0, 0.0, -lean as f64, 1.0, lean as f64 * base as f64, 0.0]);
                        surface.push_transform(&transform(frame));
                        glyphs(surface, font, size, ink.color, &placed, text, base);
                        surface.pop();
                    }
                    let size = size as f64;
                    let base = oy + base as f64;
                    if ink.underline {
                        let t = (rm.underline_size as f64).max(0.06 * size);
                        let y = base - rm.underline_offset as f64;
                        fill(surface, &rect_path(Rect::new(x0, y, x1, y + t), 0.0), ink.color);
                    }
                    if ink.strike {
                        let t = (rm.strikethrough_size as f64).max(0.06 * size);
                        let y = base - rm.strikethrough_offset as f64;
                        fill(surface, &rect_path(Rect::new(x0, y - t / 2.0, x1, y + t / 2.0), 0.0), ink.color);
                    }
                }
                PositionedLayoutItem::InlineBox(bx) => match attach.get(bx.id as usize) {
                    Some(Attach::Pic(p)) => {
                        let place = Affine::translate(((ox + bx.x as f64).round(), (oy + bx.y as f64).round()));
                        draw_pic(surface, store, p, place, bx.width as f64, bx.height as f64);
                    }
                    Some(Attach::Ruby { base, reading }) => {
                        let bxx = ox + bx.x as f64;
                        let bxy = oy + bx.y as f64;
                        let w = bx.width as f64;
                        draw_layout(surface, store, &base.layout, &base.string, &[], bxx + (w - base.layout.width() as f64) / 2.0, bxy, EVERYWHERE);
                        // The reading sits just over the base's ink.
                        let size = base.layout.lines().next().and_then(|l| l.runs().next().map(|r| r.font_size())).unwrap_or(16.0) as f64;
                        let base_line = base.layout.first_baseline().unwrap_or(0.0) as f64;
                        let reading_base = reading.layout.first_baseline().unwrap_or(0.0) as f64;
                        let target = bxy + base_line - 0.92 * size - 0.08 * size;
                        draw_layout(surface, store, &reading.layout, &reading.string, &[], bxx + (w - reading.layout.width() as f64) / 2.0, target - reading_base, EVERYWHERE);
                    }
                    None => {}
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_document_is_one_page() {
        let pages = slices(500.0, &[100.0, 300.0], &[], 1000.0, 16.0, 32.0);
        assert_eq!(pages, vec![Slice { from: 0.0, to: 500.0, before: 0.0, after: 0.0 }]);
    }

    #[test]
    fn pages_are_cut_between_blocks_under_the_limit() {
        let blocks: Vec<f32> = (1..30).map(|n| n as f32 * 100.0).collect();
        let pages = slices(3000.0, &blocks, &[], 1000.0, 16.0, 32.0);
        assert!(pages.len() >= 3);
        for p in &pages {
            assert!(p.len() <= 1000.0, "{p:?}");
            assert!(p.to == 3000.0 || blocks.contains(&p.to), "cut at a block: {p:?}");
        }
        assert_eq!(pages.last().map(|p| p.to), Some(3000.0));
        for w in pages.windows(2) {
            assert_eq!(w[0].to, w[1].from);
        }
    }

    #[test]
    fn a_block_too_long_for_a_page_is_cut_between_its_lines() {
        let lines: Vec<f32> = (0..300).map(|n| n as f32 * 20.0).collect();
        let pages = slices(6000.0, &[], &lines, 1000.0, 16.0, 32.0);
        for p in &pages {
            assert!(p.len() <= 1000.0);
            assert!(p.to == 6000.0 || lines.contains(&p.to));
        }
    }

    /// A sample with a heading, a list, code, a formula and a CJK
    /// paragraph, its body said `n` times.
    fn sample(n: usize) -> String {
        let mut out = String::from("# Spring Notes\n\nA plain opening line.\n\n- first item\n- second item\n\n```rust\nfn main() { println!(\"hi\"); }\n```\n\n");
        for k in 0..n {
            out.push_str(&format!("## Part {k}\n\n春天的早晨，河边的柳树发了新芽。第{k}段。\n\nSome *slanted* and **heavy** words, with `code` and $x^2$.\n\n"));
        }
        out
    }

    fn pdf_of(text: &str) -> lopdf::Document {
        let mut p = Preview::new(crate::Options { width: 700, scale: 2.0, theme: crate::Theme::light(), base_dir: ".".into() }, Box::new(crate::NoFormulas));
        p.set_text(text);
        let bytes = p.pdf("sample").expect("a PDF");
        assert!(bytes.starts_with(b"%PDF-"));
        lopdf::Document::load_mem(&bytes).expect("a PDF that reads back")
    }

    /// Each page's height in points.
    fn heights(doc: &lopdf::Document) -> Vec<f32> {
        doc.get_pages()
            .values()
            .map(|&id| {
                let page = doc.get_object(id).and_then(|o| o.as_dict()).expect("a page");
                let mbox = page.get(b"MediaBox").and_then(|o| o.as_array()).expect("a media box");
                let n = |o: &lopdf::Object| o.as_float().or_else(|_| o.as_i64().map(|i| i as f32)).unwrap_or(0.0);
                n(&mbox[3]) - n(&mbox[1])
            })
            .collect()
    }

    #[test]
    fn a_document_becomes_one_long_page_of_text() {
        let doc = pdf_of(&sample(3));
        assert_eq!(doc.get_pages().len(), 1);
        let text = doc.extract_text(&[1]).expect("text");
        let squeezed: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        for words in ["SpringNotes", "firstitem", "secondItem", "fnmain()", "春天的早晨，河边的柳树发了新芽。", "slanted", "heavy"] {
            assert!(squeezed.to_lowercase().contains(&words.to_lowercase()), "{words:?} in {text:?}");
        }
        assert!(heights(&doc)[0] > 72.0);
    }

    #[test]
    fn a_long_document_is_cut_into_pages_under_the_limit() {
        let doc = pdf_of(&sample(900));
        let pages = heights(&doc);
        assert!(pages.len() >= 2, "{pages:?}");
        assert!(pages.iter().all(|&h| h <= MAX_PAGE_PT), "{pages:?}");
        // Cut only between blocks: every page ends on a whole paragraph.
        let last = doc.extract_text(&[pages.len() as u32]).expect("text");
        assert!(last.contains("Part 899"), "the end on the last page");
        let mut seen = 0;
        for n in 1..=pages.len() as u32 {
            let words = doc.extract_text(&[n]).expect("text");
            let paragraphs = words.matches("春天的早晨").count();
            assert_eq!(paragraphs, words.matches("新芽。").count(), "page {n} holds whole paragraphs");
            seen += paragraphs;
        }
        assert_eq!(seen, 900, "each paragraph on one page, once");
    }

    #[test]
    fn a_code_block_longer_than_a_page_is_cut_between_its_lines() {
        let lines: Vec<String> = (0..3000).map(|n| format!("row{n:04} = {n}")).collect();
        let doc = pdf_of(&format!("# Numbers\n\n```\n{}\n```\n", lines.join("\n")));
        let pages = heights(&doc);
        assert!(pages.len() >= 2, "{pages:?}");
        assert!(pages.iter().all(|&h| h <= MAX_PAGE_PT), "{pages:?}");
        let all: String = (1..=pages.len() as u32).map(|n| doc.extract_text(&[n]).expect("text")).collect();
        for n in [0, 1499, 2999] {
            assert_eq!(all.matches(&format!("row{n:04}")).count(), 1, "row {n} once");
        }
        assert_eq!(all.matches("row").count(), 3000, "every row once");
    }

    #[test]
    fn a_vertical_page_is_as_tall_as_its_columns_and_its_text_reads() {
        let mut p = Preview::new(crate::Options { width: 1400, scale: 2.0, theme: crate::Theme::light(), base_dir: ".".into() }, Box::new(crate::NoFormulas));
        p.set_height(900);
        p.set_vertical(Some(crate::Vertical { latin: yumete_core::export::Latin::Turn, hanging: false }));
        p.set_text("# 春\n\n春天的早晨，河边的柳树发了新芽。\n");
        let doc = lopdf::Document::load_mem(&p.pdf("vertical").expect("a PDF")).expect("a PDF that reads back");
        assert_eq!(heights(&doc), vec![900.0 * PT_PER_PX / 2.0]);
        let text: String = doc.extract_text(&[1]).expect("text").chars().filter(|c| !c.is_whitespace()).collect();
        assert!(text.contains("春天的早晨，河边的柳树发了新芽。"), "{text:?}");
    }

    #[test]
    fn with_nowhere_to_cut_a_page_is_cut_at_the_limit() {
        let pages = slices(2500.0, &[], &[], 1000.0, 16.0, 32.0);
        assert!(pages.iter().all(|p| p.len() <= 1000.0 + 0.01));
        assert_eq!(pages.last().map(|p| p.to), Some(2500.0));
    }
}
