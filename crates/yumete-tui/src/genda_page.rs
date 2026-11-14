//! **跟打** (`:genda`): the typing trainer's page.
//!
//! ```text
//! ╭ 本段 ────────────╮╭ 常用单字 前500 ─────────────────────────╮
//! │ 速度   112.40    ││ 的一是了不在有个人这上中大为来我到出要 │
//! │ 击键     5.82    ││ 以时和地们得可下对生也子就过能他会多发 │
//! ╰──────────────────╯╰────────────────────────────────────────╯
//! ╭ 文章 ────────────╮╭ 跟打 · 靈明 ───────────────────────────╮
//! │ 第 3/9 段        ││ 的一是了不在有个人这上中大为来我到出要 │
//! ╰──────────────────╯╰────────────────────────────────────────╯
//!  F3 重打  C-p 下一段  C-u 上一段  C-l 乱序  Tab 历史  Esc 暂停
//! ```
//!
//! The model is [`yumete_core::genda`]; this file holds the page, lends the
//! loop the pad (an [`Editor`] held in Insert mode, so the IME composes,
//! picks and punctuates there as it does in the text) and draws it all.
//! Every key reaches the page before the IME, which is what lets the key
//! fields be counted while the built-in IME holds the keys.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::Frame;

use yumete_config::Config;
use yumete_core::genda::{self, Ask, Choice, Mark, Record, Run, Stroke, Totals};
use yumete_core::input::{Key, Mode};
use yumete_core::say;
use yumete_core::Editor;
use yumete_ime::ImeSession;

use crate::put_text;
use crate::theme::{Accent, Palette};

/// Where the records are kept, in the editor's data directory.
const HISTORY_FILE: &str = "genda.jsonl";

/// Below this many columns the left column folds into one row of figures.
const WIDE: u16 = 80;

/// How often the open page redraws while its clock runs.
const CLOCK_TICK: Duration = Duration::from_millis(250);

/// **The page in the front end**: open, keys, pad, close, one round.
/// The loop and `--keys` both go through it.
#[derive(Default)]
pub struct Seat {
    page: Option<Page>,
    /// The pad, while it is not lent to the loop for a key.
    pad: Option<Box<Editor>>,
    /// **A clock that moves by this many milliseconds a key**, for pictures
    /// that come out the same every time; `None` is the wall clock.
    pub tick: Option<u64>,
    ticks: u64,
    epoch: Option<Instant>,
    /// The key the pad is about to be lent for, as read before it.
    pending: Option<Pending>,
    /// Yume's most frequent characters, worked out the first time asked.
    common: Option<Vec<char>>,
}

struct Pending {
    stroke: Stroke,
    /// The first candidate of the first page, while a code was composed.
    first: String,
    at: Duration,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Typing,
    History,
}

struct Page {
    name: String,
    chars: Vec<char>,
    /// The order in force: `chars`, or a shuffle of them.
    order: Vec<char>,
    shuffled: bool,
    length: Option<usize>,
    index: usize,
    /// Attempts begun at each segment, for 重打.
    tries: Vec<usize>,
    /// The run in hand has been counted among them.
    counted: bool,
    run: Run,
    view: View,
    history: Vec<Record>,
    /// The history row picked, counted from the newest.
    picked: usize,
    totals: Totals,
    latest: Option<Record>,
    all_done: bool,
    /// Esc stopped the page: keys wait for Enter, and a second Esc closes.
    held: bool,
    /// The scheme's name while the built-in IME holds the keys; empty under
    /// the system's input method.
    scheme: String,
    file: Option<PathBuf>,
}

impl Page {
    fn count(&self) -> usize {
        genda::segment_count(self.order.len(), self.length)
    }

    fn target(&self) -> Vec<char> {
        self.order[genda::segment(self.order.len(), self.length, self.index)].to_vec()
    }

    fn ime_label(&self) -> String {
        match self.scheme.is_empty() {
            true => say!("genda.system-ime"),
            false => self.scheme.clone(),
        }
    }
}

impl Seat {
    pub fn open(&self) -> bool {
        self.page.is_some()
    }

    /// **The pad is taking keys**: the caret is a bar in it and the IME
    /// composes there.
    pub fn typing(&self) -> bool {
        self.page.as_ref().is_some_and(|p| p.view == View::Typing && !p.run.finished() && !p.all_done && !self.paused())
    }

    fn paused(&self) -> bool {
        self.page.as_ref().is_some_and(|p| p.held)
    }

    /// The loop's clock: a redraw while the segment's clock runs.
    pub fn due_in(&self) -> Option<Duration> {
        self.page.as_ref().filter(|p| p.run.started() && !p.run.paused()).map(|_| CLOCK_TICK)
    }

    fn clock(&self) -> Duration {
        match self.tick {
            Some(_) => Duration::from_millis(self.ticks),
            None => self.epoch.map_or(Duration::ZERO, |e| e.elapsed()),
        }
    }

    /// **Does this key belong to the page?** Taken, it is done here; not
    /// taken, it goes to the pad ([`Seat::lend_pad`]) while typing, and to
    /// the editor behind otherwise, which only ever gets a `:`.
    pub fn took(&mut self, editor: &mut Editor, ime: &mut ImeSession, key: Option<Key>, f3: bool) -> bool {
        self.pending = None;
        let Some(page) = self.page.as_mut() else { return false };
        if editor.mode().is_prompt() {
            return false;
        }
        page.scheme = match ime.holding_the_keys() {
            true => ime.scheme_name().to_string(),
            false => String::new(),
        };
        if let Some(step) = self.tick {
            self.ticks += step;
        }
        let composing = ime.holding_the_keys() && ime.is_composing();
        if f3 {
            if composing {
                ime.escape();
            }
            self.retry(editor);
            return true;
        }
        let Some(key) = key else { return true };
        match key {
            Key::Ctrl('p' | 'u' | 'l') => {
                if composing {
                    ime.escape();
                }
                match key {
                    Key::Ctrl('p') => self.step(editor, true),
                    Key::Ctrl('u') => self.step(editor, false),
                    _ => self.shuffle(editor),
                }
                return true;
            }
            Key::Char(':') if !self.typing() => return false,
            _ => {}
        }
        let page = self.page.as_mut().expect("open");
        if page.view == View::History {
            let rows = page.history.len();
            match key {
                Key::Char('j') | Key::Down => page.picked = (page.picked + 1).min(rows.saturating_sub(1)),
                Key::Char('k') | Key::Up => page.picked = page.picked.saturating_sub(1),
                Key::Char('y') => self.copy(editor),
                Key::Char('Y') => self.copy_history(editor),
                Key::Tab | Key::Esc | Key::Char('q') => self.history(editor),
                _ => {}
            }
            return true;
        }
        if !self.typing() {
            let now = self.clock();
            let page = self.page.as_mut().expect("open");
            match key {
                Key::Enter if page.held => {
                    page.held = false;
                    page.run.resume(now);
                    editor.set_status(String::new());
                }
                Key::Esc => self.close(editor),
                Key::Char('y') => self.copy(editor),
                Key::Tab => self.history(editor),
                _ => {}
            }
            return true;
        }
        match key {
            Key::Esc if !composing => {
                let now = self.clock();
                let page = self.page.as_mut().expect("open");
                page.run.pause(now);
                page.held = true;
                editor.set_status(say!("genda.paused"));
                return true;
            }
            Key::Enter if !composing => return true,
            Key::Tab if !composing => {
                self.history(editor);
                return true;
            }
            _ => {}
        }
        let stroke = Stroke { yume: ime.holding_the_keys(), composing, backspace: key == Key::Backspace };
        let first = match composing && ime.page() == 0 {
            true => ime.page_candidates().first().map(|c| c.text.clone()).unwrap_or_default(),
            false => String::new(),
        };
        let at = self.clock();
        let page = self.page.as_mut().expect("open");
        page.run.key(stroke, at);
        self.pending = Some(Pending { stroke, first, at });
        false
    }

    /// **The pad, lent to the loop for the key [`Seat::took`] gave back.**
    pub fn lend_pad(&mut self) -> Option<Box<Editor>> {
        self.pending.as_ref()?;
        self.pad.take()
    }

    /// **The pad, for a lone Shift tap mid-code**: the tap is a key, and the
    /// code it commits lands in the pad. `None` when no code is composed
    /// there, and the tap is only the language switch.
    pub fn lend_pad_for_tap(&mut self, ime: &ImeSession) -> Option<Box<Editor>> {
        if !self.typing() || !(ime.holding_the_keys() && ime.is_composing()) {
            return None;
        }
        let stroke = Stroke { yume: true, composing: true, backspace: false };
        let at = self.clock();
        self.page.as_mut()?.run.key(stroke, at);
        self.pending = Some(Pending { stroke, first: String::new(), at });
        self.pad.take()
    }

    /// **The pad back after its key**: what it holds now is read into the run.
    pub fn take_back_pad(&mut self, editor: &mut Editor, mut pad: Box<Editor>) {
        if pad.mode().is_prompt() {
            pad.on_key(Key::Esc);
        }
        if pad.mode() != Mode::Insert {
            pad.on_key(Key::Char('A'));
        }
        let text = pad.field_text();
        self.pad = Some(pad);
        let (Some(pending), Some(page)) = (self.pending.take(), self.page.as_mut()) else { return };
        if !pending.first.is_empty() {
            let typed = page.run.typed();
            let now: Vec<char> = genda::strip(&text);
            let same = typed.iter().zip(&now).take_while(|(a, b)| a == b).count();
            let added: String = now[same..].iter().collect();
            if !added.is_empty() && added != pending.first {
                page.run.picked();
            }
        }
        if page.run.text_now(&text, pending.stroke, pending.at) {
            self.finish(editor);
        }
    }

    /// Text pasted while the page is open goes nowhere: a paste is not typing.
    pub fn pasted(&self) -> bool {
        self.open()
    }

    /// **What the core was asked**: `:genda` and its family, `:q`.
    pub fn settle(&mut self, editor: &mut Editor, ime: &ImeSession) {
        let article = editor.take_genda_article();
        for ask in editor.take_genda_asks() {
            match ask {
                Ask::Open if self.page.is_none() => {
                    if let Some((name, text)) = article.clone() {
                        self.start(editor, ime, name, genda::strip(&text));
                    }
                }
                Ask::Open => {}
                // Counted by the core, which hands it over as an article.
                Ask::Top(..) => {}
                Ask::Close => self.close(editor),
                Ask::Next => self.step(editor, true),
                Ask::Previous => self.step(editor, false),
                Ask::Retry => self.retry(editor),
                Ask::Shuffle => self.shuffle(editor),
                Ask::Copy => self.copy(editor),
                Ask::CopyHistory => self.copy_history(editor),
                Ask::History => self.history(editor),
                Ask::Length(length) => {
                    if let Some(page) = self.page.as_mut() {
                        page.length = length;
                        page.index = 0;
                        self.restart();
                        editor.set_status(match length {
                            Some(n) => say!("genda.length-set", n),
                            None => say!("genda.length-all"),
                        });
                    }
                }
                Ask::Article(Choice::Buffer) => {
                    if let Some((name, text)) = article.clone() {
                        self.replace_article(editor, name, genda::strip(&text));
                    }
                }
                Ask::Article(Choice::Common(band)) => {
                    let common = self.common.get_or_insert_with(|| ime.frequent_chars(
                        1500,
                        yumete_core::messages::language() != yumete_core::messages::Language::Traditional,
                    ));
                    let chars: Vec<char> = common.iter().copied().skip(band * 500).take(500).collect();
                    match chars.is_empty() {
                        true => editor.set_status(say!("genda.no-frequency")),
                        false => self.replace_article(editor, Choice::band_name(band), chars),
                    }
                }
            }
        }
    }

    fn start(&mut self, editor: &mut Editor, ime: &ImeSession, name: String, chars: Vec<char>) {
        if chars.is_empty() {
            editor.set_status(say!("genda.empty-article"));
            return;
        }
        let file = editor.kept_file(HISTORY_FILE);
        let history = file
            .as_ref()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .map(|text| genda::read_records(&text))
            .unwrap_or_default();
        self.epoch.get_or_insert_with(Instant::now);
        self.page = Some(Page {
            name,
            order: chars.clone(),
            chars,
            shuffled: false,
            length: Some(genda::DEFAULT_LENGTH),
            index: 0,
            tries: Vec::new(),
            counted: false,
            run: Run::default(),
            view: View::Typing,
            history,
            picked: 0,
            totals: Totals::default(),
            latest: None,
            all_done: false,
            held: false,
            scheme: match ime.holding_the_keys() {
                true => ime.scheme_name().to_string(),
                false => String::new(),
            },
            file,
        });
        editor.set_genda_open(true);
        editor.set_status(String::new());
        self.restart();
    }

    fn replace_article(&mut self, editor: &mut Editor, name: String, chars: Vec<char>) {
        let Some(page) = self.page.as_mut() else { return };
        if chars.is_empty() {
            editor.set_status(say!("genda.empty-article"));
            return;
        }
        page.name = name;
        page.order = chars.clone();
        page.chars = chars;
        page.shuffled = false;
        page.index = 0;
        self.restart();
    }

    /// The segments changed: the attempts are counted afresh.
    fn restart(&mut self) {
        if let Some(page) = self.page.as_mut() {
            page.tries = vec![0; page.count()];
            page.index = page.index.min(page.count().saturating_sub(1));
        }
        self.load();
    }

    /// **A fresh run at the segment in hand**, with an empty pad.
    fn load(&mut self) {
        let Some(page) = self.page.as_mut() else { return };
        if page.run.started() && !page.counted {
            if let Some(tries) = page.tries.get_mut(page.index) {
                *tries += 1;
            }
        }
        page.run = Run::new(page.target());
        page.counted = false;
        page.all_done = false;
        page.held = false;
        page.view = View::Typing;
        let mut pad = Box::new(Editor::new());
        pad.open_field("", true);
        self.pad = Some(pad);
        self.pending = None;
    }

    fn step(&mut self, editor: &mut Editor, on: bool) {
        let Some(page) = self.page.as_mut() else { return };
        let count = page.count();
        match on {
            true if page.index + 1 >= count => {
                editor.set_status(say!("genda.last-segment"));
                return;
            }
            false if page.index == 0 => {
                editor.set_status(say!("genda.first-segment"));
                return;
            }
            true => page.index += 1,
            false => page.index -= 1,
        }
        editor.set_status(String::new());
        self.load();
    }

    /// **重打**: the segment in hand, or, before its first key, the one just
    /// finished, which is what a hand reaching for F3 straight after a
    /// segment wants again.
    fn retry(&mut self, editor: &mut Editor) {
        let Some(page) = self.page.as_mut() else { return };
        let just_done = page.latest.as_ref().is_some_and(|r| r.segment == page.index);
        if !page.run.started() && just_done && !page.all_done {
            page.index -= 1;
        }
        editor.set_status(String::new());
        self.load();
    }

    fn shuffle(&mut self, editor: &mut Editor) {
        let Some(page) = self.page.as_mut() else { return };
        page.shuffled = !page.shuffled;
        page.order = match page.shuffled {
            true => {
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(1, |d| d.as_nanos() as u64);
                genda::shuffled(&page.chars, self.tick.map_or(seed, |_| 7))
            }
            false => page.chars.clone(),
        };
        page.index = 0;
        editor.set_status(match page.shuffled {
            true => say!("genda.shuffle-on"),
            false => say!("genda.shuffle-off"),
        });
        self.restart();
    }

    fn history(&mut self, editor: &mut Editor) {
        let now = self.clock();
        let Some(page) = self.page.as_mut() else { return };
        page.view = match page.view {
            View::Typing => {
                page.run.pause(now);
                page.picked = 0;
                View::History
            }
            View::History => View::Typing,
        };
        editor.set_status(String::new());
    }

    /// **The segment is done**: kept, added up, said, and the next one up.
    fn finish(&mut self, editor: &mut Editor) {
        let now = self.clock();
        let Some(page) = self.page.as_mut() else { return };
        let retry = page.tries.get(page.index).copied().unwrap_or(0);
        if let Some(tries) = page.tries.get_mut(page.index) {
            *tries += 1;
        }
        page.counted = true;
        let record = Record {
            at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            segment: page.index + 1,
            retry,
            scheme: page.scheme.clone(),
            article: page.name.clone(),
            version: version(),
            ..page.run.record(now)
        };
        if let Some(file) = &page.file {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            use std::io::Write as _;
            if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
                let _ = writeln!(out, "{}", record.to_line());
            }
        }
        editor.set_status(record.score_line());
        page.totals.add(&record);
        page.history.push(record.clone());
        page.latest = Some(record);
        match page.index + 1 < page.count() {
            true => {
                page.index += 1;
                self.load();
            }
            false => page.all_done = true,
        }
    }

    /// The record `y` copies: the one picked in the history, or the latest.
    fn copy(&mut self, editor: &mut Editor) {
        let Some(page) = self.page.as_ref() else { return };
        let record = match page.view {
            View::History => page.history.iter().rev().nth(page.picked),
            View::Typing => page.latest.as_ref().or(page.history.last()),
        };
        match record {
            Some(record) => {
                editor.send_to_clipboard(record.score_line());
                editor.set_status(say!("genda.copied"));
            }
            None => editor.set_status(say!("genda.no-record")),
        }
    }

    fn copy_history(&mut self, editor: &mut Editor) {
        let Some(page) = self.page.as_ref() else { return };
        match page.history.is_empty() {
            true => editor.set_status(say!("genda.no-record")),
            false => {
                editor.send_to_clipboard(genda::tsv(&page.history));
                editor.set_status(say!("genda.copied-history", page.history.len()));
            }
        }
    }

    fn close(&mut self, editor: &mut Editor) {
        self.page = None;
        self.pad = None;
        self.pending = None;
        editor.set_genda_open(false);
        editor.set_status(String::new());
    }
}

/// `X.Y` of this build, for the score line.
fn version() -> String {
    env!("CARGO_PKG_VERSION").split('.').take(2).collect::<Vec<_>>().join(".")
}

/// The figures of the segment in hand, or of the one just done while the
/// next has not begun.
fn shown_record(seat: &Seat, page: &Page) -> (Record, bool) {
    match (&page.latest, page.run.started()) {
        (Some(latest), false) => (latest.clone(), true),
        _ => (page.run.record(seat.clock()), false),
    }
}

fn dash_or(value: Option<f64>, f: impl Fn(f64) -> String) -> String {
    value.map_or_else(|| "—".to_string(), f)
}

/// **Draw the page over `area`**; gives the pad's caret while typing, for
/// the candidate panel and the system's input method to stand by.
pub fn draw(frame: &mut Frame, seat: &Seat, config: &Config, area: Rect) -> Option<Position> {
    let page = seat.page.as_ref()?;
    let ink = Palette::of(config);
    let ground = crate::chrome::panel_ground(ink);
    let base = Style::default().bg(ground);
    for y in area.y..area.y + area.height {
        put_text(frame.buffer_mut(), area.x, y, area.x + area.width, &" ".repeat(area.width as usize), base);
    }
    if area.height < 4 || area.width < 20 {
        return None;
    }
    let hints = match (page.view, seat.paused() || page.all_done) {
        (View::History, _) => say!("genda.hints-history"),
        (_, true) => say!("genda.hints-paused"),
        _ => say!("genda.hints"),
    };
    let foot_y = area.y + area.height - 1;
    put_text(frame.buffer_mut(), area.x + 1, foot_y, area.x + area.width, &hints, base.fg(ink.quiet()));
    let body = Rect { height: area.height - 1, ..area };
    let ring = |frame: &mut Frame, rect: Rect, title: String| -> Rect {
        crate::chrome::draw(frame, rect, &crate::chrome::Ring {
            rounded: config.panel.rounded,
            open_sides: false,
            border: base.fg(ink.rule()),
            ground: base,
            title: Some((title, base.fg(ink.gold()))),
            foot: None,
        })
    };
    if page.view == View::History {
        let inner = ring(frame, body, say!("genda.box.history", page.history.len()));
        draw_history(frame, page, ink, base, inner);
        return None;
    }
    let (record, _) = shown_record(seat, page);
    let (left, right) = match body.width >= WIDE {
        true => {
            let w = 26.min(body.width / 3);
            (Some(Rect { width: w, ..body }), Rect { x: body.x + w, width: body.width - w, ..body })
        }
        false => (None, body),
    };
    let right = match left {
        Some(left) => {
            draw_left(frame, page, &record, (ink, base), left, &ring);
            right
        }
        None => {
            let line = [
                (say!("genda.label.speed"), record.speed_text()),
                (say!("genda.label.kps"), dash_or(record.keys_per_second(), |v| format!("{v:.2}"))),
                (say!("genda.label.code-length"), dash_or(record.code_length(), |v| format!("{v:.2}"))),
                (say!("genda.label.errors"), record.errors.to_string()),
                (say!("genda.label.elapsed"), genda::clock_text(record.millis)),
                (String::new(), say!("genda.part", page.index + 1, page.count())),
            ];
            let mut x = right.x + 1;
            for (label, value) in line {
                if !label.is_empty() {
                    x = put_text(frame.buffer_mut(), x, right.y, right.x + right.width, &label, base.fg(ink.quiet()));
                    x += 1;
                }
                x = put_text(frame.buffer_mut(), x, right.y, right.x + right.width, &value, base.fg(ink.text()));
                x += 2;
            }
            Rect { y: right.y + 1, height: right.height - 1, ..right }
        }
    };
    if right.height < 6 {
        return None;
    }
    let top = (right.height * 11 / 20).max(3);
    let article = Rect { height: top, ..right };
    let pad_rect = Rect { y: right.y + top, height: right.height - top, ..right };
    let inner = ring(frame, article, page.name.clone());
    draw_article(frame, page, ink, base, inner);
    let inner = ring(frame, pad_rect, format!("{} · {}", say!("genda.box.pad"), page.ime_label()));
    if page.all_done {
        put_text(frame.buffer_mut(), inner.x + 1, inner.y, inner.x + inner.width, &say!("genda.all-done"), base.fg(ink.quiet()));
        return None;
    }
    let pad = seat.pad.as_ref()?;
    let caret = draw_pad(frame, pad, ink, base, inner);
    caret.filter(|_| seat.typing())
}

fn draw_left(
    frame: &mut Frame,
    page: &Page,
    record: &Record,
    (ink, base): (Palette, Style),
    left: Rect,
    ring: &dyn Fn(&mut Frame, Rect, String) -> Rect,
) {
    let keyed = |v: Option<f64>, f: fn(f64) -> String| dash_or(v, f);
    let keys = record.keys;
    let this = vec![
        (say!("genda.label.speed"), record.speed_text()),
        (say!("genda.label.kps"), keyed(record.keys_per_second(), |v| format!("{v:.2}"))),
        (say!("genda.label.code-length"), keyed(record.code_length(), |v| format!("{v:.2}"))),
        (say!("genda.label.key-accuracy"), keyed(record.key_accuracy(), |v| format!("{v:.2}%"))),
        (say!("genda.label.corrections"), record.corrections.to_string()),
        (say!("genda.label.backspaces"), keys.map_or("—".into(), |k| k.backspaces.to_string())),
        (say!("genda.label.picks"), keys.map_or("—".into(), |k| k.picks.to_string())),
        (say!("genda.label.errors"), record.errors.to_string()),
        (say!("genda.label.word-rate"), format!("{:.2}%", record.word_rate())),
        (say!("genda.label.elapsed"), genda::clock_text(record.millis)),
        (say!("genda.label.chars"), format!("{}/{}", record.chars, page.run.target().len().max(record.chars))),
    ];
    let info = vec![
        (String::new(), page.name.clone()),
        (
            say!("genda.label.length"),
            page.length.map_or_else(|| say!("genda.whole"), |n| n.to_string()),
        ),
        (String::new(), say!("genda.part", page.index + 1, page.count())),
        (
            say!("genda.label.order"),
            match page.shuffled {
                true => say!("genda.shuffle-on"),
                false => say!("genda.shuffle-off"),
            },
        ),
        (say!("genda.label.ime"), page.ime_label()),
    ];
    let t = &page.totals;
    let total = vec![
        (say!("genda.label.segments"), t.segments.to_string()),
        (say!("genda.label.average"), format!("{:.2}", t.speed())),
        (say!("genda.label.kps"), keyed(t.keys_per_second(), |v| format!("{v:.2}"))),
        (say!("genda.label.code-length"), keyed(t.code_length(), |v| format!("{v:.2}"))),
        (say!("genda.label.fastest"), format!("{:.2}", t.fastest)),
    ];
    let boxes = [
        (say!("genda.box.this"), this),
        (say!("genda.box.article"), info),
        (say!("genda.box.total"), total),
    ];
    let mut y = left.y;
    let bottom = left.y + left.height;
    for (n, (title, rows)) in boxes.into_iter().enumerate() {
        let want = rows.len() as u16 + 2;
        let room = bottom.saturating_sub(y);
        // The last box takes what is left over.
        let height = match n {
            2 => room,
            _ => want.min(room),
        };
        if height < 3 {
            break;
        }
        let rect = Rect { y, height, ..left };
        let inner = ring(frame, rect, title);
        for (i, (label, value)) in rows.iter().enumerate().take(inner.height as usize) {
            let row = inner.y + i as u16;
            let end = inner.x + inner.width;
            let x = match label.is_empty() {
                true => inner.x + 1,
                false => {
                    put_text(frame.buffer_mut(), inner.x + 1, row, end, label, base.fg(ink.quiet()));
                    inner.x + 8
                }
            };
            put_text(frame.buffer_mut(), x, row, end, value, base.fg(ink.text()));
        }
        y += height;
    }
}

/// Lay `count` characters into rows of `width` cells; gives each row's start.
fn rows_of(chars: &[char], width: usize) -> Vec<usize> {
    let mut starts = vec![0];
    let mut used = 0;
    for (i, &c) in chars.iter().enumerate() {
        let w = yumete_cjk::char_width(c).max(1);
        if used + w > width && used > 0 {
            starts.push(i);
            used = 0;
        }
        used += w;
    }
    starts
}

/// **The segment, each character marked**, scrolled so the next one shows.
fn draw_article(frame: &mut Frame, page: &Page, ink: Palette, base: Style, inner: Rect) {
    let target = page.run.target();
    let width = inner.width.saturating_sub(2) as usize;
    if width == 0 || inner.height == 0 {
        return;
    }
    let starts = rows_of(target, width);
    let next = page.run.typed().len().min(target.len());
    let at_row = starts.iter().rposition(|&s| s <= next).unwrap_or(0);
    let first = at_row.saturating_sub(inner.height as usize / 3);
    for (r, &start) in starts.iter().enumerate().skip(first).take(inner.height as usize) {
        let end = starts.get(r + 1).copied().unwrap_or(target.len());
        let mut x = inner.x + 1;
        let y = inner.y + (r - first) as u16;
        for (i, c) in target.iter().enumerate().take(end).skip(start) {
            let style = match page.run.mark(i) {
                Mark::Right => base.fg(ink.green()),
                Mark::Wrong => base.fg(ink.text()).bg(ink.washed(Accent::Mark)),
                Mark::Next => base.fg(ink.text()).add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                Mark::Untyped => base.fg(ink.text()),
            };
            x = put_text(frame.buffer_mut(), x, y, inner.x + inner.width, &c.to_string(), style);
        }
    }
}

/// **What has been typed**, wrapped, with its caret.
fn draw_pad(frame: &mut Frame, pad: &Editor, ink: Palette, base: Style, inner: Rect) -> Option<Position> {
    let text: Vec<char> = pad.field_text().chars().collect();
    let width = inner.width.saturating_sub(2) as usize;
    if width == 0 || inner.height == 0 {
        return None;
    }
    let caret = pad.cursor().min(text.len());
    let starts = rows_of(&text, width);
    let at_row = starts.iter().rposition(|&s| s <= caret).unwrap_or(0);
    let first = (at_row + 1).saturating_sub(inner.height as usize);
    let mut shown = None;
    for (r, &start) in starts.iter().enumerate().skip(first).take(inner.height as usize) {
        let end = starts.get(r + 1).copied().unwrap_or(text.len());
        let y = inner.y + (r - first) as u16;
        let mut x = inner.x + 1;
        for (i, c) in text.iter().enumerate().take(end).skip(start) {
            if i == caret {
                shown = Some(Position::new(x, y));
            }
            x = put_text(frame.buffer_mut(), x, y, inner.x + inner.width, &c.to_string(), base.fg(ink.text()));
        }
        if caret >= end && r == at_row {
            shown = Some(Position::new(x.min(inner.x + inner.width - 1), y));
        }
    }
    shown
}

/// **The history**, newest first, the picked row in the band's colour.
fn draw_history(frame: &mut Frame, page: &Page, ink: Palette, base: Style, inner: Rect) {
    let heads = genda::columns();
    let rows: Vec<Vec<String>> = page.history.iter().rev().map(Record::cells).collect();
    let mut widths: Vec<usize> = heads.iter().map(|h| yumete_cjk::str_width(h)).collect();
    for row in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(yumete_cjk::str_width(cell));
        }
    }
    let end = inner.x + inner.width;
    let put_row = |frame: &mut Frame, y: u16, cells: &[String], style: Style| {
        let mut x = inner.x + 1;
        for (cell, w) in cells.iter().zip(&widths) {
            put_text(frame.buffer_mut(), x, y, end, cell, style);
            x = x.saturating_add(*w as u16 + 2);
            if x >= end {
                break;
            }
        }
    };
    if inner.height == 0 {
        return;
    }
    put_row(frame, inner.y, &heads, base.fg(ink.gold()));
    let room = inner.height.saturating_sub(1) as usize;
    let first = page.picked.saturating_sub(room.saturating_sub(1));
    for (n, row) in rows.iter().enumerate().skip(first).take(room) {
        let y = inner.y + 1 + (n - first) as u16;
        let style = match n == page.picked {
            true => base.fg(ink.text()).bg(ink.band()),
            false => base.fg(ink.text()),
        };
        if n == page.picked {
            put_text(frame.buffer_mut(), inner.x, y, end, &" ".repeat(inner.width as usize), style);
        }
        put_row(frame, y, row, style);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An editor holding `text`, in Normal, and an IME that holds no keys:
    /// the keys go straight into the pad, as under the system's input method.
    fn seated(text: &str, data: Option<&std::path::Path>) -> (Editor, ImeSession, Seat) {
        let mut ed = Editor::new();
        ed.on_key(Key::Char('i'));
        for c in text.chars() {
            ed.on_key(Key::Char(c));
        }
        ed.on_key(Key::Esc);
        if let Some(dir) = data {
            ed.knows_its_paths(dir.join("config.toml"), dir.to_path_buf());
            ed.set_remembering(true);
        }
        let seat = Seat { tick: Some(100), ..Seat::default() };
        (ed, ImeSession::empty(yumete_ime::Scheme::LINGMING), seat)
    }

    /// Keys as the loop gives them: the page first, the pad when it gives
    /// them back, the editor behind otherwise, and the page settled after each.
    fn keys(seat: &mut Seat, ed: &mut Editor, ime: &mut ImeSession, keys: &[Key]) {
        for key in keys {
            if !seat.took(ed, ime, Some(*key), false) {
                match seat.lend_pad() {
                    Some(mut pad) => {
                        pad.on_key(*key);
                        seat.take_back_pad(ed, pad);
                    }
                    None => {
                        ed.on_key(*key);
                    }
                }
            }
            seat.settle(ed, ime);
        }
    }

    fn typed(text: &str) -> Vec<Key> {
        text.chars()
            .map(|c| match c {
                '\n' => Key::Enter,
                '\u{1b}' => Key::Esc,
                '\u{8}' => Key::Backspace,
                c => Key::Char(c),
            })
            .collect()
    }

    fn command(seat: &mut Seat, ed: &mut Editor, ime: &mut ImeSession, line: &str) {
        keys(seat, ed, ime, &typed(&format!(":{line}\n")));
    }

    #[test]
    fn a_finished_segment_is_said_kept_and_followed_by_the_next() {
        let dir = std::env::temp_dir().join(format!("yumete-genda-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (mut ed, mut ime, mut seat) = seated("abcdefghij\nklmno", Some(&dir));
        command(&mut seat, &mut ed, &mut ime, "genda");
        assert!(seat.open());
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "genda-length 10");
        keys(&mut seat, &mut ed, &mut ime, &typed("abcdefghiX\u{8}j"));
        let said = ed.status().to_string();
        assert!(said.starts_with(&say!("genda.line.segment", 1)), "{said}");
        assert!(said.contains(&say!("genda.line.first")), "{said}");
        let page = seat.page.as_ref().unwrap();
        assert_eq!(page.index, 1);
        assert_eq!(page.run.target(), &['k', 'l', 'm', 'n', 'o']);
        let kept = genda::read_records(&std::fs::read_to_string(dir.join(HISTORY_FILE)).unwrap());
        assert_eq!(kept.len(), 1);
        assert_eq!((kept[0].chars, kept[0].errors, kept[0].corrections), (10, 0, 1));
        assert_eq!(kept[0].millis, 1100);
        // F3 before the next segment's first key types the one just done again.
        seat.took(&mut ed, &mut ime, None, true);
        keys(&mut seat, &mut ed, &mut ime, &typed("abcdefghij"));
        assert!(ed.status().contains(&say!("genda.line.retry", 1)), "{}", ed.status());
        // A page opened again reads the two records back.
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "q");
        assert!(!seat.open());
        command(&mut seat, &mut ed, &mut ime, "genda");
        assert_eq!(seat.page.as_ref().unwrap().history.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn esc_holds_the_page_and_a_second_esc_closes_it() {
        let (mut ed, mut ime, mut seat) = seated("天地玄黄", None);
        command(&mut seat, &mut ed, &mut ime, "genda");
        keys(&mut seat, &mut ed, &mut ime, &typed("天\u{1b}"));
        assert_eq!(ed.status(), say!("genda.paused"));
        // Held, a key types nothing.
        keys(&mut seat, &mut ed, &mut ime, &typed("地"));
        assert_eq!(seat.page.as_ref().unwrap().run.typed(), &['天']);
        keys(&mut seat, &mut ed, &mut ime, &typed("\n地"));
        assert_eq!(seat.page.as_ref().unwrap().run.typed(), &['天', '地']);
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}\u{1b}"));
        assert!(!seat.open());
    }

    #[test]
    fn the_history_copies_one_line_or_the_whole_table() {
        let (mut ed, mut ime, mut seat) = seated("天地玄黄", None);
        command(&mut seat, &mut ed, &mut ime, "genda");
        // A `:` typed in the pad is text: Esc holds the page first.
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "genda-copy");
        assert_eq!(ed.status(), say!("genda.no-record"));
        keys(&mut seat, &mut ed, &mut ime, &typed("\n天地玄黄"));
        let line = ed.status().to_string();
        keys(&mut seat, &mut ed, &mut ime, &[Key::Char('y')]);
        assert_eq!(ed.take_clipboard_request().as_deref(), Some(line.as_str()));
        keys(&mut seat, &mut ed, &mut ime, &[Key::Tab, Key::Char('Y')]);
        let table = ed.take_clipboard_request().unwrap();
        assert_eq!(table.lines().count(), 2);
        assert_eq!(table.lines().next().unwrap(), genda::columns().join("\t"));
        command(&mut seat, &mut ed, &mut ime, "genda-copy-history");
        assert_eq!(ed.status(), say!("genda.copied-history", 1));
    }

    #[test]
    fn the_family_steps_shuffles_and_sets_the_length() {
        let (mut ed, mut ime, mut seat) = seated("一二三四五六七八九十", None);
        command(&mut seat, &mut ed, &mut ime, "genda");
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "genda-length 3");
        assert_eq!(seat.page.as_ref().unwrap().count(), 4);
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('p'), Key::Ctrl('p')]);
        assert_eq!(seat.page.as_ref().unwrap().run.target(), &['七', '八', '九']);
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('u')]);
        assert_eq!(seat.page.as_ref().unwrap().index, 1);
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('l')]);
        let page = seat.page.as_ref().unwrap();
        assert!(page.shuffled);
        assert_eq!(page.order, genda::shuffled(&page.chars, 7));
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "genda-shuffle");
        assert_eq!(seat.page.as_ref().unwrap().order, seat.page.as_ref().unwrap().chars);
    }
}
