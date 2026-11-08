//! **Drawing a viewport** of a laid-out document with vello_cpu: only the
//! blocks that reach into it are drawn, so a screen costs the same at the top
//! of a long file as at its end.

use std::sync::Arc;

use parley::{Layout, PositionedLayoutItem};
use vello_cpu::kurbo::{Affine, BezPath, Rect, RoundedRect, Shape, Stroke};
use vello_cpu::peniko::color::AlphaColor;
use vello_cpu::{Image, ImageSource, Pixmap, RenderContext, Resources};

use crate::layout::{Attach, Ink, Item, Laid, Text};
use crate::theme::Rgba;

fn colour(c: Rgba) -> AlphaColor<vello_cpu::peniko::color::Srgb> {
    AlphaColor::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Draws viewports, keeping its buffers between them.
pub(crate) struct Painter {
    ctx: RenderContext,
    res: Resources,
}

impl Painter {
    pub fn new() -> Painter {
        Painter { ctx: RenderContext::new(1, 1), res: Resources::new() }
    }

    /// **Draw `blocks` (each with its top) from `top` down, `w` × `h`**:
    /// straight (not premultiplied) RGBA, row by row.
    pub fn paint(&mut self, blocks: &[(f32, &Laid)], ground: Rgba, top: f32, w: u16, h: u16) -> Vec<u8> {
        if self.ctx.width() != w || self.ctx.height() != h {
            self.ctx = RenderContext::new(w, h);
        }
        let ctx = &mut self.ctx;
        ctx.reset();
        ctx.set_transform(Affine::IDENTITY);
        ctx.set_paint(colour(ground));
        ctx.fill_rect(&Rect::new(0.0, 0.0, w as f64, h as f64));
        for (y, laid) in blocks {
            if y + laid.height < top - 64.0 || *y > top + h as f32 + 64.0 {
                continue;
            }
            let oy = (y - top) as f64;
            for item in &laid.items {
                draw_item(ctx, &mut self.res, item, oy, h as f64);
            }
        }
        ctx.flush();
        let mut px = Pixmap::new(w, h);
        ctx.render(&mut px, &mut self.res);
        // The ground is opaque, so premultiplied is straight.
        px.take_rgba8(vello_cpu::peniko::ImageAlphaType::AlphaPremultiplied)
    }
}

fn draw_item(ctx: &mut RenderContext, res: &mut Resources, item: &Item, oy: f64, h: f64) {
    match item {
        Item::Rect { r, fill, radius } => {
            let r = *r + vello_cpu::kurbo::Vec2::new(0.0, oy);
            if r.y1 < 0.0 || r.y0 > h {
                return;
            }
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(colour(*fill));
            if *radius > 0.0 {
                ctx.fill_path(&RoundedRect::from_rect(r, *radius as f64).to_path(0.1));
            } else {
                ctx.fill_rect(&r);
            }
        }
        Item::Frame { r, color, width, radius } => {
            let r = *r + vello_cpu::kurbo::Vec2::new(0.0, oy);
            if r.y1 < 0.0 || r.y0 > h {
                return;
            }
            let half = *width as f64 / 2.0;
            let r = r.inset(-half);
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(colour(*color));
            ctx.set_stroke(Stroke::new(*width as f64));
            ctx.stroke_path(&RoundedRect::from_rect(r, *radius as f64).to_path(0.1));
        }
        Item::Pic { r, pix } => {
            let r = *r + vello_cpu::kurbo::Vec2::new(0.0, oy);
            if r.y1 < 0.0 || r.y0 > h {
                return;
            }
            draw_pic(ctx, pix, r.x0, r.y0, r.width(), r.height());
        }
        Item::Lines { points, color, width } => {
            let mut path = BezPath::new();
            for (k, (x, y)) in points.iter().enumerate() {
                let p = (*x as f64, *y as f64 + oy);
                if k == 0 {
                    path.move_to(p);
                } else {
                    path.line_to(p);
                }
            }
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(colour(*color));
            ctx.set_stroke(Stroke::new(*width as f64).with_caps(vello_cpu::kurbo::Cap::Round).with_join(vello_cpu::kurbo::Join::Round));
            ctx.stroke_path(&path);
        }
        Item::Text { x, y, text } => {
            let top = *y as f64 + oy;
            if top + text.layout.height() as f64 + 64.0 < 0.0 || top - 64.0 > h {
                return;
            }
            draw_text(ctx, res, text, *x as f64, top, h);
        }
    }
}

/// A picture scaled into a box, its corner on whole pixels.
fn draw_pic(ctx: &mut RenderContext, pix: &Arc<Pixmap>, x: f64, y: f64, w: f64, h: f64) {
    let (iw, ih) = (pix.width() as f64, pix.height() as f64);
    let (x, y) = (x.round(), y.round());
    ctx.set_transform(Affine::translate((x, y)) * Affine::scale_non_uniform(w / iw, h / ih));
    ctx.set_paint(Image { image: ImageSource::Pixmap(pix.clone()), sampler: Default::default() });
    ctx.fill_rect(&Rect::new(0.0, 0.0, iw, ih));
}

fn draw_text(ctx: &mut RenderContext, res: &mut Resources, text: &Text, ox: f64, oy: f64, h: f64) {
    draw_layout(ctx, res, &text.layout, &text.attach, ox, oy, h);
}

fn draw_layout(ctx: &mut RenderContext, res: &mut Resources, layout: &Layout<Ink>, attach: &[Attach], ox: f64, oy: f64, h: f64) {
    for line in layout.lines() {
        let m = line.metrics();
        let (lt, lb) = (oy + m.block_min_coord as f64, oy + m.block_max_coord as f64);
        if lb < -64.0 || lt > h + 64.0 {
            continue;
        }
        // Grounds first, so a run's ground never covers its neighbour's ink.
        for item in line.items() {
            let PositionedLayoutItem::GlyphRun(g) = item else { continue };
            let ink = g.style().brush;
            let Some(ground) = ink.ground else { continue };
            let size = g.run().font_size() as f64;
            let (x0, x1) = (g.offset() as f64, (g.offset() + g.advance()) as f64);
            let base = g.baseline() as f64;
            ctx.set_transform(Affine::translate((ox, oy)));
            let pad = match (ink.frame, ink.mono) {
                (true, _) => 0.3 * size,
                (false, true) => 0.2 * size,
                (false, false) => 0.05 * size,
            };
            // From the size, not the font's metrics: a ground over Latin and
            // 漢字 in two fonts is one height.
            let (up, down) = if ink.mono { (1.0, 0.32) } else { (1.0, 0.3) };
            let r = Rect::new(x0 - pad, base - up * size, x1 + pad, base + down * size);
            let radius = if ink.mono || ink.frame { 0.25 * size } else { 0.1 * size };
            ctx.set_paint(colour(ground));
            ctx.fill_path(&RoundedRect::from_rect(r, radius).to_path(0.1));
            if ink.frame {
                ctx.set_paint(colour([ink.color[0], ink.color[1], ink.color[2], 0x50]));
                ctx.set_stroke(Stroke::new(0.07 * size));
                ctx.stroke_path(&RoundedRect::from_rect(r, radius).to_path(0.1));
            }
        }
        for item in line.items() {
            match item {
                PositionedLayoutItem::GlyphRun(g) => {
                    let run = g.run();
                    let ink = g.style().brush;
                    let rm = run.font_metrics();
                    let size = run.font_size() as f64;
                    let (x0, x1) = (g.offset() as f64, (g.offset() + g.advance()) as f64);
                    let base = g.baseline() as f64;
                    ctx.set_transform(Affine::translate((ox, oy)));
                    let skew = run.synthesis().skew().map(|a| Affine::skew((a as f64).to_radians().tan(), 0.0));
                    ctx.set_paint(colour(ink.color));
                    let font = run.font();
                    let coords: Vec<i16> = run.normalized_coords().iter().map(|c| c.to_bits()).collect();
                    let mut b = ctx.glyph_run(res, font).font_size(run.font_size()).hint(false).normalized_coords(&coords);
                    if let Some(s) = skew {
                        b = b.glyph_transform(s);
                    }
                    let _ = b.fill_glyphs(g.positioned_glyphs().map(|gl| vello_cpu::Glyph { id: gl.id, x: gl.x, y: gl.y }));
                    if ink.underline {
                        let t = (rm.underline_size as f64).max(0.06 * size);
                        let y = base - rm.underline_offset as f64;
                        ctx.set_paint(colour(ink.color));
                        ctx.fill_rect(&Rect::new(x0, y, x1, y + t));
                    }
                    if ink.strike {
                        let t = (rm.strikethrough_size as f64).max(0.06 * size);
                        let y = base - rm.strikethrough_offset as f64;
                        ctx.set_paint(colour(ink.color));
                        ctx.fill_rect(&Rect::new(x0, y - t / 2.0, x1, y + t / 2.0));
                    }
                }
                PositionedLayoutItem::InlineBox(bx) => match attach.get(bx.id as usize) {
                    Some(Attach::Pic(p)) => draw_pic(ctx, p, ox + bx.x as f64, oy + bx.y as f64, bx.width as f64, bx.height as f64),
                    Some(Attach::Ruby { base, reading }) => {
                        let bxx = ox + bx.x as f64;
                        let bxy = oy + bx.y as f64;
                        let w = bx.width as f64;
                        draw_layout(ctx, res, base, &[], bxx + (w - base.width() as f64) / 2.0, bxy, h);
                        // The reading sits just over the base's ink.
                        let size = base.lines().next().and_then(|l| l.runs().next().map(|r| r.font_size())).unwrap_or(16.0) as f64;
                        let base_line = base.first_baseline().unwrap_or(0.0) as f64;
                        let reading_base = reading.first_baseline().unwrap_or(0.0) as f64;
                        let target = bxy + base_line - 0.92 * size - 0.08 * size;
                        draw_layout(ctx, res, reading, &[], bxx + (w - reading.width() as f64) / 2.0, target - reading_base, h);
                    }
                    None => {}
                },
            }
        }
    }
}
