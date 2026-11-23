//! **What yumete learned** (`:yume-words`, `空格 z u`): the page, the Yume
//! app's ⌃⇧U beside the text.
//!
//! ```text
//!              词                编码              ┆来源
//!  ────────────┬────────────────────────────────────────────────────
//!   词库    23 │ 词库
//!   调频    41 │  霜花           ┆fkhh fkh         ┆yumete
//!   固顶     2 │  指尖           ┆zjia             ┆yumete＋应用
//!              ├─↓ 还有 10 条 ───────────────────────────────────────
//!              │  「指尖」：yumete 学到的编码 zjia，宇梦 App 里也有（zjia）
//!              │  删掉之后  只删 yumete 这一份；宇梦 App 那一份还在 …
//!              │  在        ~/.local/share/yumete/yume/lingming/user_dict.txt 第 7 行
//!  ────────────┴────────────────────────────────────────────────────
//!   jk 上下, hl 换页, / 搜索, d 删除, u 恢复, y 复制这条, …
//!  TBL.NOR  yumete 学到的词   靈明   只看 yumete                第 2 / 23 条
//! ```
//!
//! **Two layers** (see [`yumete_ime::user_words`]): what yumete learned, and
//! the Yume app's, which yumete only reads. The app's rows are hidden until
//! `Tab`, drawn quiet, and refused by `d`. `d` takes yumete's copy — a whole
//! word with every code, or one row of 调频 or 固顶 — out of its file and out
//! of the engine at once; `u` puts back, latest first, until the page closes.
//! A row both layers hold stays after `d`, as the app's.
//!
//! No key edits a file in a buffer: yumete writes those files itself while
//! it types, and a buffer open on one would race it.

use std::cell::Cell;

use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::Clear;
use ratatui::Frame;

use yumete_config::Config;
use yumete_core::input::{Key, Mode};
use yumete_core::say;
use yumete_core::Editor;
use yumete_ime::{Deleted, Held, ImeSession, WordList, WordRow};

use crate::{put_fit, put_text};

/// Below this many columns the lists' column folds away.
const LISTS_FROM: u16 = 60;

/// The lists' column, wide enough for 「拼音调频  999」.
const LISTS_WIDE: u16 = 16;

/// The narrowest a column is drawn, so a list of short words still reads as
/// a table.
const MIN_COLUMN: u16 = 10;

/// **The page in the front end**: open, keys, close. The loop and `--keys`
/// both go through it.
#[derive(Default)]
pub struct Seat {
    page: Option<Page>,
}

#[derive(Default)]
struct Page {
    /// Which list, by its place among the scheme's.
    list: usize,
    /// The cursor's row among the rows shown.
    row: usize,
    /// `Tab`: the app's rows shown too.
    all: bool,
    search: String,
    /// The `/` line is being typed.
    searching: bool,
    /// What `d` took, latest last.
    deleted: Vec<Deleted>,
    /// Rows `d` took yumete's copy of while the app keeps its own: shown as
    /// the app's until the page closes, so the row does not jump away.
    kept: Vec<(WordList, String, String)>,
    /// The first row drawn, kept between frames so the list scrolls.
    first: Cell<usize>,
    /// **The scheme `s` turned the page to**, read beside the one that
    /// types; `None` while the page shows the scheme that types. `s` is a
    /// question about what to look at, so it leaves the input method as it
    /// was — the status line still says what the next letter does.
    other: Option<ImeSession>,
    /// A file was written through [`Page::other`]: the scheme that types
    /// may share it (a 拼音 record several schemes keep), so it looks again
    /// when the page closes.
    wrote_beside: bool,
}

/// The list's own name.
fn list_name(list: WordList) -> String {
    match list {
        WordList::Words => say!("yume.words.kind.words"),
        WordList::Freq => say!("yume.words.kind.freq"),
        WordList::PinyinFreq => say!("yume.words.kind.pinyin-freq"),
        WordList::Pins => say!("yume.words.kind.pins"),
    }
}

/// The columns of a list, by name: each list its own.
fn headings(list: WordList) -> Vec<String> {
    let (word, code, source) =
        (say!("yume.words.column.word"), say!("yume.words.column.code"), say!("yume.words.column.source"));
    match list {
        WordList::Words => vec![word, code, source],
        WordList::Freq | WordList::PinyinFreq => vec![code, word, say!("yume.words.column.count"), source],
        WordList::Pins => vec![code, word, say!("yume.words.column.place"), source],
    }
}

fn source(held: Held) -> String {
    match held {
        Held::Own => say!("yume.words.source.own"),
        Held::App => say!("yume.words.source.app"),
        Held::Both => say!("yume.words.source.both"),
    }
}

/// A row's cells, in its list's columns. The number is yumete's where it has
/// one: that is the row's own.
fn cells(list: WordList, row: &WordRow) -> Vec<String> {
    let number = row.own.or(row.app).map(|n| n.to_string()).unwrap_or_default();
    match list {
        WordList::Words => vec![row.word.clone(), row.code.clone(), source(row.held)],
        _ => vec![row.code.clone(), row.word.clone(), number, source(row.held)],
    }
}

/// Rows as TSV, the columns' names first.
fn tsv(list: WordList, rows: &[&WordRow]) -> String {
    let mut out = headings(list).join("\t");
    out.push('\n');
    for row in rows {
        out.push_str(&cells(list, row).join("\t"));
        out.push('\n');
    }
    out
}

fn matches(row: &WordRow, search: &str) -> bool {
    let low = search.to_lowercase();
    row.word.contains(search) || row.code.to_lowercase().contains(&low)
}

impl Page {
    /// The session whose words the page shows.
    fn session<'a>(&'a self, ime: &'a ImeSession) -> &'a ImeSession {
        self.other.as_ref().unwrap_or(ime)
    }

    fn list(&self, ime: &ImeSession) -> Option<WordList> {
        let lists = self.session(ime).word_lists();
        lists.get(self.list.min(lists.len().saturating_sub(1))).copied()
    }

    /// The rows of `list` this page shows: yumete's, the app's after `Tab`
    /// (and those `d` left as the app's), narrowed by the search.
    fn rows(&self, ime: &ImeSession, list: WordList, searched: bool) -> Vec<WordRow> {
        self.session(ime)
            .word_rows(list)
            .into_iter()
            .filter(|r| {
                r.held != Held::App
                    || self.all
                    || self.kept.iter().any(|(l, c, w)| *l == list && *c == r.code && *w == r.word)
            })
            .filter(|r| !searched || self.search.is_empty() || matches(r, &self.search))
            .collect()
    }

    fn shown(&self, ime: &ImeSession) -> (Option<WordList>, Vec<WordRow>) {
        match self.list(ime) {
            Some(list) => (Some(list), self.rows(ime, list, true)),
            None => (None, Vec::new()),
        }
    }

    /// `s`: **the next scheme whose 碼表 is there**, in menu order, round to
    /// the start — the one that types read from `ime` itself, any other
    /// read beside it.
    fn turn_to_the_next_scheme(&mut self, ime: &ImeSession) {
        let start = self.session(ime).scheme();
        let mut next = start.next();
        while next != start {
            if next == ime.scheme() && ime.available() {
                self.other = None;
                return;
            }
            if let Some(other) = ime.word_lists_of(next) {
                self.other = Some(other);
                return;
            }
            next = next.next();
        }
    }
}

impl Seat {
    pub fn open(&self) -> bool {
        self.page.is_some()
    }

    /// Text pasted while the page is open goes nowhere.
    pub fn pasted(&self) -> bool {
        self.open()
    }

    /// **Does this key belong to the page?** Every key does but `:`, which
    /// opens the command line over it (`:q` closes it).
    pub fn took(&mut self, editor: &mut Editor, ime: &mut ImeSession, key: Option<Key>) -> bool {
        let Some(page) = self.page.as_mut() else { return false };
        if editor.mode().is_prompt() {
            return false;
        }
        let Some(key) = key else { return true };
        let lists = page.session(ime).word_lists().len().max(1);
        match key {
            Key::Char(':') => return false,
            Key::Char('j') | Key::Down => page.row += 1,
            Key::Char('k') | Key::Up => page.row = page.row.saturating_sub(1),
            Key::Char('g') | Key::Home => page.row = 0,
            Key::Char('G') | Key::End => page.row = usize::MAX,
            Key::Char('h') | Key::Left => {
                page.list = (page.list.min(lists - 1) + lists - 1) % lists;
                page.row = 0;
            }
            Key::Char('l') | Key::Right => {
                page.list = (page.list.min(lists - 1) + 1) % lists;
                page.row = 0;
            }
            // **The search is typed on the editor's own value line**, as the
            // settings panel's is: the input method composes there and a lone
            // Shift switches 中/英, so a word is found by what it is written as.
            Key::Char('/') => {
                page.search.clear();
                page.searching = true;
                page.row = 0;
                editor.open_value_search_line(&say!("yume.words.search", ""), "");
            }
            Key::Esc => page.search.clear(),
            Key::Tab => {
                page.all = !page.all;
                page.row = 0;
            }
            Key::Char('d') => self.delete(editor, ime),
            Key::Char('u') => self.restore(editor, ime),
            Key::Char('y') => self.copy(editor, ime, false),
            Key::Char('Y') => self.copy(editor, ime, true),
            // The next scheme in menu order, as `空格 z n` walks them; the
            // page shows that one's words, and the one that types stays.
            Key::Char('s') => {
                page.turn_to_the_next_scheme(ime);
                page.list = 0;
                page.row = 0;
                page.kept.clear();
            }
            Key::Char('q') => self.close(editor, ime),
            _ => {}
        }
        self.clamp(ime);
        true
    }

    fn clamp(&mut self, ime: &ImeSession) {
        let Some(page) = self.page.as_mut() else { return };
        let (_, rows) = page.shown(ime);
        page.row = page.row.min(rows.len().saturating_sub(1));
    }

    /// **`d`: yumete's copy of the row**, out of its file and the engine.
    fn delete(&mut self, editor: &mut Editor, ime: &mut ImeSession) {
        let Some(page) = self.page.as_mut() else { return };
        let (Some(list), rows) = page.shown(ime) else {
            editor.set_status(say!("yume.words.no-row"));
            return;
        };
        let Some(row) = rows.get(page.row) else {
            editor.set_status(say!("yume.words.no-row"));
            return;
        };
        if row.held == Held::App {
            editor.set_status(say!("yume.words.refused-app"));
            return;
        }
        page.wrote_beside |= page.other.is_some();
        let session = match page.other.as_mut() {
            Some(other) => other,
            None => ime,
        };
        let Some(gone) = session.delete_own_row(list, row) else {
            editor.set_status(say!("yume.words.refused-app"));
            return;
        };
        page.deleted.push(gone);
        if row.held == Held::Both && !page.all {
            page.kept.push((list, row.code.clone(), row.word.clone()));
        }
        editor.set_status(match list {
            WordList::Words => say!("yume.words.deleted-word", &row.word, row.code.split_whitespace().count().max(1)),
            WordList::Freq | WordList::PinyinFreq => say!("yume.words.deleted-freq", &row.code, &row.word),
            WordList::Pins => say!("yume.words.deleted-pin", &row.code, &row.word),
        });
    }

    /// **`u`: what `d` took last, back where it stood.**
    fn restore(&mut self, editor: &mut Editor, ime: &mut ImeSession) {
        let Some(page) = self.page.as_mut() else { return };
        let Some(gone) = page.deleted.pop() else {
            editor.set_status(say!("yume.words.nothing-to-restore"));
            return;
        };
        page.wrote_beside |= page.other.is_some();
        let session = match page.other.as_mut() {
            Some(other) => other,
            None => ime,
        };
        if gone.tag() != session.scheme().tag() {
            page.deleted.push(gone);
            editor.set_status(say!("yume.words.restore-other-scheme"));
            return;
        }
        let list = gone.list();
        session.restore_own_row(gone);
        if let Some(at) = session.word_lists().iter().position(|l| *l == list) {
            page.list = at;
        }
        editor.set_status(say!("yume.words.restored"));
    }

    /// `y` the row, `Y` the list as shown, as TSV.
    fn copy(&mut self, editor: &mut Editor, ime: &ImeSession, whole: bool) {
        let Some(page) = self.page.as_ref() else { return };
        let (Some(list), rows) = page.shown(ime) else {
            editor.set_status(say!("yume.words.no-row"));
            return;
        };
        if whole {
            match rows.is_empty() {
                true => editor.set_status(say!("yume.words.no-row")),
                false => {
                    editor.send_to_clipboard(tsv(list, &rows.iter().collect::<Vec<_>>()));
                    editor.set_status(say!("yume.words.copied-page", rows.len()));
                }
            }
            return;
        }
        match rows.get(page.row) {
            Some(row) => {
                editor.send_to_clipboard(cells(list, row).join("\t"));
                editor.set_status(say!("yume.words.copied"));
            }
            None => editor.set_status(say!("yume.words.no-row")),
        }
    }

    /// **What the core was asked**: `:yume-words`, and `:q` over the page.
    pub fn settle(&mut self, editor: &mut Editor, ime: &mut ImeSession) {
        if editor.take_word_page_request() && self.page.is_none() {
            // What the last commits taught is in the files before they are shown.
            ime.save_learning();
            ime.refresh_user_words();
            self.page = Some(Page::default());
            editor.set_word_page_open(true);
        }
        if editor.take_word_page_close() {
            self.close(editor, ime);
        }
        // The search narrows as it is typed; Enter keeps it, Esc clears it.
        if let Some(page) = self.page.as_mut().filter(|p| p.searching) {
            let typed = match editor.take_value_answer() {
                Some(answer) => {
                    page.searching = false;
                    answer.unwrap_or_default()
                }
                None if editor.mode() != Mode::Value => {
                    page.searching = false;
                    page.search.clone()
                }
                None => editor.value_typed().unwrap_or_default().to_string(),
            };
            if typed != page.search {
                page.search = typed;
                page.row = 0;
            }
            self.clamp(ime);
        }
    }

    fn close(&mut self, editor: &mut Editor, ime: &mut ImeSession) {
        if self.page.as_ref().is_some_and(|p| p.searching) {
            editor.close_value_line();
        }
        if self.page.take().is_some_and(|p| p.wrote_beside) {
            ime.refresh_user_words();
        }
        editor.set_word_page_open(false);
        editor.set_status(String::new());
    }
}

/// **What the row is, what `d` does to it, and where it lives.**
fn detail(list: WordList, row: &WordRow) -> [String; 3] {
    let n = |v: Option<i64>| v.unwrap_or(0);
    let (what, after) = match (list, row.held) {
        (WordList::Words, Held::Own) => {
            (say!("yume.words.detail.word-own", &row.word, &row.code), say!("yume.words.after.word-own", &row.code, &row.word))
        }
        (WordList::Words, Held::Both) => (
            say!("yume.words.detail.word-both", &row.word, &row.code, &row.app_code),
            say!("yume.words.after.word-both"),
        ),
        (WordList::Words, Held::App) => (say!("yume.words.detail.word-app", &row.word, &row.code), say!("yume.words.after.app")),
        (WordList::Freq | WordList::PinyinFreq, Held::Own) => {
            (say!("yume.words.detail.freq-own", &row.code, &row.word, n(row.own)), say!("yume.words.after.freq-own"))
        }
        (WordList::Freq | WordList::PinyinFreq, Held::Both) => (
            say!("yume.words.detail.freq-both", &row.code, &row.word, n(row.own), n(row.app), n(row.own) + n(row.app)),
            say!("yume.words.after.freq-both", n(row.app)),
        ),
        (WordList::Freq | WordList::PinyinFreq, Held::App) => {
            (say!("yume.words.detail.freq-app", &row.code, &row.word, n(row.app)), say!("yume.words.after.app"))
        }
        (WordList::Pins, held) => {
            let place = match held {
                Held::App => n(row.app),
                _ => n(row.own),
            };
            let what = match (held, place) {
                (Held::App, 0) => say!("yume.words.detail.unpin-app", &row.code, &row.word),
                (Held::App, _) => say!("yume.words.detail.pin-app", &row.code, &row.word, place),
                (_, 0) => say!("yume.words.detail.unpin-own", &row.code, &row.word),
                _ => say!("yume.words.detail.pin-own", &row.code, &row.word, place),
            };
            let after = match held {
                Held::Own => say!("yume.words.after.pin-own"),
                Held::Both => say!("yume.words.after.pin-both"),
                Held::App => say!("yume.words.after.app"),
            };
            (what, after)
        }
    };
    let file = match row.held {
        Held::App => row.app_file.as_deref(),
        _ => row.file.as_deref(),
    };
    let place = match (file, row.line.filter(|_| row.held != Held::App)) {
        (Some(path), Some(line)) => say!("yume.words.file-line", crate::settings_page::shorten(path), line),
        (Some(path), None) => crate::settings_page::shorten(path),
        (None, _) => String::new(),
    };
    [what, after, place]
}

/// The widths of a list's columns: each as wide as its widest cell, the last
/// taking what is left.
fn widths(heads: &[String], rows: &[Vec<String>], room: u16) -> Vec<u16> {
    let cap = (room / 3).max(6);
    let mut out: Vec<u16> = heads
        .iter()
        .enumerate()
        .map(|(i, head)| {
            let widest = rows.iter().map(|r| yumete_cjk::str_width(&r[i])).chain([yumete_cjk::str_width(head)]).max();
            (widest.unwrap_or(0) as u16 + 2).max(MIN_COLUMN).min(cap)
        })
        .collect();
    let used: u16 = out.iter().take(out.len().saturating_sub(1)).sum();
    if let Some(last) = out.last_mut() {
        *last = room.saturating_sub(used);
    }
    out
}

fn fill(buf: &mut ratatui::buffer::Buffer, from: u16, to: u16, y: u16, style: Style) {
    for x in from..to {
        if let Some(cell) = buf.cell_mut((x, y)) {
            cell.set_symbol(" ").set_style(style);
        }
    }
}

/// **The page, drawn over the whole of `area`.** No caret: nothing is typed
/// into a cell here, and the `/` line is on the status line.
pub fn draw(frame: &mut Frame, seat: &Seat, ime: &ImeSession, config: &Config, area: Rect) -> Option<Position> {
    let page = seat.page.as_ref()?;
    if area.width == 0 || area.height == 0 {
        return None;
    }
    let ink = crate::theme::Palette::of(config);
    let ground = ink.ground(yumete_config::rung::CHROME);
    let text = ground.fg(ink.text());
    let head = ground.fg(ink.gold()).add_modifier(Modifier::BOLD);
    let quiet = ground.fg(ink.quiet());
    let rule_ink = ground.fg(ink.rule());
    frame.render_widget(Clear, area);
    let buf = frame.buffer_mut();
    let right = area.x + area.width;
    for y in area.y..area.y + area.height {
        fill(buf, area.x, right, y, ground);
    }
    let lists = page.session(ime).word_lists();
    let (list, rows) = page.shown(ime);
    let at = page.row.min(rows.len().saturating_sub(1));
    let side = match area.width >= LISTS_FROM {
        true => LISTS_WIDE,
        false => 0,
    };
    let split = area.x + side;
    let from = split + 2;

    // ---- The status line, at the foot ---------------------------------------
    let bottom = area.y + area.height - 1;
    if area.height >= 3 {
        let bar = crate::status_bar_style(config);
        fill(buf, area.x, right, bottom, bar);
        let mut x = put_text(buf, area.x + 1, bottom, right, "TBL.NOR", bar.fg(ink.gold()).add_modifier(Modifier::BOLD)) + 2;
        let show = match page.all {
            true => say!("yume.words.show-all"),
            false => say!("yume.words.show-own"),
        };
        let mut said = format!("{}   {}   {show}", say!("yume.words.title"), page.session(ime).scheme_name());
        if page.searching || !page.search.is_empty() {
            said.push_str(&format!("   {}", say!("yume.words.search", &page.search)));
        }
        let position = match rows.len() {
            0 => String::new(),
            n => say!("yume.words.position", at + 1, n),
        };
        let wide = yumete_cjk::str_width(&position) as u16;
        let ends = right.saturating_sub(wide + 4);
        x = put_text(buf, x, bottom, ends, &said, bar);
        let _ = x;
        if !position.is_empty() {
            put_text(buf, right.saturating_sub(wide + 1), bottom, right, &position, bar);
        }
    }

    // ---- The keys, above it --------------------------------------------------
    let keys = match page.searching {
        true => say!("yume.words.keys-search"),
        false => say!("yume.words.keys"),
    };
    let keys_rows = crate::settings_page::fold_pairs(&keys, right.saturating_sub(area.x + 2) as usize);
    let room = bottom.saturating_sub(area.y + 3);
    let keys_high = (keys_rows.len() as u16).min(room / 2);
    let keys_at = bottom.saturating_sub(keys_high);
    for (i, row) in keys_rows.iter().take(keys_high as usize).enumerate() {
        put_text(buf, area.x + 2, keys_at + i as u16, right, row, text);
    }
    let foot = keys_at.saturating_sub(1).max(area.y + 2);

    // ---- The columns' names, a rule, the lists' column ------------------------
    let heads = list.map(headings).unwrap_or_default();
    let shown_cells: Vec<Vec<String>> = list.map(|l| rows.iter().map(|r| cells(l, r)).collect()).unwrap_or_default();
    let col_widths = widths(&heads, &shown_cells, right.saturating_sub(from));
    let mut xs = Vec::new();
    let mut x = from;
    for w in &col_widths {
        xs.push(x);
        x += w;
    }
    for (i, name) in heads.iter().enumerate() {
        if i > 0 {
            put_text(buf, xs[i].saturating_sub(1), area.y, right, "┆", rule_ink);
        }
        put_text(buf, xs[i], area.y, right, name, head);
    }
    let rule = |buf: &mut ratatui::buffer::Buffer, y: u16, from: u16| {
        for x in from..right {
            if let Some(cell) = buf.cell_mut((x, y)) {
                cell.set_symbol("─").set_style(quiet);
            }
        }
    };
    if area.y + 1 < foot {
        rule(buf, area.y + 1, area.x);
        if side > 0 {
            put_text(buf, split, area.y + 1, right, "┬", quiet);
        }
    }
    if foot > area.y + 1 && foot < bottom {
        rule(buf, foot, area.x);
        if side > 0 {
            put_text(buf, split, foot, right, "┴", quiet);
        }
    }
    for y in (area.y + 2..foot).filter(|_| side > 0) {
        put_text(buf, split, y, right, "│", quiet);
    }
    for (i, l) in lists.iter().enumerate().filter(|_| side > 0) {
        let y = area.y + 2 + i as u16;
        if y >= foot {
            break;
        }
        let name = list_name(*l);
        let count = page.rows(ime, *l, false).len().to_string();
        let style = match Some(*l) == list {
            true => head,
            false => text,
        };
        put_text(buf, area.x + 2, y, split, &name, style);
        let cw = yumete_cjk::str_width(&count) as u16;
        put_text(buf, split.saturating_sub(cw + 2), y, split, &count, style);
    }

    // ---- What the row is, under the list --------------------------------------
    let block_high: u16 = 4;
    let has_block = foot >= area.y + 2 + block_high + 3 && !rows.is_empty();
    let list_ends = match has_block {
        true => foot - block_high,
        false => foot,
    };
    let top = area.y + 2;
    let Some(list) = list else {
        put_fit(buf, from, top, right, &say!("yume.words.unread"), quiet);
        return None;
    };
    put_fit(buf, from.saturating_sub(1), top, right, &list_name(list), ground.fg(ink.text()).add_modifier(Modifier::BOLD));
    let first_row = top + 1;
    if rows.is_empty() {
        let empty = match (page.search.is_empty(), page.all) {
            (false, _) => say!("yume.words.no-match", &page.search),
            (true, false) => say!("yume.words.empty"),
            (true, true) => say!("yume.words.empty-all"),
        };
        put_fit(buf, from, first_row, right, &empty, quiet);
        return None;
    }
    let room = list_ends.saturating_sub(first_row) as usize;
    let was = page.first.get();
    let first = match room {
        0 => at,
        _ if at < was => at,
        _ if at >= was + room => at + 1 - room,
        _ => was.min(rows.len().saturating_sub(room.min(rows.len()))),
    };
    page.first.set(first);
    for (i, row) in rows.iter().enumerate().skip(first).take(room) {
        let y = first_row + (i - first) as u16;
        let here = i == at;
        let band = match here {
            true => ink.ground(yumete_config::rung::HEAD),
            false => ground,
        };
        if here {
            fill(buf, from, right, y, band);
        }
        let ink_of = match row.held {
            Held::App => band.fg(ink.quiet()),
            _ => band.fg(ink.text()),
        };
        let row_cells = &shown_cells[i];
        let last = row_cells.len() - 1;
        for (c, cell) in row_cells.iter().enumerate() {
            let ends = xs.get(c + 1).map_or(right, |next| next.saturating_sub(1));
            if c > 0 {
                put_text(buf, xs[c].saturating_sub(1), y, right, "┆", band.fg(ink.rule()));
            }
            let style = match (c == last, row.held) {
                (true, Held::App) => band.fg(ink.quiet()),
                (true, _) => band.fg(ink.gold()),
                _ => ink_of,
            };
            let room = ends.saturating_sub(xs[c]) as usize;
            put_text(buf, xs[c], y, ends, &crate::elide(cell, room), style);
        }
    }
    let below = rows.len().saturating_sub(first + room);
    if has_block {
        let rule_at = list_ends;
        rule(buf, rule_at, split + (side > 0) as u16);
        if side > 0 {
            put_text(buf, split, rule_at, right, "├", quiet);
        }
        if below > 0 {
            put_text(buf, from, rule_at, right, &format!(" {} ", say!("yume.words.more", below)), quiet);
        }
        let lines = detail(list, &rows[at]);
        let labels = ["", &say!("yume.words.label.after"), &say!("yume.words.label.in")].map(str::to_string);
        let label_wide = labels.iter().map(|l| yumete_cjk::str_width(l)).max().unwrap_or(0) as u16 + 2;
        for (i, (label, line)) in labels.iter().zip(lines.iter()).enumerate() {
            let y = rule_at + 1 + i as u16;
            if y >= foot {
                break;
            }
            let at = match label.is_empty() {
                true => from,
                false => {
                    put_text(buf, from, y, right, label, quiet);
                    from + label_wide
                }
            };
            let room = right.saturating_sub(at + 1) as usize;
            // A path gives up its head, so the file's name stays in sight.
            let line = match i {
                2 => crate::elide_head(line, room),
                _ => crate::elide(line, room),
            };
            put_text(buf, at, y, right, &line, text);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use yumete_ime::{Scheme, UserWordRoots};

    /// `jk` is 你 and 你好 in the 碼表; `df` is 他 and 她.
    const TABLE: &str = "jk 你 你好\ndf 他 她\n";

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yumete-words-page-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(root: &std::path::Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// A session over made-up folders: the app has 你们 and a 调频 row; yumete
    /// has 他们, 你们 too, a 调频 row on the app's key and a pin.
    fn session(name: &str) -> (ImeSession, std::path::PathBuf, std::path::PathBuf) {
        let app = scratch(&format!("{name}-app"));
        let own = scratch(&format!("{name}-own"));
        put(&app, "data/lingming_user_dict.txt", "你们\tjk\n他俩\tdf\n");
        put(&app, "schemes/lingming/freq.txt", "#tick\t10\ndf\t她\t5\t2.0e0\t10\n");
        put(&app, "schemes/lingming/pins.txt", "jk\t你\t2\n");
        put(&own, "lingming/user_dict.txt", "他们\tdf\n你们\tjk\n");
        put(&own, "lingming/freq.txt", "#tick\t4\ndf\t她\t2\t1.0e0\t4\n");
        put(&own, "lingming/pins.txt", "df\t她\t1\n");
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, TABLE);
        ime.set_user_word_roots(UserWordRoots { app: Some(app.clone()), own: Some(own.clone()) });
        (ime, app, own)
    }

    fn opened(editor: &mut Editor, ime: &mut ImeSession) -> Seat {
        let mut seat = Seat::default();
        assert!(editor.execute(":yume-words").is_ok());
        seat.settle(editor, ime);
        assert!(seat.open());
        seat
    }

    fn press(seat: &mut Seat, editor: &mut Editor, ime: &mut ImeSession, keys: &str) {
        for c in keys.chars() {
            let key = match c {
                '\t' => Key::Tab,
                '\n' => Key::Enter,
                c => Key::Char(c),
            };
            assert!(seat.took(editor, ime, Some(key)), "{c:?} is the page's");
        }
    }

    fn shot(seat: &Seat, ime: &ImeSession) -> String {
        crate::theme::settle(&Config::default(), None);
        let backend = ratatui::backend::TestBackend::new(100, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| { draw(f, seat, ime, &Config::default(), f.area()); }).unwrap();
        crate::buffer_to_text(terminal.backend().buffer())
    }

    #[test]
    fn the_page_shows_each_list_in_its_own_columns() {
        let (mut ime, ..) = session("columns");
        let mut editor = Editor::new();
        let mut seat = opened(&mut editor, &mut ime);
        let frame = shot(&seat, &ime);
        for want in [
            say!("yume.words.kind.words"),
            say!("yume.words.kind.freq"),
            say!("yume.words.kind.pins"),
            say!("yume.words.column.code"),
            say!("yume.words.column.source"),
            "他们".to_string(),
            say!("yume.words.source.both"),
            say!("yume.words.label.after"),
            say!("yume.words.show-own"),
        ] {
            assert!(frame.contains(&want), "{want}:\n{frame}");
        }
        assert!(!frame.contains("他俩"), "the app's alone wait for Tab:\n{frame}");
        press(&mut seat, &mut editor, &mut ime, "l");
        let frame = shot(&seat, &ime);
        assert!(frame.contains(&say!("yume.words.column.count")), "{frame}");
        assert!(frame.contains(&say!("yume.words.detail.freq-both", "df", "她", 2, 5, 7)), "summed:\n{frame}");
        press(&mut seat, &mut editor, &mut ime, "l");
        assert!(shot(&seat, &ime).contains(&say!("yume.words.column.place")));
        press(&mut seat, &mut editor, &mut ime, "l");
        assert!(shot(&seat, &ime).contains("他们"), "round to the first");
    }

    #[test]
    fn tab_shows_the_apps_and_d_refuses_them() {
        let (mut ime, app, _) = session("tab");
        let before = std::fs::read_to_string(app.join("data/lingming_user_dict.txt")).unwrap();
        let mut editor = Editor::new();
        let mut seat = opened(&mut editor, &mut ime);
        press(&mut seat, &mut editor, &mut ime, "\t");
        let frame = shot(&seat, &ime);
        assert!(frame.contains("他俩") && frame.contains(&say!("yume.words.show-all")), "{frame}");
        press(&mut seat, &mut editor, &mut ime, "Gd");
        assert_eq!(editor.status(), say!("yume.words.refused-app"));
        assert_eq!(std::fs::read_to_string(app.join("data/lingming_user_dict.txt")).unwrap(), before);
    }

    #[test]
    fn d_and_u_in_each_list() {
        let (mut ime, _, own) = session("delete");
        let mut editor = Editor::new();
        let mut seat = opened(&mut editor, &mut ime);
        // 词库: 他们 is yumete's alone, 你们 both layers'.
        press(&mut seat, &mut editor, &mut ime, "d");
        assert_eq!(std::fs::read_to_string(own.join("lingming/user_dict.txt")).unwrap(), "你们\tjk\n");
        press(&mut seat, &mut editor, &mut ime, "d");
        assert_eq!(std::fs::read_to_string(own.join("lingming/user_dict.txt")).unwrap(), "");
        let frame = shot(&seat, &ime);
        assert!(frame.contains("你们") && frame.contains(&say!("yume.words.source.app")), "the row stays, as the app's:\n{frame}");
        press(&mut seat, &mut editor, &mut ime, "uu");
        assert_eq!(std::fs::read_to_string(own.join("lingming/user_dict.txt")).unwrap(), "他们\tdf\n你们\tjk\n");
        // 调频.
        press(&mut seat, &mut editor, &mut ime, "ld");
        assert!(!std::fs::read_to_string(own.join("lingming/freq.txt")).unwrap().contains("她"));
        press(&mut seat, &mut editor, &mut ime, "u");
        assert!(std::fs::read_to_string(own.join("lingming/freq.txt")).unwrap().contains("df\t她\t2\t"));
        // 固顶.
        press(&mut seat, &mut editor, &mut ime, "ld");
        assert_eq!(std::fs::read_to_string(own.join("lingming/pins.txt")).unwrap(), "");
        press(&mut seat, &mut editor, &mut ime, "u");
        assert_eq!(std::fs::read_to_string(own.join("lingming/pins.txt")).unwrap(), "df\t她\t1\n");
        press(&mut seat, &mut editor, &mut ime, "u");
        assert_eq!(editor.status(), say!("yume.words.nothing-to-restore"));
    }

    #[test]
    fn y_copies_the_row_and_capital_y_the_list() {
        let (mut ime, ..) = session("copy");
        let mut editor = Editor::new();
        let mut seat = opened(&mut editor, &mut ime);
        press(&mut seat, &mut editor, &mut ime, "y");
        assert_eq!(editor.status(), say!("yume.words.copied"));
        assert_eq!(tsv(WordList::Words, &[]).lines().count(), 1, "the names alone");
        press(&mut seat, &mut editor, &mut ime, "Y");
        assert_eq!(editor.status(), say!("yume.words.copied-page", 2));
        let row = &ime.word_rows(WordList::Words)[0];
        assert_eq!(cells(WordList::Words, row).join("\t"), "他们\tdf\tyumete");
    }

    /// Keys as the loop gives them: the page first, the editor's line when
    /// the page gives them back, and the page settled after each.
    fn typed(seat: &mut Seat, editor: &mut Editor, ime: &mut ImeSession, keys: &[Key]) {
        for &key in keys {
            if !seat.took(editor, ime, Some(key)) {
                editor.on_key(key);
            }
            seat.settle(editor, ime);
        }
    }

    /// **`/` is typed on the editor's value line**, which the input method
    /// composes on: the list narrows as it is typed, Enter keeps the search
    /// and Esc on the line clears it.
    #[test]
    fn search_narrows_and_q_and_colon_q_close() {
        let (mut ime, ..) = session("search");
        let mut editor = Editor::new();
        let mut seat = opened(&mut editor, &mut ime);
        typed(&mut seat, &mut editor, &mut ime, &[Key::Char('/'), Key::Char('j')]);
        assert_eq!(editor.mode(), Mode::Value);
        assert!(crate::composes_here(&editor), "the input method composes there");
        assert_eq!(seat.page.as_ref().map(|p| p.search.as_str()), Some("j"), "narrowed as it is typed");
        typed(&mut seat, &mut editor, &mut ime, &[Key::Char('k'), Key::Enter]);
        assert_eq!(editor.mode(), Mode::Normal);
        let frame = shot(&seat, &ime);
        assert!(frame.contains("你们") && !frame.contains("他们"), "{frame}");
        typed(&mut seat, &mut editor, &mut ime, &[Key::Char('/'), Key::Char('d'), Key::Esc]);
        assert_eq!(seat.page.as_ref().map(|p| p.search.as_str()), Some(""), "Esc on the line clears it");
        assert!(shot(&seat, &ime).contains("他们"));
        press(&mut seat, &mut editor, &mut ime, "q");
        assert!(!seat.open());
        let mut seat = opened(&mut editor, &mut ime);
        assert!(!seat.took(&mut editor, &mut ime, Some(Key::Char(':'))), "`:` opens the line");
        assert!(editor.execute(":q").is_ok());
        seat.settle(&mut editor, &mut ime);
        assert!(!seat.open(), ":q closes the page, not the editor");
    }

    /// **A lone Shift switches 中/英 on the `/` line**, as on every prompt.
    #[test]
    fn a_lone_shift_switches_the_language_on_the_search_line() {
        let (mut ime, ..) = session("shift");
        let mut editor = Editor::new();
        let mut seat = opened(&mut editor, &mut ime);
        typed(&mut seat, &mut editor, &mut ime, &[Key::Char('/')]);
        let chinese = ime.is_chinese();
        let mut settings = crate::settings_page::Seat::default();
        let mut genda = crate::genda_page::Seat::default();
        let mut borrowed = crate::Borrow::default();
        assert!(crate::route_a_shift_tap(&mut ime, &mut editor, &mut settings, &mut genda, &mut borrowed, yumete_ime::FuncKey::ShiftL));
        assert_ne!(ime.is_chinese(), chinese);
        assert_eq!(editor.mode(), Mode::Value, "the line stays open");
    }

    #[test]
    fn space_z_u_asks_for_the_page() {
        let (mut ime, ..) = session("space");
        let mut editor = Editor::new();
        for key in [Key::Char(' '), Key::Char('z'), Key::Char('u')] {
            editor.on_key(key);
        }
        let mut seat = Seat::default();
        seat.settle(&mut editor, &mut ime);
        assert!(seat.open());
    }

    #[test]
    fn with_nothing_read_the_page_says_so() {
        let mut ime = ImeSession::from_table_text(Scheme::LINGMING, TABLE);
        let mut editor = Editor::new();
        let seat = opened(&mut editor, &mut ime);
        assert!(shot(&seat, &ime).contains(&say!("yume.words.unread")));
    }

    /// **Before the input method was ever turned on**, the page names the
    /// scheme the session was made for, not the placeholder its empty
    /// engine holds.
    #[test]
    fn a_session_with_no_table_names_its_own_scheme() {
        let mut ime = ImeSession::empty(Scheme::PINYIN);
        let mut editor = Editor::new();
        let seat = opened(&mut editor, &mut ime);
        let frame = shot(&seat, &ime);
        assert!(frame.contains("宇浩拼音") && !frame.contains("靈明"), "{frame}");
    }

    /// **The `/` line keeps the language the writer was in**, as the
    /// editor's own `/` does; the settings panel's value line still opens
    /// in 英.
    #[test]
    fn the_search_line_keeps_the_language() {
        let (mut ime, ..) = session("keeps");
        let mut editor = Editor::new();
        let mut seat = opened(&mut editor, &mut ime);
        typed(&mut seat, &mut editor, &mut ime, &[Key::Char('/')]);
        assert_eq!(editor.mode(), Mode::Value);
        assert!(!editor.prompt_opens_in_english());
        let mut borrow = crate::Borrow::default();
        let door = borrow.crossing_into(Some(Mode::Normal), Mode::Value, editor.prompt_opens_in_english(), true);
        assert!(!door.toggle, "中 stays 中");
        typed(&mut seat, &mut editor, &mut ime, &[Key::Esc]);
        editor.open_value_line("value: ", "");
        assert!(editor.prompt_opens_in_english());
    }

    /// **`s` turns the page, not the input method**: the scheme that types
    /// and its 中/英 are as they were, while the page reads another's words.
    #[test]
    fn s_shows_another_scheme_and_leaves_the_one_that_types() {
        let (mut ime, ..) = session("turn");
        let chinese = ime.is_chinese();
        let mut editor = Editor::new();
        let mut seat = opened(&mut editor, &mut ime);
        press(&mut seat, &mut editor, &mut ime, "s");
        assert_eq!(ime.scheme(), Scheme::LINGMING, "the one that types stays");
        assert_eq!(ime.is_chinese(), chinese);
        let page = seat.page.as_ref().unwrap();
        if let Some(other) = &page.other {
            assert_ne!(other.scheme(), Scheme::LINGMING);
            assert!(shot(&seat, &ime).contains(other.scheme_name()));
        }
        press(&mut seat, &mut editor, &mut ime, "q");
        assert_eq!(ime.scheme(), Scheme::LINGMING);
    }
}
