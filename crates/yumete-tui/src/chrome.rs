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
pub fn room(area: Rect, vertical: bool) -> (u16, u16) {
    match vertical {
        false => (area.width * 2 / 3, area.height / 3),
        true => (area.width / 3, area.height * 2 / 3),
    }
}

pub enum Anchor {
    /// **The corner the caret is not in.** A fixed corner is right half the
    /// time and covers what is being worked on the other half. One rule for
    /// both layouts, because in both of them the caret has a column.
    ///
    /// `bottom` is the row the floats stack up from — the top of the footer;
    /// `vertical` picks which way [`room`] is turned.
    Caret { at: (u16, u16), bottom: u16, vertical: bool },
    /// The middle of the page: the picker and the question, both of which hold
    /// every key while they are open.
    Centre,
}

/// Fit a box `want` cells big into `area`, or `None` when there is no room.
///
/// ⚠️ **A caret-anchored box is held to [`room`]** — 2026-09-17 「面板不應該
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
        Anchor::Caret { at, bottom, vertical } => {
            // ⚠️ **The policy caps belong to the caller, this only asks
            // whether it physically fits** (2026-09-18). A 竪書 note is two
            // thirds of the page *tall* by design — half-page refusal here
            // threw it away and nothing was drawn at all. What protects the
            // caret is that one axis always separates them: 橫排 the box is a
            // third tall and stands in the third the caret is not in; 竪排 it
            // is a third wide and stands in the third the caret is not in.
            let _ = vertical;
            if bottom < area.y + height {
                return None;
            }
            let far = area.x + area.width.saturating_sub(width);
            let x = match at.0 >= area.x + area.width / 2 {
                true => area.x,
                false => far,
            };
            // Along the bottom, unless the caret is standing in the rows the
            // box would take — then the top, which is the corner diagonally
            // opposite. Asked as 「would this cover the line you are on」
            // rather than 「is the caret low on the page」: it only has to move
            // when it is actually in the way, and one that jumps to the top
            // the moment you pass the middle of the page moves for no reason.
            let low = bottom - height;
            let y = match at.1 >= low && at.1 < bottom {
                true => area.y,
                false => low,
            };
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
