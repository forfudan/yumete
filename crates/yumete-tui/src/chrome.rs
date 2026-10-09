//! **One ring, drawn one way** (2026-09-18).
//!
//! Everything this front end floats over the page is a box with the same
//! four parts: the ground it clears, the ring around it, a name in the
//! top-left, and — the part nobody remembers — the 漢字 half-cell rule. That
//! was written out eight times: the 浮框 (`panel.rs`), the 鍵表 and 命令選單
//! (`draw_list`), the 問句 (`draw_query`), the 挑選器 and its preview, and the
//! 候選欄's two (`vertical.rs`, `draw_candidates`). Eight copies of a rule is
//! seven places for it to be forgotten, and 「竪排提示框缺一角」 is what being
//! forgotten looks like.
//!
//! The colours come in as [`Style`]s rather than as a palette, because the two
//! families do not share one: the editor's surfaces are drawn from
//! [`crate::theme::Palette`], the candidate list from the IME's own skin. What
//! they do share is the shape, and the shape is what lives here.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Block, BorderType, Borders, Clear, Widget};
use ratatui::Frame;

/// What a ring looks like. The rectangle it goes on is [`draw`]'s argument.
pub struct Ring {
    /// `[panel] rounded` — the reader's own answer, asked of the config by the
    /// caller so this file never reads settings.
    pub rounded: bool,
    /// The ring itself: its ink on the panel's ground.
    pub border: Style,
    /// The ground, painted over the whole rectangle.
    pub ground: Style,
    /// The name in the top-left corner, in whatever ink names are drawn in
    /// here — 金 for the editor's panels. `None` for a ring with no name.
    pub title: Option<(String, Style)>,
    /// **A word on the bottom wall**, where the title is on the top one
    /// (2026-10-06 定：「can cover the border (at the left side of the bottom
    /// border, just like other temp panels) to save one line」).
    ///
    /// The counter 「1/47」 lives here. It is about the panel rather than about
    /// anything in it, which is what a frame is for — and it costs no row.
    pub foot: Option<(String, Style)>,
}

/// Clear `rect`, draw the ring on it, and hand back the inside.
///
/// The returned rectangle is the block's own `inner` — one cell in on every
/// side — so a caller never works the border's width out for itself.
pub fn draw(frame: &mut Frame, rect: Rect, ring: &Ring) -> Rect {
    frame.render_widget(Clear, rect);
    // **A 漢字 cannot be covered by halves** (#286). It owns two cells, and the
    // renderer skips whatever a wide glyph covers — so a border written into
    // the second of them is stored and then never emitted, and the panel opens
    // with its whole left wall missing. Blank the glyph; the wall gets a cell.
    crate::vertical::clear_wide_left_edge(frame.buffer_mut(), rect);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(match ring.rounded {
            true => BorderType::Rounded,
            false => BorderType::Plain,
        })
        .border_style(ring.border)
        .style(ring.ground);
    if let Some((text, style)) = &ring.title {
        // **One cell in from the corner** (2026-09-18: 「能不能往右移一格
        // 半角，這樣就能對齊」). The ring's contents start one cell past the
        // wall; a title written hard against the corner sits a cell to their
        // left, and the two left edges being nearly-but-not-quite the same is
        // worse than either alignment on its own.
        block = block.title(Span::styled(format!(" {text}"), *style));
    }
    let inner = block.inner(rect);
    block.render(rect, frame.buffer_mut());
    // The foot is written after the wall, over it: one cell in from the
    // corner, the same inset as the title.
    if let Some((text, style)) = &ring.foot {
        if rect.height >= 2 {
            let y = rect.y + rect.height - 1;
            let room = rect.width.saturating_sub(2) as usize;
            let mut x = rect.x + 1;
            let buf = frame.buffer_mut();
            for c in text.chars() {
                let w = yumete_cjk::str_width(&c.to_string()) as u16;
                if (x + w).saturating_sub(rect.x + 1) as usize > room {
                    break;
                }
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_symbol(&c.to_string()).set_style(*style);
                }
                x += w;
            }
        }
    }
    inner
}

/// **Where a floating thing stands** (2026-09-18).
///
/// Two answers, and the difference is whether the reader is still writing:
/// something to glance at keeps out of the caret's way, and something that has
/// taken the keys stands in the middle, where it cannot be missed.
/// **How much of the page a floating box may take** (2026-09-18 定).
///
/// 橫排 two thirds wide and one third tall; 竪排 the transpose. Two numbers,
/// and each of them answers a different question:
///
/// - **The short side protects the caret.** A box one third tall stands in
///   the top third or the bottom third, and the caret is in neither — which is
///   what the old 「half and half」 rule was really buying. Once the height
///   does that job the width is free to be chosen for reading.
/// - **The long side is the measure.** Two thirds of a 118-column page is 39
///   漢字 to a line, which is what prose wants; the whole width would be 59,
///   past what an eye reads without losing its place, and it would take those
///   rows away from the manuscript **whole** rather than by halves.
///
/// 竪排 turns both round, and for the same reason: there a wide box cuts the
/// tops off many 縱 at once (「這樣不會打破行文」).
///
/// **A key table is scanned once rather than read**, so neither argument above
/// is about it: there is no measure to keep to, and a third of a 24-row
/// terminal is six rows inside the ring — which the 23 items of the `空格`
/// menu do not fit into at any column count. Measured 2026-10-09 against the
/// reading room: 100×20 loses 4 of them, 100×16 loses 14, and 80×12 is cut to
/// three. So it takes the whole width and **half** the page.
///
/// Half, and no longer the half-plus-one it took until 2026-10-09: 「超過一半
/// 確實危險，寧可更寬」. The row it gave up costs nothing above 16 rows — the
/// menu reaches that cap only on a short terminal, and it answers depth it
/// cannot have by spreading into another column.
/// **A field list sits between the two.** 記錄 and the five 信息 panels are
/// read, so the measure applies to their width — but a column of
/// `name  value` pairs is not a paragraph, and a third of a short terminal is
/// two rows inside the ring, too few to wrap a cell value into. They take the
/// reading width and the scanning depth.
pub fn room(area: Rect, vertical: bool, what: Use) -> (u16, u16) {
    match (what, vertical) {
        (Use::Read, false) => (area.width * 2 / 3, area.height / 3),
        (Use::Read, true) => (area.width / 3, area.height * 2 / 3),
        (Use::Fields, false) => (area.width * 2 / 3, area.height / 2),
        (Use::Fields, true) => (area.width / 2, area.height * 2 / 3),
        // Layout does not turn this one round: a key table is drawn across in
        // both layouts, so there is no 縱 for a wide box to cut the top off.
        (Use::Scan, _) => (area.width, area.height / 2),
    }
}

/// **What a float is for**, which is the whole of what its size depends on.
///
/// Three kinds, and before 2026-10-09 they were three expressions in three
/// places: the reading room here, `area.height / 2 + 1` in `panel::draw`, and
/// `(area.height - 2) / 2` beside the key table — the last two spellings of
/// one intent, kept in step by hand, and the field list silently taking one
/// from each.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Use {
    /// Prose: a 百科 entry, a footnote, a note about the text.
    Read,
    /// `name  value` pairs: 記錄 and the five 信息 panels.
    Fields,
    /// A key table: scanned once, for the one row being looked for.
    Scan,
}

pub enum Anchor {
    /// **One side is fixed, the other flips** (2026-10-09 定).
    ///
    /// 橫排 the box stands at the **right** and moves only up and down:
    /// 「we always write from left to right, so the line start can always be
    /// visible」. What a float may cover is the tail of the lines it sits on,
    /// never their beginnings — and a float that also slid left and right
    /// jumped across the page as the caret passed the middle column, for no
    /// reason the reader could see.
    ///
    /// 竪排 is the same argument turned: there the 縱 run top to bottom and
    /// right to left, so the box stands at the **bottom** and flips left and
    /// right, 「面板永遠在下而不是在右」, defaulting to the left — the side
    /// the writing has not reached yet.
    ///
    /// `bottom` is the row the floats stack up from — the top of the footer;
    /// `vertical` picks which way [`room`] is turned, and which side is fixed.
    Caret { at: (u16, u16), bottom: u16, vertical: bool },
    /// **貼着光標，畫在它下面那一行**（2026-10-08 定）。
    ///
    /// 補全那張單子用的是這一個，而不是「光標不在的那個角」。原話：「this
    /// autocompletion seems to be better below the cursor (as the best practice)」——
    /// 三家都這樣（VS Code、helix 的 `completion`、nvim-cmp），理由是這張單子
    /// 說的是「你剛打的那半個詞接下來是什麼」，眼睛要在詞與單子之間來回；
    /// 擺到屏幕角上就是把那一來回拉成半個屏幕。
    ///
    /// 下面裝不下就畫在上面（同三家）；左邊貼光標那一欄，靠右最多推到裝得下為止。
    UnderCaret { at: (u16, u16), bottom: u16 },
    /// The middle of the page: the picker and the question, both of which hold
    /// every key while they are open.
    Centre,
}

/// Fit a box `want` cells big into `area`, or `None` when there is no room.
///
/// Warning: **A caret-anchored box is held to [`room`]** — 2026-09-17 「面板不應該
/// 超過頁面的四分之一」, refined 2026-09-18 to two thirds by one third, which
/// is two ninths and reads better. A centred one has the keys, so the page
/// behind it is not being read and the cap does not apply.
pub fn place(area: Rect, want: (u16, u16), anchor: Anchor) -> Option<Rect> {
    let (width, height) = want;
    if width < 8 || width > area.width || height > area.height {
        return None;
    }
    match anchor {
        Anchor::Centre => Some(Rect::new(
            area.x + (area.width.saturating_sub(width)) / 2,
            area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        )),
        Anchor::UnderCaret { at, bottom } => {
            // 下面裝得下就畫在下面，否則翻到上面。`bottom` 是頁腳那一行，
            // 所以比的是「光標這一行之下、頁腳之上」還有幾行。
            let y = match bottom.saturating_sub(at.1 + 1) >= height {
                true => at.1 + 1,
                // 上面也裝不下就貼頂——蓋住幾行正文，總比一格都不畫好。
                false => at.1.saturating_sub(height).max(area.y),
            };
            let far = area.x + area.width.saturating_sub(width);
            Some(Rect::new(at.0.min(far), y, width, height))
        }
        Anchor::Caret { at, bottom, vertical } => {
            // Warning: **The policy caps belong to the caller, this only asks
            // whether it physically fits** (2026-09-18). A 竪書 note is two
            // thirds of the page *tall* by design — half-page refusal here
            // threw it away and nothing was drawn at all.
            if bottom < area.y + height {
                return None;
            }
            let right = area.x + area.width.saturating_sub(width);
            let low = bottom - height;
            // **Four corners, tried in order**, the first that does not sit on
            // the caret. The fixed side comes first twice, so the box only
            // leaves it when both of its corners are in the way.
            let corners = match vertical {
                false => [(right, low), (right, area.y), (area.x, low), (area.x, area.y)],
                true => [(area.x, low), (right, low), (area.x, area.y), (right, area.y)],
            };
            // 「Would this cover the place you are typing」, asked of the whole
            // box rather than of its rows: with the column fixed, a caret at
            // the start of a long line is not under a box that stands at the
            // right of the page, and a float that moved for it would be
            // moving for nothing.
            let covers = |(x, y): &(u16, u16)| {
                at.0 >= *x && at.0 < x + width && at.1 >= *y && at.1 < y + height
            };
            // All four covered means the box is wider than half the page and
            // taller than half of it too. Draw it at the first corner anyway:
            // covering the caret is worse than not covering it, and better
            // than drawing nothing at all (the same answer `UnderCaret` gives).
            let (x, y) = corners.iter().find(|c| !covers(c)).copied().unwrap_or(corners[0]);
            Some(Rect::new(x, y, width, height))
        }
    }
}

/// **The ground everything that floats stands on** — 第 85 檔 (2026-09-18).
///
/// One function rather than a constant spelled at each call, because the
/// mistake this exists to prevent is *half* a panel changing: the ring was
/// moved off the paper's colour first and the rows inside it were not, so every
/// line of the `:` menu dragged a patch of the old colour behind it. The ring
/// and its contents read this.
pub fn panel_ground(ink: crate::theme::Palette) -> ratatui::style::Color {
    ink.at(yumete_config::rung::FLOAT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **貼着光標那一種：下面放得下就放下面，放不下翻到上面**（2026-10-08）。
    #[test]
    fn a_box_under_the_caret_stands_on_the_next_line() {
        let page = Rect::new(0, 0, 80, 24);
        // 腳着那一行是 22（底下兩行歸狀態欄與提示行）。
        let bottom = 22;
        let under = |at: (u16, u16), height: u16| {
            place(page, (20, height), Anchor::UnderCaret { at, bottom }).expect("放得下")
        };
        // 光標在第 3 行、第 12 欄：盒子的左上角就在它正下方。
        let it = under((12, 3), 6);
        assert_eq!((it.x, it.y), (12, 4), "左邊貼光標那一欄，上邊貼它下一行");
        // 下面只剩兩行，盒子要六行：翻到上面去，底邊挨着光標那一行。
        let it = under((12, 20), 6);
        assert_eq!((it.x, it.y), (12, 14), "裝不下就畫在上面");
        // 靠右的光標：往左推到裝得下為止，不許伸出紙外。
        let it = under((75, 3), 6);
        assert_eq!((it.x, it.y), (60, 4), "靠右就推回來");
    }

    /// **一邊釘死，另一邊翻**（2026-10-09 定）。橫排釘右邊，竪排釘下邊；
    /// 只有當那個角真的壓在光標上纔換一個角。
    #[test]
    fn a_caret_anchored_box_keeps_its_fixed_side_and_flips_on_the_other() {
        let page = Rect::new(0, 0, 80, 24);
        let bottom = 22;
        let at = |want, caret, vertical| {
            let it = place(page, want, Anchor::Caret { at: caret, bottom, vertical })
                .expect("放得下");
            (it.x, it.y)
        };
        // 橫排：盒子 40 寬擺在右邊就是第 40 欄起，下邊是第 14 行起。
        let small = (40, 8);
        // 光標在左上 —— 右下那個角本來就不壓它。
        assert_eq!(at(small, (2, 2), false), (40, 14), "右下");
        // 光標在**左下**：行還是那幾行，可盒子在右半邊，壓不到它。
        // 從前這一格會翻到上面去，而那一翻什麽也沒買到。
        assert_eq!(at(small, (2, 18), false), (40, 14), "左下的光標不讓它動");
        // 光標在右下 —— 這纔是真的壓上了，翻到右上。
        assert_eq!(at(small, (60, 18), false), (40, 0), "右上");
        // **兩個右角一起壓着纔讓出橫軸。** 要兩個都壓得到，盒子得高過半頁：
        // 12 行的盒子，上角佔 0..12、下角佔 10..22，第 11 行兩個都在。
        assert_eq!(at((40, 12), (60, 11), false), (0, 10), "左下");

        // 竪排轉九十度：釘下邊，左右翻，缺省朝左——字還沒走到的那一側。
        assert_eq!(at(small, (60, 2), true), (0, 14), "左下");
        assert_eq!(at(small, (2, 2), true), (0, 14), "上面的光標不讓它動");
        assert_eq!(at(small, (2, 18), true), (40, 14), "右下");
        // 同樣地，兩個下角一起壓着纔讓出竪軸：50 寬的盒子左角佔 0..50、
        // 右角佔 30..80，第 40 欄兩個都在。
        assert_eq!(at((50, 8), (40, 18), true), (0, 0), "兩個下角都壓着就上去");
    }
}
