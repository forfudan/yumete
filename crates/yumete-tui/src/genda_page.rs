//! **跟打** (`:genda`): the typing trainer's page.
//!
//! ```text
//! ╭ 本段 ────────────╮╭ 常用单字 前500 ─────────────────────────╮
//! │ 速度   112.40    ││ 的一是了不在有个人这上中大为来我到出要 │
//! │ 击键     5.82    ││ 以时和地们得可下对生也子就过能他会多发 │
//! │ 当量     1.42    ││                                        │
//! ╰──────────────────╯╰ 的 a ──────────────────────────────────╯
//! ╭ 文章 ────────────╮╭ 跟打 · 靈明 ───────────────────────────╮
//! │ 段长 25 第 3/9 段││ 的一是了不在有个人这上中大为来我到出要 │
//! │ 目标 速度≥120    │╰────────────────────────────────────────╯
//! ╰──────────────────╯╭ 成绩 ──────────────────────────────────╮
//! ╭ 累计 ────────────╮│ 段  速度    击键  码长  当量  字数  错字 │
//! │ 段数   2         ││ 2   112.40  5.82  2.80  1.42  25    0    │
//! ╰──────────────────╯╰────────────────────────────────────────╯
//!  F3 重打  C-p 下一段  C-u 上一段  C-l 乱序  Tab 历史  Esc 暂停  C-o 全文 …
//! ```
//!
//! The code hint sits on the article box's bottom wall; in the pad a 漢字
//! that came in a word is gold. Under the pad, from [`TABLE_FROM`] rows,
//! the results: this session's segments newest first, then the earlier
//! history dimmed. A finished segment says only its number and speed on
//! the command row; the whole score line is the table's row and `y`'s.
//!
//! Esc while a code is composed clears the code; Esc otherwise pauses, and
//! another Esc keeps the page paused, because a shape-code hand presses Esc
//! twice to be sure. `q` while paused closes, as `:q` does.
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
use yumete_core::command::Switch;
use yumete_core::genda::{self, Ask, Choice, Daily, Goal, Mark, Pressed, Record, Run, Stroke, Totals};
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

/// **From this many rows of page** (a terminal of 24) the right column has
/// the results table under the pad; below it, the article and the pad
/// share the column, and the figures are the left column's or the one row's.
const TABLE_FROM: u16 = 23;

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
    /// `:genda-hint`, `:genda-auto` (kept the other way round, so the
    /// default is on) and the goal: the reader's choices, kept while the
    /// editor runs, through the page closing and opening again.
    hint: bool,
    manual_next: bool,
    goal: Goal,
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

/// **A segment of one run's missed or slow characters**, typed in place
/// of the article's segment until it is done.
#[derive(Clone)]
struct Drill {
    label: String,
    chars: Vec<char>,
    /// The segment it was drawn from, from 1.
    segment: usize,
    /// Attempts at it before this one, for 重打.
    tries: usize,
}

/// What comes after a finished segment: brought up at once, or on Enter
/// while `:genda-auto` is off.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Then {
    /// The segment after.
    Next,
    /// The same one, the goal not met.
    Again,
    /// The drill is done: back to the article's segment in hand.
    Back,
}

/// What 重打 brings back before the next run's first key: the segment just
/// finished, or the drill.
#[derive(Clone)]
enum Redo {
    Segment(usize),
    Drill(Drill),
}

struct Page {
    name: String,
    chars: Vec<char>,
    /// The order in force: `chars`, or a shuffle of them.
    order: Vec<char>,
    shuffled: bool,
    length: Option<usize>,
    /// The last length given as a number, which 全文 turned off goes back to.
    chosen: usize,
    index: usize,
    /// Attempts begun at each segment, for 重打.
    tries: Vec<usize>,
    /// The run in hand has been counted among them.
    counted: bool,
    run: Run,
    view: View,
    history: Vec<Record>,
    /// How many of `history` were read from the file at opening: those
    /// after are this session's.
    earlier: usize,
    /// The history row picked, counted from the newest.
    picked: usize,
    totals: Totals,
    latest: Option<Record>,
    all_done: bool,
    /// Esc stopped the page: keys wait for Enter, `q` closes, and another
    /// Esc stays (a shape-code hand presses Esc again to be sure a code is
    /// cleared, and that must not close the page).
    held: bool,
    /// The scheme's name while the built-in IME holds the keys; empty under
    /// the system's input method.
    scheme: String,
    file: Option<PathBuf>,
    drill: Option<Drill>,
    /// The latest finished run's missed and slow characters; `None` before
    /// one is finished.
    last_missed: Option<Vec<char>>,
    last_slow: Vec<char>,
    /// A finished segment waiting for Enter, `:genda-auto` being off.
    waiting: Option<Then>,
    redo: Option<Redo>,
    /// The next character and its code, while the hint is on and Yume has
    /// a code for it.
    hint: Option<String>,
    daily: Daily,
}

impl Page {
    fn count(&self) -> usize {
        genda::segment_count(self.order.len(), self.length)
    }

    fn target(&self) -> Vec<char> {
        match &self.drill {
            Some(drill) => drill.chars.clone(),
            None => self.order[genda::segment(self.order.len(), self.length, self.index)].to_vec(),
        }
    }

    /// The attempts counted for what is being typed: the drill's, or the segment's.
    fn tries_mut(&mut self) -> Option<&mut usize> {
        match &mut self.drill {
            Some(drill) => Some(&mut drill.tries),
            None => self.tries.get_mut(self.index),
        }
    }

    /// The history by local calendar day, for the 累計 box and the history's title.
    fn add_up_the_days(&mut self) {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let today = yumete_core::clock::local_day(now as i64);
        self.daily = Daily::of(&self.history, |at| yumete_core::clock::local_day(at as i64), today);
    }

    /// The article box's name: the article's, or the drill's after it.
    fn title(&self) -> String {
        match &self.drill {
            Some(drill) => format!("{} · {}", self.name, drill.label),
            None => self.name.clone(),
        }
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
            Key::Ctrl('p' | 'u' | 'l' | 'e' | 's' | 'o') => {
                if composing {
                    ime.escape();
                }
                match key {
                    Key::Ctrl('p') => self.step(editor, true),
                    Key::Ctrl('u') => self.step(editor, false),
                    Key::Ctrl('e') => self.drill(editor, false),
                    Key::Ctrl('s') => self.drill(editor, true),
                    Key::Ctrl('o') => self.set_whole(editor, Switch::Flip),
                    _ => self.shuffle(editor),
                }
                return true;
            }
            // The hint leaves a code being composed alone: it changes what
            // is shown, not what is typed.
            Key::Ctrl('t') => {
                self.set_hint(editor, ime, Switch::Flip);
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
                Key::Enter => {
                    if let Some(then) = page.waiting.take() {
                        self.go_on(then);
                    }
                }
                Key::Char('q') => self.close(editor),
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
        let stroke = Stroke {
            yume: ime.holding_the_keys(),
            composing,
            backspace: key == Key::Backspace,
            key: Pressed::of(key),
        };
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
        let stroke = Stroke { yume: true, composing: true, backspace: false, key: Some(Pressed::Shift) };
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
                Ask::Length(length) => self.set_length(editor, length),
                Ask::Whole(switch) => self.set_whole(editor, switch),
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
                Ask::Hint(switch) => self.set_hint(editor, ime, switch),
                Ask::Auto(switch) => {
                    self.manual_next = !switch.over(!self.manual_next);
                    editor.set_status(match self.manual_next {
                        false => say!("genda.auto-on"),
                        true => say!("genda.auto-off"),
                    });
                    // Turned on over a segment waiting for Enter, it goes on at once.
                    if !self.manual_next {
                        if let Some(then) = self.page.as_mut().and_then(|p| p.waiting.take()) {
                            self.go_on(then);
                        }
                    }
                }
                Ask::GoalSpeed(v) => self.set_goal(editor, |g| g.speed = genda::from_hundredths(v)),
                Ask::GoalKps(v) => self.set_goal(editor, |g| g.kps = genda::from_hundredths(v)),
                Ask::GoalAccuracy(v) => self.set_goal(editor, |g| g.accuracy = genda::from_hundredths(v)),
                Ask::GoalClean(switch) => self.set_goal(editor, |g| g.clean = switch.over(g.clean)),
                Ask::GoalOff => self.set_goal(editor, |g| *g = Goal::default()),
                Ask::GoalShow => self.set_goal(editor, |_| {}),
                Ask::RetryWrong => self.drill(editor, false),
                Ask::RetrySlow => self.drill(editor, true),
            }
        }
        self.look_up_the_hint(ime);
        let whole = self.page.as_ref().is_some_and(|p| p.length.is_none());
        editor.set_genda_switches(genda::Switches { hint: self.hint, auto: !self.manual_next, clean: self.goal.clean, whole });
    }

    /// **A segment length**, `None` for 全文, from the article's first
    /// segment: the segments are cut afresh, and a place in one cut names
    /// nothing in another.
    fn set_length(&mut self, editor: &mut Editor, length: Option<usize>) {
        let Some(page) = self.page.as_mut() else { return };
        page.length = length;
        if let Some(n) = length {
            page.chosen = n;
        }
        page.index = 0;
        self.restart();
        editor.set_status(match length {
            Some(n) => say!("genda.length-set", n),
            None => say!("genda.length-all"),
        });
    }

    /// **全文 or the length chosen** (`:genda-whole`, C-o).
    fn set_whole(&mut self, editor: &mut Editor, switch: Switch) {
        let Some(page) = self.page.as_ref() else { return };
        let length = match switch.over(page.length.is_none()) {
            true => None,
            false => Some(page.chosen),
        };
        self.set_length(editor, length);
    }

    /// **The next character's code**, while the hint is on, Yume holds the
    /// keys and its table has a code for it: `字 code`, and the candidate's
    /// place after when it is not the first.
    fn look_up_the_hint(&mut self, ime: &ImeSession) {
        let on = self.hint && ime.holding_the_keys();
        let Some(page) = self.page.as_mut() else { return };
        let next = page.run.target().get(page.run.typed().len()).copied();
        page.hint = match (on && page.view == View::Typing && !page.run.finished(), next) {
            (true, Some(c)) => ime.quickest_code(&c.to_string()).map(|(code, place)| match place {
                0 => format!("{c} {code}"),
                n => format!("{c} {code} {}", n + 1),
            }),
            _ => None,
        };
    }

    fn set_hint(&mut self, editor: &mut Editor, ime: &ImeSession, switch: Switch) {
        self.hint = switch.over(self.hint);
        self.look_up_the_hint(ime);
        editor.set_status(match (self.hint, ime.holding_the_keys()) {
            (false, _) => say!("genda.hint-off"),
            (true, true) => say!("genda.hint-on"),
            (true, false) => say!("genda.hint-unavailable"),
        });
    }

    /// Change the goal, or not, and say it.
    fn set_goal(&mut self, editor: &mut Editor, change: impl FnOnce(&mut Goal)) {
        change(&mut self.goal);
        editor.set_status(say!("genda.goal-is", self.goal.text()));
    }

    /// **错字重打 / 慢字重打**: a segment of the latest finished run's missed
    /// characters, or of its slowest ([`genda::SLOW`]).
    fn drill(&mut self, editor: &mut Editor, slow: bool) {
        let Some(page) = self.page.as_mut() else { return };
        let Some(missed) = page.last_missed.clone() else {
            editor.set_status(say!("genda.no-last"));
            return;
        };
        let chars = match slow {
            true => page.last_slow.clone(),
            false => missed,
        };
        if chars.is_empty() {
            editor.set_status(say!("genda.no-wrong"));
            return;
        }
        let label = match slow {
            true => say!("genda.drill.slow"),
            false => say!("genda.drill.wrong"),
        };
        let segment = page.latest.as_ref().map_or(page.index + 1, |r| r.segment);
        page.drill = Some(Drill { label, chars, segment, tries: 0 });
        page.waiting = None;
        editor.set_status(String::new());
        self.load();
    }

    /// **Bring up what comes after a finished segment.**
    fn go_on(&mut self, then: Then) {
        let Some(page) = self.page.as_mut() else { return };
        match then {
            Then::Next => page.index += 1,
            Then::Again => {}
            Then::Back => page.drill = None,
        }
        self.load();
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
            chosen: genda::DEFAULT_LENGTH,
            index: 0,
            tries: Vec::new(),
            counted: false,
            run: Run::default(),
            view: View::Typing,
            earlier: history.len(),
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
            drill: None,
            last_missed: None,
            last_slow: Vec::new(),
            waiting: None,
            redo: None,
            hint: None,
            daily: Daily::default(),
        });
        if let Some(page) = self.page.as_mut() {
            page.add_up_the_days();
        }
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
            page.drill = None;
        }
        self.load();
    }

    /// **A fresh run at the segment in hand** (or the drill), with an empty pad.
    fn load(&mut self) {
        let Some(page) = self.page.as_mut() else { return };
        if page.run.started() && !page.counted {
            if let Some(tries) = page.tries_mut() {
                *tries += 1;
            }
        }
        page.run = Run::new(page.target());
        page.counted = false;
        page.all_done = false;
        page.held = false;
        page.waiting = None;
        page.redo = None;
        page.view = View::Typing;
        let mut pad = Box::new(Editor::new());
        pad.open_field("", true);
        self.pad = Some(pad);
        self.pending = None;
    }

    fn step(&mut self, editor: &mut Editor, on: bool) {
        let Some(page) = self.page.as_mut() else { return };
        // Out of a drill, the step is from the article's segment in hand.
        page.drill = None;
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
    /// finished (a drill too), which is what a hand reaching for F3 straight
    /// after a segment wants again.
    fn retry(&mut self, editor: &mut Editor) {
        let Some(page) = self.page.as_mut() else { return };
        if !page.run.started() {
            match page.redo.take() {
                Some(Redo::Segment(index)) => page.index = index,
                Some(Redo::Drill(drill)) => page.drill = Some(drill),
                None => {}
            }
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

    /// **The segment is done**: kept, added up, said, and what comes after
    /// brought up, or left for Enter while `:genda-auto` is off. A goal not
    /// met brings up the same segment again.
    fn finish(&mut self, editor: &mut Editor) {
        let now = self.clock();
        let goal = self.goal;
        let manual = self.manual_next;
        let Some(page) = self.page.as_mut() else { return };
        let retry = page.tries_mut().map_or(0, |tries| {
            *tries += 1;
            *tries - 1
        });
        page.counted = true;
        let record = Record {
            at: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()),
            segment: page.drill.as_ref().map_or(page.index + 1, |d| d.segment),
            retry,
            scheme: page.scheme.clone(),
            article: page.title(),
            version: version(),
            ..page.run.record(now)
        };
        page.last_missed = Some(page.run.missed());
        page.last_slow = page.run.slowest(genda::SLOW);
        let missed = goal.missed(&record);
        if let Some(file) = &page.file {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            use std::io::Write as _;
            if let Ok(mut out) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
                let _ = writeln!(out, "{}", record.to_line());
            }
        }
        // A short line: the whole score line is the table's row and `y`'s.
        let said = |typing: bool| {
            let short = match typing {
                true => say!("genda.done-typing", record.segment, record.speed_text()),
                false => say!("genda.done", record.segment, record.speed_text()),
            };
            match missed.is_empty() {
                true => short,
                false => format!("{} {short}", say!("genda.goal-missed", missed.join(" "))),
            }
        };
        let (ready, typing) = (said(false), said(true));
        page.totals.add(&record);
        page.history.push(record.clone());
        page.latest = Some(record);
        page.add_up_the_days();
        let then = match () {
            _ if !missed.is_empty() => Then::Again,
            _ if page.drill.is_some() => Then::Back,
            _ if page.index + 1 < page.count() => Then::Next,
            _ => {
                page.all_done = true;
                editor.set_status(ready);
                return;
            }
        };
        if manual {
            page.waiting = Some(then);
            editor.set_status(ready);
            return;
        }
        // The next run takes the keys at once, so `y` waits for Esc.
        editor.set_status(typing);
        let redo = match &page.drill {
            Some(drill) => Redo::Drill(drill.clone()),
            None => Redo::Segment(page.index),
        };
        self.go_on(then);
        if let Some(page) = self.page.as_mut() {
            page.redo = Some(redo);
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
    let hints = match (page.view, seat.paused() || page.all_done || page.waiting.is_some()) {
        (View::History, _) => say!("genda.hints-history"),
        (_, true) => say!("genda.hints-paused"),
        _ => say!("genda.hints"),
    };
    let foot_y = area.y + area.height - 1;
    put_text(frame.buffer_mut(), area.x + 1, foot_y, area.x + area.width, &whole_hints(&hints, area.width.saturating_sub(1) as usize), base.fg(ink.quiet()));
    let body = Rect { height: area.height - 1, ..area };
    let ring_with_foot = |frame: &mut Frame, rect: Rect, title: String, foot: Option<String>| -> Rect {
        crate::chrome::draw(frame, rect, &crate::chrome::Ring {
            rounded: config.panel.rounded,
            open_sides: false,
            border: base.fg(ink.rule()),
            ground: base,
            title: Some((title, base.fg(ink.gold()))),
            foot: foot.map(|text| (text, base.fg(ink.text()))),
        })
    };
    let ring = |frame: &mut Frame, rect: Rect, title: String| ring_with_foot(frame, rect, title, None);
    if page.view == View::History {
        let d = &page.daily;
        let title = format!(
            "{} · {}",
            say!("genda.box.history", page.history.len()),
            say!("genda.daily.title", d.today_chars, hours(d.today_millis), d.chars, d.days, d.streak),
        );
        let inner = ring(frame, body, title);
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
            draw_left(frame, (page, seat.goal), &record, (ink, base), left, &ring);
            right
        }
        None => {
            let line = [
                (say!("genda.label.speed"), record.speed_text()),
                (say!("genda.label.kps"), dash_or(record.keys_per_second(), |v| format!("{v:.2}"))),
                (say!("genda.label.code-length"), dash_or(record.code_length(), |v| format!("{v:.2}"))),
                (say!("genda.label.equivalence"), dash_or(record.equivalence(), |v| format!("{v:.2}"))),
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
    let (top, pad_height) = split(right.height, area.height >= TABLE_FROM);
    let article = Rect { height: top, ..right };
    let pad_rect = Rect { y: right.y + top, height: pad_height, ..right };
    let results = right.height - top - pad_height;
    if results >= 3 {
        let rect = Rect { y: pad_rect.y + pad_height, height: results, ..right };
        let inner = ring(frame, rect, say!("genda.box.results"));
        draw_results(frame, page, ink, base, inner);
    }
    let inner = ring_with_foot(frame, article, page.title(), page.hint.clone());
    draw_article(frame, page, ink, base, inner);
    let inner = ring(frame, pad_rect, format!("{} · {}", say!("genda.box.pad"), page.ime_label()));
    if page.all_done {
        put_text(frame.buffer_mut(), inner.x + 1, inner.y, inner.x + inner.width, &say!("genda.all-done"), base.fg(ink.quiet()));
        return None;
    }
    let pad = seat.pad.as_ref()?;
    let caret = draw_pad(frame, pad, &page.run, ink, base, inner);
    caret.filter(|_| seat.typing())
}

/// **The foot's hints that fit in `room` cells**, whole: the hints are
/// parted by two spaces, and one cut in half (`C-t 提`) names a key and no
/// deed, so those that do not fit are left out from the end.
fn whole_hints(hints: &str, room: usize) -> String {
    let mut out = String::new();
    for hint in hints.split("  ") {
        let wanted = match out.is_empty() {
            true => hint.to_string(),
            false => format!("{out}  {hint}"),
        };
        if yumete_cjk::str_width(&wanted) > room {
            break;
        }
        out = wanted;
    }
    out
}

/// **The right column's rows**: the article box's and the pad's. With the
/// table, about two sevenths and a quarter, the table taking the rest, so a
/// taller window shows more rows of results; without, the article a little
/// over half.
fn split(height: u16, table: bool) -> (u16, u16) {
    match table {
        true => ((height * 2 / 7).max(5), (height / 4).max(4)),
        false => {
            let top = (height * 11 / 20).max(3);
            (top, height - top)
        }
    }
}

/// The results table's columns, of [`genda::columns`]: 段 速度 擊鍵 碼長
/// 當量 字數 錯字 用時 回改 鍵準 打詞 時間 文章.
const RESULT_COLUMNS: [usize; 13] = [1, 2, 3, 4, 5, 6, 7, 8, 11, 12, 13, 0, 15];

/// The article column, the last: it counts this many cells when the
/// columns are fitted, and a longer name is cut at the panel's edge.
const ARTICLE_CELLS: usize = 8;

/// **The order the table gives its columns up in** when the panel is too
/// narrow, as places in [`RESULT_COLUMNS`]: the article and the clock time
/// first (the history has both, and this session's rows follow one
/// another), then 打詞, 鍵準, 回改 and 當量; 段 and 速度 are never given up.
const GIVEN_UP: [usize; 11] = [12, 11, 10, 9, 8, 4, 7, 6, 5, 3, 2];

/// **Which columns fit in `room` cells**, given each one's width: a cell
/// before the first and two between, the columns in [`GIVEN_UP`] order
/// left out until the rest fit.
fn fitted(widths: &[usize], room: usize) -> Vec<bool> {
    let mut kept = vec![true; widths.len()];
    let wanted = |kept: &[bool]| {
        let shown: Vec<usize> = widths.iter().zip(kept).filter(|(_, k)| **k).map(|(w, _)| *w).collect();
        1 + shown.iter().sum::<usize>() + 2 * shown.len().saturating_sub(1)
    };
    for &place in &GIVEN_UP {
        if wanted(&kept) <= room {
            break;
        }
        if let Some(k) = kept.get_mut(place) {
            *k = false;
        }
    }
    kept
}

/// **The results under the pad**, newest first: this session's segments,
/// the latest in gold, then the history's earlier records dimmed, as many
/// as fit. Tab shows them all, with their details.
fn draw_results(frame: &mut Frame, page: &Page, ink: Palette, base: Style, inner: Rect) {
    if inner.height == 0 || inner.width < 4 {
        return;
    }
    let end = inner.x + inner.width;
    let pick = |all: Vec<String>| -> Vec<String> { RESULT_COLUMNS.iter().map(|&i| all[i].clone()).collect() };
    let heads = pick(genda::columns());
    let room = inner.height.saturating_sub(1) as usize;
    let rows: Vec<(Vec<String>, Style)> = page
        .history
        .iter()
        .enumerate()
        .rev()
        .take(room)
        .map(|(i, record)| {
            let style = match () {
                _ if i < page.earlier => base.fg(ink.furniture()),
                _ if i + 1 == page.history.len() => base.fg(ink.gold()),
                _ => base.fg(ink.text()),
            };
            (pick(record.cells()), style)
        })
        .collect();
    let mut widths: Vec<usize> = heads.iter().map(|h| yumete_cjk::str_width(h)).collect();
    for (row, _) in &rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(yumete_cjk::str_width(cell));
        }
    }
    let mut fitting = widths.clone();
    if let Some(last) = fitting.last_mut() {
        *last = (*last).min(ARTICLE_CELLS);
    }
    let kept = fitted(&fitting, inner.width as usize);
    let put_row = |frame: &mut Frame, y: u16, cells: &[String], style: Style| {
        let mut x = inner.x + 1;
        for ((cell, w), _) in cells.iter().zip(&widths).zip(&kept).filter(|(_, k)| **k) {
            put_text(frame.buffer_mut(), x, y, end, cell, style);
            x = x.saturating_add(*w as u16 + 2);
        }
    };
    put_row(frame, inner.y, &heads, base.fg(ink.quiet()));
    if rows.is_empty() && room > 0 {
        put_text(frame.buffer_mut(), inner.x + 1, inner.y + 1, end, &say!("genda.no-record"), base.fg(ink.quiet()));
    }
    for (n, (row, style)) in rows.iter().enumerate() {
        put_row(frame, inner.y + 1 + n as u16, row, *style);
    }
}

/// `h:mm:ss`, for a day's time at the keys.
fn hours(millis: u64) -> String {
    let s = millis / 1000;
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

fn draw_left(
    frame: &mut Frame,
    (page, goal): (&Page, Goal),
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
        (say!("genda.label.equivalence"), keyed(record.equivalence(), |v| format!("{v:.2}"))),
        (say!("genda.label.key-accuracy"), keyed(record.key_accuracy(), |v| format!("{v:.2}%"))),
        (say!("genda.label.corrections"), record.corrections.to_string()),
        (say!("genda.label.backspaces"), keys.map_or("—".into(), |k| k.backspaces.to_string())),
        (say!("genda.label.picks"), keys.map_or("—".into(), |k| k.picks.to_string())),
        (say!("genda.label.errors"), record.errors.to_string()),
        (say!("genda.label.word-rate"), format!("{:.2}%", record.word_rate())),
        (say!("genda.label.elapsed"), genda::clock_text(record.millis)),
        (say!("genda.label.chars"), format!("{}/{}", record.chars, page.run.target().len().max(record.chars))),
    ];
    // The article's name is the article box's title, beside it.
    let info = vec![
        // One row for both, so the 累計 box below keeps a row for today.
        (
            say!("genda.label.length"),
            format!(
                "{}  {}",
                page.length.map_or_else(|| say!("genda.whole"), |n| n.to_string()),
                say!("genda.part", page.index + 1, page.count()),
            ),
        ),
        (
            say!("genda.label.order"),
            match page.shuffled {
                true => say!("genda.shuffle-on"),
                false => say!("genda.shuffle-off"),
            },
        ),
        (say!("genda.label.ime"), page.ime_label()),
        (say!("genda.label.goal"), goal.text()),
    ];
    let t = &page.totals;
    let d = &page.daily;
    let total = vec![
        (say!("genda.label.segments"), t.segments.to_string()),
        (say!("genda.label.average"), format!("{:.2}", t.speed())),
        (say!("genda.label.kps"), keyed(t.keys_per_second(), |v| format!("{v:.2}"))),
        (say!("genda.label.code-length"), keyed(t.code_length(), |v| format!("{v:.2}"))),
        (say!("genda.label.fastest"), format!("{:.2}", t.fastest)),
        (say!("genda.label.today"), say!("genda.daily.today", d.today_chars, hours(d.today_millis))),
        (say!("genda.label.overall"), say!("genda.daily.overall", d.chars, d.days)),
        (say!("genda.label.streak"), say!("genda.daily.streak", d.streak)),
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

/// **What has been typed**, wrapped, with its caret; a 漢字 that came in a
/// word in gold, the picture of 打詞率 ([`Run::word_marks`]).
fn draw_pad(frame: &mut Frame, pad: &Editor, run: &Run, ink: Palette, base: Style, inner: Rect) -> Option<Position> {
    let text: Vec<char> = pad.field_text().chars().collect();
    let width = inner.width.saturating_sub(2) as usize;
    if width == 0 || inner.height == 0 {
        return None;
    }
    // The marks go by the run's positions, which are the pad's while it
    // holds nothing the run strips (a line break).
    let marks = match genda::strip(&pad.field_text()).len() == text.len() {
        true => run.word_marks(),
        false => Vec::new(),
    };
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
            let ink_of = match marks.get(i) {
                Some(true) => ink.gold(),
                _ => ink.text(),
            };
            x = put_text(frame.buffer_mut(), x, y, inner.x + inner.width, &c.to_string(), base.fg(ink_of));
        }
        if caret >= end && r == at_row {
            shown = Some(Position::new(x.min(inner.x + inner.width - 1), y));
        }
    }
    shown
}

/// The history table's columns, of [`genda::columns`]: up to the article,
/// less 當量, 峰值 and 停留, so the article still fits a hundred columns.
/// Those and what follows the article are in the picked row's details
/// under it ([`Record::details`]).
const TABLE_COLUMNS: [usize; 13] = [0, 1, 2, 3, 4, 6, 7, 8, 11, 12, 13, 14, 15];

/// **The history**, newest first, the picked row inked across, and
/// its details at the foot.
fn draw_history(frame: &mut Frame, page: &Page, ink: Palette, base: Style, inner: Rect) {
    // Two rows of details and one between, when the table keeps three of its own.
    let inner = match (inner.height >= 6, page.history.iter().rev().nth(page.picked)) {
        (true, Some(record)) => {
            let top = inner.y + inner.height - 2;
            let end = inner.x + inner.width;
            let details = record.details();
            let (first, second) = details.split_at(3);
            for (row, items) in [first, second].into_iter().enumerate() {
                let mut x = inner.x + 1;
                for (label, value) in items {
                    x = put_text(frame.buffer_mut(), x, top + row as u16, end, label, base.fg(ink.quiet()));
                    x = put_text(frame.buffer_mut(), x + 1, top + row as u16, end, value, base.fg(ink.text()));
                    x += 3;
                }
            }
            Rect { height: inner.height - 3, ..inner }
        }
        _ => inner,
    };
    let pick = |all: Vec<String>| -> Vec<String> { TABLE_COLUMNS.iter().map(|&i| all[i].clone()).collect() };
    let heads = pick(genda::columns());
    let rows: Vec<Vec<String>> = page.history.iter().rev().map(|r| pick(r.cells())).collect();
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
        // The picked row is inked, ink and ground changing places, as the
        // `:` menu's pick and a focused sidebar's row are: a quiet band
        // between rows of the same ink is lost.
        let style = match n == page.picked {
            true => ink.reversed(base.fg(ink.text())),
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
        let latest = seat.page.as_ref().unwrap().latest.clone().unwrap();
        assert_eq!(ed.status(), say!("genda.done-typing", 1, latest.speed_text()));
        assert!(latest.score_line().contains(&say!("genda.line.first")));
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
        assert_eq!(seat.page.as_ref().unwrap().latest.as_ref().map(|r| r.retry), Some(1));
        // A page opened again reads the two records back.
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "q");
        assert!(!seat.open());
        command(&mut seat, &mut ed, &mut ime, "genda");
        assert_eq!(seat.page.as_ref().unwrap().history.len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn esc_holds_the_page_a_second_esc_stays_and_q_closes_it() {
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
        assert!(seat.open() && seat.paused());
        keys(&mut seat, &mut ed, &mut ime, &typed("q"));
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
        let line = page(&seat).latest.as_ref().unwrap().score_line();
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

    /// The page open on `text` in segments of `length`, held so a command can be typed.
    fn opened(text: &str, length: usize) -> (Editor, ImeSession, Seat) {
        let (mut ed, mut ime, mut seat) = seated(text, None);
        command(&mut seat, &mut ed, &mut ime, "genda");
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, &format!("genda-length {length}"));
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        (ed, ime, seat)
    }

    fn page(seat: &Seat) -> &Page {
        seat.page.as_ref().unwrap()
    }

    #[test]
    fn with_auto_off_a_finished_segment_waits_for_enter() {
        let (mut ed, mut ime, mut seat) = opened("一二三四五六七八九", 3);
        command(&mut seat, &mut ed, &mut ime, "genda-auto off");
        assert_eq!(ed.status(), say!("genda.auto-off"));
        keys(&mut seat, &mut ed, &mut ime, &typed("\n一二三"));
        assert!(page(&seat).waiting.is_some() && page(&seat).run.finished());
        assert!(!seat.typing());
        // Typed keys go nowhere while it waits; Enter brings up the next.
        keys(&mut seat, &mut ed, &mut ime, &typed("四"));
        assert_eq!((page(&seat).index, page(&seat).run.typed().len()), (0, 3));
        keys(&mut seat, &mut ed, &mut ime, &typed("\n"));
        assert_eq!(page(&seat).run.target(), &['四', '五', '六']);
        // F3 while waiting types the same segment again.
        keys(&mut seat, &mut ed, &mut ime, &typed("四五六"));
        seat.took(&mut ed, &mut ime, None, true);
        assert_eq!((page(&seat).index, page(&seat).run.started()), (1, false));
        assert!(page(&seat).waiting.is_none());
    }

    #[test]
    fn a_goal_not_met_brings_the_segment_up_again() {
        let (mut ed, mut ime, mut seat) = opened("一二三四五六", 3);
        command(&mut seat, &mut ed, &mut ime, "genda-goal-clean");
        assert_eq!(ed.status(), say!("genda.goal-is", say!("genda.goal.clean")));
        keys(&mut seat, &mut ed, &mut ime, &typed("\n一十三"));
        assert!(ed.status().starts_with(&say!("genda.goal-missed", say!("genda.goal.clean"))), "{}", ed.status());
        assert_eq!((page(&seat).index, page(&seat).run.started()), (0, false));
        keys(&mut seat, &mut ed, &mut ime, &typed("一二三"));
        assert_eq!(page(&seat).latest.as_ref().map(|r| r.retry), Some(1));
        assert_eq!(page(&seat).index, 1);
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "genda-goal-speed 120");
        command(&mut seat, &mut ed, &mut ime, "genda-goal off");
        assert_eq!(seat.goal, Goal::default());
        command(&mut seat, &mut ed, &mut ime, "genda-goal-kps 6.5");
        assert_eq!(seat.goal.kps, Some(6.5));
        command(&mut seat, &mut ed, &mut ime, "genda-goal-kps off");
        assert_eq!(seat.goal.kps, None);
    }

    #[test]
    fn the_missed_and_the_slow_make_a_drill_and_then_the_article_goes_on() {
        let (mut ed, mut ime, mut seat) = opened("天地人和春夏秋冬", 4);
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('e')]);
        assert_eq!(ed.status(), say!("genda.no-last"));
        keys(&mut seat, &mut ed, &mut ime, &typed("\n天他\u{8}地人和"));
        assert_eq!(page(&seat).index, 1);
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('e')]);
        assert_eq!(page(&seat).run.target(), &['地']);
        assert!(page(&seat).title().ends_with(&say!("genda.drill.wrong")));
        keys(&mut seat, &mut ed, &mut ime, &typed("地"));
        let latest = page(&seat).latest.clone().unwrap();
        assert_eq!((latest.segment, latest.chars), (1, 1));
        assert!(latest.article.ends_with(&say!("genda.drill.wrong")));
        // Back to the segment it was drawn after, and F3 brings the drill again.
        assert_eq!(page(&seat).run.target(), &['春', '夏', '秋', '冬']);
        seat.took(&mut ed, &mut ime, None, true);
        assert_eq!(page(&seat).run.target(), &['地']);
        keys(&mut seat, &mut ed, &mut ime, &typed("地"));
        assert_eq!(page(&seat).latest.as_ref().map(|r| r.retry), Some(1));
        // The slowest of a one-character drill is that character.
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('s')]);
        assert_eq!(page(&seat).run.target(), &['地']);
        keys(&mut seat, &mut ed, &mut ime, &typed("地春夏秋冬"));
        assert_eq!(page(&seat).index, 1);
        assert_eq!(page(&seat).history.len(), 5);
        assert!(page(&seat).all_done);
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('e')]);
        assert_eq!(ed.status(), say!("genda.no-wrong"));
    }

    #[test]
    fn the_hint_shows_the_next_characters_code_while_yume_holds_the_keys() {
        let (mut ed, _, mut seat) = seated("天地", None);
        let mut ime = ImeSession::from_table_text(yumete_ime::Scheme::LINGMING, "ab 天\nab 夫\ncd 地\n");
        command(&mut seat, &mut ed, &mut ime, "genda");
        assert_eq!(page(&seat).hint, None);
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('t')]);
        assert_eq!(ed.status(), say!("genda.hint-on"));
        assert_eq!(page(&seat).hint.as_deref(), Some("天 ab"));
        keys(&mut seat, &mut ed, &mut ime, &typed("天"));
        assert_eq!(page(&seat).hint.as_deref(), Some("地 cd"));
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('t')]);
        assert_eq!(page(&seat).hint, None);
        // Under the system's input method there is no code to show.
        let (mut ed, mut ime, mut seat) = seated("天地", None);
        command(&mut seat, &mut ed, &mut ime, "genda");
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('t')]);
        assert_eq!(ed.status(), say!("genda.hint-unavailable"));
        assert_eq!(page(&seat).hint, None);
    }

    #[test]
    fn yumes_keys_give_the_equivalence_and_the_day_is_added_up() {
        let (mut ed, _, mut seat) = seated("dk;a", None);
        let mut ime = ImeSession::from_table_text(yumete_ime::Scheme::LINGMING, "ab 天\n");
        command(&mut seat, &mut ed, &mut ime, "genda");
        keys(&mut seat, &mut ed, &mut ime, &typed("dk;a"));
        let latest = page(&seat).latest.clone().unwrap();
        let strokes = latest.strokes.clone().unwrap();
        // dk 1.1, k; 1.7, ;a 1.2.
        assert_eq!((strokes.pairs, format!("{:.2}", strokes.mean())), (3, "1.33".to_string()));
        assert!(latest.score_line().contains(&say!("genda.line.equivalence", "1.33")));
        assert_eq!((page(&seat).daily.today_chars, page(&seat).daily.streak), (4, 1));
        // In the results table the key fields are measured: no `—`.
        let (_, rows) = drawn(&seat, 100, 30);
        let row = &rows[table_heads(&rows) + 1];
        assert!(row.contains("1.33") && !row.contains('—'), "{row}");
    }

    /// The pad drawn after a word and a single character: the word in gold.
    #[test]
    fn a_word_is_drawn_in_gold_in_the_pad() {
        let (mut ed, mut ime, mut seat) = opened("天地人和", 4);
        keys(&mut seat, &mut ed, &mut ime, &typed("\n"));
        // A commit of two characters, then one of one.
        let mut pad = seat.lend_pad_for_test();
        pad.insert_committed("天地");
        seat.take_back_pad(&mut ed, pad);
        let mut pad = seat.lend_pad_for_test();
        pad.insert_committed("人");
        seat.take_back_pad(&mut ed, pad);
        let config = Config::default();
        crate::theme::settle(&config, None);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| {
            draw(frame, &seat, &config, Rect::new(0, 0, 100, 30));
        })
        .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = crate::buffer_to_text(&buffer);
        let (y, row) = text.lines().enumerate().find(|(_, l)| l.contains("天地人") && !l.contains('和')).unwrap();
        let x = yumete_cjk::str_width(&row[..row.find('天').unwrap()]) as u16;
        let ink = Palette::of(&config);
        let fg = |dx: u16| buffer.cell((x + dx, y as u16)).map(|c| c.fg);
        let (gold, text) = (Some(ink.gold()), Some(ink.text()));
        assert_eq!((fg(0), fg(2), fg(4)), (gold, gold, text));
    }

    /// The page drawn at `width` by `height`, as a buffer and as text rows.
    fn drawn(seat: &Seat, width: u16, height: u16) -> (ratatui::buffer::Buffer, Vec<String>) {
        let config = Config::default();
        crate::theme::settle(&config, None);
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| {
                draw(frame, seat, &config, Rect::new(0, 0, width, height));
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        let text = crate::buffer_to_text(&buffer);
        (buffer, text.lines().map(str::to_string).collect())
    }

    /// The row of the results table's heads.
    fn table_heads(rows: &[String]) -> usize {
        rows.iter().position(|l| l.contains(&say!("genda.box.results"))).expect("the results panel") + 1
    }

    /// The colour of the first cell of row `y` inside the results panel.
    fn first_cell_ink(buffer: &ratatui::buffer::Buffer, rows: &[String], y: usize) -> Option<ratatui::style::Color> {
        let row = &rows[y];
        let wall = row.find("││").expect("the panel's wall") + "││".len();
        let x = yumete_cjk::str_width(&row[..wall]) as u16 + 1;
        buffer.cell((x, y as u16)).map(|c| c.fg)
    }

    #[test]
    fn the_results_table_lists_the_segments_newest_first_the_latest_in_gold() {
        let (mut ed, mut ime, mut seat) = opened("一二三四五六七八九", 3);
        keys(&mut seat, &mut ed, &mut ime, &typed("\n一二三四十六"));
        let latest = page(&seat).latest.clone().unwrap();
        assert_eq!((latest.segment, latest.errors), (2, 1));
        // The command row has a short line; `y`, after Esc, copies the whole score line.
        assert_eq!(ed.status(), say!("genda.done-typing", 2, latest.speed_text()));
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}y"));
        assert_eq!(ed.take_clipboard_request(), Some(latest.score_line()));
        let (buffer, rows) = drawn(&seat, 100, 30);
        let head = table_heads(&rows);
        let heads = &rows[head];
        let at = |name: &str| heads.find(name).unwrap_or_else(|| panic!("{name} in {heads}"));
        assert!(at(&say!("genda.label.segment")) < at(&say!("genda.label.speed")));
        assert!(at(&say!("genda.label.speed")) < at(&say!("genda.label.kps")));
        let (newer, older) = (&rows[head + 1], &rows[head + 2]);
        assert!(newer.contains("││ 2 ") && newer.contains(&latest.speed_text()), "{newer}");
        assert!(older.contains("││ 1 ") && !older.contains('/'), "{older}");
        // Under the system's input method the key fields are not measured.
        assert!(newer.contains('—'), "{newer}");
        assert!(!rows[head + 3].contains("││ 3 "), "{}", rows[head + 3]);
        let ink = Palette::of(&Config::default());
        assert_eq!(first_cell_ink(&buffer, &rows, head + 1), Some(ink.gold()));
        assert_eq!(first_cell_ink(&buffer, &rows, head + 2), Some(ink.text()));
    }

    #[test]
    fn with_auto_off_or_at_the_end_the_short_line_says_y_alone() {
        let (mut ed, mut ime, mut seat) = opened("一二三", 3);
        keys(&mut seat, &mut ed, &mut ime, &typed("\n一二三"));
        assert!(page(&seat).all_done);
        let latest = page(&seat).latest.clone().unwrap();
        assert_eq!(ed.status(), say!("genda.done", 1, latest.speed_text()));
    }

    #[test]
    fn the_history_read_at_opening_is_dimmed_under_this_sessions_rows() {
        let (mut ed, mut ime, mut seat) = opened("一二三四五六", 3);
        let before = Record { segment: 9, chars: 3, millis: 3000, ..Record::default() };
        if let Some(page) = seat.page.as_mut() {
            page.history.insert(0, before);
            page.earlier = 1;
        }
        let (buffer, rows) = drawn(&seat, 100, 30);
        let ink = Palette::of(&Config::default());
        let head = table_heads(&rows);
        assert!(rows[head + 1].contains("││ 9 "), "{}", rows[head + 1]);
        assert_eq!(first_cell_ink(&buffer, &rows, head + 1), Some(ink.furniture()));
        keys(&mut seat, &mut ed, &mut ime, &typed("\n一二三"));
        let (buffer, rows) = drawn(&seat, 100, 30);
        assert!(rows[head + 1].contains("││ 1 "), "{}", rows[head + 1]);
        assert_eq!(first_cell_ink(&buffer, &rows, head + 1), Some(ink.gold()));
        assert!(rows[head + 2].contains("││ 9 "), "{}", rows[head + 2]);
        assert_eq!(first_cell_ink(&buffer, &rows, head + 2), Some(ink.furniture()));
    }

    #[test]
    fn a_narrow_table_gives_up_the_article_the_clock_and_the_rates_first() {
        let (mut ed, mut ime, mut seat) = opened("一二三四五六", 3);
        keys(&mut seat, &mut ed, &mut ime, &typed("\n一二三"));
        let has = |row: &str, name: &str| row.contains(&yumete_core::messages::say(name, &[]));
        let kept = [
            "genda.label.segment",
            "genda.label.speed",
            "genda.label.kps",
            "genda.label.code-length",
            "genda.label.chars",
            "genda.label.errors",
            "genda.label.elapsed",
        ];
        let (_, wide) = drawn(&seat, 140, 34);
        let heads = &wide[table_heads(&wide)];
        for name in kept.iter().chain(&["genda.label.word-rate", "genda.label.time", "genda.label.article"]) {
            assert!(has(heads, name), "{name}: {heads}");
        }
        // Seventy columns: the left column folds and the table keeps the seven.
        let (_, narrow) = drawn(&seat, 70, 24);
        let heads = &narrow[table_heads(&narrow)];
        for name in kept {
            assert!(has(heads, name), "{name}: {heads}");
        }
        for name in ["genda.label.word-rate", "genda.label.time", "genda.label.article"] {
            assert!(!has(heads, name), "{name}: {heads}");
        }
        // Under 24 rows there is no table.
        let (_, short) = drawn(&seat, 100, 22);
        assert!(!short.iter().any(|l| l.contains(&say!("genda.box.results"))));
        // The order the columns are given up in.
        let widths = [2, 6, 4, 4, 4, 4, 4, 8, 4, 4, 6, 11, 8];
        let shown = |room| fitted(&widths, room).iter().filter(|k| **k).count();
        assert_eq!((shown(200), shown(72), shown(40), shown(1)), (13, 11, 6, 2));
        let at_72 = fitted(&widths, 72);
        assert!(!at_72[11] && !at_72[12] && at_72[10]);
    }

    #[test]
    fn the_foot_leaves_out_a_hint_that_does_not_fit_whole() {
        assert_eq!(whole_hints("F3 重打  C-t 提示", 20), "F3 重打  C-t 提示");
        assert_eq!(whole_hints("F3 重打  C-t 提示", 15), "F3 重打");
        assert_eq!(whole_hints("F3 重打  C-t 提示", 3), "");
    }

    /// A record whose key fields and hands were measured, as under Yume.
    fn measured(segment: usize) -> Record {
        Record {
            segment,
            chars: 3,
            millis: 3000,
            keys: Some(genda::Keys { keys: 9, backspaces: 0, picks: 0 }),
            strokes: Some(genda::Strokes {
                equivalence: 12.0,
                pairs: 8,
                alternating: 5.5,
                same_hand: 2.5,
                top_key: "Space".into(),
                top_count: 3,
            }),
            peak_chars: 3,
            peak_millis: 1500,
            stay_millis: 800,
            stay_text: "二".into(),
            scheme: "靈明".into(),
            ..Record::default()
        }
    }

    #[test]
    fn the_picked_history_row_is_inked_across() {
        let (mut ed, mut ime, mut seat) = opened("一二三四五六", 3);
        if let Some(page) = seat.page.as_mut() {
            page.history = vec![measured(1), measured(2), measured(3)];
        }
        keys(&mut seat, &mut ed, &mut ime, &[Key::Tab, Key::Char('j')]);
        let (buffer, rows) = drawn(&seat, 100, 30);
        let ink = Palette::of(&Config::default());
        // The ring's top, the heads, then the rows newest first: 3, 2, 1.
        let (other, picked) = (2, 3);
        assert!(rows[1].contains(&say!("genda.label.segment")), "{}", rows[1]);
        // From wall to wall: the first cell and the last inside the ring.
        for x in [1, 98] {
            let cell = buffer.cell((x, picked as u16)).unwrap();
            assert_eq!((cell.bg, cell.fg), (ink.text(), crate::chrome::panel_ground(ink)), "x {x}");
        }
        assert_eq!(buffer.cell((1, other as u16)).unwrap().bg, crate::chrome::panel_ground(ink));
    }

    #[test]
    fn copying_carries_the_details_the_table_has_no_column_for() {
        let (mut ed, mut ime, mut seat) = opened("一二三四五六", 3);
        let record = measured(1);
        if let Some(page) = seat.page.as_mut() {
            page.history = vec![record.clone()];
        }
        keys(&mut seat, &mut ed, &mut ime, &[Key::Tab, Key::Char('y')]);
        let line = ed.take_clipboard_request().unwrap();
        assert_eq!(line, record.score_line());
        for (label, value) in record.details() {
            assert!(line.contains(&value), "{label} {value}: {line}");
        }
        assert!(line.contains(&say!("genda.line.top-key", "Space×3")), "{line}");
        keys(&mut seat, &mut ed, &mut ime, &[Key::Char('Y')]);
        let table = ed.take_clipboard_request().unwrap();
        let (heads, row) = (table.lines().next().unwrap(), table.lines().nth(1).unwrap());
        for (label, value) in record.details() {
            let at = heads.split('\t').position(|h| h == label).unwrap();
            assert_eq!(row.split('\t').nth(at), Some(value.as_str()), "{label}");
        }
    }

    #[test]
    fn c_o_and_genda_whole_flip_between_the_length_and_the_whole_article() {
        let (mut ed, mut ime, mut seat) = opened("一二三四五六七八九十", 3);
        keys(&mut seat, &mut ed, &mut ime, &typed("\n"));
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('p')]);
        assert_eq!(page(&seat).index, 1);
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('o')]);
        assert_eq!((page(&seat).length, page(&seat).index, page(&seat).count()), (None, 0, 1));
        assert_eq!(page(&seat).run.target().len(), 10);
        assert_eq!(ed.status(), say!("genda.length-all"));
        assert!(seat.typing());
        // Back to the length chosen, from the first segment.
        keys(&mut seat, &mut ed, &mut ime, &typed("一二"));
        keys(&mut seat, &mut ed, &mut ime, &[Key::Ctrl('o')]);
        assert_eq!((page(&seat).length, page(&seat).index), (Some(3), 0));
        assert_eq!(page(&seat).run.target(), &['一', '二', '三']);
        assert_eq!(ed.status(), say!("genda.length-set", 3));
        // The command, and `:genda-length` choosing what off goes back to.
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "genda-whole on");
        assert_eq!(page(&seat).length, None);
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "genda-whole on");
        assert_eq!(page(&seat).length, None);
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "genda-length 4");
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "genda-length all");
        keys(&mut seat, &mut ed, &mut ime, &typed("\u{1b}"));
        command(&mut seat, &mut ed, &mut ime, "genda-whole");
        assert_eq!(page(&seat).length, Some(4));
    }

    impl Seat {
        /// The pad, lent as for a key the built-in IME held, so what is put
        /// in it before it comes back is one commit.
        fn lend_pad_for_test(&mut self) -> Box<Editor> {
            let at = self.clock();
            let stroke = Stroke { yume: true, ..Stroke::default() };
            self.pending = Some(Pending { stroke, first: String::new(), at });
            self.pad.take().expect("the pad")
        }
    }
}
