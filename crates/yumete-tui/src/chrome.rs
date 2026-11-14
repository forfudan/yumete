//! **One ring, drawn one way.**
//!
//! Everything this front end floats over the page is a box with the same
//! four parts: the ground it clears, the ring around it, a name in the
//! top-left, and — the part nobody remembers — the 漢字 half-cell rule. Eight
//! things draw it: the 浮框 (`panel.rs`), the 鍵表 and 命令選單
//! (`draw_list`), the 問句 (`draw_query`), the 挑選器 and its preview, and the
//! 候選欄's two (`vertical.rs`, `draw_candidates`). Eight copies of a rule would
//! be seven places for it to be forgotten, and 「竪排提示框缺一角」 is what being
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
    /// **只畫上下兩道牆**。
    ///
    /// 行內 diff 那一條帶子要「看着像正文的一部分」：通欄、左右不收口，上下兩道
    /// 橫綫把它和上下文隔開就夠。左右畫了反而像一扇飄在上面的窗。
    pub open_sides: bool,
    /// The ring itself: its ink on the panel's ground.
    pub border: Style,
    /// The ground, painted over the whole rectangle.
    pub ground: Style,
    /// The name in the top-left corner, in whatever ink names are drawn in
    /// here — 金 for the editor's panels. `None` for a ring with no name.
    pub title: Option<(String, Style)>,
    /// **A word on the bottom wall**, where the title is on the top one: at
    /// the left of the bottom border, as on the other temporary panels, so it
    /// saves a line.
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
        .borders(match ring.open_sides {
            true => Borders::TOP | Borders::BOTTOM,
            false => Borders::ALL,
        })
        .border_type(match ring.rounded {
            true => BorderType::Rounded,
            false => BorderType::Plain,
        })
        .border_style(ring.border)
        .style(ring.ground);
    if let Some((text, style)) = &ring.title {
        // **One cell in from the corner**, so the title lines up. The ring's contents start one cell
        // past the wall; a title written hard against the corner sits a cell to their left, and the
        // two left edges being nearly-but-not-quite the same is worse than either alignment on its
        // own.
        block = block.title(Span::styled(format!(" {text}"), *style));
    }
    let inner = block.inner(rect);
    block.render(rect, frame.buffer_mut());
    // The foot is written after the wall, over it, with a cell of air either
    // side — `╰ 1/87 ──`, as the title is `╭ 命令 ──` and as a floating panel
    // writes its count, so the three left edges line up.
    if let Some((text, style)) = &ring.foot {
        let text = format!(" {text} ");
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

/// **Where a floating thing stands.**
///
/// Two answers, and the difference is whether the reader is still writing:
/// something to glance at keeps out of the caret's way, and something that has
/// taken the keys stands in the middle, where it cannot be missed.
/// **How much of the page a floating box may take.**
///
/// 橫排 two thirds wide and one third tall; 竪排 the transpose. Two numbers,
/// and each of them answers a different question:
///
/// - **The short side protects the caret.** A box one third tall stands in
///   the top third or the bottom third, and the caret is in neither. Once the height
///   does that job the width is free to be chosen for reading.
/// - **The long side is the measure.** Two thirds of a 118-column page is 39
///   漢字 to a line, which is what prose wants; the whole width would be 59,
///   past what an eye reads without losing its place, and it would take those
///   rows away from the manuscript **whole** rather than by halves.
///
/// 竪排 turns both round, and for the same reason: there a wide box cuts the
/// tops off many 縱 at once, breaking the run of the text.
///
/// **A key table is scanned once rather than read**, so neither argument above
/// is about it: there is no measure to keep to, and a third of a 24-row
/// terminal is six rows inside the ring — which the 23 items of the `空格`
/// menu do not fit into at any column count. So it
/// takes the whole width and **half** the page — and half is where it
/// *starts*, not where it must stop.
///
/// A key menu grows past this room, one row at a time, while its meanings
/// would otherwise be cut, and never over the caret's line (`panel::draw`).
/// Going wider does not fit `空格`'s 43 keys — on an 80-column page five
/// columns cut 29 of the 43 meanings to 「文件挑選…」. Covering more of the
/// text for the moment a menu is open costs less than a menu nobody can read,
/// as long as the line being written stays in sight. More than half is
/// dangerous as a starting point; wider is better.
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
/// Three kinds, answered in one place: three expressions in three places
/// would be spellings of one intent kept in step by hand, and the field list
/// would silently take one from each.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Use {
    /// Prose: a 百科 entry, a footnote, a note about the text.
    Read,
    /// `name  value` pairs: 記錄 and the five 信息 panels.
    Fields,
    /// A key table: scanned once, for the one row being looked for. Its room
    /// is where it starts; a key menu grows past it to keep its meanings
    /// whole (`panel::draw`).
    Scan,
}

/// **光標那一點，連同要讓開的那一段**。
///
/// `keep` 是 `(光標之前幾格, 從光標起算幾格)`——浮動的面板要讓開的是**那個詞**，
/// 不是光標站的那一格：一個漢字就佔兩格，而面板在講的那個詞條常常好幾個字。
/// 格數而不是位置，所以橫竪兩種版面共用一個值，各自往自己行文的方向攤開。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spot {
    pub at: (u16, u16),
    pub keep: (u16, u16),
}

impl Spot {
    /// 只有一格要讓開的那一種——沒有詞可問的地方（測試、沒有正文的面板）。
    pub fn cell(at: (u16, u16)) -> Spot {
        Spot { at, keep: (0, 1) }
    }
}

pub enum Anchor {
    /// **通欄，貼着某幾行**——行內 diff 那一條帶子。
    ///
    /// 它不是一扇飄在正文上的窗，而是「這幾行**此刻**換一種畫法」：寬度就是整個
    /// 編輯區（連行號槽一起），所以上下兩道橫綫一拉，看着就像正文裏多出來的一
    /// 段。插一行真的虛行要動 `wrap::Row` 和九十處讀它的地方（`development.md`
    /// 的 item 248）；蓋一層在觀感上沒有分別，而它本來就是臨時的——`Esc` 或者光
    /// 標走開就收。
    ///
    /// **落點只有上下兩個**：寬度是整個編輯區，所以「右下、右上」就是「下、上」
    /// ——通欄之後 [`Anchor::Caret`] 那四個角塌成兩個，規矩還是同一條（先站不擋
    /// 光標的那一個）。不跟着光標走：帶子蓋的那幾行裏光標一動它就跳。
    Band { at: (u16, u16), bottom: u16 },
    /// **One side is fixed, the other flips.**
    ///
    /// 橫排 the box stands at the **right** and moves only up and down: we
    /// write from left to right, so the line start must always be visible.
    /// What a float may cover is the tail of the lines it sits on, never their
    /// beginnings — and a float that also slid left and right would jump
    /// across the page as the caret passed the middle column, for no reason
    /// the reader could see.
    ///
    /// 竪排 is the same argument turned: there the 縱 run top to bottom and
    /// right to left, so the box stands at the **bottom** (never at the right)
    /// and flips left and right, defaulting to the left — the side
    /// the writing has not reached yet.
    ///
    /// `bottom` is the row the floats stack up from — the top of the footer;
    /// `vertical` picks which way [`room`] is turned, and which side is fixed.
    ///
    /// `keep` 是**要讓開的那一段**，不是光標那一格：`(光標之前幾格, 從光標起算
    /// 幾格)`。橫排往左右量，竪排往上下量。只讓一個格子的話，一個漢字會被蓋掉
    /// 右半邊，而一個好幾個字的詞條被蓋掉大半。
    Caret { at: (u16, u16), keep: (u16, u16), bottom: u16, vertical: bool },
    /// **貼着光標，畫在它下面那一行**。
    ///
    /// 補全那張單子用的是這一個，而不是「光標不在的那個角」——光標底下是慣例，
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
/// Warning: **A caret-anchored box is held to [`room`]** — two thirds by one third, two ninths of
/// the page, which keeps it small and reads well. A centred one has the keys, so the page behind it
/// is not being read and the cap does not apply.
pub fn place(area: Rect, want: (u16, u16), anchor: Anchor) -> Option<Rect> {
    let (width, height) = want;
    if width < 8 || width > area.width || height > area.height {
        return None;
    }
    match anchor {
        // 通欄：`x` 和寬度都由 `area` 說了算，只剩上下兩個落點。
        Anchor::Band { at, bottom } => {
            if bottom < area.y + height {
                return None;
            }
            let low = bottom - height;
            // 先站下面；光標正落在那幾行裏纔翻到上面。兩個都擋得住（帶子比半頁
            // 還高）就照樣站下面——擋着總比一格不畫好，同 [`Anchor::Caret`]。
            let y = match at.1 >= low && at.1 < low + height {
                false => low,
                true => area.y,
            };
            Some(Rect::new(area.x, y, area.width, height))
        }
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
        Anchor::Caret { at, keep, bottom, vertical } => {
            // Warning: **The policy caps belong to the caller, this only asks
            // whether it physically fits.** A 竪書 note is two thirds of the
            // page *tall* by design — a half-page refusal here would throw it
            // away and nothing would be drawn at all.
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
            //
            // Warning: **問的是整個詞，不是光標那一格**。一個
            // 漢字佔兩格，而面板說的那個詞條常常好幾個字——只問一格的話，箱子
            // 從那一格的右邊起畫，蓋掉的正是它在講的那個詞。`keep` 把那一段攤
            // 開：橫排往左右，竪排往上下，因為讓開的方向就是行文的方向。
            let (back, on) = keep;
            let (from, to) = match vertical {
                false => ((at.0.saturating_sub(back), at.1), (at.0 + on, at.1 + 1)),
                true => ((at.0, at.1.saturating_sub(back)), (at.0 + 2, at.1 + on)),
            };
            let covers = |(x, y): &(u16, u16)| {
                from.0 < x + width && to.0 > *x && from.1 < y + height && to.1 > *y
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

/// **The ground everything that floats stands on** — 第 85 檔.
///
/// One function rather than a constant spelled at each call, because the
/// mistake this exists to prevent is *half* a panel changing: a ring moved off
/// the paper's colour without the rows inside it has every line of the `:`
/// menu dragging a patch of the old colour behind it. The ring
/// and its contents read this.
pub fn panel_ground(ink: crate::theme::Palette) -> ratatui::style::Color {
    ink.at(yumete_config::rung::FLOAT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **浮動的窗讓開整個詞，不只是光標那一格**。
    ///
    /// 光標下的字是全角的，只讓一格會蓋掉它的一半；詞條常常好幾個字，只讓光標
    /// 那一格照樣蓋掉那個詞。
    #[test]
    fn a_float_stands_clear_of_the_whole_word() {
        let page = Rect::new(0, 0, 80, 24);
        let bottom = 22;
        let put = |at: (u16, u16), keep: (u16, u16), want: (u16, u16)| {
            place(page, want, Anchor::Caret { at, keep, bottom, vertical: false }).expect("放得下")
        };
        let hits = |b: Rect, from: u16, to: u16, y: u16| {
            b.y <= y && y < b.y + b.height && b.x < to && b.x + b.width > from
        };

        // 「文高曲」在一行的末尾，光標停在中間那個字上：詞佔 46..52，而光標只在 48。
        // 箱子站右邊、站那一行所在的那一半；只問 48 一格的話會從 49 起畫，蓋掉
        // 「曲」——四個角都讓開 46..52。
        let wide = put((48, 12), (2, 4), (30, 8));
        assert!(!hits(wide, 46, 52, 12), "蓋住了詞：{wide:?}");

        // 一個漢字自己也是兩格。箱子寬到只剩一條縫的時候，那兩格都得讓開。
        let one = put((40, 12), (0, 2), (40, 8));
        assert!(!hits(one, 40, 42, 12), "蓋住了光標底下那個字：{one:?}");

        // 讓不開就照舊畫在第一個角上——蓋住總比一格不畫好，同這個檔裏別處的規矩。
        let huge = put((40, 12), (0, 2), (80, 20));
        assert_eq!((huge.width, huge.height), (80, 20), "放不下也要畫出來");
    }

    /// **帶子只有上下兩個落點，先下，光標在裏頭纔翻上**。
    #[test]
    fn a_band_stands_clear_of_the_caret() {
        let page = Rect::new(0, 0, 80, 24);
        let bottom = 22;
        let band = |at: (u16, u16), height: u16| {
            place(page, (80, height), Anchor::Band { at, bottom }).expect("放得下")
        };
        // 光標在上半頁：帶子站底下，腳貼着頁腳。
        let low = band((12, 3), 6);
        assert_eq!((low.x, low.y, low.width), (0, 16, 80), "通欄，站下面");
        // 光標正落在那幾行裏：翻到頁頂。
        let high = band((12, 18), 6);
        assert_eq!((high.x, high.y, high.width), (0, 0, 80), "擋着就翻上去");
        // 光標在底下那一行之外（第 15 行，帶子從 16 起）：照舊站下面。
        assert_eq!(band((12, 15), 6).y, 16, "差一行，不用翻");
    }

    /// **貼着光標那一種：下面放得下就放下面，放不下翻到上面**。
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

    /// **一邊釘死，另一邊翻**。橫排釘右邊，竪排釘下邊；
    /// 只有當那個角真的壓在光標上纔換一個角。
    #[test]
    fn a_caret_anchored_box_keeps_its_fixed_side_and_flips_on_the_other() {
        let page = Rect::new(0, 0, 80, 24);
        let bottom = 22;
        let at = |want, caret, vertical| {
            let it = place(page, want, Anchor::Caret { at: caret, keep: (0, 1), bottom, vertical })
                .expect("放得下");
            (it.x, it.y)
        };
        // 橫排：盒子 40 寬擺在右邊就是第 40 欄起，下邊是第 14 行起。
        let small = (40, 8);
        // 光標在左上 —— 右下那個角本來就不壓它。
        assert_eq!(at(small, (2, 2), false), (40, 14), "右下");
        // 光標在**左下**：行還是那幾行，可盒子在右半邊，壓不到它。
        // 不翻到上面去：那一翻什麽也買不到。
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
