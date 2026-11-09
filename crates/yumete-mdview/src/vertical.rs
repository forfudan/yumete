//! **A vertical page** (2026-11-08): the preview set in 縱 from the right
//! edge, columns read right to left, the page going on to the left, when the
//! text window is vertical and `:preview-vertical` is on.
//!
//! The preview is publication, as `:export` is, so nothing here reads how
//! the draft is shown (`:render`, `[editor] tatechuyoko`): the rules are the
//! vertical export's ([`yumete_core::export`]).
//!
//! - **漢字 stand upright**, shaped top to bottom with harfrust in the face
//!   parley chose for them, which brings the font's own vertical forms and
//!   metrics; CJK punctuation is turned into its vertical form first
//!   ([`yumete_cjk::vertical_form`]), and in a Chinese text curly quotes
//!   become corner brackets, `“”` as `『』` and `‘’` as `「」`.
//! - **Half-width letters and digits** go as `[export] vertical_latin`
//!   says ([`yumete_core::export::latin_runs`]): turned a quarter turn
//!   clockwise, across in one square (縱中橫), or upright one to a square.
//! - **Columns break with 禁則**: no closing mark or stop at a column's head,
//!   no opening mark at its foot, a doubled dash or ellipsis never split; a
//!   stop that does not fit gives up its blank half, or takes the character
//!   before it on to the next column, or hangs below the column when
//!   `[export] hanging` says so. Every column of a paragraph but its last
//!   is justified to the full length.
//! - **Formulas** turn with the text (a display formula on a column of its
//!   own, two squares in from its head); **tables** are vertical, as a 年表 in
//!   史記 is: each source row a column, the head row on the right; a
//!   **figure's** picture stays upright beside its caption; **code** is
//!   turned whole, a block and a span alike, 漢字 with it, as the export
//!   turns it.
//!
//! A vertical block's coordinates: `x` from the block's right edge, so
//! negative; `y` from the page's top. Its marks and columns count how far
//! from that edge each begins.

use std::collections::HashMap;
use std::ops::Range;
use harfrust::{Direction, ShapeOptions, ShaperData, UnicodeBuffer};
use parley::{FontData, FontFamily, FontStyle, FontWeight, LineHeight, StyleProperty, WhiteSpaceCollapse};
use vello_cpu::kurbo::Affine;
use yumete_core::export::{Latin, Orientation, CODE_HANG, CODE_HOOK};

use crate::layout::{closing_mark, ink_of, opening_mark, push_marks, ranged_at, rect, Ctx, Engine, Ink, Item, Laid, Out, Pix, Ranged, Run, Style};
use crate::parse::{Align, Block, BlockKind, Callout, Inline, InlineKind, Marks};
use crate::theme::Rgba;

/// **How a vertical page sets what a vertical export decides**: `[export]
/// vertical_latin` and `[export] hanging`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vertical {
    pub latin: Latin,
    pub hanging: bool,
}

impl Vertical {
    /// What tells one apart from another in a cache key.
    pub(crate) fn key(&self) -> u8 {
        (self.latin == Latin::Jlreq) as u8 | (self.hanging as u8) << 1
    }
}

/// A character shaped down a column: its glyph, its advance down the
/// column and where it is drawn from the column's axis and the top of its
/// square, in ems.
#[derive(Debug, Clone, Copy)]
pub(crate) struct UpGlyph {
    gid: u32,
    adv: f32,
    dx: f32,
    dy: f32,
}

/// Column pitch, in ems of the type: a column and the gap beside it — the
/// line height `:export-as-preview` writes into a vertical page.
const PITCH: f32 = yumete_core::as_preview::COLUMN_PITCH;

// ── What stands in a column ─────────────────────────────────────────────────

/// How a unit breaks and squeezes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Cls {
    Normal,
    Open,
    Close,
    Stop,
    Middle,
    Bang,
    /// A dash or an ellipsis: two of them are never split.
    Pair,
    /// Small kana and the iteration marks: never at a column's head.
    Small,
    Space,
    Break,
}

fn class_of(c: char) -> Cls {
    use Cls::*;
    match c {
        '「' | '『' | '（' | '《' | '〈' | '【' | '〔' | '〖' | '［' | '｛' | '“' | '‘' | '〝' => Open,
        '」' | '』' | '）' | '》' | '〉' | '】' | '〕' | '〗' | '］' | '｝' | '”' | '’' | '〟' => Close,
        '。' | '，' | '、' | '．' => Stop,
        '：' | '；' | '・' => Middle,
        '！' | '？' | '‼' | '⁇' | '⁈' | '⁉' => Bang,
        '—' | '―' | '…' | '‥' | '⋯' => Pair,
        'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'っ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ' | 'ゕ' | 'ゖ' | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ッ' | 'ャ'
        | 'ュ' | 'ョ' | 'ヮ' | 'ヵ' | 'ヶ' | 'ー' | '々' | '〻' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ' => Small,
        _ => Normal,
    }
}

/// May not begin a column (行頭禁則).
fn no_start(c: Cls) -> bool {
    matches!(c, Cls::Close | Cls::Stop | Cls::Middle | Cls::Bang | Cls::Small)
}

/// May not end a column (行末禁則).
fn no_end(c: Cls) -> bool {
    c == Cls::Open
}

/// What a unit looks like.
#[derive(Clone)]
enum Look {
    /// A character standing upright: its glyph drawn from the axis and the
    /// top of its square.
    Up { font: FontData, coords: Vec<i16>, size: f32, gid: u32, dx: f32, dy: f32 },
    /// Glyphs set across and turned a quarter turn clockwise, their baseline
    /// `base` from the axis.
    Turn { font: FontData, coords: Vec<i16>, size: f32, glyphs: Vec<(u32, f32, f32)>, base: f32 },
    /// Glyphs set across in one square: `width` wide, squeezed into `room`,
    /// their baseline `base` below the square's top, their middle `shift`
    /// right of the axis.
    Across { font: FontData, coords: Vec<i16>, size: f32, glyphs: Vec<(u32, f32, f32)>, width: f32, room: f32, base: f32, shift: f32 },
    /// A picture: upright, or turned (a formula).
    Pic { pix: Pix, w: f32, h: f32, turned: bool },
    Gap,
    Break,
}

/// **One thing in a column**: how far it reaches down the column (`adv`)
/// and across it (`thick`), and what it is.
#[derive(Clone)]
struct Unit {
    look: Look,
    adv: f32,
    thick: f32,
    /// The em its squeezing is measured in.
    em: f32,
    cls: Cls,
    /// The character it shows, as written (corner brackets for quotes).
    ch: char,
    ink: Ink,
    /// Where it is in the text shown.
    text: Range<usize>,
    /// The ruby over it, if any.
    ruby: Option<u16>,
    /// No column may begin with it: it goes with the unit before.
    glue: bool,
    /// Its blank first or last half given up.
    lead: bool,
    tail: bool,
}

impl Unit {
    #[allow(clippy::too_many_arguments)]
    fn new(look: Look, adv: f32, thick: f32, em: f32, cls: Cls, ch: char, ink: Ink, text: Range<usize>) -> Unit {
        Unit { look, adv, thick, em, cls, ch, ink, text, ruby: None, glue: false, lead: false, tail: false }
    }

    /// How far it reaches down the column, squeezed.
    fn eff(&self) -> f32 {
        self.adv - if self.lead { self.em / 2.0 } else { 0.0 } - if self.tail { self.em / 2.0 } else { 0.0 }
    }
}

/// **A text read into units**, before it is broken into columns.
pub(crate) struct Units {
    units: Vec<Unit>,
    /// The text as shown, its quotes turned into corner brackets.
    shown: String,
    /// Where each source line's text begins (byte, line).
    src: Vec<(usize, usize)>,
    /// Each ruby's reading, read into units.
    readings: Vec<Vec<Unit>>,
    /// The type size.
    size: f32,
}

impl Units {
    /// How long the text is, unbroken.
    fn length(&self) -> f32 {
        self.units.iter().map(Unit::eff).sum()
    }
}

/// **A text broken into columns**: each column's units and where each
/// stands down it, from its top.
pub(crate) struct VText {
    cols: Vec<Vec<(Unit, f32)>>,
    shown: String,
    src: Vec<(usize, usize)>,
    readings: Vec<Vec<Unit>>,
    size: f32,
}

impl VText {
    /// How far down its longest column reaches.
    fn longest(&self) -> f32 {
        self.cols.iter().map(|c| c.last().map_or(0.0, |(u, y)| y + u.eff())).fold(0.0, f32::max)
    }
}

/// A half-width character: below U+1100, where the CJK blocks begin, and
/// neither a space nor a control.
fn half_width(c: char) -> bool {
    (c as u32) < 0x1100 && !c.is_whitespace() && !c.is_control()
}

/// What a character of a text is, for laying it down a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Seg {
    /// Upright in its square.
    Up,
    /// In run `n` of [`Units`]' runs.
    Run(usize),
    Gap,
    Break,
}

/// A run of characters set one way: how, and whether it is small (a
/// footnote's number, a superscript).
struct LatinRun {
    bytes: Range<usize>,
    how: Orientation,
    small: bool,
}

impl Engine {
    // ── Shaping down a column ───────────────────────────────────────────────

    /// **Shape `chars` down a column in `font`**, keeping each character's
    /// glyph by face so it is shaped once.
    fn shape_upright(&mut self, font: &FontData, chars: &[char]) {
        let (id, index) = (font.data.id(), font.index);
        let mut missing: Vec<char> = chars.iter().copied().filter(|c| !self.upright.contains_key(&(id, index, *c))).collect();
        missing.sort_unstable();
        missing.dedup();
        if missing.is_empty() {
            return;
        }
        let Ok(face) = harfrust::FontRef::from_index(font.data.data(), font.index) else {
            for c in missing {
                self.upright.insert((id, index, c), None);
            }
            return;
        };
        let data = self.shapers.entry((id, index)).or_insert_with(|| ShaperData::new(&face));
        let shaper = data.shaper(&face).build();
        let upem = shaper.units_per_em().max(1) as f32;
        let text: String = missing.iter().collect();
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(&text);
        buffer.set_direction(Direction::TopToBottom);
        buffer.set_script(harfrust::script::HAN);
        if let Ok(lang) = self.han.tag().parse::<harfrust::Language>() {
            buffer.set_language(lang);
        }
        let shaped = shaper.shape(buffer, ShapeOptions::new());
        let starts: Vec<usize> = text.char_indices().map(|(b, _)| b).collect();
        for (info, pos) in shaped.glyph_infos().iter().zip(shaped.glyph_positions()) {
            let Ok(k) = starts.binary_search(&(info.cluster as usize)) else { continue };
            let glyph = UpGlyph {
                gid: info.glyph_id,
                adv: -pos.y_advance as f32 / upem,
                dx: pos.x_offset as f32 / upem,
                dy: -pos.y_offset as f32 / upem,
            };
            // A character shaped into more than one glyph keeps its first.
            self.upright.entry((id, index, missing[k])).or_insert(Some(glyph));
        }
        for c in missing {
            self.upright.entry((id, index, c)).or_insert(None);
        }
    }

    // ── A text into units ───────────────────────────────────────────────────

    /// **Read `runs` into units** in style `st`; a Latin word longer than
    /// `len` is cut into pieces that fit.
    pub(crate) fn units(&mut self, runs: &[Inline], st: Style, len: f32) -> Units {
        let latin = self.vertical.map_or(Latin::Turn, |v| v.latin);
        let mut text = String::new();
        // Each range's marks, its own colour, and whether it is small.
        let mut ranges: Vec<Ranged> = Vec::new();
        let mut atoms: Vec<(usize, Look, f32, f32)> = Vec::new();
        let mut rubies: Vec<(Range<usize>, String)> = Vec::new();
        let mut src: Vec<(usize, usize)> = Vec::new();
        for run in runs {
            if src.last().is_none_or(|s| s.1 != run.line) {
                src.push((text.len(), run.line));
            }
            match &run.kind {
                InlineKind::Text { text: t, marks } => {
                    let a = text.len();
                    text.push_str(t);
                    ranges.push((a..text.len(), *marks, None, marks.sup || marks.sub));
                }
                InlineKind::Break => text.push('\n'),
                InlineKind::NoteRef(n) => {
                    let a = text.len();
                    text.push_str(&n.to_string());
                    ranges.push((a..text.len(), Marks::default(), Some(self.theme.footnote), true));
                }
                InlineKind::Formula { tex, display } => {
                    let size = st.size * 1.05;
                    // Under `jlreq` one letter or digit, or a numeric
                    // fraction, stands upright as the same letter would.
                    let upright = !display && latin == Latin::Jlreq && yumete_core::export::stands_upright(tex);
                    match self.formula(tex, *display, st.color, size) {
                        Some(f) => {
                            let pad = 0.1 * st.size;
                            // **Never longer than the column it is set in**
                            // (review, 2026-11-09): in a table's cell that is
                            // the band, and a turned formula wider than the
                            // band ran out of its cell; it is scaled down to
                            // fit, as a picture is.
                            let long = if upright { f.height } else { f.width };
                            let k = ((len - 2.0 * pad) / long.max(1.0)).clamp(0.05, 1.0);
                            let (w, h) = (f.width * k, f.height * k);
                            atoms.push(match upright {
                                true => (text.len(), Look::Pic { pix: f.pix, w, h, turned: false }, h + 2.0 * pad, w),
                                false => (text.len(), Look::Pic { pix: f.pix, w, h, turned: true }, w + 2.0 * pad, h),
                            });
                        }
                        None => {
                            let a = text.len();
                            text.push_str(tex);
                            ranges.push((a..text.len(), Marks { code: true, ..Marks::default() }, Some(self.theme.error), false));
                        }
                    }
                }
                InlineKind::Image { src: path, alt } => match self.image(path, (len * 0.5).max(1.0)) {
                    Some(pix) => {
                        let (w, h) = (pix.width() as f32, pix.height() as f32);
                        let k = (len / h).min(1.0);
                        atoms.push((text.len(), Look::Pic { pix, w: w * k, h: h * k, turned: false }, h * k, w * k));
                    }
                    None => {
                        let a = text.len();
                        text.push_str(if alt.is_empty() { path } else { alt });
                        ranges.push((a..text.len(), Marks { kbd: true, ..Marks::default() }, Some(self.theme.muted), false));
                    }
                },
                InlineKind::Ruby { base, reading, marks } => {
                    let a = text.len();
                    text.push_str(base);
                    ranges.push((a..text.len(), *marks, None, false));
                    rubies.push((a..text.len(), reading.clone()));
                }
            }
        }
        if src.is_empty() {
            src.push((0, 0));
        }
        // Found by halving, not by walking every stretch at every character.
        let code_at = |at: usize| ranged_at(&ranges, at).is_some_and(|(_, m, ..)| m.code || m.kbd);
        let small_at = |at: usize| ranged_at(&ranges, at).is_some_and(|(_, _, _, small)| *small);
        // Code is verbatim: turned whole, 漢字 and all (2026-11-08).
        let verbatim_at = |at: usize| st.mono || ranged_at(&ranges, at).is_some_and(|(_, m, ..)| m.code);

        // **The text as shown**: curly quotes as corner brackets in Chinese,
        // never in code. Each is three bytes, as its bracket is. Chinese is
        // what the export calls Chinese (2026-11-09): the same rule, not the
        // writing system the faces are chosen by.
        let typed: Vec<(usize, char)> = text.char_indices().collect();
        let quotes = self.quotes && !st.mono;
        let shown: String = match quotes {
            true => {
                let plain: Vec<char> = typed.iter().map(|&(at, c)| if code_at(at) { '\u{3000}' } else { c }).collect();
                let turned = yumete_core::export::corner_quotes(&plain);
                typed.iter().zip(turned).map(|(&(at, c), t)| if code_at(at) || t.len_utf8() != c.len_utf8() { c } else { t }).collect()
            }
            false => text.clone(),
        };
        let chars: Vec<(usize, char)> = shown.char_indices().collect();

        // **How each character stands.** Latin runs as `vertical_latin`
        // says, outside code and small type; code turned whole, whatever it
        // says, under either `vertical_latin`, as a turned Latin word is (it
        // is never set upright or 縱中橫); a key's half-width runs turned
        // whole; small type across, or turned when long; a lone half-width
        // mark turned or upright, as `text-orientation: mixed` sets it.
        let masked: Vec<char> = chars.iter().map(|&(at, c)| if code_at(at) || small_at(at) || st.mono { '\u{3000}' } else { c }).collect();
        let mut latin_runs: Vec<LatinRun> = Vec::new();
        let mut seg = vec![Seg::Up; chars.len()];
        let byte = |k: usize| chars.get(k).map_or(shown.len(), |c| c.0);
        for (r, how) in yumete_core::export::latin_runs(&masked, latin) {
            for s in &mut seg[r.clone()] {
                *s = Seg::Run(latin_runs.len());
            }
            latin_runs.push(LatinRun { bytes: byte(r.start)..byte(r.end), how, small: false });
        }
        let mut k = 0;
        while k < chars.len() {
            let (at, c) = chars[k];
            if seg[k] != Seg::Up {
                k += 1;
                continue;
            }
            if c == '\n' {
                seg[k] = Seg::Break;
                k += 1;
                continue;
            }
            let small = small_at(at);
            let code = code_at(at) || st.mono;
            if small || code {
                // A run of code, of half-width characters in a key (spaces
                // too), or of small type, set one way.
                let whole = small || verbatim_at(at);
                let joins = |c: char| if whole { c != '\n' } else { half_width(c) || c == ' ' };
                if !joins(c) {
                    k += 1;
                    continue;
                }
                let from = k;
                let same = |at: usize| small_at(at) == small && (code_at(at) || st.mono) == code && verbatim_at(at) == verbatim_at(chars[from].0);
                while k < chars.len() && seg[k] == Seg::Up && joins(chars[k].1) && same(chars[k].0) {
                    k += 1;
                }
                let n = k - from;
                let all_digits = chars[from..k].iter().all(|p| p.1.is_ascii_digit());
                let how = match small {
                    true if n <= 2 || (all_digits && n <= 3) => Orientation::Across,
                    _ => Orientation::Turned,
                };
                for s in &mut seg[from..k] {
                    *s = Seg::Run(latin_runs.len());
                }
                latin_runs.push(LatinRun { bytes: byte(from)..byte(k), how, small });
                continue;
            }
            // A mark UAX #50 stands upright (`×`, `±`) stands, as
            // `text-orientation: mixed` leaves it in the export (2026-11-09).
            // **A curly quote that stays curly is turned** (2026-11-09,
            // reported: `“go”` stood its quotes upright): UAX #50 gives it R,
            // and a browser turns it, as the export's page shows it.
            if c == ' ' || c == '\u{a0}' || c == '\t' {
                seg[k] = Seg::Gap;
            } else if (half_width(c) && !yumete_core::export::upright_mark(c)) || matches!(c, '“' | '”' | '‘' | '’') {
                seg[k] = Seg::Run(latin_runs.len());
                latin_runs.push(LatinRun { bytes: at..at + c.len_utf8(), how: Orientation::Turned, small: false });
            }
            k += 1;
        }

        // **What is drawn**: CJK punctuation in its vertical form, the same
        // number of bytes, so every range still holds.
        let drawn: String = chars
            .iter()
            .zip(&seg)
            .map(|(&(at, c), s)| match (s, code_at(at)) {
                (Seg::Up, false) => yumete_cjk::vertical_form(c).filter(|v| v.len_utf8() == c.len_utf8()).unwrap_or(c),
                _ => c,
            })
            .collect();

        // Shaped across by parley, which picks each character's face.
        let (doc, all) = self.faces();
        let fam_of = |lang: u8| if lang == 0 { doc.as_ref() } else { all[(lang as usize - 1).min(2)].as_ref() };
        let family = if st.mono { doc.mono.as_str() } else { doc.sans.as_str() };
        let theme = self.theme.clone();
        let mut b = self.lcx.ranged_builder(&mut self.fonts, &drawn, 1.0, true);
        b.push_default(StyleProperty::FontFamily(FontFamily::from(family)));
        b.push_default(StyleProperty::Locale(parley::Language::parse(self.han.tag()).ok()));
        b.push_default(StyleProperty::FontSize(st.size));
        b.push_default(StyleProperty::FontWeight(FontWeight::new(st.weight)));
        b.push_default(StyleProperty::LineHeight(LineHeight::FontSizeRelative(1.0)));
        b.push_default(StyleProperty::WhiteSpaceCollapse(WhiteSpaceCollapse::Preserve));
        if st.italic {
            b.push_default(StyleProperty::FontStyle(FontStyle::Italic));
        }
        b.push_default(StyleProperty::Brush(Ink { color: st.color, ..Ink::default() }));
        for (r, marks, color, small) in &ranges {
            let mut ink = ink_of(&theme, marks, st.color, st.mono);
            if let Some(c) = color {
                ink.color = *c;
            }
            push_marks(&mut b, r.clone(), marks, st, fam_of(marks.lang));
            if *small {
                b.push(StyleProperty::FontSize(st.size * 0.6), r.clone());
            }
            if ink != (Ink { color: st.color, ..Ink::default() }) {
                b.push(StyleProperty::Brush(ink), r.clone());
            }
        }
        let mut layout = b.build(&drawn);
        layout.break_all_lines(None);

        // Every cluster: where it is, its face, its glyphs across.
        struct Cl {
            at: Range<usize>,
            font: FontData,
            coords: Vec<i16>,
            size: f32,
            ink: Ink,
            glyphs: Vec<(u32, f32, f32, f32)>,
            adv: f32,
            asc: f32,
            desc: f32,
            cap: f32,
        }
        let mut cls: Vec<Cl> = Vec::new();
        for line in layout.lines() {
            for run in line.runs() {
                let font = run.font().clone();
                let size = run.font_size();
                let m = run.font_metrics();
                let coords: Vec<i16> = run.normalized_coords().iter().map(|c| c.to_bits()).collect();
                for c in run.clusters() {
                    cls.push(Cl {
                        at: c.text_range(),
                        font: font.clone(),
                        coords: coords.clone(),
                        size,
                        ink: c.style().brush,
                        glyphs: c.glyphs().map(|g| (g.id, g.x, g.y, g.advance)).collect(),
                        adv: c.advance(),
                        asc: m.ascent,
                        desc: m.descent,
                        cap: m.cap_height.unwrap_or(0.7 * size),
                    });
                }
            }
        }
        cls.sort_by_key(|c| c.at.start);
        // The upright characters, shaped down a column a face at a time.
        let mut wanted: HashMap<(u64, u32), (FontData, Vec<char>)> = HashMap::new();
        for c in &cls {
            let k = chars.partition_point(|p| p.0 < c.at.start);
            if seg.get(k) == Some(&Seg::Up) {
                if let Some(ch) = drawn[c.at.clone()].chars().next() {
                    wanted.entry((c.font.data.id(), c.font.index)).or_insert_with(|| (c.font.clone(), Vec::new())).1.push(ch);
                }
            }
        }
        for (_, (font, chars)) in wanted {
            self.shape_upright(&font, &chars);
        }

        // **The units**, in the order of the text.
        let mut units: Vec<Unit> = Vec::new();
        let mut atoms = atoms.into_iter().peekable();
        let seg_of = |at: usize| seg.get(chars.partition_point(|p| p.0 < at)).copied().unwrap_or(Seg::Up);
        let char_at = |at: usize| shown[at..].chars().next().unwrap_or(' ');
        let mut k = 0;
        while k < cls.len() {
            while let Some((_, look, adv, thick)) = atoms.next_if(|a| a.0 <= cls[k].at.start) {
                let at = cls[k].at.start;
                units.push(Unit::new(look, adv, thick, st.size, Cls::Normal, '\u{fffc}', Ink { color: st.color, ..Ink::default() }, at..at));
            }
            let c = &cls[k];
            match seg_of(c.at.start) {
                Seg::Break => {
                    units.push(Unit::new(Look::Break, 0.0, 0.0, st.size, Cls::Break, '\n', c.ink, c.at.clone()));
                    k += 1;
                }
                Seg::Gap => {
                    units.push(Unit::new(Look::Gap, c.adv, 0.0, st.size, Cls::Space, ' ', c.ink, c.at.clone()));
                    k += 1;
                }
                Seg::Up => {
                    let ch = char_at(c.at.start);
                    let drawn_ch = drawn[c.at.clone()].chars().next().unwrap_or(' ');
                    let single = drawn[c.at.clone()].chars().count() == 1;
                    let glyph = self.upright.get(&(c.font.data.id(), c.font.index, drawn_ch)).copied().flatten().filter(|_| single);
                    let look = match (glyph, c.glyphs.first()) {
                        (Some(g), _) => Look::Up { font: c.font.clone(), coords: c.coords.clone(), size: c.size, gid: g.gid, dx: g.dx * c.size, dy: g.dy * c.size },
                        // Not shaped down: the glyph across, centred in its square.
                        (None, Some(g)) => Look::Up {
                            font: c.font.clone(),
                            coords: c.coords.clone(),
                            size: c.size,
                            gid: g.0,
                            dx: -c.adv / 2.0 + g.1,
                            dy: (c.size + c.cap) / 2.0 + g.2,
                        },
                        (None, None) => Look::Gap,
                    };
                    let adv = glyph.map_or(c.size, |g| g.adv * c.size);
                    units.push(Unit::new(look, adv, c.size, c.size, class_of(ch), ch, c.ink, c.at.clone()));
                    k += 1;
                }
                Seg::Run(r) => {
                    let end = latin_runs[r].bytes.end;
                    let mut j = k;
                    while j < cls.len() && cls[j].at.start < end {
                        j += 1;
                    }
                    let run = &latin_runs[r];
                    let group = &cls[k..j];
                    match run.how {
                        Orientation::Turned => {
                            // Words, cut at spaces and where the face changes.
                            let mut pieces: Vec<(Range<usize>, bool)> = Vec::new();
                            let mut from = 0;
                            for (n, c) in group.iter().enumerate() {
                                let space = shown[c.at.clone()].chars().all(char::is_whitespace);
                                if space {
                                    if from < n {
                                        pieces.push((from..n, false));
                                    }
                                    pieces.push((n..n + 1, true));
                                    from = n + 1;
                                } else if n > from && (group[n - 1].font != c.font || group[n - 1].size != c.size) {
                                    pieces.push((from..n, false));
                                    from = n;
                                }
                            }
                            if from < group.len() {
                                pieces.push((from..group.len(), false));
                            }
                            let mut joined = false;
                            for (piece, space) in pieces {
                                let first = &group[piece.start];
                                if space {
                                    units.push(Unit::new(Look::Gap, first.adv, 0.0, st.size, Cls::Space, ' ', first.ink, first.at.clone()));
                                    joined = false;
                                    continue;
                                }
                                // At most a column long: a longer word is cut.
                                let mut glyphs = Vec::new();
                                let mut pen = 0.0f32;
                                let mut start = piece.start;
                                let flush = |units: &mut Vec<Unit>, glyphs: &mut Vec<(u32, f32, f32)>, pen: &mut f32, a: usize, b: usize, glue: bool| {
                                    let c0 = &group[a];
                                    let base = if run.small { 0.15 * st.size } else { 0.0 } - (c0.asc - c0.desc) / 2.0;
                                    let look = Look::Turn { font: c0.font.clone(), coords: c0.coords.clone(), size: c0.size, glyphs: std::mem::take(glyphs), base };
                                    let ch = shown[c0.at.clone()].chars().next().unwrap_or('a');
                                    // One mark turned on its own breaks as it would
                                    // upright: a turned `“` ends no column.
                                    let cls = match b - a {
                                        1 => class_of(ch),
                                        _ => Cls::Normal,
                                    };
                                    let mut u = Unit::new(look, *pen, c0.asc + c0.desc, st.size, cls, ch, c0.ink, c0.at.start..group[b - 1].at.end);
                                    u.glue = glue;
                                    units.push(u);
                                    *pen = 0.0;
                                };
                                for n in piece.clone() {
                                    let c = &group[n];
                                    if pen > 0.0 && pen + c.adv > len {
                                        flush(&mut units, &mut glyphs, &mut pen, start, n, joined);
                                        joined = false;
                                        start = n;
                                    }
                                    for g in &c.glyphs {
                                        glyphs.push((g.0, pen + g.1, g.2));
                                        pen += g.3;
                                    }
                                    if c.glyphs.is_empty() {
                                        pen += c.adv;
                                    }
                                }
                                flush(&mut units, &mut glyphs, &mut pen, start, piece.end, joined);
                                joined = true;
                            }
                        }
                        Orientation::Across => {
                            let c0 = &group[0];
                            let mut glyphs = Vec::new();
                            let mut pen = 0.0f32;
                            for c in group {
                                for g in &c.glyphs {
                                    glyphs.push((g.0, pen + g.1, g.2));
                                    pen += g.3;
                                }
                            }
                            let (adv, room, base, shift) = match run.small {
                                true => (c0.size * 1.05, 0.95 * st.size, c0.cap + 0.15 * c0.size, 0.3 * st.size),
                                false => (st.size, st.size * 1.05, (st.size + c0.cap) / 2.0, 0.0),
                            };
                            let look = Look::Across { font: c0.font.clone(), coords: c0.coords.clone(), size: c0.size, glyphs, width: pen, room, base, shift };
                            let ch = char_at(c0.at.start);
                            units.push(Unit::new(look, adv, st.size, st.size, Cls::Normal, ch, c0.ink, c0.at.start..group[group.len() - 1].at.end));
                            // A footnote's number goes with what it follows.
                            if run.small {
                                if let Some(u) = units.last_mut() {
                                    u.glue = true;
                                    u.thick = 0.0;
                                }
                            }
                        }
                        Orientation::Upright => {
                            for c in group {
                                let glyphs: Vec<(u32, f32, f32)> = c.glyphs.iter().scan(0.0f32, |pen, g| {
                                    let at = (g.0, *pen + g.1, g.2);
                                    *pen += g.3;
                                    Some(at)
                                }).collect();
                                let look = Look::Across {
                                    font: c.font.clone(),
                                    coords: c.coords.clone(),
                                    size: c.size,
                                    glyphs,
                                    width: c.adv,
                                    room: st.size,
                                    base: (st.size + c.cap) / 2.0,
                                    shift: 0.0,
                                };
                                units.push(Unit::new(look, st.size, st.size, st.size, Cls::Normal, char_at(c.at.start), c.ink, c.at.clone()));
                            }
                        }
                    }
                    k = j;
                }
            }
        }
        for (_, look, adv, thick) in atoms {
            let at = shown.len();
            units.push(Unit::new(look, adv, thick, st.size, Cls::Normal, '\u{fffc}', Ink { color: st.color, ..Ink::default() }, at..at));
        }

        // **Ruby**: the reading read into units of half the size; a reading
        // longer than its base spaces the base out to its length.
        //
        // **Each unit's group is found by halving, and the pads are put in in
        // one pass**, not by walking every unit for every group and inserting
        // into the middle of them, which a paragraph with a reading on each
        // of its thousands of lines would make quadratic. The groups are in
        // order and never overlap, as the text they cover was pushed.
        let mut readings = Vec::new();
        let mut inside: Vec<Vec<usize>> = vec![Vec::new(); rubies.len()];
        for (n, u) in units.iter().enumerate() {
            let k = rubies.partition_point(|(r, _)| r.end <= u.text.start);
            if rubies.get(k).is_some_and(|(r, _)| r.contains(&u.text.start)) {
                inside[k].push(n);
            }
        }
        // The pads to go before and after a unit.
        let mut before: HashMap<usize, Unit> = HashMap::new();
        let mut after: HashMap<usize, Unit> = HashMap::new();
        for (k, (r, reading)) in rubies.into_iter().enumerate() {
            let id = readings.len() as u16;
            let small = Style { size: st.size * 0.5, weight: 400.0, color: st.color, ..st };
            let run = [Inline { line: 0, kind: InlineKind::Text { text: reading, marks: Marks::default() } }];
            let read = self.units(&run, small, f32::MAX);
            let length = read.length();
            readings.push(read.units);
            let inside = &inside[k];
            let (Some(&a), Some(&z)) = (inside.first(), inside.last()) else { continue };
            for &n in inside {
                units[n].ruby = Some(id);
                units[n].glue = n != a;
            }
            let base: f32 = inside.iter().map(|&n| units[n].adv).sum();
            if length > base + 0.5 {
                let half = (length - base) / 2.0;
                let pad = |glue: bool| {
                    let mut u = Unit::new(Look::Gap, half, 0.0, st.size, Cls::Normal, '\u{200b}', Ink::default(), r.start..r.start);
                    u.ruby = Some(id);
                    u.glue = glue;
                    u
                };
                after.insert(z, pad(true));
                units[a].glue = true;
                before.insert(a, pad(false));
            }
        }
        if !before.is_empty() {
            let mut padded = Vec::with_capacity(units.len() + before.len() + after.len());
            for (n, u) in units.into_iter().enumerate() {
                padded.extend(before.remove(&n));
                padded.push(u);
                padded.extend(after.remove(&n));
            }
            units = padded;
        }

        // **Two full-width marks side by side give up the blank half between
        // them**, as across the page (see `layout.rs`): a closing mark's
        // blank half is its lower one down a column, an opening mark's its
        // upper.
        if self.squeeze && !st.mono {
            for n in 1..units.len() {
                let (a, b) = (units[n - 1].ch, units[n].ch);
                let both_up = matches!(units[n - 1].look, Look::Up { .. }) && matches!(units[n].look, Look::Up { .. });
                if !both_up {
                    continue;
                }
                if closing_mark(a, self.han) && (closing_mark(b, self.han) || opening_mark(b)) {
                    units[n - 1].tail = true;
                } else if opening_mark(a) && opening_mark(b) {
                    units[n].lead = true;
                }
            }
        }
        Units { units, shown, src, readings, size: st.size }
    }

    /// **`runs` in columns `len` long**: the first begins `first` down,
    /// the others `rest` down.
    /// Every column but the last is justified when `justify` says so: a
    /// paragraph's are, a heading's and a caption's are not.
    pub(crate) fn vtext(&mut self, runs: &[Inline], st: Style, len: f32, first: f32, rest: f32, justify: bool) -> VText {
        let units = self.units(runs, st, (len - rest.max(first)).max(st.size));
        let hanging = self.vertical.is_some_and(|v| v.hanging);
        columns(units, len, first, rest, hanging, justify)
    }

    // ── Units onto the page ─────────────────────────────────────────────────

    /// **Put `vt` on the page**, its first column's right edge at `right`,
    /// its columns' tops at `top`, a column `pitch` apart at least; returns
    /// how wide it is. `line` is the source line of a text with none.
    pub(crate) fn put(&self, vt: &VText, right: f32, top: f32, pitch: f32, out: &mut Out, line: usize) -> f32 {
        let mut x = right;
        let size = vt.size;
        for col in &vt.cols {
            let thick = col.iter().map(|(u, _)| u.thick).fold(size, f32::max);
            let p = pitch.max(thick + (pitch - size));
            let axis = x - p / 2.0;
            let first = col.iter().find(|(u, _)| !u.text.is_empty()).map_or(vt.shown.len(), |(u, _)| u.text.start);
            let src_line = {
                let k = vt.src.partition_point(|s| s.0 <= first).saturating_sub(1);
                vt.src.get(k).map_or(line, |s| s.1)
            };
            out.marks.push((-x, src_line));
            let words: String = col.iter().map(|(u, _)| &vt.shown[u.text.clone()]).collect();
            out.cols.push((-x, words));
            self.emit(col, axis, top, out);
            // Ruby: each reading centred on its base, right of the column.
            let mut seen: Vec<u16> = Vec::new();
            for (u, _) in col {
                let Some(id) = u.ruby else { continue };
                if seen.contains(&id) {
                    continue;
                }
                seen.push(id);
                let base: Vec<&(Unit, f32)> = col.iter().filter(|(v, _)| v.ruby == Some(id) && !matches!(v.look, Look::Gap)).collect();
                let (Some(a), Some(z)) = (base.first(), base.last()) else { continue };
                let (y0, y1) = (a.1, z.1 + z.0.eff());
                let Some(reading) = vt.readings.get(id as usize) else { continue };
                let length: f32 = reading.iter().map(Unit::eff).sum();
                let mut y = (y0 + y1) / 2.0 - length / 2.0;
                let placed: Vec<(Unit, f32)> = reading
                    .iter()
                    .map(|r| {
                        let at = y;
                        y += r.eff();
                        (r.clone(), at)
                    })
                    .collect();
                // Clear of the base's square by a tenth of it.
                let r_thick = reading.iter().map(|r| r.thick).fold(0.0, f32::max);
                self.emit(&placed, axis + 0.6 * size + r_thick / 2.0, top, out);
            }
            x -= p;
        }
        right - x
    }

    /// **Draw `units`** down the column whose axis is `axis`, from `top`.
    fn emit(&self, units: &[(Unit, f32)], axis: f32, top: f32, out: &mut Out) {
        let px = self.scale;
        // Grounds first: code, highlights, keys.
        let mut n = 0;
        while n < units.len() {
            let Some(ground) = units[n].0.ink.ground else {
                n += 1;
                continue;
            };
            let mut m = n;
            while m + 1 < units.len() && units[m + 1].0.ink.ground == Some(ground) {
                m += 1;
            }
            let size = units[n].0.em;
            let (y0, y1) = (top + units[n].1 - 0.15 * size, top + units[m].1 + units[m].0.eff() + 0.15 * size);
            out.items.push(Item::Rect { r: rect(axis - 0.6 * size, y0, 1.2 * size, y1 - y0), fill: ground, radius: 0.25 * size });
            n = m + 1;
        }
        // Upright characters, one run a face and colour.
        let mut batch: Option<Run> = None;
        let flush = |batch: &mut Option<Run>, out: &mut Out| {
            if let Some(run) = batch.take() {
                out.items.push(Item::Glyphs(Box::new(run)));
            }
        };
        for (u, y) in units {
            let y = top + y;
            match &u.look {
                Look::Up { font, coords, size, gid, dx, dy } => {
                    let lift = if u.lead { u.em / 2.0 } else { 0.0 };
                    let glyph = vello_cpu::Glyph { id: *gid, x: axis + dx, y: y + dy - lift };
                    let cell = rect(axis - size / 2.0, y - lift, *size, u.adv);
                    let same = batch.as_ref().is_some_and(|b| b.font == *font && b.size == *size && b.color == u.ink.color && b.coords == *coords);
                    if !same {
                        flush(&mut batch, out);
                        batch = Some(Run { font: font.clone(), size: *size, coords: coords.clone(), color: u.ink.color, transform: Affine::IDENTITY, glyphs: Vec::new(), bounds: cell });
                    }
                    if let Some(b) = batch.as_mut() {
                        b.glyphs.push(glyph);
                        b.bounds = b.bounds.union(cell);
                    }
                }
                Look::Turn { font, coords, size, glyphs, base } => {
                    flush(&mut batch, out);
                    let transform = Affine::translate(((axis + base) as f64, y as f64)) * Affine::rotate(std::f64::consts::FRAC_PI_2);
                    let glyphs = glyphs.iter().map(|&(id, x, gy)| vello_cpu::Glyph { id, x, y: gy }).collect();
                    let bounds = rect(axis - u.thick / 2.0 - size * 0.2, y, u.thick + size * 0.4, u.adv);
                    out.items.push(Item::Glyphs(Box::new(Run { font: font.clone(), size: *size, coords: coords.clone(), color: u.ink.color, transform, glyphs, bounds })));
                }
                Look::Across { font, coords, size, glyphs, width, room, base, shift } => {
                    flush(&mut batch, out);
                    let sx = if *width > *room { room / width } else { 1.0 };
                    let x0 = axis + shift - width * sx / 2.0;
                    let transform = Affine::translate((x0 as f64, (y + base) as f64)) * Affine::scale_non_uniform(sx as f64, 1.0);
                    let glyphs = glyphs.iter().map(|&(id, x, gy)| vello_cpu::Glyph { id, x, y: gy }).collect();
                    let bounds = rect(x0 - size * 0.2, y, width * sx + size * 0.4, u.adv.max(*size));
                    out.items.push(Item::Glyphs(Box::new(Run { font: font.clone(), size: *size, coords: coords.clone(), color: u.ink.color, transform, glyphs, bounds })));
                }
                Look::Pic { pix, w, h, turned } => {
                    flush(&mut batch, out);
                    let pad = (u.adv - if *turned { *w } else { *h }).max(0.0) / 2.0;
                    match turned {
                        true => out.items.push(Item::Turned { r: rect((axis - h / 2.0).round(), (y + pad).round(), *h, *w), pix: pix.clone() }),
                        false => out.items.push(Item::Pic { r: rect((axis - w / 2.0).round(), (y + pad).round(), *w, *h), pix: pix.clone() }),
                    }
                }
                Look::Gap | Look::Break => {}
            }
        }
        flush(&mut batch, out);
        // Lines along the column: an underline (a link) on its left, as
        // 專名號 go; a strike down its middle.
        for (want, side) in [(0u8, -0.62f32), (1, 0.0)] {
            let mut n = 0;
            while n < units.len() {
                let on = |u: &Unit| if want == 0 { u.ink.underline } else { u.ink.strike };
                if !on(&units[n].0) {
                    n += 1;
                    continue;
                }
                let mut m = n;
                while m + 1 < units.len() && on(&units[m + 1].0) {
                    m += 1;
                }
                let size = units[n].0.em;
                let t = (0.06 * size).max(px);
                let (y0, y1) = (top + units[n].1, top + units[m].1 + units[m].0.eff());
                out.items.push(Item::Rect { r: rect(axis + side * size - t / 2.0, y0, t, y1 - y0), fill: units[n].0.ink.color, radius: 0.0 });
                n = m + 1;
            }
        }
    }
}

// ── Columns ─────────────────────────────────────────────────────────────────

/// Whether a column may break between units `k - 1` and `k`.
fn breakable(units: &[Unit], k: usize) -> bool {
    let (a, b) = (&units[k - 1], &units[k]);
    if b.glue || b.cls == Cls::Space || no_start(b.cls) || no_end(a.cls) {
        return false;
    }
    !(a.cls == Cls::Pair && b.cls == Cls::Pair && a.ch == b.ch)
}

/// **Break `units` into columns `len` long**, with 禁則, then justify them.
fn columns(units: Units, len: f32, first: f32, rest: f32, hanging: bool, justify: bool) -> VText {
    let Units { mut units, shown, src, readings, size } = units;
    // Each column: its units' indices and tops, and whether a break the
    // writer asked for ends it.
    let mut cols: Vec<(Vec<(usize, f32)>, bool)> = vec![(Vec::new(), false)];
    let mut y = first;
    let mut i = 0;
    let slack = 0.5;
    while i < units.len() {
        let u = &units[i];
        if u.cls == Cls::Break {
            if let Some(last) = cols.last_mut() {
                last.1 = true;
            }
            cols.push((Vec::new(), false));
            y = rest;
            i += 1;
            continue;
        }
        let empty = cols.last().is_none_or(|c| c.0.is_empty());
        // No space at a column's head.
        if u.cls == Cls::Space && empty {
            i += 1;
            continue;
        }
        let eff = u.eff();
        if y + eff <= len + slack || empty {
            if let Some(c) = cols.last_mut() {
                c.0.push((i, y));
            }
            y += eff;
            i += 1;
            continue;
        }
        // It does not fit.
        if u.cls == Cls::Space {
            cols.push((Vec::new(), false));
            y = rest;
            i += 1;
            continue;
        }
        if no_start(u.cls) {
            // A stop hangs below the column when the page says so; else a
            // mark with a blank half gives it up to stay (追い込み).
            let hangs = hanging && u.cls == Cls::Stop;
            let squeezes = matches!(u.cls, Cls::Close | Cls::Stop) && matches!(u.look, Look::Up { .. }) && !u.tail && y + eff - u.em / 2.0 <= len + slack;
            if hangs || squeezes {
                if squeezes && !hangs {
                    units[i].tail = true;
                }
                if let Some(c) = cols.last_mut() {
                    c.0.push((i, y));
                }
                cols.push((Vec::new(), false));
                y = rest;
                i += 1;
                continue;
            }
        }
        // Back to where a column may break, taking what follows it on to
        // the next column (追い出し).
        let Some(head) = cols.last().and_then(|c| c.0.first()).map(|p| p.0) else { break };
        let mut k = i;
        while k > head && !breakable(&units, k) {
            k -= 1;
        }
        if k == head {
            k = i;
        }
        let moved: Vec<usize> = match cols.last_mut() {
            Some(c) => {
                let at = c.0.iter().position(|p| p.0 >= k).unwrap_or(c.0.len());
                c.0.drain(at..).map(|p| p.0).collect()
            }
            None => Vec::new(),
        };
        cols.push((Vec::new(), false));
        y = rest;
        for n in moved {
            if units[n].cls == Cls::Space && cols.last().is_none_or(|c| c.0.is_empty()) {
                continue;
            }
            if let Some(c) = cols.last_mut() {
                c.0.push((n, y));
            }
            y += units[n].eff();
        }
    }
    // A space at a column's foot is not drawn.
    for (col, _) in &mut cols {
        while col.last().is_some_and(|p| units[p.0].cls == Cls::Space) {
            col.pop();
        }
    }
    let count = cols.len();
    let mut out: Vec<Vec<(Unit, f32)>> = Vec::with_capacity(count);
    for (k, (col, hard)) in cols.into_iter().enumerate() {
        let mut col: Vec<(Unit, f32)> = col.into_iter().map(|(n, y)| (units[n].clone(), y)).collect();
        if justify && k + 1 < count && !hard {
            justify_column(&mut col, len);
        }
        out.push(col);
    }
    while out.len() > 1 && out.last().is_some_and(|c| c.is_empty()) {
        out.pop();
    }
    VText { cols: out, shown, src, readings, size }
}

/// **兩端對齊 down a column**: it ends flush at `len`. The room goes first a
/// quarter square beside punctuation and between 漢字 and a Latin word, and
/// what is left evenly between all (clreq §3.6). A column more than a third
/// short (a long word went on to the next) is left as it is.
fn justify_column(col: &mut [(Unit, f32)], len: f32) {
    // A mark hung below the column stays hung.
    let hung = col.last().is_some_and(|(u, y)| y + u.eff() > len + 0.5);
    let body = if hung { col.len() - 1 } else { col.len() };
    if body < 2 {
        return;
    }
    let end = col[body - 1].1 + col[body - 1].0.eff();
    let mut room = len - end;
    if room <= 0.5 || room > len / 3.0 {
        return;
    }
    let gaps = body - 1;
    let mut extra = vec![0.0f32; gaps];
    for (g, e) in extra.iter_mut().enumerate() {
        if room <= 0.0 {
            break;
        }
        let (a, b) = (&col[g].0, &col[g + 1].0);
        let punct = matches!(a.cls, Cls::Stop | Cls::Close | Cls::Middle | Cls::Bang) || b.cls == Cls::Open;
        let mixed = matches!(a.look, Look::Up { .. }) != matches!(b.look, Look::Up { .. });
        if (punct || mixed) && !b.glue {
            let take = (a.em * 0.25).min(room);
            *e += take;
            room -= take;
        }
    }
    if room > 0.0 {
        let free = (0..gaps).filter(|&g| !col[g + 1].0.glue).count().max(1);
        for (g, e) in extra.iter_mut().enumerate() {
            if !col[g + 1].0.glue {
                *e += room / free as f32;
            }
        }
    }
    let mut shift = 0.0;
    for k in 1..col.len() {
        if k - 1 < gaps {
            shift += extra[k - 1];
        }
        col[k].1 += shift;
    }
}

// ── Blocks ──────────────────────────────────────────────────────────────────

/// **The page a vertical block is laid on**: where the columns begin, how
/// long they are, and how wide the window is (what is set across, code and
/// pictures, is kept to a part of it).
#[derive(Clone, Copy)]
pub(crate) struct Page {
    pub top: f32,
    pub len: f32,
    pub width: f32,
}

/// The caption of a table, if `block` is one: Pandoc's `Table: …` or `: …`
/// paragraph, its runs with that taken off.
pub(crate) fn caption_of(block: &Block) -> Option<Vec<Inline>> {
    let BlockKind::Paragraph(runs) = &block.kind else { return None };
    let first = runs.first()?;
    let InlineKind::Text { text, marks } = &first.kind else { return None };
    let rest = text.strip_prefix("Table:").or_else(|| text.strip_prefix(':').filter(|_| text.starts_with(": ")))?;
    let mut out = runs.clone();
    out[0] = Inline { line: first.line, kind: InlineKind::Text { text: rest.trim_start().to_string(), marks: *marks } };
    Some(out)
}

/// A paragraph that is one picture and nothing else: its source and alt.
pub(crate) fn figure_of(runs: &[Inline]) -> Option<(&str, &str)> {
    let mut found = None;
    for run in runs {
        match &run.kind {
            InlineKind::Image { src, alt } if found.is_none() => found = Some((src.as_str(), alt.as_str())),
            InlineKind::Text { text, .. } if text.trim().is_empty() => {}
            InlineKind::Break => {}
            _ => return None,
        }
    }
    found
}

/// Move what `items` draw by (`dx`, `dy`).
fn shift(items: &mut [Item], dx: f32, dy: f32) {
    let v = vello_cpu::kurbo::Vec2::new(dx as f64, dy as f64);
    for item in items {
        match item {
            Item::Text { x, y, .. } => {
                *x += dx;
                *y += dy;
            }
            Item::Rect { r, .. } | Item::Frame { r, .. } | Item::Pic { r, .. } | Item::Turned { r, .. } => *r = *r + v,
            Item::Lines { points, .. } => points.iter_mut().for_each(|p| {
                p.0 += dx;
                p.1 += dy;
            }),
            Item::Glyphs(run) => {
                run.transform = Affine::translate(v) * run.transform;
                run.bounds = run.bounds + v;
            }
        }
    }
}

impl Engine {
    /// **Lay a top-level block out down the page**, with the caption a
    /// table takes.
    pub(crate) fn vtop(&mut self, block: &Block, caption: Option<&[Inline]>, page: Page) -> Laid {
        let cx = Ctx { color: self.theme.text, depth: 0, tight: false, size: self.base() };
        let mut out = Out::default();
        let mut u = 0.0;
        match (&block.kind, caption) {
            (BlockKind::Table { align, head, rows }, caption) => self.vtable(align, head, rows, block.lines.0, caption, &mut u, page.top, page.len, &mut out, cx, page),
            _ => self.vblock(block, &mut u, page.top, page.len, &mut out, cx, page),
        }
        let (top, bottom) = self.vmargins(block, cx);
        Laid { items: out.items, height: u, top, bottom, marks: out.marks, right: 0.0, cols: out.cols }
    }

    /// The room a block asks for before and after it, along the page.
    fn vmargins(&self, block: &Block, cx: Ctx) -> (f32, f32) {
        let em = self.base();
        match &block.kind {
            BlockKind::Heading { .. } => (1.25 * em, 0.5 * em),
            BlockKind::Paragraph(runs) if figure_of(runs).is_none() => (0.0, 0.0),
            BlockKind::List { .. } if cx.depth > 0 => (0.0, 0.0),
            BlockKind::Footnotes(_) => (2.0 * em, em),
            _ => (0.75 * em, 0.75 * em),
        }
    }

    /// Blocks one after another, their margins collapsed.
    #[allow(clippy::too_many_arguments)]
    fn vstack(&mut self, blocks: &[Block], u: &mut f32, top: f32, len: f32, out: &mut Out, cx: Ctx, page: Page) {
        let mut after = 0.0f32;
        for (k, b) in blocks.iter().enumerate() {
            let (before, next) = self.vmargins(b, cx);
            if k > 0 {
                *u += after.max(before);
            }
            self.vblock(b, u, top, len, out, cx, page);
            after = next;
        }
    }

    /// **One block down the page**, from `u` leftwards, its columns from
    /// `top`, `len` long.
    #[allow(clippy::too_many_arguments)]
    fn vblock(&mut self, block: &Block, u: &mut f32, top: f32, len: f32, out: &mut Out, cx: Ctx, page: Page) {
        let px = self.scale;
        let em = cx.size;
        let first = block.lines.0;
        match &block.kind {
            BlockKind::Heading { level, text } => {
                let n = (*level as usize).clamp(1, 6) - 1;
                let k = yumete_core::as_preview::VERTICAL_HEADING_SIZES[n];
                let color = self.theme.headings[n];
                let st = Style { size: cx.size * k, weight: 600.0, color, mono: false, line_height: 1.25, italic: false, anywhere: false };
                // A chapter's title stands two squares in, a section's one.
                let indent = match level {
                    1 => 2.0 * em,
                    2 => em,
                    _ => 0.0,
                };
                let vt = self.vtext(text, st, len, indent, indent, false);
                *u += self.put(&vt, -*u, top, yumete_core::as_preview::VERTICAL_HEADING_PITCH * st.size, out, first);
                if *level <= 2 {
                    *u += 0.2 * st.size;
                    out.items.push(Item::Rect { r: rect(-*u - px, top, px, len), fill: self.theme.border, radius: 0.0 });
                    *u += px;
                }
            }
            BlockKind::Paragraph(runs) => {
                if let Some((src, alt)) = figure_of(runs) {
                    if self.vfigure(src, alt, first, u, top, len, out, cx, page) {
                        return;
                    }
                }
                let st = self.body(&cx);
                let indent = if cx.tight { 0.0 } else { 2.0 * em };
                let vt = self.vtext(runs, st, len, indent, 0.0, true);
                *u += self.put(&vt, -*u, top, PITCH * em, out, first);
            }
            BlockKind::Maths(tex) => {
                out.marks.push((*u, first));
                match self.formula(tex, true, cx.color, cx.size * 1.1) {
                    Some(f) => {
                        // Turned, on a column of its own, two squares in from
                        // its head (JLREQ §3.7.4), smaller if it is longer.
                        let room = (len - 2.0 * em).max(em);
                        let k = (room / f.width).min(1.0);
                        let (thick, along) = (f.height * k, f.width * k);
                        let p = thick + (PITCH - 1.0) * em;
                        let axis = -*u - p / 2.0;
                        out.items.push(Item::Turned { r: rect((axis - thick / 2.0).round(), (top + 2.0 * em).round(), thick, along), pix: f.pix });
                        *u += p;
                    }
                    None => {
                        let st = Style { size: cx.size * 0.85, mono: true, color: self.theme.error, ..self.body(&cx) };
                        let run = [Inline { line: first, kind: InlineKind::Text { text: tex.clone(), marks: Marks::default() } }];
                        let vt = self.vtext(&run, st, len, 2.0 * em, 2.0 * em, false);
                        *u += self.put(&vt, -*u, top, PITCH * st.size, out, first);
                    }
                }
            }
            BlockKind::Rule => {
                out.marks.push((*u, first));
                let p = 1.5 * em;
                let axis = -*u - p / 2.0;
                out.items.push(Item::Rect { r: rect(axis - 2.0 * px, top, 4.0 * px, len), fill: self.theme.border, radius: 0.0 });
                *u += p;
            }
            BlockKind::Code { .. } | BlockKind::FrontMatter(_) | BlockKind::Html(_) => self.island(block, u, top, len, out, cx, page),
            BlockKind::Quote(body) => {
                let u0 = *u;
                let bar = 0.25 * self.base();
                let inner = Ctx { color: self.theme.quote, ..cx };
                let gap = em;
                self.vstack(body, u, top + bar + gap, len - bar - gap, out, inner, page);
                out.items.push(Item::Rect { r: rect(-*u, top, *u - u0, bar), fill: self.theme.quote_bar, radius: 0.0 });
            }
            BlockKind::Callout { kind, name, title, fenced, body } => self.vcallout(*kind, name, title.as_deref(), *fenced, body, block.lines, u, top, len, out, cx, page),
            BlockKind::List { ordered, start, loose, items } => {
                let indent = 2.0 * em;
                let inner = Ctx { depth: cx.depth + 1, tight: !loose, ..cx };
                for (k, item) in items.iter().enumerate() {
                    if k > 0 && *loose {
                        *u += 0.5 * em;
                    }
                    let u0 = *u;
                    out.marks.push((*u, item.lines.0));
                    self.vstack(&item.body, u, top + indent, len - indent, out, inner, page);
                    if item.body.is_empty() || *u == u0 {
                        *u = u0 + PITCH * em;
                    }
                    let axis = -u0 - PITCH * em / 2.0;
                    let ink = self.theme.list_marker;
                    match item.task {
                        Some(done) => {
                            let side = 0.9 * em;
                            let (bx, by) = (axis - side / 2.0, top + indent - side - 0.4 * em);
                            match done {
                                true => {
                                    out.items.push(Item::Rect { r: rect(bx, by, side, side), fill: self.theme.link, radius: 3.0 * px });
                                    let p = |a: f32, b: f32| (bx + a * side, by + b * side);
                                    out.items.push(Item::Lines { points: vec![p(0.22, 0.52), p(0.42, 0.72), p(0.78, 0.30)], color: self.theme.background, width: 2.0 * px });
                                }
                                false => out.items.push(Item::Frame { r: rect(bx, by, side, side), color: self.theme.muted, width: 1.2 * px, radius: 3.0 * px }),
                            }
                        }
                        None if *ordered => {
                            let st = Style { color: ink, ..self.body(&cx) };
                            let vt = self.number(&format!("{}.", start + k as u64), st, len, item.lines.0);
                            let length = vt.longest();
                            let mut marker = Out::default();
                            self.put(&vt, axis + PITCH * em / 2.0, top + indent - length - 0.4 * em, PITCH * em, &mut marker, item.lines.0);
                            out.items.extend(marker.items);
                        }
                        None => {
                            let d = 0.34 * em;
                            let (bx, by) = (axis - d / 2.0, top + indent - 0.9 * em - d / 2.0);
                            match cx.depth {
                                0 => out.items.push(Item::Rect { r: rect(bx, by, d, d), fill: ink, radius: d / 2.0 }),
                                1 => out.items.push(Item::Frame { r: rect(bx + 0.5 * px, by + 0.5 * px, d - px, d - px), color: ink, width: px, radius: d / 2.0 }),
                                _ => out.items.push(Item::Rect { r: rect(bx + 0.1 * d, by + 0.1 * d, 0.8 * d, 0.8 * d), fill: ink, radius: 0.0 }),
                            }
                        }
                    }
                }
            }
            BlockKind::Table { align, head, rows } => self.vtable(align, head, rows, first, None, u, top, len, out, cx, page),
            BlockKind::Footnotes(notes) => {
                out.items.push(Item::Rect { r: rect(-*u - px, top, px, len * 0.3), fill: self.theme.border, radius: 0.0 });
                *u += px + em;
                let inner = Ctx { color: self.theme.muted, size: cx.size * 0.875, tight: true, ..cx };
                let indent = 2.0 * inner.size;
                for (k, note) in notes.iter().enumerate() {
                    if k > 0 {
                        *u += 0.25 * em;
                    }
                    let u0 = *u;
                    out.marks.push((*u, note.lines.0));
                    self.vstack(&note.body, u, top + indent, len - indent, out, inner, page);
                    if *u == u0 {
                        *u += PITCH * inner.size;
                    }
                    let st = Style { color: self.theme.footnote, ..self.body(&inner) };
                    let vt = self.number(&format!("{}.", note.number), st, len, note.lines.0);
                    let length = vt.longest();
                    let mut marker = Out::default();
                    self.put(&vt, -u0, top + indent - length - 0.4 * inner.size, PITCH * inner.size, &mut marker, note.lines.0);
                    out.items.extend(marker.items);
                }
            }
        }
    }

    /// **A list's or a note's number**, set as JLREQ sets numbers whatever
    /// `vertical_latin` says: `1.` and `12` across in one square, longer
    /// ones turned. A marker is the page's furniture, not the writer's text.
    fn number(&mut self, number: &str, st: Style, len: f32, line: usize) -> VText {
        let kept = self.vertical;
        if let Some(v) = self.vertical.as_mut() {
            v.latin = Latin::Jlreq;
        }
        let run = [Inline { line, kind: InlineKind::Text { text: number.to_string(), marks: Marks::default() } }];
        let vt = self.vtext(&run, st, len, 0.0, 0.0, false);
        self.vertical = kept;
        vt
    }

    /// **Something set across on a vertical page**: front matter, HTML.
    #[allow(clippy::too_many_arguments)]
    fn island(&mut self, block: &Block, u: &mut f32, top: f32, len: f32, out: &mut Out, cx: Ctx, page: Page) {
        if let BlockKind::Code { info, lines } = &block.kind {
            return self.vcode(info, lines, block.lines, u, top, len, out, cx);
        }
        let em = self.base();
        let room = (page.width * 0.75).min(48.0 * em).max(8.0 * em);
        let mut b = Out::default();
        let mut y = 0.0;
        self.block(block, 0.0, room, &mut y, &mut b, cx);
        // **Front matter is framed as wide as its lines**, as the export's
        // page sets it (`max-inline-size`, no wider than its text): framed
        // the whole of `room`, a few short lines would take most of the
        // first screen across.
        if let BlockKind::FrontMatter(_) = block.kind {
            let words = |i: &Item| match i {
                Item::Text { x, text, .. } => Some((*x, x + text.layout.width())),
                _ => None,
            };
            let pad = b.items.iter().filter_map(words).map(|(x, _)| x).fold(f32::MAX, f32::min);
            let end = b.items.iter().filter_map(words).map(|(_, end)| end).fold(0.0, f32::max);
            if pad < f32::MAX {
                for item in &mut b.items {
                    if let Item::Frame { r, .. } = item {
                        r.x1 = r.x1.min(r.x0 + f64::from(end + pad));
                    }
                }
            }
        }
        let w = b.items.iter().map(|i| match i {
            Item::Text { x, text, .. } => x + text.layout.width(),
            Item::Rect { r, .. } | Item::Frame { r, .. } => r.x1 as f32,
            _ => 0.0,
        }).fold(0.0, f32::max).max(em);
        shift(&mut b.items, -*u - w, top);
        out.items.extend(b.items);
        out.marks.push((*u, block.lines.0));
        *u += w;
    }

    /// **A code block down the page** (2026-11-08): the block as it is set
    /// across — its colours, its ground — turned a quarter turn clockwise
    /// whole, every character with its line, 漢字 too: code is verbatim, and
    /// its columns line up only if nothing in it stands up. Each line runs
    /// down the page as a turned Latin word does, the next on its left.
    ///
    /// A line longer than the column wraps at the column's foot, wherever
    /// that falls, as a terminal wraps; what goes on hangs [`CODE_HANG`]
    /// cells in, the cells keeping their places, with [`CODE_HOOK`] in the
    /// hang in the dim colour. The export's `pre` does the same in CSS.
    #[allow(clippy::too_many_arguments)]
    fn vcode(&mut self, info: &str, lines: &[String], span: (usize, usize), u: &mut f32, top: f32, len: f32, out: &mut Out, cx: Ctx) {
        let px = self.scale;
        let pad = 16.0 * px;
        let st = self.code_style(&cx);
        let fenced = span.1 + 1 - span.0 > lines.len();
        let first = span.0 + usize::from(fenced);
        let code = self.code_text(info, lines, first);
        let mut layout = self.code_layout(&code.text, &code.colours, st);
        layout.break_all_lines(None);

        // Every cluster, by source line: its face, colour, glyphs, advance.
        struct Cl {
            at: Range<usize>,
            font: FontData,
            coords: Vec<i16>,
            size: f32,
            color: Rgba,
            glyphs: Vec<(u32, f32, f32)>,
            adv: f32,
        }
        let mut rows: Vec<Vec<Cl>> = Vec::new();
        let mut metrics: Option<(f32, f32)> = None;
        for line in layout.lines() {
            let mut row = Vec::new();
            for run in line.runs() {
                let m = run.font_metrics();
                metrics.get_or_insert((m.ascent, m.descent));
                let coords: Vec<i16> = run.normalized_coords().iter().map(|c| c.to_bits()).collect();
                for c in run.clusters() {
                    if code.text[c.text_range()].chars().all(|ch| ch == '\n' || ch == '\r') {
                        continue;
                    }
                    let mut pen = 0.0;
                    let glyphs = c.glyphs().map(|g| {
                        let at = (g.id, pen + g.x, g.y);
                        pen += g.advance;
                        at
                    }).collect();
                    row.push(Cl { at: c.text_range(), font: run.font().clone(), coords: coords.clone(), size: run.font_size(), color: c.style().brush.color, glyphs, adv: c.advance() });
                }
            }
            rows.push(row);
        }
        let (asc, desc) = metrics.unwrap_or((0.8 * st.size, 0.2 * st.size));
        let lh = st.size * st.line_height;
        let base = (lh - (asc + desc)) / 2.0 + asc;
        // One cell: what a half-width character is wide in the code face.
        let cell = rows.iter().flatten().find(|c| code.text[c.at.clone()].chars().all(|ch| ch.is_ascii_graphic() || ch == ' ')).map_or(0.6 * st.size, |c| c.adv);
        let hang = f32::from(CODE_HANG) * cell;
        let room = (len - 2.0 * pad).max(4.0 * cell);

        // **Wrapped at the column's foot**: each line a source line's run of
        // clusters, and whether it goes on from the line before.
        let mut wrapped: Vec<(usize, bool, Vec<usize>)> = Vec::new();
        for (k, row) in rows.iter().enumerate() {
            let mut line = (k, false, Vec::new());
            let mut pen = 0.0;
            for (n, c) in row.iter().enumerate() {
                let limit = if line.1 { room - hang } else { room };
                if pen > 0.0 && pen + c.adv > limit + 0.01 {
                    wrapped.push(std::mem::replace(&mut line, (k, true, Vec::new())));
                    pen = 0.0;
                }
                line.2.push(n);
                pen += c.adv;
            }
            wrapped.push(line);
        }
        let longest = wrapped.iter().map(|(k, on, cl)| cl.iter().map(|&n| rows[*k][n].adv).sum::<f32>() + if *on { hang } else { 0.0 }).fold(0.0, f32::max);
        let thick = wrapped.len() as f32 * lh + 2.0 * pad;
        let along = (longest + 2.0 * pad).min(len).max(2.0 * pad + cell);

        // Across, the box's top left at the origin; down the page, turned
        // about its top right corner.
        let right = -*u;
        let turn = Affine::translate((right as f64, top as f64)) * Affine::rotate(std::f64::consts::FRAC_PI_2);
        let down = |hx: f32, hy: f32| (right - hy, top + hx);
        out.items.push(Item::Rect { r: rect(right - thick, top, thick, along), fill: self.theme.code_block, radius: 6.0 * px });
        if fenced {
            out.marks.push((*u, span.0));
        }
        let mut batch: Option<Run> = None;
        let flush = |batch: &mut Option<Run>, out: &mut Out| {
            if let Some(run) = batch.take() {
                out.items.push(Item::Glyphs(Box::new(run)));
            }
        };
        for (v, (k, on, cl)) in wrapped.iter().enumerate() {
            let y0 = pad + v as f32 * lh;
            let mut x = pad + if *on { hang } else { 0.0 };
            out.marks.push((*u + y0, first + k));
            out.cols.push((*u + y0, cl.iter().map(|&n| &code.text[rows[*k][n].at.clone()]).collect()));
            if *on {
                let hook: Vec<(f32, f32)> = CODE_HOOK.iter().map(|&(a, b)| down(pad + a * hang, y0 + b * lh)).collect();
                let w = (0.07 * st.size).max(px);
                out.items.push(Item::Lines { points: hook[..3].to_vec(), color: self.theme.muted, width: w });
                out.items.push(Item::Lines { points: hook[3..].to_vec(), color: self.theme.muted, width: w });
            }
            for &n in cl {
                let c = &rows[*k][n];
                let same = batch.as_ref().is_some_and(|b| b.font == c.font && b.size == c.size && b.color == c.color && b.coords == c.coords);
                if !same {
                    flush(&mut batch, out);
                    batch = Some(Run { font: c.font.clone(), size: c.size, coords: c.coords.clone(), color: c.color, transform: turn, glyphs: Vec::new(), bounds: rect(right - y0 - lh, top + x, lh, 0.0) });
                }
                if let Some(b) = batch.as_mut() {
                    b.glyphs.extend(c.glyphs.iter().map(|&(id, gx, gy)| vello_cpu::Glyph { id, x: x + gx, y: y0 + base + gy }));
                    b.bounds = b.bounds.union(rect(right - y0 - lh, top + x, lh, c.adv));
                }
                x += c.adv;
            }
            flush(&mut batch, out);
        }
        *u += thick;
    }

    /// **A figure**: its picture upright, as large as the column allows and
    /// no wider than most of the window, its caption down beside it on the
    /// left. `false` when the picture cannot be read.
    #[allow(clippy::too_many_arguments)]
    fn vfigure(&mut self, src: &str, alt: &str, line: usize, u: &mut f32, top: f32, len: f32, out: &mut Out, cx: Ctx, page: Page) -> bool {
        let em = cx.size;
        // Its size from its header (2026-11-09): it was drawn at full size
        // first only to be measured, 4000 × 3000 pixels a few hundred
        // megabytes on every layout of the block.
        let Some((w0, h0)) = self.image_size(src) else { return false };
        let room = (page.width * 0.45).min(w0 * len / h0.max(1.0)).max(1.0);
        let Some(pix) = self.image(src, room) else { return false };
        let (w, h) = (pix.width() as f32, pix.height() as f32);
        out.marks.push((*u, line));
        out.items.push(Item::Pic { r: rect((-*u - w).round(), top.round(), w, h), pix });
        *u += w;
        if !alt.trim().is_empty() {
            *u += 0.4 * em;
            let st = Style { size: em * 0.875, color: self.theme.muted, ..self.body(&cx) };
            let run = [Inline { line, kind: InlineKind::Text { text: alt.to_string(), marks: Marks::default() } }];
            let vt = self.vtext(&run, st, len - em, em, em, false);
            *u += self.put(&vt, -*u, top, PITCH * st.size, out, line);
        }
        true
    }

    /// **A callout down the page**: a bar along its head in its colour and
    /// its title in the first column (GitHub's alert), or a tinted box
    /// around it (a `:::` container).
    #[allow(clippy::too_many_arguments)]
    fn vcallout(
        &mut self,
        kind: Callout,
        name: &str,
        title: Option<&[Inline]>,
        fenced: bool,
        body: &[Block],
        span: (usize, usize),
        u: &mut f32,
        top: f32,
        len: f32,
        out: &mut Out,
        cx: Ctx,
        page: Page,
    ) {
        let px = self.scale;
        let em = cx.size;
        let colour = self.theme.callout(kind);
        let default_title = match (fenced, kind) {
            (true, Callout::Details) => "Details".to_string(),
            (true, _) => name.to_uppercase(),
            (false, _) => {
                let mut c: Vec<char> = name.to_lowercase().chars().collect();
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
        let u0 = *u;
        let at = out.items.len();
        let pad = if fenced { em } else { 0.0 };
        let bar = if fenced { 0.0 } else { 0.25 * self.base() };
        let (inner_top, inner_len) = (top + bar + em * if fenced { 0.75 } else { 1.0 }, len - bar - em * if fenced { 1.5 } else { 1.0 });
        *u += pad;
        let st = Style { size: em, weight: 600.0, color: if kind == Callout::Details { cx.color } else { colour }, mono: false, line_height: 1.6, italic: false, anywhere: false };
        let vt = self.vtext(&title_runs, st, inner_len, 0.0, 0.0, false);
        *u += self.put(&vt, -*u, inner_top, PITCH * em, out, span.0);
        if !body.is_empty() {
            *u += 0.25 * em;
            self.vstack(body, u, inner_top, inner_len, out, cx, page);
        }
        *u += pad;
        match fenced {
            true => out.items.insert(at, Item::Rect { r: rect(-*u, top, *u - u0, len), fill: self.theme.callout_ground(kind), radius: 8.0 * px }),
            false => out.items.push(Item::Rect { r: rect(-*u, top, *u - u0, bar), fill: colour, radius: 0.0 }),
        }
    }

    /// **A vertical table**, as a 年表 in 史記 is set: each source row a
    /// column read from the right, the head row first; each source column
    /// a band down the page, as long as its longest cell (a long one shares
    /// what is left of the column and wraps); the caption down the right
    /// of it.
    #[allow(clippy::too_many_arguments)]
    fn vtable(
        &mut self,
        align: &[Align],
        head: &[Vec<Inline>],
        rows: &[(usize, Vec<Vec<Inline>>)],
        first: usize,
        caption: Option<&[Inline]>,
        u: &mut f32,
        top: f32,
        len: f32,
        out: &mut Out,
        cx: Ctx,
        page: Page,
    ) {
        let _ = page;
        let px = self.scale;
        let em = cx.size;
        if let Some(caption) = caption {
            let st = Style { size: em * 0.9, ..self.body(&cx) };
            let vt = self.vtext(caption, st, len, 0.0, 0.0, false);
            *u += self.put(&vt, -*u, top, PITCH * st.size, out, first);
            *u += 0.25 * em;
        }
        let nband = align.len().max(head.len()).max(rows.iter().map(|r| r.1.len()).max().unwrap_or(0)).max(1);
        let empty: Vec<Inline> = Vec::new();
        let pad = 0.4 * em;
        // **More columns than a column of the page holds at a square each**
        // (2026-11-09): the table goes on in a second band to the left, and a
        // third, each with every row, so that no cell is past the foot.
        let per = (((len - px) / (em + 2.0 * pad + px)).floor() as usize).max(1);
        if nband > per {
            let parts = nband.div_ceil(per);
            let size = nband.div_ceil(parts);
            let cut = |row: &[Vec<Inline>], from: usize| -> Vec<Vec<Inline>> {
                (from..(from + size).min(nband)).map(|b| row.get(b).cloned().unwrap_or_default()).collect()
            };
            for (k, from) in (0..nband).step_by(size).enumerate() {
                if k > 0 {
                    *u += em;
                }
                let part_align: Vec<Align> = (from..(from + size).min(nband)).map(|b| align.get(b).copied().unwrap_or_default()).collect();
                let part_head = cut(head, from);
                let part_rows: Vec<(usize, Vec<Vec<Inline>>)> = rows.iter().map(|(line, row)| (*line, cut(row, from))).collect();
                // The caption is down already, beside the first band.
                self.vtable(&part_align, &part_head, &part_rows, first, None, u, top, len, out, cx, page);
            }
            return;
        }
        let pitch = 1.6 * em;
        let body = self.body(&cx);
        let all: Vec<(usize, &[Vec<Inline>])> = std::iter::once((first, head)).chain(rows.iter().map(|r| (r.0, r.1.as_slice()))).collect();
        // Every cell read once; its length unbroken.
        let mut cells: Vec<Vec<Units>> = Vec::with_capacity(all.len());
        for (ri, (_, row)) in all.iter().enumerate() {
            let st = match ri {
                0 => Style { weight: 600.0, color: self.theme.table_head, ..body },
                _ => body,
            };
            cells.push((0..nband).map(|b| self.units(row.get(b).unwrap_or(&empty), st, len)).collect());
        }
        let natural: Vec<f32> = (0..nband).map(|b| cells.iter().map(|r| r[b].length()).fold(0.0, f32::max) + 2.0 * pad).collect();
        // The bands' lengths: each its longest cell's, when the column has
        // room; else the short ones keep theirs and the long share the rest.
        let room = len - (nband + 1) as f32 * px;
        let short = |n: f32| n <= 6.0 * em + 2.0 * pad;
        let total: f32 = natural.iter().sum();
        let bands: Vec<f32> = if total <= room {
            natural.clone()
        } else {
            let kept: f32 = natural.iter().filter(|n| short(**n)).sum();
            let long: f32 = natural.iter().filter(|n| !short(**n)).sum::<f32>().max(1.0);
            let left = (room - kept).max(0.0);
            natural.iter().map(|&n| if short(n) { n } else { (n * left / long).max(3.0 * em + 2.0 * pad) }).collect()
        };
        // **Still too long** (2026-11-09): with many columns even the short
        // cells did not fit, and the last of them ran off the foot of the
        // page, where nothing could bring them back. Every band then gives
        // way down to a square, its cells wrapping as a long one does, so
        // the table is never longer than the column, as the export has it.
        //
        // Warning: **only a band longer than a square gives way, and the floor
        // counts a shorter one as it is** (2026-11-09, reported: a table with
        // empty cells still ran past the foot): the floor was a square for
        // every band, and a short band was stretched up to a square while the
        // long ones were shrunk to fit the floor that did not count it.
        let least = em + 2.0 * pad;
        let given: f32 = bands.iter().sum();
        let floor: f32 = bands.iter().map(|&b| b.min(least)).sum();
        let bands: Vec<f32> = match given > room && given > floor {
            true => {
                let k = ((room - floor) / (given - floor)).clamp(0.0, 1.0);
                bands.iter().map(|&b| if b > least { least + (b - least) * k } else { b }).collect()
            }
            false => bands,
        };
        let table_len: f32 = bands.iter().sum::<f32>() + (nband + 1) as f32 * px;
        // Each cell in its band, and each row as thick as its fullest cell.
        let mut laid: Vec<Vec<VText>> = Vec::with_capacity(cells.len());
        let mut thick: Vec<f32> = Vec::with_capacity(cells.len());
        //
        // **A word longer than its band is cut** (2026-11-09, reported: a URL
        // in a cell ran past the foot of the page): a cell was read with the
        // page's column length, so a word that fitted the page and not the
        // band stood out of its cell. Such a cell is read again at the band's
        // length, the word cut anywhere, as the export breaks it.
        for (ri, row) in cells.into_iter().enumerate() {
            let mut r: Vec<VText> = Vec::with_capacity(nband);
            for (b, cell) in row.into_iter().enumerate() {
                let inner = (bands[b] - 2.0 * pad).max(em);
                let cell = match cell.units.iter().any(|unit| unit.adv > inner + 0.5) {
                    true => {
                        let st = match ri {
                            0 => Style { weight: 600.0, color: self.theme.table_head, ..body },
                            _ => body,
                        };
                        self.units(all[ri].1.get(b).unwrap_or(&empty), st, inner)
                    }
                    false => cell,
                };
                r.push(columns(cell, inner, 0.0, 0.0, false, false));
            }
            let most = r.iter().map(|vt| vt.cols.len().max(1)).max().unwrap_or(1);
            thick.push(most as f32 * pitch + 2.0 * pad);
            laid.push(r);
        }
        let u0 = *u;
        let width: f32 = thick.iter().sum::<f32>() + (thick.len() + 1) as f32 * px;
        let mut x = -*u - px;
        for (ri, row) in laid.iter().enumerate() {
            let line = all[ri].0;
            out.marks.push((-x, line));
            if ri > 0 && ri % 2 == 0 {
                out.items.push(Item::Rect { r: rect(x - thick[ri], top, thick[ri], table_len), fill: self.theme.stripe, radius: 0.0 });
            }
            let mut y = top + px;
            for (b, vt) in row.iter().enumerate() {
                let length = vt.longest();
                let inner = bands[b] - 2.0 * pad;
                let how = align.get(b).copied().unwrap_or_default();
                let offset = match (how, ri) {
                    (Align::Center, _) | (Align::Plain, 0) => (inner - length).max(0.0) / 2.0,
                    (Align::Right, _) => (inner - length).max(0.0),
                    _ => 0.0,
                };
                // A cell of fewer columns than its row stands at the row's right.
                let mut cell = Out::default();
                self.put(vt, x - pad, y + pad + offset, pitch, &mut cell, line);
                out.items.extend(cell.items);
                out.cols.extend(cell.cols);
                y += bands[b] + px;
            }
            x -= thick[ri] + px;
        }
        // The lines: between the bands across the table, between the rows
        // down it, and round it.
        let right = -u0;
        let mut y = top;
        out.items.push(Item::Rect { r: rect(right - width, y, width, px), fill: self.theme.border, radius: 0.0 });
        for band in &bands {
            y += band + px;
            out.items.push(Item::Rect { r: rect(right - width, y, width, px), fill: self.theme.border, radius: 0.0 });
        }
        let mut x = right;
        out.items.push(Item::Rect { r: rect(x - px, top, px, table_len), fill: self.theme.border, radius: 0.0 });
        for t in &thick {
            x -= t + px;
            out.items.push(Item::Rect { r: rect(x - px, top, px, table_len), fill: self.theme.border, radius: 0.0 });
        }
        *u += width;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fonts::Han;
    use crate::{Formula, Formulas, Options, Preview, Theme};
    use vello_cpu::kurbo::Rect;

    /// A formula renderer that draws every formula as a box, wider the
    /// longer its TeX.
    struct Boxes;

    impl Formulas for Boxes {
        fn draw(&self, tex: &str, _: bool, _: [u8; 3], size: f32) -> Result<Formula, String> {
            let (w, h) = ((tex.len() as f32 * size * 0.5).max(4.0) as u32, (size * 1.2) as u32);
            let img = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
            let mut png = Vec::new();
            img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
            Ok(Formula { png, baseline: h as f32 * 0.8 })
        }
    }

    fn page(text: &str, latin: Latin, height: u32) -> Preview {
        let options = Options { width: 800, scale: 1.0, theme: Theme::light(), base_dir: std::env::temp_dir() };
        let mut p = Preview::new(options, Box::new(Boxes));
        p.set_height(height);
        p.set_vertical(Some(Vertical { latin, hanging: false }));
        p.set_text(text);
        p
    }

    /// Every column's words, in reading order.
    fn columns_of(p: &Preview) -> Vec<String> {
        p.visible_text(0.0, p.size().1).lines().map(str::to_string).collect()
    }

    fn engine(latin: Latin, hanging: bool) -> Engine {
        let mut e = Engine::new(Theme::light(), 1.0, std::env::temp_dir(), Box::new(Boxes));
        e.vertical = Some(Vertical { latin, hanging });
        e
    }

    fn body() -> Style {
        Style { size: 16.0, weight: 400.0, color: [0, 0, 0, 255], mono: false, line_height: 1.6, italic: false, anywhere: false }
    }

    fn runs(text: &str) -> Vec<Inline> {
        match crate::parse::parse(text).into_iter().next().map(|b| b.kind) {
            Some(BlockKind::Paragraph(runs)) => runs,
            other => panic!("not a paragraph: {other:?}"),
        }
    }

    /// Each unit of `text` that is drawn, with how it stands.
    fn looks(e: &mut Engine, text: &str) -> Vec<(String, &'static str)> {
        let u = e.units(&runs(text), body(), 10_000.0);
        u.units
            .iter()
            .filter(|x| !matches!(x.look, Look::Gap | Look::Break))
            .map(|x| {
                let how = match &x.look {
                    Look::Up { .. } => "up",
                    Look::Turn { .. } => "turned",
                    Look::Across { .. } => "across",
                    Look::Pic { turned: true, .. } => "pic turned",
                    Look::Pic { turned: false, .. } => "pic upright",
                    Look::Gap | Look::Break => "",
                };
                (u.shown[x.text.clone()].to_string(), how)
            })
            .collect()
    }

    fn how<'a>(looks: &'a [(String, &'static str)], text: &str) -> Vec<&'a str> {
        looks.iter().filter(|l| l.0 == text).map(|l| l.1).collect()
    }

    /// **`turn`** (2026-11-08): every run of letters and digits turned
    /// whole, its marks with it; every inline formula turned.
    #[test]
    fn under_turn_every_latin_run_lies_down() {
        let mut e = engine(Latin::Turn, false);
        let l = looks(&mut e, "我们1958年来，12个人，涨了5%，don’t say NHK，写 $x$ 和 $x+y$。");
        for word in ["1958", "12", "5%", "NHK"] {
            assert_eq!(how(&l, word), ["turned"], "{word}: {l:?}");
        }
        assert_eq!(how(&l, "don’t"), ["turned"], "the apostrophe is in the word: {l:?}");
        assert_eq!(l.iter().filter(|x| x.1 == "pic turned").count(), 2, "{l:?}");
        assert_eq!(how(&l, "我"), ["up"]);
    }

    /// **`jlreq`** (2026-11-08): a word turned whole, capitals upright one to
    /// a square, one digit upright, two across in one square, more turned, a
    /// number with its unit one way; `$x$` and `\frac12` upright, a longer
    /// formula turned.
    #[test]
    fn under_jlreq_numbers_and_capitals_stand_as_jlreq_sets_them() {
        let mut e = engine(Latin::Jlreq, false);
        let l = looks(&mut e, "我们1958年来，12个人，3次，涨了5%，用 Python 写，NHK 说，又写 $x$ 和 $\\frac12$ 和 $x+y$。");
        assert_eq!(how(&l, "1958"), ["turned"], "{l:?}");
        assert_eq!(how(&l, "12"), ["across"], "{l:?}");
        assert_eq!(how(&l, "3"), ["across"], "one digit stands in its square: {l:?}");
        assert_eq!(how(&l, "5%"), ["across"], "{l:?}");
        assert_eq!(how(&l, "Python"), ["turned"], "{l:?}");
        for letter in ["N", "H", "K"] {
            assert_eq!(how(&l, letter), ["across"], "{letter} upright, one to a square: {l:?}");
        }
        assert_eq!(l.iter().filter(|x| x.1 == "pic upright").count(), 2, "{l:?}");
        assert_eq!(l.iter().filter(|x| x.1 == "pic turned").count(), 1, "{l:?}");
    }

    /// **Corner brackets for curly quotes** in Chinese (2026-11-08); an
    /// apostrophe stays, and code keeps what it says.
    #[test]
    fn curly_quotes_become_corner_brackets_in_chinese() {
        let p = page("他说：“我们走吧，‘马上’走。”don’t 写 `“x”`。\n", Latin::Turn, 600);
        let all = columns_of(&p).concat();
        assert!(all.contains("『我们走吧，「马上」走。』"), "{all}");
        assert!(all.contains("don’t"), "{all}");
        assert!(all.contains("“x”"), "code is as written: {all}");
        let mut e = engine(Latin::Turn, false);
        e.han = Han::Japanese;
        let l = looks(&mut e, "彼は“雨”と言った。");
        assert_eq!(how(&l, "“"), ["turned"], "a Japanese text keeps its quotes, turned: {l:?}");
    }

    /// **A curly quote that stays curly is turned** (2026-11-09, reported:
    /// it stood upright): orientation R, as a browser sets it in the export,
    /// and it still breaks as a quote, never ending a column.
    #[test]
    fn a_curly_quote_that_stays_is_turned() {
        let mut e = engine(Latin::Turn, false);
        e.quotes = true;
        let l = looks(&mut e, "他说“go”，又说“走”。");
        assert_eq!(how(&l, "“"), ["turned"], "{l:?}");
        assert_eq!(how(&l, "”"), ["turned"], "{l:?}");
        assert_eq!(how(&l, "『"), ["up"], "{l:?}");
        let u = e.units(&runs("他说“go”。"), body(), 10_000.0);
        let open = u.units.iter().find(|x| x.ch == '“').expect("the quote");
        assert_eq!(open.cls, Cls::Open);
    }

    /// **The preview turns quotes exactly when the export does** (2026-11-09):
    /// the page's language as the export decides it, not its writing
    /// system; an English page quoting Chinese keeps its quotes.
    #[test]
    fn quotes_turn_when_the_export_turns_them() {
        let text = "This essay is mostly written in English words, and it quotes “走吧” once, then goes on in English.\n";
        let p = page(text, Latin::Turn, 600);
        let all = columns_of(&p).concat();
        assert!(all.contains("“走吧”") && !all.contains('『'), "{all}");
        let p = page("这一段是中文，引用了“走吧”。\n", Latin::Turn, 600);
        assert!(columns_of(&p).concat().contains("『走吧』"));
    }

    /// **Under jlreq a letter with marks joined to it stands as the letter
    /// alone** (2026-11-09), the marks turned beside it as the export's page
    /// sets them; `x, y` are two letters.
    #[test]
    fn under_jlreq_a_letter_with_its_marks_stands_upright() {
        let mut e = engine(Latin::Jlreq, false);
        let l = looks(&mut e, "又 (x) 与 x. 和 x, y 写。");
        assert_eq!(how(&l, "x"), ["across", "across", "across"], "{l:?}");
        assert_eq!(how(&l, "y"), ["across"], "{l:?}");
        assert_eq!(how(&l, "("), ["turned"], "{l:?}");
        assert_eq!(how(&l, "."), ["turned"], "{l:?}");
    }

    /// **A letter with a combining mark stands whole** (review, 2026-11-09):
    /// `e` and U+0301 typed apart are one square under `jlreq`, as `é` is;
    /// the mark used to be cut off and turned on its own.
    #[test]
    fn under_jlreq_a_letter_with_a_combining_mark_stands_whole() {
        let mut e = engine(Latin::Jlreq, false);
        let l = looks(&mut e, "又 e\u{301} 与 é 写。");
        assert_eq!(how(&l, "e\u{301}"), ["across"], "{l:?}");
        assert_eq!(how(&l, "é"), ["across"], "{l:?}");
        assert!(how(&l, "\u{301}").is_empty(), "{l:?}");
    }

    /// **A vertical table never runs past the foot** (2026-11-09, reported):
    /// empty and short bands keep their length while the long ones give way
    /// to fit, and a long word or an address in a cell is cut to its band.
    #[test]
    fn a_tables_cells_stay_inside_and_above_the_foot() {
        let mut e = engine(Latin::Turn, false);
        let below = |laid: &Laid, len: f32| {
            for item in &laid.items {
                match item {
                    Item::Rect { r, .. } => assert!(r.y1 <= f64::from(len) + 1.0, "a line ends at {} on a column {len} long", r.y1),
                    Item::Glyphs(run) => assert!(run.bounds.y1 <= f64::from(len) + 1.0, "glyphs end at {} on a column {len} long", run.bounds.y1),
                    _ => {}
                }
            }
        };
        let text = "| 一 |  | 很长很长很长很长很长的一格 |  | 又是很长很长很长很长的一格 |\n|---|---|---|---|---|\n| 甲 |  | 乙 |  | 丙 |\n";
        let block = crate::parse::parse(text).into_iter().next().unwrap();
        for len in [160.0f32, 220.0, 300.0] {
            below(&e.vtop(&block, None, Page { top: 0.0, len, width: 800.0 }), len);
        }
        let text = "| 名 | 网址 |\n|---|---|\n| 甲 | https://example.com/a/very/long/path/that/goes/on/and/on/index.html |\n";
        let block = crate::parse::parse(text).into_iter().next().unwrap();
        for len in [200.0f32, 300.0, 420.0] {
            let laid = e.vtop(&block, None, Page { top: 0.0, len, width: 800.0 });
            below(&laid, len);
            let words: String = laid.cols.iter().map(|c| c.1.as_str()).collect();
            assert!(words.contains("index.html"), "{len}: {words}");
        }
    }

    /// **A turned formula in a cell is held to its band** (review,
    /// 2026-11-09): one wider than the band ran out of its cell and past the
    /// foot; it is scaled down to fit, as a picture is, and a short one keeps
    /// its size.
    #[test]
    fn a_turned_formula_in_a_cell_is_scaled_to_its_band() {
        let mut e = engine(Latin::Turn, false);
        let text = format!("| 名 | 式 |\n|---|---|\n| 甲 | ${}$ |\n| 乙 | $x+y$ |\n", "a+".repeat(40));
        let block = crate::parse::parse(&text).into_iter().next().unwrap();
        for len in [200.0f32, 300.0, 420.0] {
            let laid = e.vtop(&block, None, Page { top: 0.0, len, width: 800.0 });
            let turned: Vec<Rect> = laid.items.iter().filter_map(|item| match item {
                Item::Turned { r, .. } => Some(*r),
                _ => None,
            }).collect();
            assert_eq!(turned.len(), 2, "{len}");
            for r in &turned {
                assert!(r.y1 <= f64::from(len) + 1.0, "{len}: a formula ends at {} past the foot", r.y1);
            }
            let short = turned.iter().map(|r| r.height()).fold(f64::MAX, f64::min);
            assert!(short >= 3.0 * 16.0 * 1.05 * 0.5 - 1.0, "{len}: the short one keeps its size: {short}");
        }
    }

    const LONG: &str = "那一年秋天，我们走了很远的路，看见一座山。山上有树，树下有人，他说：“雨要来了。”我们没有回答——只是看着他。\
        后来天黑了，我们在山脚下的小屋里坐着，听外面的风声（很大的风），一直到天亮……";

    /// **禁則** (2026-11-08): at every column length no closing mark or stop
    /// begins a column, no opening mark ends one, and a doubled dash or
    /// ellipsis is never split.
    #[test]
    fn no_column_begins_with_a_stop_or_ends_with_an_opening_mark() {
        let text = format!("{LONG}{LONG}{LONG}\n");
        for height in (120..420).step_by(9) {
            let p = page(&text, Latin::Turn, height);
            let cols = columns_of(&p);
            assert!(cols.len() > 3, "{height}: {cols:?}");
            for pair in cols.windows(2) {
                let (a, b) = (&pair[0], &pair[1]);
                let head = b.chars().next().unwrap_or(' ');
                assert!(!"，。、：；！？」』）".contains(head), "{height}: a column begins with {head:?}: {cols:?}");
                let foot = a.chars().last().unwrap_or(' ');
                assert!(!"「『（".contains(foot), "{height}: a column ends with {foot:?}: {cols:?}");
                for mark in ['—', '…'] {
                    assert!(!(a.ends_with(mark) && b.starts_with(mark)), "{height}: {mark} split: {cols:?}");
                }
            }
        }
    }

    /// **Justified** (2026-11-08): every column of a paragraph but its last
    /// ends at the column's foot.
    #[test]
    fn every_column_but_the_last_reaches_the_foot() {
        let mut e = engine(Latin::Turn, false);
        let len = 16.0 * 13.3;
        let vt = e.vtext(&runs(LONG), body(), len, 32.0, 0.0, true);
        assert!(vt.cols.len() > 3);
        for (k, col) in vt.cols.iter().enumerate() {
            let (u, y) = col.last().unwrap();
            let end = y + u.eff();
            if k + 1 < vt.cols.len() {
                assert!((end - len).abs() < 1.0 || end > len, "column {k} ends at {end}, not {len}");
            } else {
                assert!(end <= len + 0.5);
            }
        }
    }

    /// **A stop that does not fit** (2026-11-08): it takes the character
    /// before it on to the next column; with `[export] hanging` it hangs
    /// below the column instead.
    #[test]
    fn a_stop_takes_its_character_along_or_hangs() {
        let text = runs("一二三四五六七八九十。后面还有字。");
        let words_of = |vt: &VText| -> Vec<String> { vt.cols.iter().map(|c| c.iter().map(|(u, _)| &vt.shown[u.text.clone()]).collect()).collect() };
        let mut e = engine(Latin::Turn, false);
        let words = words_of(&e.vtext(&text, body(), 160.0, 0.0, 0.0, false));
        assert_eq!(words[0], "一二三四五六七八九", "{words:?}");
        assert!(words[1].starts_with("十。"), "{words:?}");
        let mut e = engine(Latin::Turn, true);
        let words = words_of(&e.vtext(&text, body(), 160.0, 0.0, 0.0, false));
        assert_eq!(words[0], "一二三四五六七八九十。", "hung: {words:?}");
    }

    /// **A vertical page runs to the left** (2026-11-08): the line map grows
    /// from the right edge, each block's line comes back from its place, and
    /// a screen is the window's size.
    #[test]
    fn a_vertical_page_runs_from_the_right_and_follows_the_cursor() {
        let mut text = String::from("# 第一章\n\n");
        for k in 0..30 {
            text.push_str(&format!("第{k}段，那年秋天，我们在山脚下的小站等车，站长说雨要来了。\n\n"));
        }
        let mut p = page(&text, Latin::Turn, 400);
        assert!(p.vertical());
        assert!(p.size().1 > 1500.0, "the page goes on to the left: {}", p.size().1);
        let mut last = -1.0;
        for line in (0..text.lines().count()).step_by(2) {
            let u = p.y_for_line(line);
            assert!(u >= last, "line {line} at {u}, before {last}");
            last = u;
            assert_eq!(p.line_for_y(u + 1.0), line, "line {line}");
        }
        let at = p.y_for_line(20);
        let picture = p.render(at, 400);
        assert_eq!((picture.width, picture.height), (800, 400));
        assert!(picture.rgba.chunks(4).any(|px| px[0] < 100), "something is drawn");
        let shown = p.visible_text(at, 800.0);
        assert!(shown.contains("第10段"), "{shown}");
        assert!(!shown.contains("第2段"), "{shown}");
    }

    /// **A display formula** (2026-11-08): turned, on a column of its own,
    /// two squares from the column's head.
    #[test]
    fn a_display_formula_turns_two_squares_from_the_head() {
        let p = page("前面的字。\n\n$$\nx^2+y^2=1\n$$\n\n后面的字。\n", Latin::Turn, 400);
        let turned: Vec<Rect> = p
            .placed
            .iter()
            .flat_map(|b| b.laid.items.iter())
            .filter_map(|i| match i {
                Item::Turned { r, .. } => Some(*r),
                _ => None,
            })
            .collect();
        assert_eq!(turned.len(), 1);
        let r = turned[0];
        assert!((r.y0 - (20.0 + 32.0)).abs() <= 1.0, "two squares in: {r:?}");
        assert!(r.height() > r.width(), "turned: {r:?}");
    }

    /// **A vertical table** (2026-11-08): its caption first, then the head
    /// row as the rightmost column, each body row a column after it.
    #[test]
    fn a_table_is_set_as_a_nianbiao() {
        let text = "Table: 几个年份\n\n| 年份 | 事件 |\n| --- | --- |\n| 1958 | 建站 |\n| 2003 | 修桥 |\n";
        let p = page(text, Latin::Turn, 400);
        let cols = columns_of(&p);
        let at = |w: &str| cols.iter().position(|c| c == w).unwrap_or_else(|| panic!("{w} in {cols:?}"));
        assert!(at("几个年份") < at("年份"), "{cols:?}");
        assert!(!cols.iter().any(|c| c.contains("Table:")), "{cols:?}");
        assert!(at("事件") < at("1958") && at("1958") < at("建站") && at("建站") < at("2003"), "{cols:?}");
        assert_eq!(p.placed.len(), 1, "the caption is the table's");
    }

    /// **Front matter down the page is framed as wide as its lines**, not
    /// across most of the screen, and the columns of the text follow it.
    #[test]
    fn front_matter_down_the_page_is_framed_as_wide_as_its_lines() {
        let p = page("---\ntitle: 樣張\ndate: 1958\n---\n\n# 標題\n\n那年冬天，雪下得很大。\n", Latin::Turn, 600);
        let frame = p.placed[0].laid.items.iter().find_map(|i| if let Item::Frame { r, .. } = i { Some(*r) } else { None });
        let frame = frame.expect("the frame");
        assert!(frame.width() < 200.0, "{} of 800 across", frame.width());
        let cols = columns_of(&p);
        assert!(cols.iter().any(|c| c.contains("標題")), "{cols:?}");
    }

    /// **Ruby** (2026-11-08): the reading is drawn beside its base, and a
    /// reading longer than its base spaces the base out.
    #[test]
    fn a_reading_stands_beside_its_base() {
        let mut e = engine(Latin::Turn, false);
        let u = e.units(&runs("{这些|zhèxiēzhèxiēzhèxiē}字"), body(), 1000.0);
        assert_eq!(u.readings.len(), 1);
        let reading: f32 = u.readings[0].iter().map(Unit::eff).sum();
        let base: f32 = u.units.iter().filter(|x| x.ruby == Some(0)).map(|x| x.adv).sum();
        assert!(base + 0.5 >= reading, "the base spaced out to {reading}: {base}");
        // Several, each spaced out by a pad before and after its own base
        // and nowhere else.
        let u = e.units(&runs("{这些|zhèxiēzhèxiē}字{那|nànànànà}，{雨|yǔ}。"), body(), 1000.0);
        assert_eq!(u.readings.len(), 3);
        for id in 0..2u16 {
            let at: Vec<usize> = (0..u.units.len()).filter(|&n| u.units[n].ruby == Some(id)).collect();
            assert!(at.windows(2).all(|w| w[1] == w[0] + 1), "{id}: {at:?}");
            let (a, z) = (&u.units[at[0]], &u.units[*at.last().unwrap()]);
            assert!(matches!(a.look, Look::Gap) && !a.glue && matches!(z.look, Look::Gap) && z.glue, "{id}");
            assert!(at[1..at.len() - 1].iter().all(|&n| !matches!(u.units[n].look, Look::Gap) && u.units[n].glue), "{id}");
        }
        let short: Vec<&Unit> = u.units.iter().filter(|x| x.ruby == Some(2)).collect();
        assert!(short.len() == 1 && !matches!(short[0].look, Look::Gap) && !short[0].glue, "a short reading puts in no pad");
        let p = page("{这些|zhèxiē}字。\n", Latin::Turn, 400);
        let glyphs = p.placed[0].laid.items.iter().filter(|i| matches!(i, Item::Glyphs(_))).count();
        assert!(glyphs >= 2, "the base and the reading: {glyphs}");
    }

    /// **A code block is turned whole** (2026-11-08): no text set across,
    /// every glyph turned a quarter turn clockwise, a line to a column, 漢字
    /// too; a line longer than the column wraps at its foot onto the next,
    /// with nothing lost, and the hook in its hang.
    #[test]
    fn a_code_block_is_turned_whole_and_wraps_at_the_foot() {
        let long = "x".repeat(30) + "# 很长的注释" + &"y".repeat(30);
        let text = format!("```\nlet 甲 = 1;\n{long}\n```\n");
        let p = page(&text, Latin::Turn, 300);
        let items = &p.placed[0].laid.items;
        assert!(!items.iter().any(|i| matches!(i, Item::Text { .. })), "nothing set across");
        let turned = |i: &Item| matches!(i, Item::Glyphs(r) if r.transform.as_coeffs()[1] > 0.99);
        assert!(items.iter().any(turned), "turned glyphs");
        assert!(items.iter().all(|i| !matches!(i, Item::Glyphs(_)) || turned(i)), "every glyph turned");
        let hooks = items.iter().filter(|i| matches!(i, Item::Lines { .. })).count();
        let cols = &p.placed[0].laid.cols;
        let words: Vec<&str> = cols.iter().map(|c| c.1.as_str()).collect();
        assert_eq!(words[0], "let 甲 = 1;", "{words:?}");
        assert!(words.len() >= 3, "the long line wraps: {words:?}");
        assert_eq!(hooks, 2 * (words.len() - 2), "a hook of two strokes a wrap: {words:?}");
        assert_eq!(words[1..].concat(), long, "nothing lost: {words:?}");
        // Each line on the left of the one before, a line's pitch apart.
        assert!(cols.windows(2).all(|w| w[1].0 > w[0].0), "{cols:?}");
    }

    /// **Inline code is turned whole** (2026-11-08), 漢字 in it too, under
    /// either `vertical_latin`: never upright, never across in one square.
    #[test]
    fn inline_code_is_turned_whole_under_both_latins() {
        for latin in [Latin::Turn, Latin::Jlreq] {
            let mut e = engine(latin, false);
            let l = looks(&mut e, "写 `取值(x, 12)` 和 `N` 吧。");
            let code: Vec<&(String, &str)> = l.iter().filter(|x| "取值(x,12)N".contains(x.0.trim()) && !x.0.trim().is_empty()).collect();
            assert!(!code.is_empty() && code.iter().all(|x| x.1 == "turned"), "{latin:?}: {l:?}");
            assert_eq!(how(&l, "写"), ["up"], "{latin:?}: {l:?}");
        }
    }

    /// **Across again** (2026-11-08): the page is the horizontal one.
    #[test]
    fn set_across_again_the_page_is_horizontal() {
        let mut p = page(&format!("{LONG}\n"), Latin::Turn, 400);
        let long = p.size().1;
        p.set_vertical(None);
        assert!(!p.vertical());
        assert!(p.visible_text(0.0, 400.0).contains("那一年秋天"));
        assert_ne!(p.size().1, long);
    }

    /// **A table wider than the column holds goes on in a second band**
    /// (2026-11-09): twelve columns down a short page all stand above its
    /// foot, every cell's words drawn.
    #[test]
    fn a_table_longer_than_the_column_stays_above_its_foot() {
        let mut e = engine(Latin::Turn, false);
        let text = "| 一 | 二 | 三 | 四 | 五 | 六 | 七 | 八 | 九 | 十 | 十一 | 十二 |\n|---|---|---|---|---|---|---|---|---|---|---|---|\n| 甲 | 乙 | 丙 | 丁 | 戊 | 己 | 庚 | 辛 | 壬 | 癸 | 子 | 丑 |\n";
        let block = crate::parse::parse(text).into_iter().next().unwrap();
        for len in [120.0f32, 200.0, 400.0] {
            let laid = e.vtop(&block, None, Page { top: 0.0, len, width: 800.0 });
            for item in &laid.items {
                if let Item::Rect { r, .. } = item {
                    assert!(r.y1 <= f64::from(len) + 1.0, "a line ends at {} on a column {len} long", r.y1);
                }
            }
            let words: String = laid.cols.iter().map(|c| c.1.as_str()).collect();
            for cell in ["一", "十", "十一", "十二", "甲", "癸", "子", "丑"] {
                assert!(words.contains(cell), "{cell} at {len}: {words}");
            }
        }
        // Where it fits, it is one table, as before.
        let laid = e.vtop(&block, None, Page { top: 0.0, len: 2000.0, width: 800.0 });
        let lines = laid.items.iter().filter(|i| matches!(i, Item::Rect { .. })).count();
        assert_eq!(lines, 13 + 3, "a line between each two of twelve bands and round them, and three across");
    }

    /// **A figure is measured from its picture's header** (2026-11-09), not
    /// drawn at full size first: what is kept is the picture read and the
    /// picture at the size shown, not a full-size copy besides.
    #[test]
    fn a_figure_is_measured_without_drawing_it_at_full_size() {
        let dir = std::env::temp_dir().join(format!("yumete-vfigure-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        image::RgbaImage::from_pixel(2000, 1500, image::Rgba([200, 100, 50, 255])).save(dir.join("big pic.png")).unwrap();
        let mut e = Engine::new(Theme::light(), 1.0, dir.clone(), Box::new(Boxes));
        e.vertical = Some(Vertical { latin: Latin::Turn, hanging: false });
        assert_eq!(e.image_size("big%20pic.png"), Some((2000.0, 1500.0)));
        let block = crate::parse::parse("![一张大图](big%20pic.png)\n").into_iter().next().unwrap();
        let laid = e.vtop(&block, None, Page { top: 0.0, len: 300.0, width: 800.0 });
        let _ = std::fs::remove_dir_all(&dir);
        let pic = laid.items.iter().find_map(|i| if let Item::Pic { r, .. } = i { Some(*r) } else { None }).expect("a picture");
        // As wide as most of the window allows, 360 of 800, and as tall as that makes it.
        assert!((pic.width() - 360.0).abs() < 1.5 && (pic.height() - 270.0).abs() < 1.5, "{pic:?}");
        let raw = 2000 * 1500 * 4;
        assert!(e.kept().1 < raw + raw / 4, "kept {} bytes, the picture is {raw}", e.kept().1);
    }

    /// **Quotes round Western words stay curly** (2026-11-09), as the export
    /// keeps them; a pair holding Chinese turns.
    #[test]
    fn quotes_round_western_words_stay_curly_as_in_the_export() {
        let p = page("He said “go” and ‘stay’, the end of it.\n", Latin::Turn, 600);
        let all = columns_of(&p).concat();
        assert!(all.contains("“go”") && all.contains("‘stay’") && !all.contains('『'), "{all}");
        let p = page("他说“go”，又说“走”。\n", Latin::Turn, 600);
        let all = columns_of(&p).concat();
        assert!(all.contains("“go”") && all.contains("『走』"), "{all}");
    }

    /// **Under jlreq a single small letter stands upright** (2026-11-09), as
    /// a capital and `$x$` do; a word of small letters is turned.
    #[test]
    fn under_jlreq_a_single_small_letter_stands_upright() {
        let mut e = engine(Latin::Jlreq, false);
        let l = looks(&mut e, "变量 x 和 A，用 xy 写，又写 $x$。");
        assert_eq!(how(&l, "x"), ["across"], "upright, one to a square: {l:?}");
        assert_eq!(how(&l, "A"), ["across"], "{l:?}");
        assert_eq!(how(&l, "xy"), ["turned"], "{l:?}");
        assert_eq!(l.iter().filter(|x| x.1 == "pic upright").count(), 1, "{l:?}");
    }

    /// **A lone mark stands as the export's page stands it** (2026-11-09):
    /// `×` and `±` upright, as UAX #50 has them, `·` and `/` turned.
    #[test]
    fn a_lone_mark_stands_as_the_export_stands_it() {
        let mut e = engine(Latin::Turn, false);
        let l = looks(&mut e, "甲×乙±丙·丁/戊");
        assert_eq!(how(&l, "×"), ["up"], "{l:?}");
        assert_eq!(how(&l, "±"), ["up"], "{l:?}");
        assert_eq!(how(&l, "·"), ["turned"], "{l:?}");
        assert_eq!(how(&l, "/"), ["turned"], "{l:?}");
    }
}
