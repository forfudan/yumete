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
    /// **Whether the prose inside runs down the page** (作者 2026-09-18:
    /// 「只有百科才需要縱書，其他的都保持橫排」).
    ///
    /// Not the same question as the panel's *shape*, which follows the page
    /// for everything that floats: a key table in a 竪排 page is still a table
    /// of keys and is read across, while a 百科 entry is prose and a horizontal
    /// paragraph in the middle of a vertical page is a change of gear the eye
    /// has to make for no reason.
    pub vertical_text: bool,
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
    vertical: bool,
    panel: &Panel,
) -> Option<Rect> {
    let ink = crate::theme::Palette::of(config);
    // How much of the page this may take — two thirds by one third 橫排, the
    // transpose 竪排 (`chrome::room`).
    //
    // ⚠️ **Prose only.** The 2/3 × 1/3 shape is an argument about *reading*:
    // a measure the eye can take in, and rows of the manuscript covered by
    // halves rather than whole. A key table is not read that way — it is
    // scanned once — and a third of a 24-row terminal is seven rows, which
    // the `空格` menu does not fit into at all: it simply stopped being drawn.
    // Menus keep the older rule (half the page, as many columns as fit).
    let prose = matches!(panel.body, Body::Prose(_));
    let (room_w, room_h) = match prose {
        true => crate::chrome::room(area, vertical),
        false => (area.width, area.height / 2 + 1),
    };
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
        // **竪書的正文**（作者 2026-09-18）：一縱是一列，字往下走，縱往左排。
        // 量法轉置——能放多高就一縱多少字，需要幾縱就多寬（一個漢字兩格）。
        Body::Prose(text) if panel.vertical_text => {
            let tall = (room_h as usize).saturating_sub(2).max(1);
            // **段落之間什麽都不加**（作者 2026-09-18 定：「縮進不好看，不要
            // 縮進了，也不需要空行，就用最 compact 的狀態」）。空縱與首行縮進
            // 都試過：一個把兩段推得老遠，一個在十來縱的小框裏看着就是斷了。
            // 分段本來就有一縱的邊界在，框小的時候那已經夠了。
            //
            // 折行交給正文自己的折行器（禁則在裏面），寬度按「一縱幾個字」給，
            // 一個漢字兩格，所以乘二。
            let mut zong: Vec<String> = text
                .split('\n')
                .filter(|para| !para.trim().is_empty())
                .flat_map(|para| wrap(para, tall * 2))
                .filter(|z| !z.is_empty())
                .collect();
            // **標題是自己的一縱，排在最右**（作者 2026-09-18 定，三個辦法裏
            // 的丙）。竪排的書就是這麽做的：標題不貼在框邊上，它本身就是第一
            // 縱，金墨。所以這一支不給 `chrome` 標題，框上没有名字。
            let name: String = panel.title.chars().take(tall).collect();
            zong.insert(0, name);
            let deep = zong.iter().map(|z| z.chars().count()).max().unwrap_or(1).max(1);
            let across = zong.len().max(1);
            (across * 2, deep, zong, 1usize, 0usize, across * 2)
        }
        Body::Prose(text) => {
            // **As wide as `chrome::room` allows** (2026-09-18). What keeps
            // the caret uncovered is the *height* — a third of the page stands
            // in the top third or the bottom third and the caret is in
            // neither — so the width is free to be the reading measure
            // instead: two thirds of a 118-column page is 39 漢字 to the line.
            // ⚠️ **The ring and the padding come out of the budget first**
            // (2026-09-18). Wrapping at the room's full width and then
            // clamping the *box* to it left every line two cells too long,
            // and `put_text` stops at the border — so the last 漢字 of each
            // row was eaten: 「執掌法會加冠之」 and no 「禮。」. The bug is
            // older than the 2/3 rule; a half-width panel hid it by rarely
            // reaching the cap.
            let budget = (room_w as usize).saturating_sub(2 + pad * 2);
            let want = widest.saturating_sub(2).min(budget.max(8));
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
    let cap = room_h.saturating_sub(2).max(1) as usize;
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
            // Prose keeps to the room (above); a key menu has its own rule
            // about how many columns it may spread into.
            Body::Prose(_) => (room_w as usize).max(24),
            Body::Keys(_) => area.width as usize,
        })
        .min(area.width as usize) as u16;
    let height = (deep + 2) as u16;
    // Where it stands, and whether there is room at all — one rule, in
    // `chrome`, shared with everything else that floats (2026-09-18).
    let rect = crate::chrome::place(
        area,
        (width, height),
        crate::chrome::Anchor::Caret { at: caret, bottom, vertical },
    )?;
    crate::chrome::draw(frame, rect, &crate::chrome::Ring {
        rounded: config.panel.rounded,
        // `rule()`, the rung every other ring on the screen is drawn at.
        border: Style::default().fg(ink.rule()).bg(crate::chrome::panel_ground(ink)),
        ground: Style::default().bg(crate::chrome::panel_ground(ink)),
        // Vertical prose carries its name as its first 縱 (below), so the
        // ring is left bare — a name in both places would be the same word
        // twice, two cells apart.
        title: match panel.vertical_text && prose {
            true => None,
            false => Some((
                panel.title.clone(),
                Style::default().fg(ink.gold()).bg(crate::chrome::panel_ground(ink)),
            )),
        },
    });
    // ⚠️ **The body sets no ground of its own** (作者 2026-09-18: 「命令行文字
    // 嚴格意義上來說底色是透明的，下面是什麽顏色就是什麽底色」). The ring has
    // already painted the panel; text that carried its own copy of that colour
    // dragged a patch of the *old* one behind every line the day the panel's
    // ground moved. `Cell::set_style` patches, so a style with no `bg` keeps
    // whatever is under it.
    let ground = Style::default();
    let limit = rect.x + width - 1;
    let buf = frame.buffer_mut();
    match &panel.body {
        // 竪書：第一縱貼右邊，往左排；每個字取它的竪排字形。
        Body::Prose(_) if panel.vertical_text => {
            for (n, zong) in lines.iter().enumerate() {
                let x = match (rect.x + width).checked_sub(2 + 1 + n as u16 * 2) {
                    Some(x) if x > rect.x => x,
                    _ => break,
                };
                let ink_of = match n {
                    0 => ink.gold(),
                    _ => ink.text(),
                };
                for (i, ch) in zong.chars().enumerate() {
                    let y = rect.y + 1 + i as u16;
                    if y + 1 >= rect.y + height {
                        break;
                    }
                    let shown = yumete_cjk::vertical::vertical_form(ch).unwrap_or(ch);
                    put_text(buf, x, y, x + 2, &shown.to_string(), ground.fg(ink_of));
                }
            }
        }
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
