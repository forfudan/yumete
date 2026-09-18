//! One floating panel, and everything that used to draw its own (#299).
//!
//! A ring, a gold name in its top-left, and something inside — the shape #273
//! gave the `:` menu. Four things on the screen were already this shape and
//! each drew its own: the `空格` which-key, the footnote and `%%註釋%%` strip
//! (#294), `:write`'s safety check (#295), and the command row. What they differ
//! in is four parameters — **title, body, a tag, and where it stands** — so
//! that is what this takes.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::Frame;
use yumete_config::Config;

use crate::put_text;

/// What a panel holds. Two shapes, because there are two things to say: a
/// paragraph to read, or a list of keys to glance down.
pub enum Body {
    /// Prose, wrapped across the panel — a footnote, a comment, a warning.
    Prose(String),
    /// Key and meaning, laid out in one or two columns and left-aligned as a
    /// block: a ragged left edge on a list of two-character keys reads as
    /// noise.
    Keys(Vec<(String, String)>),
}

/// A floating panel: what it is called, what it holds, and one short thing
/// said quietly at the far end of its bottom edge.
pub struct Panel {
    pub title: String,
    pub body: Body,
    /// 「第 11 行」 — where the thing being shown is written. Right-aligned on
    /// the bottom border, out of the reading path.
    pub tag: Option<String>,
}

/// Wrap `text` to `width` cells, by the rules the page itself wraps by.
///
/// The manuscript's own wrapper (`yumete_core::wrap`), not a loop over
/// graphemes: a footnote is Chinese prose, so 禁則處理 applies to it exactly as
/// it does to the paragraph it belongs to — a 。 may not open a row, a 「 may
/// not close one. Wrapping it any other way would put a full stop alone at the
/// head of the panel's second line, which is the one thing everybody notices.
fn wrap(text: &str, width: usize) -> Vec<String> {
    // A line break in the text is a line break in the panel: a wiki entry is
    // several paragraphs and a heading, not one run of prose (#287).
    let mut rows: Vec<String> = Vec::new();
    for paragraph in text.split('\n') {
        let chars: Vec<char> = paragraph.chars().collect();
        if chars.is_empty() {
            rows.push(String::new());
            continue;
        }
        rows.extend(
            yumete_core::wrap::line_rows(paragraph, width)
                .into_iter()
                .map(|(from, to)| chars[from.min(chars.len())..to.min(chars.len())].iter().collect::<String>())
                .filter(|row| !row.is_empty()),
        );
    }
    // A note that fills its last row exactly gets an empty row after it —
    // right on the page, where the caret has to have somewhere to stand past
    // the final character, and wrong in a ring, where it is a blank row the
    // reader is asked to look at. The panel is drawn, not typed into.
    while rows.len() > 1 && rows.last().is_some_and(|r| r.is_empty()) {
        rows.pop();
    }
    rows
}

/// Draw `panel` inside `area`, keeping off `bottom` and out of the caret's way.
///
/// Returns the rectangle it covered, so the caller can add it to the list
/// `:view-hud full` reads — a panel it covered would be a panel with a hole
/// in it.
pub fn draw(
    frame: &mut Frame,
    config: &Config,
    area: Rect,
    bottom: u16,
    caret: (u16, u16),
    panel: &Panel,
) -> Option<Rect> {
    let ink = crate::theme::Palette::of(config);
    let title_w = yumete_cjk::str_width(&panel.title);
    let tag_w = panel.tag.as_deref().map(yumete_cjk::str_width).unwrap_or(0);
    // The ring costs two cells across and two rows down; the bottom edge has
    // to hold the tag as well as a corner.
    let widest = (area.width as usize).saturating_sub(2);
    if widest == 0 {
        return None;
    }

    // How wide the content wants to be, how deep it is at that width, and —
    // for a key list — how wide one of its columns is.
    // Prose gets a cell of air on each side; a key list does not — its own
    // left column is the alignment, and an indent only pushes it off centre.
    let pad: usize = match panel.body {
        Body::Prose(_) => 1,
        Body::Keys(_) => 0,
    };
    // `lines` is the prose, already wrapped; a key list draws itself from
    // `panel.body` and needs only its `count`.
    let (inner, count, lines, columns, key_w, one) = match &panel.body {
        Body::Prose(text) => {
            // **A quarter of the page at most: half the width and half the
            // height** (2026-09-17). Then whichever corner the panel takes,
            // the caret's own corner is outside it — 「光標在任何位置，面板都
            // 一定在它對面的屏幕裏」 — and the page keeps the other half of
            // both. It used to be allowed two thirds of the width, which could
            // reach across the middle and cover the caret it had moved to
            // avoid.
            let want = widest.saturating_sub(2).min(((area.width as usize) / 2).max(24));
            let longest = text.split('\n').map(yumete_cjk::str_width).max().unwrap_or(0);
            let inner = longest.min(want).max(1);
            let lines = wrap(text, inner);
            // Shrink to the longest line actually drawn: wrapping a 40-cell
            // note at 60 leaves twenty cells of ring around nothing.
            let inner = lines.iter().map(|l| yumete_cjk::str_width(l)).max().unwrap_or(1).max(1);
            (inner, lines.len(), lines, 1usize, 0usize, inner)
        }
        Body::Keys(keys) => {
            if keys.is_empty() {
                return None;
            }
            let key_w = keys.iter().map(|(k, _)| yumete_cjk::str_width(k)).max().unwrap_or(1);
            let one = keys
                .iter()
                .map(|(k, what)| key_w.max(yumete_cjk::str_width(k)) + 2 + yumete_cjk::str_width(what))
                .max()
                .unwrap_or(0);
            // **Half the page and no more.** A menu is a thing you glance at
            // beside your writing; one that fills the window has stopped
            // being a menu.
            let room = (area.height.saturating_sub(2) / 2).max(1) as usize;
            let columns = if keys.len() > room { 2 } else { 1 };
            // Each column gets its share, and what does not fit is cut
            // *inside* the column rather than beyond the border — where it
            // used to be dropped silently, leaving keys with no meanings.
            let one = one.min(widest.saturating_sub((columns - 1) * 2) / columns.max(1));
            (
                one * columns + (columns - 1) * 2,
                keys.len(),
                Vec::new(),
                columns,
                key_w,
                one,
            )
        }
    };

    // **Prose too tall for half the page is cut, not dropped** (2026-09-17).
    // The cap below used to refuse the whole panel — so a long footnote, or a
    // wiki entry of several paragraphs, simply did not appear, which reads as
    // 「the panel is unreliable」 rather than as 「there is no room」. A cut body
    // says what it can and ends in an ellipsis; the tag on the bottom border
    // still says where to read the rest.
    let cap = (area.height / 2 + 1).saturating_sub(2) as usize;
    let (count, lines) = match &panel.body {
        Body::Prose(_) if count > cap && cap > 0 => {
            let mut kept: Vec<String> = lines.into_iter().take(cap).collect();
            if let Some(last) = kept.last_mut() {
                *last = "…".to_string();
            }
            (kept.len(), kept)
        }
        _ => (count, lines),
    };
    let deep = count.div_ceil(columns);
    let width = (inner + 2 + pad * 2)
        .max(title_w + 4)
        .max(tag_w + 4)
        .min(match &panel.body {
            // Prose keeps to half the width (above); a key menu has its own
            // rule about how many columns it may spread into.
            Body::Prose(_) => (area.width as usize / 2).max(24),
            Body::Keys(_) => area.width as usize,
        })
        .min(area.width as usize) as u16;
    let height = (deep + 2) as u16;
    // It may take half the page and no more, and it must leave the page
    // something: at a very small window there is nowhere to put a panel, and
    // covering the manuscript with one is worse than not drawing it.
    if height > area.height / 2 + 1 || bottom < area.y + height || width < 8 {
        return None;
    }

    // **The corner the caret is not in.** A fixed corner is right half the
    // time and covers what you are working on the other half. One rule for
    // both layouts, because in both of them the caret has a column.
    let far = area.x + area.width.saturating_sub(width);
    let x = match caret.0 >= area.x + area.width / 2 {
        true => area.x,
        false => far,
    };
    // Along the bottom, unless the caret is standing in the rows the panel
    // would take — then the top, which is the corner diagonally opposite.
    // Asked as 「would this cover the line you are on」 rather than 「is the
    // caret low on the page」: the panel only has to move when it is actually
    // in the way, and a panel that jumps to the top the moment you pass the
    // middle of the page is a panel that moves for no reason.
    let low = bottom - height;
    let y = match caret.1 >= low && caret.1 < bottom {
        true => area.y,
        false => low,
    };
    let rect = Rect::new(x, y, width, height);
    crate::chrome::draw(frame, rect, &crate::chrome::Ring {
        rounded: config.panel.rounded,
        // `rule()`, the rung every other ring on the screen is drawn at.
        border: Style::default().fg(ink.rule()).bg(ink.paper()),
        ground: Style::default().bg(ink.paper()),
        title: Some((
            panel.title.clone(),
            Style::default().fg(ink.gold()).bg(ink.paper()),
        )),
    });
    let ground = Style::default().bg(ink.paper());
    let limit = rect.x + width - 1;
    let buf = frame.buffer_mut();
    match &panel.body {
        Body::Prose(_) => {
            for (i, line) in lines.iter().enumerate() {
                let x = rect.x + 1 + pad as u16;
                put_text(buf, x, rect.y + 1 + i as u16, limit, line, ground.fg(ink.text()));
            }
        }
        Body::Keys(keys) => {
            for (i, (key, what)) in keys.iter().enumerate() {
                let (column, row) = (i / deep, i % deep);
                let x = rect.x + 1 + (column * (one + 2)) as u16;
                let y = rect.y + 1 + row as u16;
                put_text(buf, x, y, limit, key, ground.fg(ink.gold()));
                put_text(buf, x + key_w as u16 + 2, y, limit, what, ground.fg(ink.text()));
            }
        }
    }
    // Quietly, on the bottom edge and hard right: it is where the thing is
    // written, not part of what it says.
    if let Some(tag) = &panel.tag {
        let x = limit.saturating_sub(tag_w as u16 + 1);
        put_text(buf, x, rect.y + height - 1, limit, tag, ground.fg(ink.quiet()));
    }
    Some(rect)
}
