//! **Laying blocks out** with parley: real type sizes, the system's fonts,
//! and every visual line marked with the source line it came from.
//!
//! The look is GitHub's Markdown stylesheet, which VS Code's preview also
//! uses: 16 px type at line height 1.6, headings 2 / 1.5 / 1.25 em with a rule
//! under the top two, 16 px between blocks, lists indented 2 em, quotes with a
//! 0.25 em bar, code on a rounded ground. Every length here is that CSS pixel
//! times the scale (`s`), so a layout is in device pixels.

use std::path::PathBuf;
use std::sync::Arc;

use parley::{
    Alignment, AlignmentOptions, FontContext, FontFamily, FontStyle, FontWeight, InlineBox, InlineBoxKind, Layout,
    LayoutContext, LineHeight, StyleProperty, VerticalAlign, WhiteSpaceCollapse,
};
use vello_cpu::kurbo::Rect;
use vello_cpu::Pixmap;

use crate::parse::{Align, Block, BlockKind, Callout, Inline, InlineKind, Item as ListItem, Marks};
use crate::cache::Lru;
use crate::fonts::{families, Families, Han};
use crate::theme::{Rgba, Theme};
use crate::Formulas;

/// A picture, shared between layouts.
pub(crate) type Pix = Arc<Pixmap>;

/// **The brush text is drawn with**: its colour and what is drawn with it.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct Ink {
    pub color: Rgba,
    /// A ground behind the run: inline code, `==highlight==`, `<kbd>`.
    pub ground: Option<Rgba>,
    /// A frame around the ground (`<kbd>`).
    pub frame: bool,
    pub underline: bool,
    pub strike: bool,
    /// Set in the monospace face: its ground is padded as code's is.
    pub mono: bool,
}

/// What an inline box holds.
pub(crate) enum Attach {
    Pic(Pix),
    /// A ruby base and its reading, both centred in the box.
    Ruby { base: Box<Layout<Ink>>, reading: Box<Layout<Ink>> },
}

/// Laid-out text: the layout, what its inline boxes hold, and where each
/// source line's text begins (byte, line).
pub(crate) struct Text {
    pub layout: Layout<Ink>,
    pub attach: Vec<Attach>,
    pub src: Vec<(usize, usize)>,
    /// The text laid out.
    pub string: String,
}

/// **A code block's text**: its lines joined, each token's colour, and
/// where each source line begins (byte, line).
pub(crate) struct CodeText {
    pub text: String,
    pub colours: Vec<(std::ops::Range<usize>, Rgba)>,
    pub src: Vec<(usize, usize)>,
}

/// **One thing drawn**, at a place within its top-level block.
pub(crate) enum Item {
    Text { x: f32, y: f32, text: Box<Text> },
    Rect { r: Rect, fill: Rgba, radius: f32 },
    Frame { r: Rect, color: Rgba, width: f32, radius: f32 },
    Pic { r: Rect, pix: Pix },
    /// A stroked polyline: a task's tick.
    Lines { points: Vec<(f32, f32)>, color: Rgba, width: f32 },
    /// Glyphs placed by hand: a vertical page's characters.
    Glyphs(Box<Run>),
    /// A picture turned a quarter turn clockwise into `r`: a formula down a
    /// vertical page.
    Turned { r: Rect, pix: Pix },
}

/// **Glyphs of one face, size and colour**, drawn through `transform`.
pub(crate) struct Run {
    pub font: parley::FontData,
    pub size: f32,
    pub coords: Vec<i16>,
    pub color: Rgba,
    pub transform: vello_cpu::kurbo::Affine,
    pub glyphs: Vec<vello_cpu::Glyph>,
    /// Where they fall, in the block's coordinates.
    pub bounds: Rect,
}

/// **A top-level block, laid out**: what it draws, how tall it is, the
/// margins it asks for above and below, and its line marks — each visual
/// line's top and the source line it shows, counted from the block's first.
pub(crate) struct Laid {
    pub items: Vec<Item>,
    pub height: f32,
    pub top: f32,
    pub bottom: f32,
    pub marks: Vec<(f32, usize)>,
    /// The widest anything reaches, from the page's left.
    pub right: f32,
    /// A vertical page's columns: how far from the block's right edge each
    /// begins, and its words.
    pub cols: Vec<(f32, String)>,
}


/// A text style.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Style {
    pub size: f32,
    pub weight: f32,
    pub color: Rgba,
    pub mono: bool,
    pub line_height: f32,
    pub italic: bool,
    /// A word too long for its line may break anywhere: a table's last resort.
    pub anywhere: bool,
}

/// Where a block is being laid: inherited by what is inside it.
#[derive(Clone, Copy)]
pub(crate) struct Ctx {
    /// The ink of plain text here.
    pub color: Rgba,
    /// How deep in lists.
    pub depth: u8,
    /// In a tight list: paragraphs take no margin.
    pub tight: bool,
    /// The body type size, device pixels (smaller in the footnotes).
    pub size: f32,
}

/// How many formulas are kept drawn.
pub(crate) const FORMULAS_KEPT: usize = 1024;
/// How many bytes of pictures are kept: two thirds for the pictures as read,
/// one third for them scaled to the column.
pub(crate) const IMAGE_BYTES_KEPT: usize = 96 << 20;

/// When a picture's file last changed.
pub(crate) type Stamp = u64;

/// A styled stretch of a paragraph's text: its bytes, its marks, its colour
/// if it has one of its own, and whether it is set small (a superscript).
pub(crate) type Ranged = (std::ops::Range<usize>, Marks, Option<Rgba>, bool);

/// **The stretch byte `at` is in**, if any. The stretches are pushed as the
/// text grows, so they are in order and never overlap, and one is found by
/// halving: asked at every character of a paragraph, a walk over all of them
/// would make a long paragraph quadratic to lay out.
pub(crate) fn ranged_at(ranges: &[Ranged], at: usize) -> Option<&Ranged> {
    let k = ranges.partition_point(|(r, ..)| r.end <= at);
    ranges.get(k).filter(|(r, ..)| r.contains(&at))
}

/// A formula, drawn: its picture, size and baseline (from its top).
#[derive(Clone)]
pub(crate) struct FormulaPic {
    pub pix: Pix,
    pub width: f32,
    pub height: f32,
    pub baseline: f32,
}

/// **What lays blocks out**, with what it keeps between layouts: the fonts,
/// the formulas and pictures drawn so far.
pub(crate) struct Engine {
    pub fonts: FontContext,
    pub lcx: LayoutContext<Ink>,
    pub theme: Theme,
    /// Device pixels to a CSS pixel.
    pub scale: f32,
    /// Where relative image paths start.
    pub base_dir: PathBuf,
    pub formulas: Box<dyn Formulas>,
    /// The document's writing system, and the faces of each.
    pub han: Han,
    /// **Whether curly quotes become corner brackets down a vertical page**
    /// (2026-11-09): the export's rule, the document in Chinese as
    /// [`yumete_core::export::document_language`] decides it.
    pub quotes: bool,
    families: [Arc<Families>; 3],
    /// `[export] font_family`, which the faces of each writing system begin
    /// with.
    font_list: String,
    /// Whether adjacent full-width marks are squeezed.
    pub squeeze: bool,
    /// Formulas drawn, at most [`FORMULAS_KEPT`] of them.
    formula_cache: Lru<(String, bool, Rgba, u32), Option<FormulaPic>>,
    /// Pictures read from disk, with the file's stamp when read, and those
    /// pictures scaled to the column: each kept to a budget in bytes.
    images: Lru<String, (Stamp, Option<Arc<image::RgbaImage>>)>,
    scaled: Lru<(String, u32, Stamp), Pix>,
    /// Each picture's size in pixels, read from its file's header, with the
    /// file's stamp when read.
    sizes: std::collections::HashMap<String, (Stamp, Option<(u32, u32)>)>,
    /// Set vertically, and how (`:preview-vertical`).
    pub vertical: Option<crate::vertical::Vertical>,
    /// Characters shaped down a column, by face: each one's glyph, advance
    /// and offsets, in ems.
    pub(crate) upright: std::collections::HashMap<(u64, u32, char), Option<crate::vertical::UpGlyph>>,
    pub(crate) shapers: std::collections::HashMap<(u64, u32), harfrust::ShaperData>,
}

impl Engine {
    pub fn new(theme: Theme, scale: f32, base_dir: PathBuf, formulas: Box<dyn Formulas>) -> Engine {
        Engine {
            fonts: FontContext::new(),
            lcx: LayoutContext::new(),
            theme,
            scale,
            base_dir,
            formulas,
            han: Han::default(),
            quotes: false,
            squeeze: true,
            families: [Han::Simplified, Han::Traditional, Han::Japanese].map(|h| Arc::new(families(h, ""))),
            font_list: String::new(),
            formula_cache: Lru::new(FORMULAS_KEPT),
            images: Lru::new(IMAGE_BYTES_KEPT / 3 * 2),
            scaled: Lru::new(IMAGE_BYTES_KEPT / 3),
            sizes: std::collections::HashMap::new(),
            vertical: None,
            upright: std::collections::HashMap::new(),
            shapers: std::collections::HashMap::new(),
        }
    }

    /// Set the font list the writing is set in; `true` when it changed.
    pub fn set_font_list(&mut self, list: &str) -> bool {
        if self.font_list == list {
            return false;
        }
        self.font_list = list.to_string();
        self.families = [Han::Simplified, Han::Traditional, Han::Japanese].map(|h| Arc::new(families(h, list)));
        true
    }

    /// One CSS pixel.
    fn px(&self) -> f32 {
        self.scale
    }

    /// The body text size.
    pub fn base(&self) -> f32 {
        yumete_core::as_preview::BODY_PX * self.scale
    }

    pub(crate) fn body(&self, cx: &Ctx) -> Style {
        let line_height = yumete_core::as_preview::LINE_HEIGHT;
        Style { size: cx.size, weight: 400.0, color: cx.color, mono: false, line_height, italic: false, anywhere: false }
    }

    // ── Pictures ────────────────────────────────────────────────────────────

    /// The formula `tex`, drawn at `size` in `ink`.
    pub(crate) fn formula(&mut self, tex: &str, display: bool, ink: Rgba, size: f32) -> Option<FormulaPic> {
        let key = (tex.to_string(), display, ink, (size * 10.0) as u32);
        if let Some(made) = self.formula_cache.get(&key) {
            return made;
        }
        let made = self.formulas.draw(tex, display, [ink[0], ink[1], ink[2]], size).ok().and_then(|f| {
            let pix = Pixmap::from_png(std::io::Cursor::new(&f.png)).ok()?;
            let (w, h) = (pix.width() as f32, pix.height() as f32);
            Some(FormulaPic { pix: Arc::new(pix), width: w, height: h, baseline: f.baseline })
        });
        self.formula_cache.put(key, made.clone(), 1);
        made
    }

    /// The picture at `src`, at most `room` wide: drawn at its own size (a
    /// CSS pixel a pixel), smaller to fit.
    pub(crate) fn image(&mut self, src: &str, room: f32) -> Option<Pix> {
        if src.contains("://") || src.is_empty() {
            return None;
        }
        // Read again when the file changed since it was read.
        let stamp = self.image_stamp(src);
        let raw = match self.images.get(&src.to_string()) {
            Some((was, made)) if was == stamp => made,
            _ => {
                let path = self.base_dir.join(percent_decoded(src));
                let made = image::open(&path).ok().map(|i| Arc::new(i.to_rgba8()));
                let cost = made.as_ref().map_or(1, |i| i.as_raw().len());
                self.images.put(src.to_string(), (stamp, made.clone()), cost);
                made
            }
        }?;
        let (w0, h0) = (raw.width() as f32 * self.scale, raw.height() as f32 * self.scale);
        let k = (room / w0).min(1.0);
        let (w, h) = ((w0 * k).round().max(1.0) as u32, (h0 * k).round().max(1.0) as u32);
        if w > u16::MAX as u32 || h > u16::MAX as u32 {
            return None;
        }
        let key = (src.to_string(), w, stamp);
        if let Some(p) = self.scaled.get(&key) {
            return Some(p);
        }
        let resized = if (w, h) == (raw.width(), raw.height()) {
            (*raw).clone()
        } else {
            image::imageops::resize(&*raw, w, h, image::imageops::FilterType::Triangle)
        };
        let meta = vello_cpu::PixelMetadata { may_have_transparency: true, alpha_type: vello_cpu::peniko::ImageAlphaType::Alpha };
        let pix = Arc::new(Pixmap::from_parts(resized.into_raw(), w as u16, h as u16, meta));
        self.scaled.put(key, pix.clone(), w as usize * h as usize * 4);
        Some(pix)
    }

    /// **How large the picture at `src` is drawn at its own size** (a CSS
    /// pixel a pixel), in device pixels (2026-11-09): read from the file's
    /// header, without decoding the picture, and kept until the file
    /// changes. `None` for one that cannot be read.
    pub(crate) fn image_size(&mut self, src: &str) -> Option<(f32, f32)> {
        if src.contains("://") || src.is_empty() {
            return None;
        }
        let stamp = self.image_stamp(src);
        let size = match self.sizes.get(src) {
            Some(&(was, size)) if was == stamp => size,
            _ => {
                let size = image::image_dimensions(self.base_dir.join(percent_decoded(src))).ok();
                self.sizes.insert(src.to_string(), (stamp, size));
                size
            }
        }?;
        Some((size.0 as f32 * self.scale, size.1 as f32 * self.scale))
    }

    /// **When the picture at `src` last changed**: its file's modification
    /// time and length, or zero for one that is not a local file.
    pub(crate) fn image_stamp(&self, src: &str) -> Stamp {
        if src.contains("://") || src.is_empty() {
            return 0;
        }
        let Ok(meta) = std::fs::metadata(self.base_dir.join(percent_decoded(src))) else { return 0 };
        let time = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
        time ^ meta.len().rotate_left(32)
    }

    /// How many formulas and how many bytes of pictures are kept.
    pub(crate) fn kept(&self) -> (usize, usize) {
        (self.formula_cache.len(), self.images.cost() + self.scaled.cost())
    }

    // ── Text ────────────────────────────────────────────────────────────────

    /// **Lay `runs` out** in style `st`, `width` wide (or unbroken).
    pub(crate) fn text(&mut self, runs: &[Inline], st: Style, width: Option<f32>, align: Alignment) -> Text {
        let mut text = String::new();
        let mut ranges: Vec<Ranged> = Vec::new(); // marks, colour, superscript
        let mut boxes: Vec<InlineBox> = Vec::new();
        let mut attach: Vec<Attach> = Vec::new();
        let mut src: Vec<(usize, usize)> = Vec::new();
        for run in runs {
            if src.last().is_none_or(|s| s.1 != run.line) {
                src.push((text.len(), run.line));
            }
            match &run.kind {
                InlineKind::Text { text: t, marks } => {
                    let a = text.len();
                    text.push_str(t);
                    ranges.push((a..text.len(), *marks, None, false));
                }
                InlineKind::Break => text.push('\n'),
                InlineKind::NoteRef(n) => {
                    let a = text.len();
                    text.push_str(&n.to_string());
                    ranges.push((a..text.len(), Marks::default(), Some(self.theme.footnote), true));
                }
                InlineKind::Formula { tex, display } => {
                    let size = st.size * 1.05;
                    match self.formula(tex, *display, st.color, size) {
                        Some(f) => {
                            boxes.push(InlineBox {
                                id: attach.len() as u64,
                                kind: InlineBoxKind::InFlow,
                                index: text.len(),
                                width: f.width,
                                height: f.height,
                                baseline: Some(f.baseline),
                                vertical_align: VerticalAlign::BASELINE,
                            });
                            attach.push(Attach::Pic(f.pix));
                        }
                        None => {
                            let a = text.len();
                            text.push_str(tex);
                            let marks = Marks { code: true, ..Marks::default() };
                            ranges.push((a..text.len(), marks, Some(self.theme.error), false));
                        }
                    }
                }
                InlineKind::Image { src: path, alt } => {
                    let room = width.unwrap_or(f32::MAX).max(1.0);
                    match self.image(path, room) {
                        Some(pix) => {
                            boxes.push(InlineBox {
                                id: attach.len() as u64,
                                kind: InlineBoxKind::InFlow,
                                index: text.len(),
                                width: pix.width() as f32,
                                height: pix.height() as f32,
                                baseline: None,
                                vertical_align: VerticalAlign::BASELINE,
                            });
                            attach.push(Attach::Pic(pix));
                        }
                        None => {
                            let a = text.len();
                            text.push_str(if alt.is_empty() { path } else { alt });
                            let marks = Marks { kbd: true, ..Marks::default() };
                            ranges.push((a..text.len(), marks, Some(self.theme.muted), false));
                        }
                    }
                }
                InlineKind::Ruby { base, reading, marks } => {
                    let base_run = [Inline { line: run.line, kind: InlineKind::Text { text: base.clone(), marks: *marks } }];
                    let b = self.text(&base_run, st, None, Alignment::Start);
                    let small = Style { size: st.size * 0.5, line_height: 1.0, weight: 400.0, ..st };
                    let reading_run = [Inline { line: run.line, kind: InlineKind::Text { text: reading.clone(), marks: Marks::default() } }];
                    let r = self.text(&reading_run, small, None, Alignment::Start);
                    let w = b.layout.width().max(r.layout.width());
                    boxes.push(InlineBox {
                        id: attach.len() as u64,
                        kind: InlineBoxKind::InFlow,
                        index: text.len(),
                        width: w,
                        height: b.layout.height(),
                        baseline: b.layout.first_baseline(),
                        vertical_align: VerticalAlign::BASELINE,
                    });
                    attach.push(Attach::Ruby { base: Box::new(b.layout), reading: Box::new(r.layout) });
                }
            }
        }
        if src.is_empty() {
            src.push((0, 0));
        }
        let doc = self.families[self.han.index()].clone();
        let all = self.families.clone();
        let fam_of = |lang: u8| -> &Families { if lang == 0 { &doc } else { &all[(lang as usize - 1).min(2)] } };
        let family = if st.mono { doc.mono.as_str() } else { doc.sans.as_str() };
        let mut b = self.lcx.ranged_builder(&mut self.fonts, &text, 1.0, true);
        b.push_default(StyleProperty::FontFamily(FontFamily::from(family)));
        b.push_default(StyleProperty::Locale(parley::Language::parse(self.han.tag()).ok()));
        b.push_default(StyleProperty::FontSize(st.size));
        b.push_default(StyleProperty::FontWeight(FontWeight::new(st.weight)));
        b.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(st.line_height)));
        b.push_default(StyleProperty::WhiteSpaceCollapse(if st.mono { WhiteSpaceCollapse::Preserve } else { WhiteSpaceCollapse::PreserveBreaks }));
        if st.italic {
            b.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        if st.anywhere {
            b.push_default(StyleProperty::OverflowWrap(parley::OverflowWrap::Anywhere));
        }
        b.push_default(StyleProperty::Brush(Ink { color: st.color, ..Ink::default() }));
        for (r, marks, color, sup) in &ranges {
            let mut ink = ink_of(&self.theme, marks, st.color, st.mono);
            if let Some(c) = color {
                ink.color = *c;
            }
            push_marks(&mut b, r.clone(), marks, st, fam_of(marks.lang));
            if *sup || marks.sup {
                b.push(StyleProperty::VerticalAlign(VerticalAlign::SUPER), r.clone());
                b.push(StyleProperty::FontSize(st.size * 0.75), r.clone());
            }
            if marks.sub {
                b.push(StyleProperty::VerticalAlign(VerticalAlign::SUB), r.clone());
                b.push(StyleProperty::FontSize(st.size * 0.75), r.clone());
            }
            if ink != (Ink { color: st.color, ..Ink::default() }) {
                b.push(StyleProperty::Brush(ink), r.clone());
            }
        }
        // **Two full-width marks side by side give up the blank half between
        // them** (clreq's 標點擠壓): 「。」」, 「」，」, 「）「」 each take one and a
        // half ems, not two. A closing mark's blank is its right half, an
        // opening mark's its left; the squeeze is a negative letter spacing on
        // the first of the two, which pulls the second back by half an em.
        // 繁體 centres 、。，；：！？ in the em, so there only the brackets
        // squeeze; Japanese centres ：； too; ！？ never give theirs up.
        if !st.mono && self.squeeze {
            let chars: Vec<(usize, char)> = text.char_indices().collect();
            let han_at = |at: usize| -> Han {
                match ranged_at(&ranges, at).map_or(0, |(_, m, _, _)| m.lang) {
                    0 => self.han,
                    n => [Han::Simplified, Han::Traditional, Han::Japanese][(n as usize - 1).min(2)],
                }
            };
            let plain = |at: usize| !ranged_at(&ranges, at).is_some_and(|(_, m, _, sup)| m.code || m.kbd || m.sup || m.sub || *sup);
            for pair in chars.windows(2) {
                let ((at, first), (_, second)) = (pair[0], pair[1]);
                let han = han_at(at);
                let squeeze = (closing_mark(first, han) && (closing_mark(second, han) || opening_mark(second))) || (opening_mark(first) && opening_mark(second));
                if squeeze && plain(at) {
                    b.push(StyleProperty::LetterSpacing(-0.5 * st.size), at..at + first.len_utf8());
                }
            }
        }
        // Quotes, dashes and ellipses between 漢字 are set in the CJK face:
        // the Latin face's are narrow and sit low (「……」 on the baseline).
        if !st.mono {
            let chars: Vec<(usize, char)> = text.char_indices().collect();
            let wide = |c: char| yumete_cjk::char_width(c) == 2 && !cjk_punctuation(c);
            for (k, &(at, c)) in chars.iter().enumerate() {
                if !cjk_punctuation(c) || ranged_at(&ranges, at).is_some_and(|(_, m, _, _)| m.code || m.kbd) {
                    continue;
                }
                let before = chars[..k].iter().rev().map(|p| p.1).find(|c| !cjk_punctuation(*c) && !c.is_whitespace());
                let after = chars[k + 1..].iter().map(|p| p.1).find(|c| !cjk_punctuation(*c) && !c.is_whitespace());
                if before.is_some_and(wide) || after.is_some_and(wide) {
                    let lang = ranged_at(&ranges, at).map_or(0, |(_, m, _, _)| m.lang);
                    b.push(StyleProperty::FontFamily(FontFamily::from(fam_of(lang).cjk_first.as_str())), at..at + c.len_utf8());
                }
            }
        }
        for bx in boxes {
            b.push_inline_box(bx);
        }
        let mut layout = b.build(&text);
        layout.break_all_lines(width);
        layout.align(align, AlignmentOptions { align_when_overflowing: false, ..AlignmentOptions::default() });
        Text { layout, attach, src, string: text }
    }

    /// The faces of each writing system: the document's, and the three a
    /// `<span lang>` may name.
    pub(crate) fn faces(&self) -> (Arc<Families>, [Arc<Families>; 3]) {
        (self.families[self.han.index()].clone(), self.families.clone())
    }

    /// Plain words in a style.
    fn words(&mut self, words: &str, st: Style, width: Option<f32>, align: Alignment) -> Text {
        let run = [Inline { line: 0, kind: InlineKind::Text { text: words.to_string(), marks: Marks::default() } }];
        self.text(&run, st, width, align)
    }

    // ── Blocks ──────────────────────────────────────────────────────────────

    /// **Lay a top-level block out** in a column `width` wide from `x`,
    /// with the caption a table takes over it (2026-11-08), in the type the
    /// vertical page sets one in.
    pub(crate) fn top(&mut self, block: &Block, caption: Option<&[Inline]>, x: f32, width: f32) -> Laid {
        let cx = Ctx { color: self.theme.text, depth: 0, tight: false, size: self.base() };
        let mut out = Out::default();
        let mut y = 0.0;
        if let Some(caption) = caption.filter(|_| matches!(block.kind, BlockKind::Table { .. })) {
            let st = Style { size: cx.size * 0.9, ..self.body(&cx) };
            let t = self.text(caption, st, Some(width), Alignment::Start);
            y += self.put_text(t, x, y, &mut out, block.lines.0);
            y += 0.25 * cx.size;
        }
        self.block(block, x, width, &mut y, &mut out, cx);
        let (top, bottom) = self.margins(block, cx);
        let right = out.items.iter().map(item_right).fold(x + width, f32::max);
        Laid { items: out.items, height: y, top, bottom, marks: out.marks, right, cols: Vec::new() }
    }

    /// The margins a block asks for above and below it.
    fn margins(&self, block: &Block, cx: Ctx) -> (f32, f32) {
        let px = self.px();
        match &block.kind {
            BlockKind::Heading { .. } => (24.0 * px, 16.0 * px),
            BlockKind::Paragraph(_) if cx.tight => (0.0, 0.0),
            BlockKind::Rule => (24.0 * px, 24.0 * px),
            BlockKind::List { .. } if cx.depth > 0 => (0.0, 0.0),
            BlockKind::Footnotes(_) => (32.0 * px, 16.0 * px),
            BlockKind::FrontMatter(_) => (0.0, 24.0 * px),
            _ => (0.0, 16.0 * px),
        }
    }

    /// Blocks one under another, their margins collapsed.
    fn stack(&mut self, blocks: &[Block], x: f32, w: f32, y: &mut f32, out: &mut Out, cx: Ctx) {
        let mut below = 0.0f32;
        for (k, b) in blocks.iter().enumerate() {
            let (top, bottom) = self.margins(b, cx);
            if k > 0 {
                *y += below.max(top);
            }
            self.block(b, x, w, y, out, cx);
            below = bottom;
        }
    }

    /// Put a text at (x, y) and mark its lines; returns its height.
    fn put_text(&self, t: Text, x: f32, y: f32, out: &mut Out, base_line: usize) -> f32 {
        let h = t.layout.height();
        for line in t.layout.lines() {
            let start = line.text_range().start;
            let k = t.src.partition_point(|s| s.0 <= start).saturating_sub(1);
            let ln = t.src.get(k).map_or(base_line, |s| s.1);
            out.marks.push((y + line.metrics().block_min_coord, ln));
        }
        out.items.push(Item::Text { x, y, text: Box::new(t) });
        h
    }

    pub(crate) fn block(&mut self, block: &Block, x: f32, w: f32, y: &mut f32, out: &mut Out, cx: Ctx) {
        let px = self.px();
        let first = block.lines.0;
        match &block.kind {
            BlockKind::Heading { level, text } => {
                // The sizes `:export-as-preview` writes into its page.
                let n = (*level as usize).clamp(1, 6) - 1;
                let (k, rule) = (yumete_core::as_preview::HEADING_SIZES[n], n < 2);
                let color = self.theme.headings[n];
                let line_height = yumete_core::as_preview::HEADING_LINE_HEIGHT;
                let st = Style { size: cx.size * k, weight: 600.0, color, mono: false, line_height, italic: false, anywhere: false };
                let t = self.text(text, st, Some(w), Alignment::Start);
                *y += self.put_text(t, x, *y, out, first);
                if rule {
                    *y += 0.3 * st.size;
                    out.items.push(Item::Rect { r: rect(x, *y, w, px), fill: self.theme.border, radius: 0.0 });
                    *y += px;
                }
            }
            BlockKind::Paragraph(runs) => {
                // A picture alone in its paragraph is a figure, its text the
                // caption under it, as the vertical page and the export set
                // one (2026-11-08).
                if let Some((src, alt)) = crate::vertical::figure_of(runs) {
                    if self.figure(src, alt, first, x, w, y, out, cx) {
                        return;
                    }
                }
                let t = self.text(runs, self.body(&cx), Some(w), Alignment::Start);
                *y += self.put_text(t, x, *y, out, first);
            }
            BlockKind::Maths(tex) => {
                out.marks.push((*y, first));
                match self.formula(tex, true, cx.color, cx.size * 1.1) {
                    Some(f) => {
                        let fx = (x + (w - f.width).max(0.0) / 2.0).round();
                        out.items.push(Item::Pic { r: rect(fx, y.round(), f.width, f.height), pix: f.pix });
                        *y += f.height;
                    }
                    None => {
                        let st = Style { size: cx.size * 0.85, mono: true, color: self.theme.error, line_height: 1.45, ..self.body(&cx) };
                        let t = self.words(tex, st, Some(w), Alignment::Center);
                        let h = t.layout.height();
                        out.items.push(Item::Text { x, y: *y, text: Box::new(t) });
                        *y += h;
                    }
                }
            }
            BlockKind::Rule => {
                out.marks.push((*y, first));
                out.items.push(Item::Rect { r: rect(x, *y, w, 4.0 * px), fill: self.theme.border, radius: 0.0 });
                *y += 4.0 * px;
            }
            BlockKind::Code { info, lines } => self.code(info, lines, block.lines, x, w, y, out, cx),
            BlockKind::Quote(body) => {
                let y0 = *y;
                let bar = 0.25 * self.base();
                let pad = self.base();
                let inner = Ctx { color: self.theme.quote, ..cx };
                self.stack(body, x + bar + pad, w - bar - pad, y, out, inner);
                out.items.push(Item::Rect { r: rect(x, y0, bar, *y - y0), fill: self.theme.quote_bar, radius: 0.0 });
            }
            BlockKind::Callout { kind, name, title, fenced, body } => {
                self.callout(*kind, name, title.as_deref(), *fenced, body, block.lines, x, w, y, out, cx)
            }
            BlockKind::List { ordered, start, loose, items } => {
                self.list(*ordered, *start, *loose, items, x, w, y, out, cx);
            }
            BlockKind::Table { align, head, rows } => self.table(align, head, rows, first, x, w, y, out, cx),
            BlockKind::FrontMatter(lines) => {
                let st = Style { size: cx.size * 0.8, mono: true, color: self.theme.muted, line_height: 1.45, ..self.body(&cx) };
                let pad = 8.0 * px;
                let t = self.words(&lines.join("\n"), st, Some(w - 2.0 * pad), Alignment::Start);
                let h = t.layout.height() + 2.0 * pad;
                out.items.push(Item::Frame { r: rect(x, *y, w, h), color: self.theme.border, width: px, radius: 6.0 * px });
                out.items.push(Item::Text { x: x + pad, y: *y + pad, text: Box::new(t) });
                out.marks.push((*y, first));
                *y += h;
            }
            BlockKind::Html(source) => {
                out.marks.push((*y, first));
                let shown: String = source.chars().take(120).collect();
                let shown = if shown.len() < source.len() { format!("{shown}…") } else { shown };
                let st = Style { size: cx.size * 0.8, mono: true, color: self.theme.muted, line_height: 1.4, ..self.body(&cx) };
                let pad = 6.0 * px;
                let t = self.words(&shown, st, Some(w - 2.0 * pad), Alignment::Start);
                let (tw, th) = (t.layout.width() + 2.0 * pad, t.layout.height() + 2.0 * pad);
                out.items.push(Item::Rect { r: rect(x, *y, tw, th), fill: self.theme.code_block, radius: 4.0 * px });
                out.items.push(Item::Frame { r: rect(x, *y, tw, th), color: self.theme.border, width: px, radius: 4.0 * px });
                out.items.push(Item::Text { x: x + pad, y: *y + pad, text: Box::new(t) });
                *y += th;
            }
            BlockKind::Footnotes(notes) => {
                out.items.push(Item::Rect { r: rect(x, *y, w, px), fill: self.theme.border, radius: 0.0 });
                *y += 16.0 * px;
                let inner = Ctx { color: self.theme.muted, size: cx.size * 0.875, tight: false, ..cx };
                let indent = 2.0 * inner.size;
                for (k, note) in notes.iter().enumerate() {
                    if k > 0 {
                        *y += 4.0 * px;
                    }
                    let mark_at = out.items.len();
                    let y0 = *y;
                    out.marks.push((*y, note.lines.0));
                    self.stack(&note.body, x + indent, w - indent, y, out, Ctx { tight: true, ..inner });
                    let st = self.body(&inner);
                    self.marker(&format!("{}.", note.number), st, x + indent, y0, mark_at, out);
                }
            }
        }
    }

    /// **A figure across the page**: its picture as large as it is and no
    /// wider than the column, its caption under it in the notes' size and
    /// ink. `false` when the picture cannot be read.
    #[allow(clippy::too_many_arguments)]
    fn figure(&mut self, src: &str, alt: &str, line: usize, x: f32, w: f32, y: &mut f32, out: &mut Out, cx: Ctx) -> bool {
        let Some(pix) = self.image(src, w.max(1.0)) else { return false };
        let (pw, ph) = (pix.width() as f32, pix.height() as f32);
        out.marks.push((*y, line));
        out.items.push(Item::Pic { r: rect(x.round(), y.round(), pw, ph), pix });
        *y += ph;
        if !alt.trim().is_empty() {
            *y += 0.4 * cx.size;
            let st = Style { size: cx.size * 0.875, color: self.theme.muted, ..self.body(&cx) };
            let t = self.words(alt, st, Some(w), Alignment::Start);
            *y += self.put_text(t, x, *y, out, line);
        }
        true
    }

    /// A list marker `text` set right against `right`, on the baseline of the
    /// first text laid since item `from`.
    fn marker(&mut self, text: &str, st: Style, right: f32, y0: f32, from: usize, out: &mut Out) {
        let t = self.words(text, st, None, Alignment::Start);
        let baseline = first_baseline(&out.items[from..]).unwrap_or(y0 + t.layout.first_baseline().unwrap_or(0.0));
        let my = baseline - t.layout.first_baseline().unwrap_or(0.0);
        let mx = right - t.layout.width() - 0.4 * st.size;
        out.items.push(Item::Text { x: mx, y: my, text: Box::new(t) });
    }

    #[allow(clippy::too_many_arguments)]
    fn list(&mut self, ordered: bool, start: u64, loose: bool, items: &[ListItem], x: f32, w: f32, y: &mut f32, out: &mut Out, cx: Ctx) {
        let indent = 2.0 * cx.size;
        let inner = Ctx { depth: cx.depth + 1, tight: !loose, ..cx };
        for (k, item) in items.iter().enumerate() {
            if k > 0 {
                *y += if loose { 16.0 * self.px() } else { 0.25 * cx.size };
            }
            let from = out.items.len();
            let y0 = *y;
            out.marks.push((*y, item.lines.0));
            self.stack(&item.body, x + indent, w - indent, y, out, inner);
            if item.body.is_empty() {
                *y += cx.size * 1.6;
            }
            let st = Style { color: self.theme.list_marker, ..self.body(&cx) };
            match item.task {
                Some(done) => {
                    let baseline = first_baseline(&out.items[from..]).unwrap_or(y0 + cx.size * 1.2);
                    let side = 0.9 * cx.size;
                    let bx = x + indent - side - 0.45 * cx.size;
                    let by = baseline - side * 0.92;
                    let px = self.px();
                    if done {
                        out.items.push(Item::Rect { r: rect(bx, by, side, side), fill: self.theme.link, radius: 3.0 * px });
                        let p = |a: f32, b: f32| (bx + a * side, by + b * side);
                        out.items.push(Item::Lines { points: vec![p(0.22, 0.52), p(0.42, 0.72), p(0.78, 0.30)], color: self.theme.background, width: 2.0 * px });
                    } else {
                        out.items.push(Item::Rect { r: rect(bx, by, side, side), fill: self.theme.background, radius: 3.0 * px });
                        out.items.push(Item::Frame { r: rect(bx, by, side, side), color: self.theme.muted, width: 1.2 * px, radius: 3.0 * px });
                    }
                }
                None if ordered => self.marker(&format!("{}.", start + k as u64), st, x + indent, y0, from, out),
                None => {
                    // GitHub's disc, circle and square, drawn.
                    let baseline = first_baseline(&out.items[from..]).unwrap_or(y0 + cx.size * 1.2);
                    let d = 0.34 * cx.size;
                    let (bx, by) = (x + indent - 0.75 * cx.size - d / 2.0, baseline - 0.33 * cx.size - d / 2.0);
                    let r = rect(bx, by, d, d);
                    let px = self.px();
                    match cx.depth {
                        0 => out.items.push(Item::Rect { r, fill: st.color, radius: d / 2.0 }),
                        1 => out.items.push(Item::Frame { r: rect(bx + 0.5 * px, by + 0.5 * px, d - px, d - px), color: st.color, width: px, radius: d / 2.0 }),
                        _ => out.items.push(Item::Rect { r: rect(bx + 0.1 * d, by + 0.1 * d, 0.8 * d, 0.8 * d), fill: st.color, radius: 0.0 }),
                    }
                }
            }
        }
    }

    /// **A code block's text** as it is set: its lines joined, a tab as four
    /// spaces, each token's colour, and where each source line begins.
    /// The first line of code is source line `first`.
    pub(crate) fn code_text(&self, info: &str, lines: &[String], first: usize) -> CodeText {
        let language = info.split_whitespace().next().and_then(yumete_core::code::Language::from_info);
        let painted = language.map(|lang| yumete_core::code::highlight(lang, lines));
        let mut runs = Vec::new();
        for (k, line) in lines.iter().enumerate() {
            let chars: Vec<char> = line.chars().collect();
            // A tab is four spaces: parley's tab is one space wide.
            let untab = |a: usize, b: usize| -> String {
                chars[a.min(chars.len())..b.min(chars.len())].iter().map(|&c| if c == '\t' { "    ".to_string() } else { c.to_string() }).collect()
            };
            let mut at = 0;
            if let Some(spans) = painted.as_ref().and_then(|p| p.get(k)) {
                for s in spans {
                    if s.start > at {
                        runs.push((k, untab(at, s.start), None));
                    }
                    let token = match s.kind {
                        yumete_core::markdown::Kind::Token(t) => Some(t),
                        _ => None,
                    };
                    runs.push((k, untab(s.start, s.end), token));
                    at = at.max(s.end.min(chars.len()));
                }
            }
            if at < chars.len() {
                runs.push((k, untab(at, chars.len()), None));
            }
            if k + 1 < lines.len() {
                runs.push((k, "\n".to_string(), None));
            }
        }
        // Built by hand: code is one colour a token, no other markup.
        let mut text = String::new();
        let mut colours = Vec::new();
        let mut src = Vec::new();
        for (k, s, token) in &runs {
            if src.last().is_none_or(|l: &(usize, usize)| l.1 != first + k) {
                src.push((text.len(), first + k));
            }
            let a = text.len();
            text.push_str(s);
            if let Some(t) = token {
                colours.push((a..text.len(), self.theme.token(*t)));
            }
        }
        if text.is_empty() {
            text.push(' ');
            src.push((0, first));
        }
        CodeText { text, colours, src }
    }

    /// The style code is set in.
    pub(crate) fn code_style(&self, cx: &Ctx) -> Style {
        Style { size: cx.size * 0.85, weight: 400.0, color: self.theme.text, mono: true, line_height: 1.45, italic: false, anywhere: false }
    }

    /// **`text` laid out in the code face**, its tokens in their colours;
    /// not yet broken into lines.
    pub(crate) fn code_layout(&mut self, text: &str, colours: &[(std::ops::Range<usize>, Rgba)], st: Style) -> Layout<Ink> {
        let mut b = self.lcx.ranged_builder(&mut self.fonts, text, 1.0, true);
        let mono = self.families[self.han.index()].mono.clone();
        b.push_default(StyleProperty::FontFamily(FontFamily::from(mono.as_str())));
        b.push_default(StyleProperty::FontSize(st.size));
        b.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(st.line_height)));
        b.push_default(StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::Preserve));
        b.push_default(StyleProperty::Brush(Ink { color: st.color, ..Ink::default() }));
        for (r, c) in colours {
            b.push(StyleProperty::Brush(Ink { color: *c, ..Ink::default() }), r.clone());
        }
        b.build(text)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn code(&mut self, info: &str, lines: &[String], span: (usize, usize), x: f32, w: f32, y: &mut f32, out: &mut Out, cx: Ctx) {
        let px = self.px();
        let pad = 16.0 * px;
        let st = self.code_style(&cx);
        // The first line of code: under the opening fence, if there is one.
        let fenced = span.1 + 1 - span.0 > lines.len();
        let first = span.0 + usize::from(fenced);
        let CodeText { text, colours, src } = self.code_text(info, lines, first);
        let mut layout = self.code_layout(&text, &colours, st);
        layout.break_all_lines(Some((w - 2.0 * pad).max(1.0)));
        layout.align(Alignment::Start, AlignmentOptions::default());
        let h = layout.height() + 2.0 * pad;
        out.items.push(Item::Rect { r: rect(x, *y, w, h), fill: self.theme.code_block, radius: 6.0 * px });
        if fenced {
            out.marks.push((*y, span.0));
        }
        let t = Text { layout, attach: Vec::new(), src, string: text };
        self.put_text(t, x + pad, *y + pad, out, first);
        *y += h;
    }

    #[allow(clippy::too_many_arguments)]
    fn callout(
        &mut self,
        kind: Callout,
        name: &str,
        title: Option<&[Inline]>,
        fenced: bool,
        body: &[Block],
        span: (usize, usize),
        x: f32,
        w: f32,
        y: &mut f32,
        out: &mut Out,
        cx: Ctx,
    ) {
        let px = self.px();
        let colour = self.theme.callout(kind);
        let default_title = match fenced {
            true => match kind {
                Callout::Details => "Details".to_string(),
                _ => name.to_uppercase(),
            },
            false => {
                let mut c = name.to_lowercase().chars().collect::<Vec<char>>();
                if let Some(f) = c.first_mut() {
                    *f = f.to_ascii_uppercase();
                }
                c.into_iter().collect()
            }
        };
        let title_runs: Vec<Inline> = match title {
            Some(t) => t.to_vec(),
            None => vec![Inline { line: span.0, kind: InlineKind::Text { text: default_title, marks: Marks::default() } }],
        };
        let title_runs = match kind {
            Callout::Details => {
                let mut v = vec![Inline { line: span.0, kind: InlineKind::Text { text: "▾ ".to_string(), marks: Marks::default() } }];
                v.extend(title_runs);
                v
            }
            _ => title_runs,
        };
        let y0 = *y;
        if fenced {
            // VitePress: a tinted, rounded box; the title bold over the body.
            let pad = 16.0 * px;
            let ground = self.theme.callout_ground(kind);
            let at = out.items.len();
            *y += pad;
            let st = Style { size: cx.size * 0.95, weight: 600.0, color: if kind == Callout::Details { cx.color } else { colour }, mono: false, line_height: 1.6, italic: false, anywhere: false };
            let t = self.text(&title_runs, st, Some(w - 2.0 * pad), Alignment::Start);
            *y += self.put_text(t, x + pad, *y, out, span.0);
            if !body.is_empty() {
                *y += 8.0 * px;
                self.stack(body, x + pad, w - 2.0 * pad, y, out, Ctx { size: cx.size * 0.95, ..cx });
            }
            *y += pad;
            out.items.insert(at, Item::Rect { r: rect(x, y0, w, *y - y0), fill: ground, radius: 8.0 * px });
        } else {
            // GitHub: a bar in the alert's colour, the title in it.
            let bar = 0.25 * self.base();
            let pad = self.base();
            *y += 8.0 * px;
            let st = Style { size: cx.size, weight: 600.0, color: colour, mono: false, line_height: 1.6, italic: false, anywhere: false };
            let t = self.text(&title_runs, st, Some(w - bar - pad), Alignment::Start);
            *y += self.put_text(t, x + bar + pad, *y, out, span.0);
            if !body.is_empty() {
                *y += 8.0 * px;
                self.stack(body, x + bar + pad, w - bar - pad, y, out, cx);
            }
            *y += 8.0 * px;
            out.items.push(Item::Rect { r: rect(x, y0, bar, *y - y0), fill: colour, radius: 0.0 });
        }
    }

    /// **A table**, CSS's automatic layout simplified: each column as wide as
    /// its widest cell when the page has room; otherwise each gets its
    /// narrowest and the room left is shared by how much more each wants.
    ///
    /// **A short cell is never broken** (the 「城市」 that came out one
    /// character a line): a cell whose whole text is at most six ems wide
    /// counts that width as its narrowest, not one 漢字's. A table that still
    /// does not fit is set smaller, down to 0.65 of the body, its short cells
    /// still whole; and if its longest words do not fit even then, a word too
    /// long for its column breaks inside itself. **A table is never cut off at
    /// the page's edge**: a picture cannot be scrolled sideways.
    #[allow(clippy::too_many_arguments)]
    fn table(
        &mut self,
        align: &[Align],
        head: &[Vec<Inline>],
        rows: &[(usize, Vec<Vec<Inline>>)],
        first: usize,
        x: f32,
        w: f32,
        y: &mut f32,
        out: &mut Out,
        cx: Ctx,
    ) {
        let px = self.px();
        let ncol = align.len().max(head.len()).max(rows.iter().map(|r| r.1.len()).max().unwrap_or(0)).max(1);
        let empty: Vec<Inline> = Vec::new();
        // Smaller type first; then, at the smallest, words broken anywhere.
        let attempts = [(1.0f32, false), (0.875, false), (0.75, false), (0.65, false), (0.65, true)];
        let sum = |v: &[f32]| v.iter().sum::<f32>();
        for (attempt, &(k, anywhere)) in attempts.iter().enumerate() {
            let k = &k;
            let size = cx.size * k;
            // Smaller type, tighter cells.
            let pad_x = if *k >= 0.875 { 13.0 * px * k } else { 7.0 * px };
            let pad_y = 6.0 * px * k;
            let body = Style { size, anywhere, ..self.body(&cx) };
            let mut cells: Vec<Vec<Text>> = Vec::new();
            let mut min_w = vec![0f32; ncol];
            let mut keep_w = vec![0f32; ncol];
            let mut max_w = vec![0f32; ncol];
            let all_rows = std::iter::once(head).chain(rows.iter().map(|r| r.1.as_slice()));
            for (ri, row) in all_rows.enumerate() {
                let mut laid = Vec::with_capacity(ncol);
                for c in 0..ncol {
                    let runs = row.get(c).unwrap_or(&empty);
                    let st = match ri {
                        0 => Style { weight: 600.0, color: self.theme.table_head, ..body },
                        _ => body,
                    };
                    let t = self.text(runs, st, None, Alignment::Start);
                    let cw = t.layout.calculate_content_widths();
                    min_w[c] = min_w[c].max(cw.min);
                    max_w[c] = max_w[c].max(cw.max);
                    // A short cell is kept whole; a long one asks for four ems at least,
                    // so a column of sentences is not squeezed to its longest word.
                    keep_w[c] = keep_w[c].max(if cw.max <= 6.0 * size { cw.max } else { cw.min.max(4.0 * size) });
                    laid.push(t);
                }
                cells.push(laid);
            }
            let furniture = ncol as f32 * 2.0 * pad_x + (ncol + 1) as f32 * px;
            let room = w - furniture;
            let last = attempt + 1 == attempts.len();
            // Keep every short cell whole while type can still shrink; at the
            // smallest, break words only if the narrowest cells cannot fit.
            if !last && ((attempt + 2 < attempts.len() && sum(&keep_w) > room) || sum(&min_w) > room) {
                continue;
            }
            // From `low` towards `high`, the room there is shared by how much
            // more each column wants.
            let share = |low: &[f32], high: &[f32]| -> Vec<f32> {
                let extra = room - sum(low);
                let flex: f32 = high.iter().zip(low).map(|(a, b)| (a - b).max(0.0)).sum::<f32>().max(1e-3);
                low.iter().zip(high).map(|(lo, hi)| lo + extra * (hi - lo).max(0.0) / flex).collect()
            };
            let colw: Vec<f32> = if sum(&max_w) <= room {
                max_w.clone()
            } else if sum(&keep_w) <= room {
                share(&keep_w, &max_w)
            } else if sum(&min_w) <= room {
                share(&min_w, &keep_w)
            } else {
                // Not even a character a column fits: the columns share what
                // there is, in proportion, and the cells overflow their own
                // column rather than the page.
                min_w.iter().map(|m| m * room.max(0.0) / sum(&min_w).max(1e-3)).collect()
            };
            let table_w = sum(&colw) + furniture;
            let ty0 = *y;
            for (ri, row) in cells.iter_mut().enumerate() {
                let line = if ri == 0 { first } else { rows[ri - 1].0 };
                out.marks.push((*y, line));
                let mut rh = 0f32;
                for (c, t) in row.iter_mut().enumerate() {
                    let al = match align.get(c).copied().unwrap_or_default() {
                        Align::Center => Alignment::Center,
                        Align::Right => Alignment::End,
                        Align::Left => Alignment::Start,
                        Align::Plain => if ri == 0 { Alignment::Center } else { Alignment::Start },
                    };
                    t.layout.break_all_lines(Some(colw[c] + 0.5));
                    t.layout.align(al, AlignmentOptions { align_when_overflowing: false, ..AlignmentOptions::default() });
                    rh = rh.max(t.layout.height());
                }
                let rh = rh + 2.0 * pad_y;
                if ri > 0 && ri % 2 == 0 {
                    out.items.push(Item::Rect { r: rect(x, *y, table_w, rh), fill: self.theme.stripe, radius: 0.0 });
                }
                let mut cx_ = x + px;
                for (c, t) in std::mem::take(row).into_iter().enumerate() {
                    let ch = t.layout.height();
                    out.items.push(Item::Text { x: cx_ + pad_x, y: *y + (rh - ch) / 2.0, text: Box::new(t) });
                    cx_ += colw[c] + 2.0 * pad_x + px;
                }
                out.items.push(Item::Rect { r: rect(x, *y, table_w, px), fill: self.theme.border, radius: 0.0 });
                *y += rh;
            }
            out.items.push(Item::Rect { r: rect(x, *y, table_w, px), fill: self.theme.border, radius: 0.0 });
            *y += px;
            let mut cx_ = x;
            out.items.push(Item::Rect { r: rect(cx_, ty0, px, *y - ty0), fill: self.theme.border, radius: 0.0 });
            for cw in &colw {
                cx_ += cw + 2.0 * pad_x + px;
                out.items.push(Item::Rect { r: rect(cx_, ty0, px, *y - ty0), fill: self.theme.border, radius: 0.0 });
            }
            return;
        }
    }
}

/// **The brush of a run with `marks`** over writing in `color`, in `theme`;
/// `mono` when the whole text is set in the monospace face.
pub(crate) fn ink_of(theme: &Theme, marks: &Marks, color: Rgba, mono: bool) -> Ink {
    let mut ink = Ink { color, ..Ink::default() };
    // Bold and italic take the theme's colours in the writing only: a
    // heading or a quotation keeps its own.
    if color == theme.text {
        if let (true, Some(c)) = (marks.emph, theme.emphasis) {
            ink.color = c;
        }
        if let (true, Some(c)) = (marks.strong, theme.strong) {
            ink.color = c;
        }
    }
    if marks.code || marks.kbd {
        ink.ground = Some(theme.code_inline);
        if color == theme.text {
            ink.color = theme.code_text;
        }
    }
    if marks.link || marks.wiki {
        ink.color = theme.link;
    }
    ink.frame = marks.kbd;
    if marks.highlight {
        ink.ground = Some(theme.highlight);
    }
    ink.underline = marks.underline || marks.wiki;
    ink.strike = marks.strike;
    ink.mono = (marks.code || marks.kbd) && !mono;
    ink
}

/// **The weight, face and size `marks` give a run** in style `st`: bold
/// heavier, emphasis in italics (漢字 in a 楷體), code in the monospace face.
pub(crate) fn push_marks(b: &mut parley::RangedBuilder<'_, Ink>, r: std::ops::Range<usize>, marks: &Marks, st: Style, faces: &Families) {
    if marks.strong {
        b.push(StyleProperty::FontWeight(FontWeight::new(if st.weight >= 600.0 { 800.0 } else { 600.0 })), r.clone());
    }
    if marks.emph {
        // Latin in italics; 漢字, which have none, in a 楷體.
        b.push(StyleProperty::FontStyle(FontStyle::Italic), r.clone());
        if !(marks.code || marks.kbd || st.mono) {
            b.push(StyleProperty::FontFamily(FontFamily::from(faces.emphasis.as_str())), r.clone());
        }
    }
    if (marks.code || marks.kbd) && !st.mono {
        b.push(StyleProperty::FontFamily(FontFamily::from(faces.mono.as_str())), r.clone());
        b.push(StyleProperty::FontSize(st.size * 0.85), r.clone());
        b.push(StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::Preserve), r);
    }
}

/// A full-width closing mark whose right half is blank in `han`'s faces.
pub(crate) fn closing_mark(c: char, han: Han) -> bool {
    match c {
        '」' | '』' | '）' | '》' | '〉' | '】' | '〕' | '〗' | '］' | '｝' | '\u{201D}' | '\u{2019}' => true,
        '，' | '。' | '、' => han != Han::Traditional,
        // ！ and ？ keep their em, as the 開明 style keeps them: half-width
        // they read as Latin marks.
        '；' | '：' => han == Han::Simplified,
        _ => false,
    }
}

/// A full-width opening mark, whose left half is blank.
pub(crate) fn opening_mark(c: char) -> bool {
    matches!(c, '「' | '『' | '（' | '《' | '〈' | '【' | '〔' | '〖' | '［' | '｛' | '\u{201C}' | '\u{2018}')
}

/// Punctuation that Chinese and Latin share, drawn differently by each.
fn cjk_punctuation(c: char) -> bool {
    matches!(c, '\u{2018}' | '\u{2019}' | '\u{201C}' | '\u{201D}' | '\u{2026}' | '\u{2014}' | '\u{2015}' | '\u{00B7}' | '\u{22EF}')
}

/// What a block is laying into.
#[derive(Default)]
pub(crate) struct Out {
    pub items: Vec<Item>,
    pub marks: Vec<(f32, usize)>,
    /// A vertical page's columns: where each begins and its words.
    pub cols: Vec<(f32, String)>,
}

/// The baseline of the first text in `items`, from the block's top.
fn first_baseline(items: &[Item]) -> Option<f32> {
    items.iter().find_map(|it| match it {
        Item::Text { y, text, .. } => text.layout.first_baseline().map(|b| y + b),
        _ => None,
    })
}

/// How far right an item reaches.
fn item_right(item: &Item) -> f32 {
    match item {
        Item::Text { x, text, .. } => x + text.layout.width(),
        Item::Rect { r, .. } | Item::Frame { r, .. } | Item::Pic { r, .. } => r.x1 as f32,
        Item::Lines { points, .. } => points.iter().map(|p| p.0).fold(0.0, f32::max),
        Item::Turned { r, .. } => r.x1 as f32,
        Item::Glyphs(run) => run.bounds.x1 as f32,
    }
}

pub(crate) fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(x as f64, y as f64, (x + w) as f64, (y + h) as f64)
}

/// `%20` and its kind undone, for a file name written in a link.
fn percent_decoded(src: &str) -> String {
    yumete_core::markdown::percent_decoded(src)
}
