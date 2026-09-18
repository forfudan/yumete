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
        block = block.title(Span::styled(text.clone(), *style));
    }
    let inner = block.inner(rect);
    block.render(rect, frame.buffer_mut());
    inner
}
