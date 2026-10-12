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
use yumete_core::say;

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
    /// **一行一條，各帶各的淡底**——改動那一格（`空格 x`，2026-10-11）。
    ///
    /// 和 `Keys` 同一個形狀（一條一行、不折行），差別只在它不分兩欄，而且每一
    /// 條自己說要什麼底色。色由 `:git-diff` 那一套來：刪掉的是 `Accent::Mark`、
    /// 新添的是 `Accent::Green`，一字不差。
    Tinted(Vec<Tint>),
}

/// **帶子裏的一行**（`空格 x`，2026-10-11）。
///
/// 兩層底色，因為它們答兩個問題：整行那一層說「這一行是改動的哪一側」，行內那
/// 幾段說「真正動了的是這幾個字」。
pub struct Tint {
    /// 要畫的那一行，**行號槽也在裏頭**——帶子從最左畫起。
    pub text: String,
    /// 整行那一層淡底。`None` ＝ 不上色。
    pub wash: Option<crate::theme::Accent>,
    /// `text` 的字節區間，上重一檔的底。
    ///
    /// Warning: **偏移要連行號槽一起算**（調用方加的那一截前綴）——`text` 是整
    /// 行，區間也必須是整行上的。
    pub marks: Vec<std::ops::Range<usize>>,
}

/// **這一扇站在哪裏**（2026-10-08）。
///
/// 兩種，因為兩種浮窗答的是兩種問題：「這是什麼」是讀一段話，讀完回到正文，
/// 所以越不擋着正文越好；「接下來是什麼」是一張跟着手走的單子，越貼着手越好。
pub enum Stand {
    /// 光標不在的那個角——浮窗的老規矩。
    Corner,
    /// 貼着光標下面那一行（裝不下就上面）。補全那張單子用這一個。
    UnderCaret,
    /// **通欄**——行內 diff 那一條帶子（2026-10-11）。
    ///
    /// 左右不收口（`open_sides`），寬度是整個編輯區，所以它看着不像一扇窗而像
    /// 「這幾行此刻換了一種畫法」。竪排不走這一種（見 `Anchor::Band` 的註釋）。
    ///
    /// 站上面還是下面由 [`crate::chrome::Anchor::Band`] 挑——通欄之後「右下、
    /// 右上」就是「下、上」。
    Band,
}

/// A floating panel: what it is called, what it holds, and one short thing
/// said quietly at the far end of its bottom edge.
pub struct Panel {
    /// 站在哪裏——見 [`Stand`]。
    pub stand: Stand,
    /// **正文裏要加重的那一段**（字符下標，不是字節；2026-10-08）。
    ///
    /// 簽名那一扇用它說「你正在填第幾個參數」——服務器說得出來
    /// （`lsp::Signature::active`），而「填到哪了」正是這一則存在的理由。
    ///
    /// Warning: **只管橫排、散文、不走 Markdown、正文裏沒有換行的那一種。**
    /// 算法是「折行不會弄丟字」：一行一行數過去，第 i 行就是原文的第
    /// `seen..seen+n` 個字。換行符不在任何一行裏（`wrap` 按 `\n` 先分段），
    /// 所以正文帶換行就不加重——寧可不加重，也不許加重錯的那一段。
    pub accent: Option<(usize, usize)>,
    pub title: String,
    /// **The 章節 line**, drawn quietly between the name and the body, with a
    /// blank line (竪排: a blank 縱) under it (2026-09-18).
    ///
    /// 「辭典 › 君山」 is where the entry was written down, not part of what it
    /// says — gold is the name, `quiet()` is the address, and the body is
    /// ordinary ink. `None` for everything that has no such line.
    pub lede: Option<String>,
    /// **Whether the body is an entry rather than a note** (2026-09-19).
    ///
    /// A 百科 entry is several paragraphs and is set like a page of a book:
    /// each one opens with a 全角 space. A footnote, a `%%註釋%%`, `:write`'s
    /// check and the `[yumete]` line are each one short paragraph *about*
    /// something, and an indent on them is a stray space at the head of the
    /// panel — which is what #287 quietly gave all four of them.
    pub entry: bool,
    pub body: Body,
    /// **散文從第幾行畫起**（2026-09-29）——浮窗翻頁用的，別處一律 0。
    ///
    /// Warning: 只管 `Body::Prose` 的橫排那一支。竪書那一支量的是「一縱幾個字」，
    /// 不是行，翻頁在那裏是另一件事。
    pub scroll: usize,
    /// **Whether the prose inside runs down the page** (2026-09-18:
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
    /// **這一扇翻得動嗎**（#426）。翻得動的時候底邊右端寫的是「怎麽翻」而不是
    /// `tag`——五種信息都是這一類，而註、`[yumete]` 那一行不是。
    pub pages: bool,
    /// **這一扇是拿來讀的，不是一張鍵表**（#426，2026-09-30 報的）。
    ///
    /// 決定它多大：讀的那幾扇按 2/3 × 1/3 擺，鍵表按整寬、半頁高——兩者都在
    /// `chrome::room` 裏，由 `chrome::Use` 分。Warning: **判準是「拿來幹什麽」，不是
    /// `Body` 是哪一種。** 記錄畫成兩欄字段（`Body::Keys`），可它是讀的——它
    /// 從前跟着 `空格` 選單一起吃了「鍵表」那一套，於是鋪滿整個屏幕寬。
    pub reading: bool,
    /// **Whether the body is Markdown and should be set as Markdown**
    /// (#53 ③, 2026-09-21).
    ///
    /// A language server's hover answer *is* Markdown — that is what the
    /// protocol says it sends — and this editor already knows how to set
    /// Markdown: `**bold**` bold with its asterisks set back, `` `code` `` in
    /// the literal's ink, a heading in 金. 2026-09-21 問：「它為什麼不能渲染
    /// markdown 呢？我覺得完全可以呀。」 So it does, through the same
    /// [`crate::markup_style`] the page itself uses — one look, one answer to
    /// 「what does `**` mean here」.
    ///
    /// Off for everything that is prose about the text rather than a document:
    /// a footnote that happens to contain an asterisk is not emphasis.
    pub marked: bool,
}

/// Whether `line` is a row of a markdown table.
pub(crate) fn is_table_row(line: &str) -> bool {
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

/// **A table, turned ninety degrees clockwise** (2026-09-18 定).
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
    // **Each band gets the depth it needs, and only a band that cannot have it
    // is cut** (2026-09-19: 「空間夠就渲染全部，不夠再摺疊，儘量保證撐滿
    // 整個高度」).
    //
    // The first cut of this divided the height evenly by the number of columns,
    // which folded a short column away while the box below it stood empty. What
    // it should ask is the question every fair share asks: **is there a depth
    // `k` such that no band needs more than it gets?** Raise `k` until the
    // bands that fit under it, plus the ones capped at it, fill the room — the
    // deep ones are cut and the shallow ones are left whole.
    let want: Vec<usize> = (0..columns)
        .map(|c| {
            rows.iter()
                .map(|r| r.get(c).map(|cell| cell.chars().count()).unwrap_or(0))
                .max()
                .unwrap_or(1)
                .max(1)
        })
        .collect();
    let rules = columns.saturating_sub(1);
    let room = tall.saturating_sub(rules).max(columns);
    let cap = match want.iter().sum::<usize>() <= room {
        // Room for all of it: nothing is cut, and the box is as deep as the
        // table really is rather than as deep as the page allows.
        true => usize::MAX,
        false => (1..=want.iter().copied().max().unwrap_or(1))
            .take_while(|k| want.iter().map(|w| (*w).min(*k)).sum::<usize>() <= room)
            .last()
            .unwrap_or(1),
    };
    let deep: Vec<usize> = want.iter().map(|w| (*w).min(cap)).collect();
    let cell = |text: &str, deep: usize| -> String {
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
            .map(|c| cell(row.get(c).map(String::as_str).unwrap_or(""), deep[c]))
            .collect::<Vec<_>>()
            .join("─")
    };
    let mut zong: Vec<String> = Vec::with_capacity(rows.len() + 1);
    zong.push(one(&rows[0]));
    if rows.len() > 1 {
        // The rule under the header, turned: a wall down the left of the
        // first 縱, crossed where the bands meet it.
        let tallest = deep.iter().sum::<usize>() + columns.saturating_sub(1);
        // The crossings stand where the bands meet the wall.
        let mut at = 0usize;
        let crossings: Vec<usize> = deep
            .iter()
            .take(columns.saturating_sub(1))
            .map(|d| {
                at += d + 1;
                at - 1
            })
            .collect();
        zong.push(
            (0..tallest)
                .map(|i| match crossings.contains(&i) {
                    true => '┼',
                    false => '│',
                })
                .collect(),
        );
    }
    zong.extend(rows[1..].iter().map(one));
    zong
}

/// `text` cut to `cells` wide, with a 「…」 where it was cut and nothing added
/// when it was not.
///
/// Warning: **「…」 is East Asian *Ambiguous*** — one cell for most readers and **two**
/// under `ambiguous_width = "wide"`, which is what a CJK reader on a CJK font
/// sets. Charged as one, every cut ran a cell long. Ask the width table.
pub(crate) fn clip(text: &str, cells: usize) -> String {
    if yumete_cjk::str_width(text) <= cells {
        return text.to_string();
    }
    let budget = cells.saturating_sub(yumete_cjk::str_width("…"));
    let mut out = String::new();
    let mut used = 0;
    for g in yumete_cjk::graphemes(text) {
        let w = yumete_cjk::grapheme_width(g).max(1);
        if used + w > budget {
            break;
        }
        out.push_str(g);
        used += w;
    }
    out.push('…');
    out
}

/// **A table, drawn as a table** — the 橫排 float's half (2026-09-19:
/// 「橫排的浮窗也渲染一下吧，然後讓他不要 wrap，如果有必要可以省略」).
///
/// A grid wrapped like prose is not a grid: the tail of every row lands under
/// the head of the next one and the columns are gone, which is what the panel
/// used to show. So a table is laid out at the width it is given and **never
/// wrapped** — what does not fit is cut, and the cut is said out loud with a
/// 「…」, cell by cell and then column by column.
pub(crate) fn table_rows(block: &[&str], budget: usize) -> Vec<String> {
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
        // Warning: **「…」 is East Asian *Ambiguous*** — one cell for most readers
        // and **two** under `ambiguous_width = "wide"`, which is what a CJK
        // reader on a CJK font sets. Charged as one, every cut cell ran two
        // cells long and pushed the last column off the panel. Ask the width
        // table, the way `mdtable::FOLD_MARK`'s own note says to.
        let mark = yumete_cjk::str_width("…");
        let over = yumete_cjk::str_width(text) > room;
        let budget = match over {
            true => room.saturating_sub(mark),
            false => room,
        };
        for g in text.chars() {
            let w = yumete_cjk::str_width(&g.to_string());
            if used + w > budget {
                break;
            }
            out.push(g);
            used += w;
        }
        if over {
            out.push('…');
            used += mark;
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
fn entry_zong(text: &str, tall: usize) -> Vec<String> {
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
                out.extend(yumete_core::zong::zong_rows(&prose, tall));
            }
        }
    }
    // 段落之間那一縱空白是作者放的，留着（2026-10-12）；**末尾那幾縱不是**——
    // 詞條以換行收尾就會多出一縱，而它在框裏只是左邊一條白邊。
    while out.last().is_some_and(|z| z.trim().is_empty()) {
        out.pop();
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
    caret: crate::chrome::Spot,
    vertical: bool,
    panel: &Panel,
) -> Option<Rect> {
    let ink = crate::theme::Palette::of(config);
    // How much of the page this may take. **Both answers live in
    // `chrome::room`** and nothing works one out here — this used to say
    // `area.height / 2 + 1` while the key table below said
    // `(area.height - 2) / 2`, two spellings of one intent that had to be
    // kept in step by hand.
    //
    // Warning: **判準是「拿來讀的嗎」，不只是 `Body` 哪一種**（2026-09-30
    // 報的：記錄那一扇「size 和 location 看着不對」）。記錄畫成兩欄字段，可它
    // 和別的四種信息一樣是讀的，該和它們一樣大。
    let prose = matches!(panel.body, Body::Prose(_));
    let what = match (prose, panel.reading) {
        (true, _) => crate::chrome::Use::Read,
        (false, true) => crate::chrome::Use::Fields,
        (false, false) => crate::chrome::Use::Scan,
    };
    let (room_w, room_h) = crate::chrome::room(area, vertical, what);
    // **How many rows of content the ring leaves**, read off `room_h` once and
    // used by everything below: the key table's depth, and the cut a body too
    // tall for the room takes.
    let cap = room_h.saturating_sub(2).max(1) as usize;
    // **框上寫得下什麽，纔由什麽定寬**（2026-10-12 報的：竪排那一扇左邊空了一
    // 大片）。下面三道底綫——名字、那一句話、讀到第幾——量的都是**畫在框綫上**
    // 的東西，而竪排一樣都不畫在那兒：名字是第一縱，數字順着左框往下走。照橫排
    // 的尺寸撐出來的那二十幾格於是全是空的。
    //
    // 所以這裏量的不是「有沒有」，是「這一扇畫不畫它」——同一個問題問一次，
    // 寬度和下面那幾支畫的人讀的是同一個答案。
    let named = !panel.title.is_empty() && !(panel.vertical_text && prose);
    let title_w = match named {
        true => yumete_cjk::str_width(&panel.title),
        false => 0,
    };
    // **翻得動的時候，底邊右端寫「怎麽翻」**（2026-09-30 定：「浮窗或者右侧
    // 面板，可以显示『PgUp/PgDn 翻页』。『␣K 进边栏』似乎反而不是很重要的信息」）。
    // 「進邊欄」挪到了命令行（`hint.rs` 的 `the_way_back_to_the_list`）。
    //
    // 這裏先量一個最寬的：翻不翻得動要等下面算完 `cap` 纔知道，而寬度要先定。
    let paged = say!("info.page-it");
    let tag_w = panel
        .tag
        .as_deref()
        .map(yumete_cjk::str_width)
        .unwrap_or(0)
        .max(match panel.pages && !panel.vertical_text {
            true => yumete_cjk::str_width(&paged),
            false => 0,
        });
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
        // Warning: **讀的那幾扇兩邊都留一格**（2026-09-30 報的：最長那一行
        // 貼着框，看着像被裁了）。鍵表不留——它自己那一列就是對齊綫，往裏縮一
        // 格反而把它推離中間。
        Body::Keys(_) if panel.reading => 1,
        Body::Keys(_) => 0,
        Body::Tinted(_) => 1,
    };
    // **Which of the lines are not the body** (2026-09-18). Three inks,
    // and the order is the same in both layouts: the name in 金, the 章節 line
    // quietly under it, then the entry. Counted rather than marked on each line
    // because they are always at the front — `gold` of them, then `quiet` of
    // them after the blank that separates the two.
    let mut gold = 0usize;
    let mut quiet: std::ops::Range<usize> = 0..0;
    // 竪排那一支自己算「讀到第幾縱／共幾縱」——它的 `count` 是「一縱幾個字」，
    // 和橫排那一段數的不是同一個維度，所以那一段跳過它（見下面）。
    let mut zong_read: Option<(usize, usize)> = None;
    // **正文原樣交出去**（2026-10-11 定）。
    //
    // 這裏從前有一支 `indented`：**空行濾掉、每段頭上加一個全角空格**
    // （2026-09-18：「所有的段落不用空行，但是加一格縮進」）。那是為了省地方——
    // 讀不完也得讓它看起來讀得完。翻得動之後省地方不再是理由，而縮進本身要錢：
    // 橫排一格，竪排是一縱裏的一個**字**，而一縱只裝十來個字。
    //
    // 原話：「橫排也不要縮進，不要移除空行，保持對齊」，連着那條更一般的
    // 「除非不行，否則應該減少邏輯特例」——所以不是給竪排開一個例外，是兩邊
    // 一起把這一層拿掉。段落之間回到空行，那是 Markdown 自己的分法。
    // `lines` is the prose, already wrapped; a key list draws itself from
    // `panel.body` and needs only its `count`.
    let (inner, count, lines, columns, key_w, one) = match &panel.body {
        // **竪書的正文**（2026-09-18）：一縱是一列，字往下走，縱往左排。
        // 量法轉置——能放多高就一縱多少字，需要幾縱就多寬（一個漢字兩格）。
        Body::Prose(text) if panel.vertical_text => {
            let tall = (room_h as usize).saturating_sub(2).max(1);
            // **段落之間不空縱，改用一格縮進**（2026-09-18 定）。空一縱在
            // 只有十來縱的小框裏把兩段推得老遠；兩格縮進太重。標題與章節行後面
            // 各空一縱，那是另一回事——那兩縱不是正文，空白就是它們與正文的界。
            //
            // 折行交給**頁面自己的竪排折行器**（`yumete_core::zong`）。
            //
            // Warning: **不要拿橫排那一支來折竪書**（2026-09-18 撞到兩次）。橫排的
            // 預算是**格**，而一縱的長度是**字**：半角字一格一個，所以 52 格
            // 裝得下 28 個字，比 26 深的那一縱把框撐破（詞條裏有「500」「>」
            // 這種就會發生）。折完按字數硬切一刀更糟——那一刀不認禁則，於是
            // 「，」被切到了下一縱的頭上。`zong` 這一支本來就是按字算的，禁則
            // 也在裏面，頁面上的竪排走的就是它。
            // **標題是自己的一縱，排在最右**（2026-09-18 定，三個辦法裏
            // 的丙）。竪排的書就是這麽做的：標題不貼在框邊上，它本身就是第一
            // 縱，金墨。所以這一支不給 `chrome` 標題，框上没有名字。
            let mut zong: Vec<String> = vec![panel.title.chars().take(tall).collect()];
            gold = 1;
            // 章節行緊貼標題（2026-09-18：「標題和章節行之間不需要空
            // 行」）——它們是同一件事的兩半，名字與它寫在哪兒，金與灰已經把
            // 兩者分開了。空的那一縱在**它們和正文之間**，只此一道。
            if let Some(lede) = &panel.lede {
                let rows = yumete_core::zong::zong_rows(lede, tall);
                quiet = zong.len()..zong.len() + rows.len();
                zong.extend(rows);
            }
            zong.push(String::new());
            zong.extend(entry_zong(text, tall));
            // **裝不下的是「縱」，不是「行」**（2026-09-18）。橫排那一段截斷
            // （下面的 `cap`）數的是行，竪排這裏一行是一個字，照它辦就變成
            // 「把最左那一縱整根換成……，框高等於縱的條數」——高度塌掉、正文
            // 無聲少一截。所以竪排自己在這裏截：框最寬只有
            // `chrome::room` 的三分之一，換算成幾縱，多的砍掉。
            //
            // Warning: **「…」接在末縱的腳下，不自己占一縱**（2026-09-18：「最後
            // 的省略號後面有個空行」）。自己占一縱的話那一縱只有一個字，底下
            // 一大片白——讀起來就是「這裏空了一行」，而它要說的是「話還没完」。
            // **放不下的那幾縱翻得到，不是丟掉**（2026-10-11 定）。從前這裏
            // `truncate` 完接一個「…」——於是那四個翻頁鍵在竪排下按了也沒有反
            // 應，**不是攔住了，是後面根本沒有字**。報的原話：「我們截取了只能
            // 顯示一頁的文本，剩下的都用…省略了」。
            //
            // 省略號一併去掉，同橫排 2026-09-30 那一條：還有多少由底邊那個數字
            // 說，而它順帶說得出「還有多少」。
            let box_w = (room_w as usize).max(24).min(area.width as usize);
            let fits = box_w.saturating_sub(2 + pad * 2) / 2;
            if fits > 0 && zong.len() > fits {
                let all = zong.len();
                let from = panel.scroll.min(all - fits);
                zong_read = Some((from + fits, all));
                zong = zong.split_off(from);
                zong.truncate(fits);
                // 金與灰數的是縱的序號，而上面剛切掉了前面幾縱。
                gold = gold.saturating_sub(from);
                quiet = quiet.start.saturating_sub(from)..quiet.end.saturating_sub(from);
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
            // Warning: **The ring and the padding come out of the budget first**
            // (2026-09-18). Wrapping at the room's full width and then
            // clamping the *box* to it left every line two cells too long,
            // and `put_text` stops at the border — so the last 漢字 of each
            // row was eaten: 「百廢具興」 and no 「。」. The bug is
            // older than the 2/3 rule; a half-width panel hid it by rarely
            // reaching the cap.
            let budget = (room_w as usize).saturating_sub(2 + pad * 2);
            let want = widest.saturating_sub(2).min(budget.max(8));
            // 章節行在最上面，灰的，底下空一行；正文每段縮進一格。橫排的標題
            // 畫在框線上（它本來就在那兒），所以這裏没有「標題後空一行」——框
            // 線已經是那一道界。
            let body = match &panel.lede {
                Some(lede) => format!("{lede}\n\n{text}"),
                None => text.to_string(),
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
            // **No deeper than the room** (`cap`, above). A menu is a thing you
            // glance at beside your writing; one that fills the window has
            // stopped being a menu.
            // Warning: **…and never deeper than the page can actually place**
            // (2026-09-19, caught in review). `chrome::place` refuses a box
            // taller than the rows between the top of the page and the footer,
            // and a refusal is drawn as *nothing at all*: on a short terminal
            // a half-pressed `空格` showed no menu, which reads as 「the key
            // did nothing」 rather than as 「there is no room」. The key table
            // had no height cap of its own — two columns was all it knew.
            let tall = bottom.saturating_sub(area.y).min(area.height);
            let deep_max = cap.min((tall.saturating_sub(2) as usize).max(1));
            // Warning: **量的要和夾的是同一個數**（2026-10-12 看出來的）。上面那個
            // `widest` 是**整個屏幕**減二，而下面 `width` 會把框夾進
            // `chrome::room`——於是一張分了欄的表按屏幕寬分欄、按房間寬畫，第二
            // 欄整個落在框外，`put_text` 停在框邊：
            //
            // ```text
            // │方案拆分      彳亍                      讀音          h│
            // │編碼          KKcu                      字集           │
            // ```
            //
            // 「讀音 h」。切在欄**裏**是這一支有意的（底下那個 `one.min(..)`，
            // 「a key whose meaning is cut short still tells the reader which
            // key it is」），切在框**外**不是。
            let widest = (room_w as usize)
                .max(24)
                .min(area.width as usize)
                .saturating_sub(2 + pad * 2)
                .max(1);
            // Too deep is answered by going **wider** first: as many columns
            // as the depth asks for. A short wide menu is still a menu, and a
            // key whose meaning is cut short still tells the reader which key
            // it is — dropping the row entirely does not.
            //
            // A column may be squeezed down to a key, the gap, and four 漢字
            // of meaning; under that the table stops being readable and the
            // answer is 「there is more」 instead of another column. The cap is
            // read off that floor rather than off the longest row, because one
            // long line — 「寫作進度：今天寫了多少，離目標還有多少」 — would
            // otherwise hold the whole menu to a single column and cut *keys*
            // to keep a sentence whole.
            let least = key_w + 2 + 8;
            let most = ((widest + 2) / (least + 2)).max(1);
            let columns = keys.len().div_ceil(deep_max).clamp(1, most);
            // Each column gets its share, and what does not fit is cut
            // *inside* the column rather than beyond the border — where it
            // used to be dropped silently, leaving keys with no meanings.
            let one = one.min(widest.saturating_sub((columns - 1) * 2) / columns.max(1));
            // Only when the width has run out too does the menu say 「there is
            // more」 — in the last cell, rather than ending as if that were all
            // the keys there are.
            let shown = keys.len().min(columns * deep_max);
            (
                one * columns + (columns - 1) * 2,
                shown,
                Vec::new(),
                columns,
                key_w,
                one,
            )
        }
        // **一欄，一條一行**——量法是 `Keys` 去掉分欄那一半。
        Body::Tinted(rows) => {
            if rows.is_empty() {
                return None;
            }
            let widest = rows
                .iter()
                .map(|row| yumete_cjk::str_width(&row.text))
                .max()
                .unwrap_or(1)
                .min(room_w.saturating_sub(2 + pad as u16 * 2) as usize)
                .max(1);
            let tall = bottom.saturating_sub(area.y).min(area.height);
            let deep_max = cap.min((tall.saturating_sub(2) as usize).max(1));
            let shown = rows.len().min(deep_max.max(1));
            (widest, shown, Vec::new(), 1, 0, widest)
        }
    };

    // **Prose too tall for half the page is cut, not dropped** (2026-09-17).
    // The cap below used to refuse the whole panel — so a long footnote, or a
    // wiki entry of several paragraphs, simply did not appear, which reads as
    // 「the panel is unreliable」 rather than as 「there is no room」. A cut body
    // says what it can and ends in an ellipsis; the tag on the bottom border
    // still says where to read the rest.
    //
    // Warning: **竪書不走這一段**，它在上面自己截過了：這裏的 `count` 對竪書是「一縱
    // 幾個字」而 `lines` 是一條條的縱，兩者不是同一個維度，照這裏辦會把最左那
    // 一縱換成「…」並且把框高壓成縱的條數。
    // **讀到第幾行 / 共幾行**，寫在底邊左端（2026-09-30 定）。`None` ＝ 整則
    // 都在眼前，那時一個數字都不寫——數字出現本身就是「還有沒露出來的」。
    let mut read: Option<(usize, usize)> = zong_read;
    // **帶子也翻得動**（2026-10-11 報的）。一塊六十行的改動在二十四行的屏幕上
    // 從前只畫得出九行，而且**一個字都不說**——讀者看見的是「改了九行」，底下
    // 那一半（新的那幾行）一次都露不出來。和散文那一支同一個辦法：`panel.scroll`
    // 挑從第幾條起，底邊左端那個 `9/120` 說還有多少。
    //
    // Warning: **帶子的 `pages` 是關着的**，所以底邊右端留給 `u`／`y` 那一句，
    // 不被「怎麼翻」頂掉——那兩個鍵纔是這一扇的用處。
    let tinted_cut = matches!(&panel.body, Body::Tinted(rows) if rows.len() > count);
    if tinted_cut {
        if let Body::Tinted(rows) = &panel.body {
            let from = panel.scroll.min(rows.len().saturating_sub(count));
            read = Some((from + count, rows.len()));
        }
    }
    let (count, lines) = match &panel.body {
        Body::Prose(_) if panel.vertical_text => (count, lines),
        Body::Prose(_) if count > cap && cap > 0 => {
            // 從第幾行起——翻過頁的話（`panel.scroll`），最多翻到「最後一屏」。
            let from = panel.scroll.min(count.saturating_sub(cap));
            let kept: Vec<String> = lines.into_iter().skip(from).take(cap).collect();
            // Warning: **省略號整個去掉了**（2026-09-30 定：「现在允许翻页
            // 了，`…` 可以不用了应该」）。它從前無條件把**最後一行換成 `…`**，
            // 於是翻到底也永遠讀不到最後那一行——它不是多畫一個記號，它是吃掉
            // 一行。「下面還有」這件事現在由底邊那個數字說，而那個數字還順帶說
            // 了「還有多少」。
            read = Some((from + kept.len(), count));
            // Warning: **`gold` 與 `quiet` 數的是行的序號，而這裏剛剛把前面幾行
            // 切掉了**（2026-10-01 上墨之後才看見）。不跟着挪的話，翻過一頁之
            // 後第一行正文被當成 `章節` 行畫成灰的，而且**不上標記**——一條
            // 詞條翻到第二頁，`` `代碼` `` 和 `**粗**` 就一個顏色都沒有了。
            gold = gold.saturating_sub(from);
            quiet = quiet.start.saturating_sub(from)..quiet.end.saturating_sub(from);
            (kept.len(), kept)
        }
        _ => (count, lines),
    };
    // 竪排的那個數字站在左框上，一格寬，往下讀——它不佔底邊，所以不進這一道。
    let read_w = match panel.vertical_text {
        true => 0,
        false => read.map_or(0, |(at, all)| yumete_cjk::str_width(&format!("{at}/{all}"))),
    };
    let deep = count.div_ceil(columns);
    let width = (inner + 2 + pad * 2)
        .max(title_w + 4)
        .max(tag_w + 4)
        // 底邊兩頭各站一個，中間至少留一格橫線。
        .max(read_w + tag_w + 7)
        // Every float keeps to its room (above); which room that is was
        // settled once, by what the float is for.
        .min((room_w as usize).max(24))
        .min(area.width as usize) as u16;
    // **帶子吃滿整個編輯區**（2026-10-11）：它要和正文逐格對齊，所以寬度不由內容
    // 說了算。`place` 的 `Anchor::Band` 也從 `area.x` 起畫，兩邊是同一個意思。
    let width = match (&panel.stand, vertical) {
        (Stand::Band, false) => area.width,
        _ => width,
    };
    let height = (deep + 2) as u16;
    // Where it stands, and whether there is room at all — one rule, in
    // `chrome`, shared with everything else that floats (2026-09-18).
    // Warning: **站在光標底下那一種只管橫排**。竦書的「下一行」在旁邊不在底下，
    // 而補全那張單子只在代碼檔裏出來（代碼檔不竦排），所以竦的那一邊回老規矩。
    let stand = match (&panel.stand, vertical) {
        (Stand::Band, false) => crate::chrome::Anchor::Band { at: caret.at, bottom },
        (Stand::UnderCaret, false) => {
            crate::chrome::Anchor::UnderCaret { at: caret.at, bottom }
        }
        _ => crate::chrome::Anchor::Caret {
            at: caret.at,
            keep: caret.keep,
            bottom,
            vertical,
        },
    };
    let rect = crate::chrome::place(area, (width, height), stand)?;
    crate::chrome::draw(frame, rect, &crate::chrome::Ring {
        foot: None,
        rounded: config.panel.rounded,
        open_sides: matches!(panel.stand, Stand::Band) && !vertical,
        // `rule()`, the rung every other ring on the screen is drawn at.
        border: Style::default().fg(ink.rule()).bg(crate::chrome::panel_ground(ink)),
        ground: Style::default().bg(crate::chrome::panel_ground(ink)),
        // Vertical prose carries its name as its first 縱 (below), so the
        // ring is left bare — a name in both places would be the same word
        // twice, two cells apart.
        // **名字是空的就不畫名字**（2026-10-08 定）。辦法是「空標題」而不是在
        // `Panel` 上再加一個開關：一扈浮窗要不要名字，就是「它叫什麼」這一個問題。
        // ① 竦書的散文把名字畫成第一縱（見下面），框上再寫一遍是同一個詞兩格之隔；
        // ② 補全那一扈有意不要名字（2026-10-08 原話：「自動補全這個 title 太吸引
        // 眼球，建議不要。這個面板就是空的標題，這樣不會干扰視線」）。
        title: named.then(|| {
            (
                panel.title.clone(),
                Style::default().fg(ink.gold()).bg(crate::chrome::panel_ground(ink)),
            )
        }),
    });
    // Warning: **The body sets no ground of its own** (2026-09-18: 「命令行文字
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
    // **帶子沒有右牆，所以最後那一格也歸它**（2026-10-11 報的）：留一格空邊，
    // 底色就比正文短一格，而正文那邊沒有這一格——對齊當場差掉。
    let limit = match matches!(panel.stand, Stand::Band) && !vertical {
        true => rect.x + width,
        false => rect.x + width - 1,
    };
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
                    // Warning: **A turned table's rules have to reach across the
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
        Body::Prose(text) => {
            // **加重那一段只在最簡單的那一種上做**——見 [`Panel::accent`]。
            let accent = panel.accent.filter(|_| !panel.marked && !text.contains('\n'));
            let mut seen = 0usize;
            for (i, line) in lines.iter().enumerate() {
                let x = rect.x + 1 + pad as u16;
                let y = rect.y + 1 + i as u16;
                let plain = ground.fg(ink_of(i));
                let here = seen;
                let wide = line.chars().count();
                seen += wide;
                // 這一行落在加重那一段裏的就拆成三截畫：前、中、後。
                if let Some((from, to)) = accent {
                    let a = from.saturating_sub(here).min(wide);
                    let b = to.saturating_sub(here).min(wide);
                    if a < b {
                        let cut = |skip: usize, take: usize| {
                            line.chars().skip(skip).take(take).collect::<String>()
                        };
                        let (head, mid, tail) = (cut(0, a), cut(a, b - a), cut(b, wide - b));
                        let mut at = x;
                        put_text(buf, at, y, limit, &head, plain);
                        at += yumete_cjk::str_width(&head) as u16;
                        put_text(buf, at, y, limit, &mid, ground.fg(ink.gold()));
                        at += yumete_cjk::str_width(&mid) as u16;
                        put_text(buf, at, y, limit, &tail, plain);
                        continue;
                    }
                }
                // Warning: **Only the body, and only when it says it is Markdown.**
                // The 章節 line and the title above it are the panel's own
                // furniture and are set in their own inks (`ink_of`); running
                // them through the markup would let an asterisk in a file name
                // change the colour of the rest of the line.
                if !panel.marked || i < gold || quiet.contains(&i) {
                    put_text(buf, x, y, limit, line, plain);
                    continue;
                }
                crate::put_marked_text(buf, x, y, limit, line, plain, ink);
            }
        }
        // **一條一行，整行上淡底**——和 `:git-diff` 逐字同一套色（2026-10-11）。
        Body::Tinted(rows) => {
            // **左右不收口的那一種從最左畫起**（2026-10-11）：沒有左牆可跳，也沒
            // 有內邊距——帶子要和正文逐格對齊，差一格就全差了。
            let band = matches!(panel.stand, Stand::Band) && !vertical;
            // 翻過頁的話從第幾條起——最多翻到「最後一屏」，同散文那一支。
            let from = panel.scroll.min(rows.len().saturating_sub(count));
            for (i, row) in rows.iter().skip(from).take(count).enumerate() {
                let x = match band {
                    true => rect.x,
                    false => rect.x + 1 + pad as u16,
                };
                let y = rect.y + 1 + i as u16;
                let style = match row.wash {
                    // **整行那一層走 `washed`，行內那幾段走 `vcs`**（2026-10-11）。
                    // 兩個數之差正是主題自己的那條理由——響度由面積定：一行攤
                    // 開是 1.08–1.12，幾個字 3:1。重的那一檔不是另挑的，就是改
                    // 動條的底色，而它旁邊那一格 `+ -` 本來就是改動條。中間那
                    // 一檔（`short_wash`，1.5）量過，離淡底太近，一眼分不出來。
                    Some(accent) => ground.bg(ink.washed(accent)).fg(ink.text()),
                    None => ground.fg(ink.text()),
                };
                // 底色要鋪**整行**，不只鋪有字那一截——半行底色讀起來是兩塊東西。
                let room = limit.saturating_sub(x) as usize;
                let wide = yumete_cjk::str_width(&row.text);
                let padded = format!("{}{}", row.text, " ".repeat(room.saturating_sub(wide)));
                put_text(buf, x, y, limit, &padded, style);
                // 真改掉的那幾段再蓋一遍，底重一檔。
                let Some(accent) = row.wash else { continue };
                let strong = ground.bg(ink.vcs(accent)).fg(ink.text());
                for at in &row.marks {
                    let (Some(before), Some(text)) =
                        (row.text.get(..at.start), row.text.get(at.clone()))
                    else {
                        continue;
                    };
                    let from = x + yumete_cjk::str_width(before) as u16;
                    put_text(buf, from, y, limit, text, strong);
                }
            }
        }
        Body::Keys(keys) => {
            // `count` is what fits, which is not always all of them (above).
            let left = keys.len() - count;
            for (i, (key, what)) in keys.iter().take(count).enumerate() {
                let (column, row) = (i / deep, i % deep);
                let x = rect.x + 1 + (column * (one + 2)) as u16;
                let y = rect.y + 1 + row as u16;
                // The last cell counts what did not fit rather than letting the
                // table end as though those were all the keys there are.
                //
                // Warning: **Never the only cell**, the same law the prose cap keeps
                // above: on a page with one row to spare, a menu whose whole
                // content is 「還有 17 個」 has told the reader nothing they can
                // act on. Then the 「…」 hangs off the one key it could draw.
                let last = left > 0 && i + 1 == count && count > 1;
                let (key, what) = match (last, left > 0 && count == 1) {
                    (true, _) => ("…".to_string(), say!("menu.more", left)),
                    (_, true) => (key.clone(), format!("{what} …")),
                    _ => (key.clone(), what.clone()),
                };
                let said = match last {
                    true => ink.quiet(),
                    false => ink.text(),
                };
                // Warning: **A column is cut at its own edge, not at the ring**
                // (2026-09-19). `limit` is the box, so a meaning longer than
                // its column was written straight across the column beside it
                // — which only showed once a menu could have more than two.
                let edge = (x + one as u16).min(limit);
                let at = x + key_w as u16 + 2;
                put_text(buf, x, y, edge, &key, ground.fg(ink.gold()));
                let room = one.saturating_sub(key_w + 2);
                put_text(buf, at, y, edge, &clip(&what, room), ground.fg(said));
            }
        }
    }
    // Quietly, on the bottom edge and hard right: it is where the thing is
    // written, not part of what it says.
    let floor = rect.y + height - 1;
    // 翻得動就寫「怎麽翻」，否則寫這一扇自己的那句話（註寫在第幾行、開哪個檔）。
    // Warning: **竪排那一扇不在框上寫鍵**（2026-10-11 定，原話「though we do not
    // write the hints on the border of the panel」）。它的框是橫的而正文是竪的，
    // 一句橫着的提示貼在那條邊上讀起來像另一扇窗的東西；數字不一樣，它是一個記
    // 號不是一句話。
    // Warning: **竪排那一扇不在框上寫「怎麼翻」**（2026-10-11 定，原話「though we
    // do not write the hints on the border of the panel」）。它的框是橫的而正文是
    // 竪的，一句橫着的提示貼在那條邊上讀起來像另一扇窗的東西；數字不一樣，它是
    // 一個記號不是一句話。
    //
    // Warning: **收的只有那一句，不是整條邊。** 面板自己那一句（`tag`）照舊寫——
    // 改動帶子在竪排下是一扇浮窗，而它那一句是 `u 還原此處改動 · y 複製先前版
    // 本`，是**按得動的兩個鍵**，不是「還有，翻得動」。第一版把兩者一起收了，金
    // 樣當場紅了一幀。
    let right = match panel.pages && read.is_some() && !panel.vertical_text {
        true => Some(paged),
        false => panel.tag.clone(),
    };
    if let Some(tag) = right {
        // **放不下就截，別讓它跑出左牆**（2026-10-11 從金樣幀上看出來的）。寬度
        // 是由內容算的，上面又被 `room_w` 和 `area.width` 夾過兩道——夾完之後這
        // 一句話可能比底邊還長。從前它照 `limit - w - 1` 起筆，於是起點落在框
        // 外，畫出來是「y 複製先前版╯」：最後一個字被角吃掉，而讀者看不出那是
        // 被截的。兩頭各留一格空氣，同左端那個數字。
        let room = (width as usize).saturating_sub(3);
        let tag = clip(&tag, room);
        let w = yumete_cjk::str_width(&tag) as u16;
        put_text(buf, limit.saturating_sub(w + 1), floor, limit, &tag, ground.fg(ink.quiet()));
    }
    // **讀到第幾行，貼底邊左端**（2026-09-30 定的，和右端那句各佔一頭：兩類
    // 不同的信息，邊框有兩頭）。
    //
    // Warning: **兩邊各留一格空氣。** 貼着角寫出來的是「╰75/90───」，讀起來像
    // 被擠出去的（報的：「现在似乎顶到了前头」）。上邊那一行的標題本來就是
    // 「╭ 文檔」，這一行照它。
    if let Some((at, all)) = read {
        match panel.vertical_text {
            // **竪排寫在左邊那條框綫上，往下讀**（2026-10-11 定）。竪書從右往
            // 左讀，所以「讀到哪了」屬於左邊那一頭，和橫排擺在底邊左端是同一個
            // 位置——都是正文走完的地方。
            //
            // Warning: **半角數字，不是漢數字。** 框綫只有一格寬而漢字佔兩格，寫
            // 上去會壓到最左那一縱的字。作者看過兩種擺法之後定的：「10 / 36
            // 這樣就行」。
            true => {
                let room = rect.height.saturating_sub(2) as usize;
                // 放不下就把兩邊的空氣擠掉；再放不下就不寫——一個截一半的
                // 「10 / 3」說的是假話。
                let said = match (format!("{at} / {all}"), format!("{at}/{all}")) {
                    (airy, _) if airy.chars().count() <= room => airy,
                    (_, tight) if tight.chars().count() <= room => tight,
                    _ => String::new(),
                };
                for (n, c) in said.chars().enumerate() {
                    put_text(
                        buf,
                        rect.x,
                        rect.y + 1 + n as u16,
                        limit,
                        &c.to_string(),
                        ground.fg(ink.quiet()),
                    );
                }
            }
            false => {
                let said = format!(" {at}/{all} ");
                put_text(buf, rect.x + 1, floor, limit, &said, ground.fg(ink.quiet()));
            }
        }
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
        zong_at(text, w, h, 0)
    }

    /// 全部畫出來的字，連成一條——找一句話在不在用得上。
    fn said(buffer: &ratatui::buffer::Buffer) -> String {
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
        }
        out
    }

    /// 同上，`scroll` ＝ 從第幾縱起畫。
    fn zong_at(text: &str, w: u16, h: u16, scroll: usize) -> (Rect, ratatui::buffer::Buffer) {
        let config = Config::default();
        // **先 settle，再畫**——同 `render_with` 那一句（§5.12.58）。`Palette::of`
        // 讀的是進程全局的明暗，沒人撥過它就停在 0（＝淺），而那和「有人定成淺」
        // 分不出來。
        crate::theme::settle(&config, None);
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut got = None;
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, w, h);
                got = draw(frame, &config, area, h, crate::chrome::Spot::cell((w - 2, 0)), true, &Panel {
                    stand: Stand::Corner,
                    accent: None,
                    pages: false,
                    reading: false,
            scroll,
                    title: "洞庭湖".into(),
                    lede: None,
                    entry: true,
                    body: Body::Prose(text.into()),
                    vertical_text: true,
                    tag: None,
                    marked: false,
                });
            })
            .unwrap();
        (got.expect("the panel is drawn"), terminal.backend().buffer().clone())
    }

    /// **竪排的浮窗翻得動，而且說得出還有多少**（2026-10-11 報的）。
    ///
    /// 從前放不下的那幾縱是被 `truncate` 丟掉的，末縱接一個「…」——於是那四個
    /// 翻頁鍵按了沒反應，**不是攔住了，是後面根本沒有字**。原話：「我們截取了只
    /// 能顯示一頁的文本，剩下的都用…省略了」。
    #[test]
    fn a_vertical_panel_pages_and_says_how_far_it_has_got() {
        let long: String = (0..18)
            .map(|n| format!("第{n}段天地玄黃宇宙洪荒日月盈昃辰宿列張寒來暑往。\n"))
            .collect();
        let (rect, first) = zong_at(&long, 60, 18, 0);
        let head = said(&first);
        assert!(!head.contains('…'), "省略號去掉了：還有多少由數字說");

        // **數字竪着寫在左邊那條框綫上**（2026-10-11 定，原話「10 / 36 這樣就
        // 行」）。竪書從右往左讀，所以「讀到哪了」屬於左邊那一頭——和橫排擺在底
        // 邊左端是同一個位置。半角，因為框綫只有一格寬。
        let down: String = (1..rect.height)
            .map(|y| first[(rect.x, rect.y + y)].symbol().to_string())
            .collect();
        assert!(down.contains('/'), "左邊那一條上寫着數字：{down:?}");
        assert!(down.chars().any(|c| c.is_ascii_digit()), "{down:?}");

        // 翻到底：畫的是最後那幾縱，和第一屏不一樣，而數字跟着走。
        let (rect2, last) = zong_at(&long, 60, 18, 999);
        assert_ne!(said(&last), head, "翻過去就換了內容");
        let down2: String = (1..rect2.height)
            .map(|y| last[(rect2.x, rect2.y + y)].symbol().to_string())
            .collect();
        assert!(down2.contains('/'), "數字還在：{down2:?}");
        assert_ne!(down2, down, "讀到第幾縱變了");
    }

    /// **一張分了欄的表，第二欄不許落在框外**（2026-10-12 看出來的）。
    ///
    /// 分欄按**整個屏幕**減二算，而框寬會被 `chrome::room` 夾一道——兩個數不是
    /// 同一個，於是第二欄整個站在框外，`put_text` 停在框邊，畫出來是
    /// 「`讀音          h`」：一個字母，而且看不出那是被切的。
    #[test]
    fn a_two_column_key_table_cuts_inside_the_column_not_at_the_border() {
        let config = Config::default();
        crate::theme::settle(&config, None);
        let (w, h) = (86u16, 16u16);
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let rows: Vec<(String, String)> = [
            ("方案拆分", "彳亍"),
            ("編碼", "KKcu"),
            ("分節編碼", "Ki-Kcu"),
            ("讀音", "háng xíng héng xìng hàng"),
            ("字集", "簡繁古臺港 CJK"),
            ("統一碼", "U+884C"),
        ]
        .iter()
        .map(|(a, b)| ((*a).to_string(), (*b).to_string()))
        .collect();
        let mut got = None;
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, w, h);
                // `bottom` 壓到六行：放不下六條，這一支於是分欄——要驗的正是
                // 分欄之後的那一下。
                got = draw(frame, &config, area, 6, crate::chrome::Spot::cell((2, 0)), false, &Panel {
                    stand: Stand::Corner,
                    accent: None,
                    pages: false,
                    reading: true,
                    scroll: 0,
                    title: "行".into(),
                    lede: None,
                    entry: false,
                    body: Body::Keys(rows),
                    vertical_text: false,
                    tag: None,
                    marked: false,
                });
            })
            .unwrap();
        let rect = got.expect("the panel is drawn");
        let buf = terminal.backend().buffer();
        // Warning: **寬字的第二格在 `TestBackend` 裏讀不出來**，所以逐行取完符號
        // 之後把空白去掉再比——量的是「讀音後面還剩幾個字」，不是版面。
        let row_at = |y: u16| -> String {
            (rect.x + 1..rect.x + rect.width - 1)
                .map(|x| buf[(x, y)].symbol())
                .collect::<String>()
                .replace(' ', "")
        };
        let found = (rect.y + 1..rect.y + rect.height - 1)
            .map(row_at)
            .find_map(|row| row.split_once("讀音").map(|(_, rest)| rest.to_string()))
            .expect("讀音 那一行畫出來了");
        // 被框邊切掉的時候剩的是一個字母（`h`）；切在欄裏剩的是一截帶省略號的
        // 讀音，一欄放得下的時候剩的是整串。
        assert!(found.chars().count() > 1, "第二欄被框邊切了：{found:?}");
        assert!(
            found.ends_with('…') || found.ends_with("hàng"),
            "切了就要看得出是切的：{found:?}"
        );
    }

    /// **兩段之間那一縱空白留着**（2026-10-12 報的：「In vertical mode, you
    /// still removed the empty line between paragraphs」）。
    ///
    /// 縮進那一層 2026-10-11 從兩邊一起拿掉了，可空行還少一處：`zong_rows` 自己
    /// 把空段落濾了，理由寫着「在框裏它是一縱空白」——而那一縱空白正是兩段之間
    /// 的界。橫排那一邊從來沒省過它。
    #[test]
    fn a_vertical_entry_keeps_the_blank_between_two_paragraphs() {
        let (_, buf) = zong("第一段天地玄黃。\n\n第二段宇宙洪荒。\n", 40, 14);
        let page = said(&buf);
        let (one, two) = (page.find('第').unwrap(), page.rfind('第').unwrap());
        assert_ne!(one, two, "兩段都畫出來了");
        // 兩段各一縱，中間空一縱：所以「第」與「第」隔着的不止一縱的寬度。
        let (rect, _) = zong("第一段天地玄黃。\n\n第二段宇宙洪荒。\n", 40, 14);
        let (rect_tight, _) = zong("第一段天地玄黃。\n第二段宇宙洪荒。\n", 40, 14);
        assert_eq!(
            rect.width,
            rect_tight.width + 2,
            "空行自己佔一縱，所以框寬多兩格"
        );
    }

    /// **竪排的框不許比正文寬**（2026-10-12 報的：「there seemed to be a minimum
    /// width of the panel so that there are much space padded at left」）。
    ///
    /// 三道底綫——名字、「怎麼翻」、讀到第幾——量的都是**畫在框綫上**的東西，而
    /// 竪排一樣都不畫在那兒：名字是第一縱，數字順着左框往下走。照橫排撐出來的那
    /// 二十幾格於是全是空的。
    #[test]
    fn a_vertical_panel_is_no_wider_than_what_it_holds() {
        let config = Config::default();
        crate::theme::settle(&config, None);
        let (w, h) = (60u16, 20u16);
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut got = None;
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, w, h);
                got = draw(frame, &config, area, h, crate::chrome::Spot::cell((w - 2, 0)), true, &Panel {
                    stand: Stand::Corner,
                    accent: None,
                    // 翻得動、而且名字很長：兩道底綫都站得最高的時候。
                    pages: true,
                    reading: false,
                    scroll: 0,
                    title: "洞庭湖畔的那一座樓".into(),
                    lede: None,
                    entry: true,
                    body: Body::Prose("短。".into()),
                    vertical_text: true,
                    tag: None,
                    marked: false,
                });
            })
            .unwrap();
        let rect = got.expect("the panel is drawn");
        // 兩縱（標題、空、正文）＝ 六格，加兩格空氣、兩格框。
        assert_eq!(rect.width, 3 * 2 + 2 + 2, "{rect:?}");
    }

    /// **一行帶行內標記的標題出現在浮窗正文裏，從前會當場崩**（2026-09-28）。
    ///
    /// `markdown::spans("# **甲**")` 交的是 `HeadingMark 0..1`/`Heading 1..7`/
    /// `Marker 2..4`/`Strong 4..5`/`Marker 5..7`——**它會重疊**。從前這一段是順着往右
    /// 切的（記一個 `from`，切 `chars[from..span.start]`），於是 `from` 已經走到 7 而下
    /// 一段從 2 開始，`chars[7..2]` 是一個反向區間，Rust 當場 panic。
    ///
    /// Warning: 這一條要真的**畫**出來纔驗得到：崩在繪製那一步，不在解析那一步。
    #[test]
    fn a_heading_with_inline_marks_does_not_crash_the_panel() {
        let config = Config::default();
        crate::theme::settle(&config, None);
        let (w, h) = (40u16, 10u16);
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, w, h);
                draw(frame, &config, area, h, crate::chrome::Spot::cell((w - 2, 0)), true, &Panel {
                    stand: Stand::Corner,
                    accent: None,
                    pages: false,
                    reading: false,
            scroll: 0,
                    title: "條目".into(),
                    lede: None,
                    // Warning: **不能是 `entry`**：那一檔會給每段加一個全角空格縮進
                    // （`indented()`），於是 `#` 不在行首、不算標題，重疊也就不出現，
                    // 這一條會假綠。
                    entry: false,
                    body: Body::Prose("# **甲**\n## 第一章 `碼`\n正文一行。".into()),
                    vertical_text: false,
                    tag: None,
                    // Warning: 這一格是閘：關着的話整段標記都不畫，也就碰不到那個洞。
                    marked: true,
                });
            })
            .unwrap();
        let text: String = terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(text.contains('甲'), "標題畫出來了，而且一路沒崩：{text:?}");
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
        let entry = "洞庭湖辭典 > 岳陽樓。\n\
                     湖面約 2579 平方千米，北納長江，南接湘資沅澧四水。\n\
                     慶曆四年春，滕子京謫守巴陵郡。越明年，政通人和，百廢具興。\n\
                     乃重修岳陽樓，增其舊制，刻唐賢今人詩賦於其上，屬予作文以記之。\n\
                     予觀夫巴陵勝狀，在洞庭一湖——銜遠山，吞長江，浩浩湯湯，橫無際涯。\n\
                     至若春和景明，波瀾不驚，上下天光，一碧萬頃，沙鷗翔集，錦鱗游泳。\n\
                     登斯樓也，則有心曠神怡，寵辱偕忘，把酒臨風，其喜洋洋者矣。\n\
                     先天下之憂而憂，後天下之樂而樂——噫，微斯人，吾誰與歸。";
        let (rect, buffer) = zong(entry, 70, 45);

        // 竪排 the room is two thirds of the page tall, and a full entry uses
        // all of it: 45 × 2/3 = 30.
        assert_eq!(rect.height, 30, "the box is {} rows deep", rect.height);
        // No 縱 may be deeper than the room — that was the wrapper's cell/字
        // confusion, and it is what tripped the cap.
        assert!(rect.height <= 45 * 2 / 3);

        // **放不下的那一截由底邊那個數字說**（2026-10-11 改的）。從前這裏是一個
        // 竪排的省略號 `︙`，而那時放不下就是丟掉；翻得動之後它換成了「讀到第幾
        // ／共幾」，而那個數還順帶說了「還有多少」——同橫排 2026-09-30 那一條。
        let text: String = (0..rect.height)
            .flat_map(|y| (0..rect.width).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(rect.x + x, rect.y + y)].symbol().to_string())
            .collect();
        assert!(!text.contains('︙'), "省略號去掉了：{text:?}");
        let down: String = (1..rect.height)
            .map(|y| buffer[(rect.x, rect.y + y)].symbol().to_string())
            .collect();
        assert!(down.contains('/'), "改由左邊那一條上的數字說：{down:?}");

        // 禁則, down the column: no 縱 may open with a mark that closes
        // something. Hand-chopping the horizontal wrapper's rows put 「，」 at
        // the head of a 縱, which is the one thing a typesetter never does.
        let head: String = (0..rect.width)
            .map(|x| buffer[(rect.x + x, rect.y + 1)].symbol().to_string())
            .collect();
        for mark in ['︐', '︑', '︒', '︓', '﹂', '︶'] {
            assert!(!head.contains(mark), "{mark} opens a 縱: {head:?}");
        }

        // Warning: **這裏從前還驗「省略號掛在末縱的腳下、不自己占一縱」**。那一條
        // 連同省略號本身一起沒了（2026-10-11）：放不下的那幾縱現在翻得到，而
        // 「還有」由底邊那個數字說。
        // And the title is still its own 縱, in the box rather than on the ring.
        assert!(text.contains('洞') && text.contains('典'), "{text:?}");
    }

    /// A short entry keeps the box short — the height follows the deepest 縱,
    /// it is not padded out to the room.
    #[test]
    fn a_short_vertical_entry_does_not_grow_a_box_it_does_not_need() {
        let (rect, _) = zong("短。", 70, 45);
        assert!(rect.height <= 6, "the box is {} rows deep", rect.height);
    }

    /// Draw the key table a half-pressed prefix puts up, on a `w × h` page.
    fn menu(keys: usize, w: u16, h: u16) -> (Option<Rect>, ratatui::buffer::Buffer) {
        let config = Config::default();
        // **先 settle，再畫**——同 `render_with` 那一句（§5.12.58）。`Palette::of`
        // 讀的是進程全局的明暗，沒人撥過它就停在 0（＝淺），而那和「有人定成淺」
        // 分不出來。
        crate::theme::settle(&config, None);
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        let mut got = None;
        terminal
            .draw(|frame| {
                let body = (0..keys)
                    .map(|n| (format!("k{n}"), format!("第{n}個動作")))
                    .collect::<Vec<_>>();
                got = draw(frame, &config, Rect::new(0, 0, w, h), h - 1, crate::chrome::Spot::cell((0, 0)), false, &Panel {
                    stand: Stand::Corner,
                    accent: None,
                    pages: false,
                    reading: false,
            scroll: 0,
                    title: "空格".into(),
                    lede: None,
                    entry: false,
                    body: Body::Keys(body),
                    vertical_text: false,
                    tag: None,
                    marked: false,
                });
            })
            .unwrap();
        (got, terminal.backend().buffer().clone())
    }

    fn written(rect: Rect, buffer: &ratatui::buffer::Buffer) -> String {
        (0..rect.height)
            .flat_map(|y| (0..rect.width).map(move |x| (x, y)))
            .map(|(x, y)| buffer[(rect.x + x, rect.y + y)].symbol().to_string())
            .collect()
    }

    /// 2026-09-19, caught in review: on a short terminal the `空格` menu drew
    /// **nothing**. The key table had no height cap of its own — one column or
    /// two — so its box came out taller than the page, `chrome::place` refused
    /// it, and `draw` answered `None`, which the caller could not tell from
    /// 「there was nothing to show」. A menu that does not fit goes wider, and
    /// says in its last cell how many keys it could not draw.
    #[test]
    fn a_menu_too_tall_for_the_page_goes_wider_and_says_what_it_cut() {
        let (rect, buffer) = menu(24, 60, 12);
        let rect = rect.expect("the menu is drawn at all");
        assert!(rect.height <= 12 && rect.y + rect.height <= 12, "{rect:?}");
        let text = written(rect, &buffer);
        assert!(text.contains('…'), "the cut is not marked: {text:?}");
        // Wider, not just shorter: the keys that did fit are in more than one
        // column, and the first of them is still there.
        assert!(rect.width > 20, "one thin column: {rect:?}");
        assert!(text.contains("k0"), "{text:?}");
    }

    /// …and on a page with room it is the whole table, with nothing cut and no
    /// mark to say so.
    #[test]
    fn a_menu_that_fits_is_drawn_whole() {
        let (rect, buffer) = menu(24, 120, 40);
        let rect = rect.expect("the menu is drawn");
        let text = written(rect, &buffer);
        assert!(text.contains("k23"), "the last key is missing: {text:?}");
        assert!(!text.contains('…'), "nothing was cut, yet it says so: {text:?}");
    }

    /// **正在填的那一段加重，其餘不動**（2026-10-08，簽名提示）。
    ///
    /// 量的是墨：`value: T` 那幾格是金的，`fn push(` 與最後那一個 `)` 不是。
    #[test]
    fn the_parameter_being_filled_is_the_only_gold_in_the_line() {
        let config = Config::default();
        crate::theme::settle(&config, None);
        let ink = crate::theme::Palette::of(&config);
        let label = "fn push(&mut self, value: T)";
        let at = label.find("value: T").expect("在");
        let span = (label[..at].chars().count(), label[..at].chars().count() + "value: T".chars().count());
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut got = None;
        terminal
            .draw(|frame| {
                got = draw(frame, &config, Rect::new(0, 0, 80, 24), 23, crate::chrome::Spot::cell((0, 0)), false, &Panel {
                    stand: Stand::Corner,
                    accent: Some(span),
                    pages: false,
                    reading: false,
                    scroll: 0,
                    title: "文檔".into(),
                    lede: None,
                    entry: false,
                    body: Body::Prose(label.into()),
                    vertical_text: false,
                    tag: None,
                    marked: false,
                });
            })
            .unwrap();
        let rect = got.expect("畫出來了");
        let buffer = terminal.backend().buffer().clone();
        // 正文那一行：框裏的第一行。每一格的字與墨一起收下來。
        let row: Vec<(String, bool)> = (1..rect.width - 1)
            .map(|x| {
                let cell = &buffer[(rect.x + x, rect.y + 1)];
                (cell.symbol().to_string(), cell.style().fg == Some(ink.gold()))
            })
            .collect();
        let gold: String = row.iter().filter(|(_, g)| *g).map(|(c, _)| c.as_str()).collect();
        let rest: String =
            row.iter().filter(|(_, g)| !*g).map(|(c, _)| c.as_str()).collect::<String>();
        assert_eq!(gold, "value: T", "金的只該是正在填的那一個參數");
        assert!(rest.trim().starts_with("fn push(&mut self,"), "別的段照舊：{rest:?}");
        assert!(rest.trim().ends_with(')'), "最後那一個括號不該是金的：{rest:?}");
    }

    /// 服務器沒說是哪一段就一格金的都沒有——寧可不加重。
    #[test]
    fn no_accent_means_no_gold_in_the_body() {
        let config = Config::default();
        crate::theme::settle(&config, None);
        let ink = crate::theme::Palette::of(&config);
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut got = None;
        terminal
            .draw(|frame| {
                got = draw(frame, &config, Rect::new(0, 0, 80, 24), 23, crate::chrome::Spot::cell((0, 0)), false, &Panel {
                    stand: Stand::Corner,
                    accent: None,
                    pages: false,
                    reading: false,
                    scroll: 0,
                    title: "文檔".into(),
                    lede: None,
                    entry: false,
                    body: Body::Prose("fn push(&mut self, value: T)".into()),
                    vertical_text: false,
                    tag: None,
                    marked: false,
                });
            })
            .unwrap();
        let rect = got.expect("畫出來了");
        let buffer = terminal.backend().buffer().clone();
        let gold = (1..rect.width - 1)
            .filter(|&x| buffer[(rect.x + x, rect.y + 1)].style().fg == Some(ink.gold()))
            .count();
        assert_eq!(gold, 0, "沒說就不加重");
    }
}
