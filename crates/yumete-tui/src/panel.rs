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
    /// **The 章節 line**, drawn quietly between the name and the body, with a
    /// blank line (竪排: a blank 縱) under it (作者 2026-09-18).
    ///
    /// 「辭典 › 真境」 is where the entry was written down, not part of what it
    /// says — gold is the name, `quiet()` is the address, and the body is
    /// ordinary ink. `None` for everything that has no such line.
    pub lede: Option<String>,
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

/// Whether `line` is a row of a markdown table.
fn is_table_row(line: &str) -> bool {
    let line = line.trim();
    line.starts_with('|') && line.len() > 1
}

/// Whether `line` is a table's `| --- | --- |` rule rather than data.
fn is_table_rule(line: &str) -> bool {
    is_table_row(line)
        && line
            .trim()
            .trim_matches('|')
            .split('|')
            .all(|cell| {
                let cell = cell.trim();
                !cell.is_empty() && cell.chars().all(|c| matches!(c, '-' | ':'))
            })
}

/// **A table, turned ninety degrees clockwise** (作者 2026-09-18 定).
///
/// 竪排 reads down and stacks 縱 leftwards, and a grid is read *across* — the
/// one thing a vertical page cannot do. So the grid is turned instead of being
/// spelled out as the `|` and `-` it is written with, which is what it used to
/// come out as: a 縱 of loose pipes and dashes for every row of the table.
///
/// Clockwise is the direction that keeps the reading order: the table's first
/// row becomes the **rightmost 縱**, read downward, and its cells run down in
/// the order the columns were written. The walls turn with it — a `|` between
/// two columns is a horizontal rule between two bands, and the rule under the
/// header is a vertical one to the left of the first 縱.
///
/// Editing is not this function's business: 竪排 locks a table read-only and
/// `t t` opens the editable view (which turns the page horizontal), so the
/// caret never has to stand inside a grid that has been turned.
fn table_zong(block: &[&str], tall: usize) -> Vec<String> {
    let rows: Vec<Vec<String>> = block
        .iter()
        .filter(|line| !is_table_rule(line))
        .map(|line| {
            line.trim()
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_string())
                .collect()
        })
        .collect();
    let columns = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if rows.is_empty() || columns == 0 {
        return Vec::new();
    }
    // How deep one cell is drawn. Every cell gets the same depth so the bands
    // line up across the 縱 — that alignment *is* the grid — and a cell too
    // long for it is cut with a 「…」 rather than made to fit (作者: 「超過就
    // 截斷，畫『…』」; the whole entry is a `t t` away).
    let band = (tall + 1) / columns;
    let deep = band.saturating_sub(1).max(1).min(
        rows.iter()
            .flat_map(|r| r.iter().map(|c| c.chars().count()))
            .max()
            .unwrap_or(1)
            .max(1),
    );
    let cell = |text: &str| -> String {
        let mut out: String = text.chars().take(deep).collect();
        if text.chars().count() > deep {
            out.pop();
            out.push('…');
        }
        // Padded to the band so the next one starts where its neighbours do.
        while out.chars().count() < deep {
            out.push(' ');
        }
        out
    };
    let one = |row: &Vec<String>| -> String {
        (0..columns)
            .map(|c| cell(row.get(c).map(String::as_str).unwrap_or("")))
            .collect::<Vec<_>>()
            .join("─")
    };
    let mut zong: Vec<String> = Vec::with_capacity(rows.len() + 1);
    zong.push(one(&rows[0]));
    if rows.len() > 1 {
        // The rule under the header, turned: a wall down the left of the
        // first 縱, crossed where the bands meet it.
        let tallest = deep * columns + columns.saturating_sub(1);
        zong.push(
            (0..tallest)
                .map(|i| match (i + 1) % (deep + 1) == 0 {
                    true => '┼',
                    false => '│',
                })
                .collect(),
        );
    }
    zong.extend(rows[1..].iter().map(one));
    zong
}

/// **A table, drawn as a table** — the 橫排 float's half (作者 2026-09-19:
/// 「橫排的浮窗也渲染一下吧，然後讓他不要 wrap，如果有必要可以省略」).
///
/// A grid wrapped like prose is not a grid: the tail of every row lands under
/// the head of the next one and the columns are gone, which is what the panel
/// used to show. So a table is laid out at the width it is given and **never
/// wrapped** — what does not fit is cut, and the cut is said out loud with a
/// 「…」, cell by cell and then column by column.
fn table_rows(block: &[&str], budget: usize) -> Vec<String> {
    let rows: Vec<Vec<String>> = block
        .iter()
        .filter(|line| !is_table_rule(line))
        .map(|line| {
            line.trim()
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_string())
                .collect()
        })
        .collect();
    let columns = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    if rows.is_empty() || columns == 0 {
        return Vec::new();
    }
    fn cell_of(row: &[String], c: usize) -> &str {
        row.get(c).map(String::as_str).unwrap_or("")
    }
    let mut wide: Vec<usize> = (0..columns)
        .map(|c| {
            rows.iter()
                .map(|r| yumete_cjk::str_width(cell_of(r, c)))
                .max()
                .unwrap_or(1)
                .max(1)
        })
        .collect();
    // Three cells to a wall (「 │ 」). Narrow the widest column one cell at a
    // time — the widest is the one with room to give — and when every column
    // is down to its floor and it still does not fit, drop the last column and
    // say so with a 「…」 in its place.
    let walls = |n: usize| n.saturating_sub(1) * 3;
    let floor = 4usize;
    while wide.iter().sum::<usize>() + walls(wide.len()) > budget && wide.len() > 1 {
        let widest = wide.iter().copied().max().unwrap_or(0);
        match widest > floor {
            true => {
                let at = wide.iter().position(|&w| w == widest).unwrap_or(0);
                wide[at] -= 1;
            }
            false => {
                wide.pop();
            }
        }
    }
    let cut = wide.len() < columns;
    let fit = |text: &str, room: usize| -> String {
        let mut out = String::new();
        let mut used = 0;
        for g in text.chars() {
            let w = yumete_cjk::str_width(&g.to_string());
            if used + w > room.saturating_sub(usize::from(yumete_cjk::str_width(text) > room)) {
                break;
            }
            out.push(g);
            used += w;
        }
        if yumete_cjk::str_width(text) > room {
            out.push('…');
            used += 1;
        }
        while used < room {
            out.push(' ');
            used += 1;
        }
        out
    };
    let one = |row: &Vec<String>| -> String {
        let mut line = (0..wide.len())
            .map(|c| fit(cell_of(row, c), wide[c]))
            .collect::<Vec<_>>()
            .join(" │ ");
        if cut {
            line.push_str(" …");
        }
        line
    };
    let mut out: Vec<String> = Vec::with_capacity(rows.len() + 1);
    out.push(one(&rows[0]));
    if rows.len() > 1 {
        out.push(
            wide.iter()
                .map(|w| "─".repeat(*w))
                .collect::<Vec<_>>()
                .join("─┼─"),
        );
    }
    out.extend(rows[1..].iter().map(one));
    out
}

/// How wide a table wants to be, before anything is cut.
fn table_width(block: &[&str]) -> usize {
    let rows: Vec<Vec<String>> = block
        .iter()
        .filter(|line| !is_table_rule(line))
        .map(|line| {
            line.trim()
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim().to_string())
                .collect()
        })
        .collect();
    let columns = rows.iter().map(|r| r.len()).max().unwrap_or(0);
    (0..columns)
        .map(|c| {
            rows.iter()
                .map(|r| yumete_cjk::str_width(r.get(c).map(String::as_str).unwrap_or("")))
                .max()
                .unwrap_or(1)
                .max(1)
        })
        .sum::<usize>()
        + columns.saturating_sub(1) * 3
}

/// Walk an entry block by block: `table` gets the tables, `prose` the rest.
fn by_block<T>(
    text: &str,
    table: &dyn Fn(&[&str]) -> Vec<T>,
    prose: &dyn Fn(&str) -> Vec<T>,
) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut at = 0;
    while at < lines.len() {
        let from = at;
        match is_table_row(lines[at]) {
            true => {
                while at < lines.len() && is_table_row(lines[at]) {
                    at += 1;
                }
                out.extend(table(&lines[from..at]));
            }
            false => {
                while at < lines.len() && !is_table_row(lines[at]) {
                    at += 1;
                }
                out.extend(prose(&lines[from..at].join("\n")));
            }
        }
    }
    out
}

/// One entry, cut into 縱: its paragraphs wrapped, its tables turned.
///
/// The two are measured differently — prose by [`yumete_core::zong::zong_rows`]
/// and a grid by [`table_zong`] — so the entry is walked block by block rather
/// than handed to one wrapper whole.
fn entry_zong(text: &str, tall: usize, indent: &dyn Fn(&str) -> String) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let lines: Vec<&str> = text.split('\n').collect();
    let mut at = 0;
    while at < lines.len() {
        match is_table_row(lines[at]) {
            true => {
                let from = at;
                while at < lines.len() && is_table_row(lines[at]) {
                    at += 1;
                }
                out.extend(table_zong(&lines[from..at], tall));
            }
            false => {
                let from = at;
                while at < lines.len() && !is_table_row(lines[at]) {
                    at += 1;
                }
                let prose = lines[from..at].join("\n");
                out.extend(yumete_core::zong::zong_rows(&indent(&prose), tall));
            }
        }
    }
    out
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
    // **Which of the lines are not the body** (作者 2026-09-18). Three inks,
    // and the order is the same in both layouts: the name in 金, the 章節 line
    // quietly under it, then the entry. Counted rather than marked on each line
    // because they are always at the front — `gold` of them, then `quiet` of
    // them after the blank that separates the two.
    let mut gold = 0usize;
    let mut quiet: std::ops::Range<usize> = 0..0;
    // 段落的縮進：一格（作者 2026-09-18：「所有的段落不用空行，但是加一格縮
    // 進」。兩格試過，太重）。一個全角空格，橫竪一樣——竪排它就是那一縱頭上的
    // 一個空位。
    let indented = |text: &str| -> String {
        text.split('\n')
            .filter(|para| !para.trim().is_empty())
            .map(|para| format!("　{para}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    // `lines` is the prose, already wrapped; a key list draws itself from
    // `panel.body` and needs only its `count`.
    let (inner, count, lines, columns, key_w, one) = match &panel.body {
        // **竪書的正文**（作者 2026-09-18）：一縱是一列，字往下走，縱往左排。
        // 量法轉置——能放多高就一縱多少字，需要幾縱就多寬（一個漢字兩格）。
        Body::Prose(text) if panel.vertical_text => {
            let tall = (room_h as usize).saturating_sub(2).max(1);
            // **段落之間不空縱，改用一格縮進**（作者 2026-09-18 定）。空一縱在
            // 只有十來縱的小框裏把兩段推得老遠；兩格縮進太重。標題與章節行後面
            // 各空一縱，那是另一回事——那兩縱不是正文，空白就是它們與正文的界。
            //
            // 折行交給**頁面自己的竪排折行器**（`yumete_core::zong`）。
            //
            // ⚠️ **不要拿橫排那一支來折竪書**（2026-09-18 撞到兩次）。橫排的
            // 預算是**格**，而一縱的長度是**字**：半角字一格一個，所以 52 格
            // 裝得下 28 個字，比 26 深的那一縱把框撐破（詞條裏有「500」「>」
            // 這種就會發生）。折完按字數硬切一刀更糟——那一刀不認禁則，於是
            // 「，」被切到了下一縱的頭上。`zong` 這一支本來就是按字算的，禁則
            // 也在裏面，頁面上的竪排走的就是它。
            // **標題是自己的一縱，排在最右**（作者 2026-09-18 定，三個辦法裏
            // 的丙）。竪排的書就是這麽做的：標題不貼在框邊上，它本身就是第一
            // 縱，金墨。所以這一支不給 `chrome` 標題，框上没有名字。
            let mut zong: Vec<String> = vec![panel.title.chars().take(tall).collect()];
            gold = 1;
            // 章節行緊貼標題（作者 2026-09-18：「標題和章節行之間不需要空
            // 行」）——它們是同一件事的兩半，名字與它寫在哪兒，金與灰已經把
            // 兩者分開了。空的那一縱在**它們和正文之間**，只此一道。
            if let Some(lede) = &panel.lede {
                let rows = yumete_core::zong::zong_rows(lede, tall);
                quiet = zong.len()..zong.len() + rows.len();
                zong.extend(rows);
            }
            zong.push(String::new());
            zong.extend(entry_zong(text, tall, &indented));
            // **裝不下的是「縱」，不是「行」**（2026-09-18）。橫排那一段截斷
            // （下面的 `cap`）數的是行，竪排這裏一行是一個字，照它辦就變成
            // 「把最左那一縱整根換成……，框高等於縱的條數」——高度塌掉、正文
            // 無聲少一截。所以竪排自己在這裏截：框最寬只有
            // `chrome::room` 的三分之一，換算成幾縱，多的砍掉。
            //
            // ⚠️ **「…」接在末縱的腳下，不自己占一縱**（作者 2026-09-18：「最後
            // 的省略號後面有個空行」）。自己占一縱的話那一縱只有一個字，底下
            // 一大片白——讀起來就是「這裏空了一行」，而它要說的是「話還没完」。
            let box_w = (room_w as usize).max(24).min(area.width as usize);
            let fits = box_w.saturating_sub(2 + pad * 2) / 2;
            if fits > 0 && zong.len() > fits {
                zong.truncate(fits);
                if let Some(last) = zong.last_mut() {
                    while last.chars().count() >= tall {
                        last.pop();
                    }
                    last.push('…');
                }
            }
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
            // 章節行在最上面，灰的，底下空一行；正文每段縮進一格。橫排的標題
            // 畫在框線上（它本來就在那兒），所以這裏没有「標題後空一行」——框
            // 線已經是那一道界。
            let body = match &panel.lede {
                Some(lede) => format!("{lede}\n\n{}", indented(text)),
                None => indented(text),
            };
            // How wide the entry wants to be: its longest paragraph, or the
            // width a table needs — whichever asks for more, clamped to the
            // room. A table is never wrapped (below), so its width is a real
            // demand and not a preference.
            let longest = by_block(
                &body,
                &|block| vec![table_width(block)],
                &|prose| prose.split('\n').map(yumete_cjk::str_width).collect(),
            )
            .into_iter()
            .max()
            .unwrap_or(0);
            let inner = longest.min(want).max(1);
            let lines = by_block(
                &body,
                &|block| table_rows(block, inner),
                &|prose| {
                    let mut rows = wrap(prose, inner);
                    // `wrap` drops the blank row a block ends with — right at
                    // the foot of a panel, wrong between two blocks, where it
                    // is the air the writer put there (the one under the 章節
                    // line, for instance).
                    if prose.ends_with('\n') {
                        rows.push(String::new());
                    }
                    rows
                },
            );
            if let Some(lede) = &panel.lede {
                quiet = 0..wrap(lede, inner).len();
            }
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
    //
    // ⚠️ **竪書不走這一段**，它在上面自己截過了：這裏的 `count` 對竪書是「一縱
    // 幾個字」而 `lines` 是一條條的縱，兩者不是同一個維度，照這裏辦會把最左那
    // 一縱換成「…」並且把框高壓成縱的條數。
    let cap = room_h.saturating_sub(2).max(1) as usize;
    let (count, lines) = match &panel.body {
        Body::Prose(_) if panel.vertical_text => (count, lines),
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
    // 三色，一條規矩，橫竪通用：名字是金，章節行是灰，其餘是墨。
    let ink_of = |n: usize| match n {
        _ if n < gold => ink.gold(),
        _ if quiet.contains(&n) => ink.quiet(),
        _ => ink.text(),
    };
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
                for (i, ch) in zong.chars().enumerate() {
                    let y = rect.y + 1 + i as u16;
                    if y + 1 >= rect.y + height {
                        break;
                    }
                    let shown = yumete_cjk::vertical::vertical_form(ch).unwrap_or(ch);
                    // ⚠️ **A turned table's rules have to reach across the
                    // gap** (2026-09-18). A 縱 is two cells wide and 「─」 is
                    // one, so a band rule came out as a dotted line with a
                    // hole between every 縱. Written twice, it joins up.
                    let shown = match shown {
                        '─' | '┼' => format!("{shown}─"),
                        _ => shown.to_string(),
                    };
                    put_text(buf, x, y, x + 2, &shown, ground.fg(ink_of(n)));
                }
            }
        }
        Body::Prose(_) => {
            for (i, line) in lines.iter().enumerate() {
                let x = rect.x + 1 + pad as u16;
                put_text(buf, x, rect.y + 1 + i as u16, limit, line, ground.fg(ink_of(i)));
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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Draw one 竪書 panel on a `w × h` page and hand back its rectangle and
    /// what was written.
    fn zong(text: &str, w: u16, h: u16) -> (Rect, ratatui::buffer::Buffer) {
        let config = Config::default();
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut got = None;
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, w, h);
                got = draw(frame, &config, area, h, (w - 2, 0), true, &Panel {
                    title: "天門真境".into(),
                    lede: None,
                    body: Body::Prose(text.into()),
                    vertical_text: true,
                    tag: None,
                });
            })
            .unwrap();
        (got.expect("the panel is drawn"), terminal.backend().buffer().clone())
    }

    /// 2026-09-18: a long entry came out **half the height it should be** and
    /// silently missing its tail. Two faults, one symptom: the row-counting cap
    /// written for 橫排 was reading 竪書's 「how many 字 to a 縱」 as 「how many
    /// rows」 — so it replaced the leftmost 縱 with an ellipsis and then made the
    /// box as deep as the *number of 縱*; and the wrapper's budget is cells
    /// while a 縱's length is 字, so 「500」 and 「>」 pushed a 縱 two 字 past the
    /// room and tripped the cap in the first place.
    #[test]
    fn a_long_vertical_entry_fills_its_height_and_says_it_was_cut() {
        // Halfwidth digits on purpose: that is what made a 縱 longer than the
        // room in 字 while still fitting it in cells.
        let entry = "天門真境辭典 > 真境。\n\
                     面積大約 500 平方千米，山地湖南到皖河，地理中心位於返塵亭南。\n\
                     在籍居民二十萬三千人四百人，第一、第三產業發達。\n\
                     大量居民在得稅低，高福利，高遺產稅。直屬年分紅：有工作的人權重高。\n\
                     返塵亭——迎客居：一裏，再往後還有一段，看它會不會被砍掉。\n\
                     山門之外另有客舍三十間，逢法會則不敷用，須往迎客居暫住。\n\
                     歷任駐守皆出自乾元字輩，掌宗座下領宗內主事者兼領之。\n\
                     水路自皖河北上，陸路過返塵亭，二者皆須驗牒方得入境。";
        let (rect, buffer) = zong(entry, 70, 45);

        // 竪排 the room is two thirds of the page tall, and a full entry uses
        // all of it: 45 × 2/3 = 30.
        assert_eq!(rect.height, 30, "the box is {} rows deep", rect.height);
        // No 縱 may be deeper than the room — that was the wrapper's cell/字
        // confusion, and it is what tripped the cap.
        assert!(rect.height <= 45 * 2 / 3);

        // What did not fit says so, in the vertical ellipsis, at the far left.
        let text: String = (0..rect.height)
            .flat_map(|y| (0..rect.width).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(rect.x + x, rect.y + y)].symbol().to_string())
            .collect();
        assert!(text.contains('︙'), "the cut is marked: {text:?}");

        // 禁則, down the column: no 縱 may open with a mark that closes
        // something. Hand-chopping the horizontal wrapper's rows put 「，」 at
        // the head of a 縱, which is the one thing a typesetter never does.
        let head: String = (0..rect.width)
            .map(|x| buffer[(rect.x + x, rect.y + 1)].symbol().to_string())
            .collect();
        for mark in ['︐', '︑', '︒', '︓', '﹂', '︶'] {
            assert!(!head.contains(mark), "{mark} opens a 縱: {head:?}");
        }

        // And the 「…」 hangs off the foot of the last 縱 rather than standing
        // in one of its own with a column of white under it.
        let ellipsis_row = (0..rect.height)
            .find(|&y| {
                (0..rect.width).any(|x| buffer[(rect.x + x, rect.y + y)].symbol() == "︙")
            })
            .expect("the ellipsis is drawn");
        assert!(ellipsis_row > 1, "the 「…」 is a column of its own: row {ellipsis_row}");
        // And the title is still its own 縱, in the box rather than on the ring.
        assert!(text.contains('天') && text.contains('境'), "{text:?}");
    }

    /// A short entry keeps the box short — the height follows the deepest 縱,
    /// it is not padded out to the room.
    #[test]
    fn a_short_vertical_entry_does_not_grow_a_box_it_does_not_need() {
        let (rect, _) = zong("短。", 70, 45);
        assert!(rect.height <= 6, "the box is {} rows deep", rect.height);
    }
}
