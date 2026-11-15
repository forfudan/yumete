//! **跟打** (`:genda`): the typing trainer's model, with no screen and no clock
//! of its own.
//!
//! An article is cut into segments; one attempt at a segment is a [`Run`],
//! fed the keys that reached the pad and the committed text after each, and
//! it ends by the community rule ([`Run::finished`]). What is kept is a
//! [`Record`] of raw counts, one JSON line each; every rate is derived on
//! reading, so a formula changed later reads the old lines the new way.
//!
//! Time is handed in by the caller as a [`Duration`] since any fixed moment,
//! so a test drives a fake clock and the front end the real one.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::say;

/// The segment lengths offered, in characters; `None` beside them is 全文.
pub const LENGTHS: [usize; 7] = [10, 15, 25, 50, 100, 200, 500];

/// The length a page opens with.
pub const DEFAULT_LENGTH: usize = 25;

/// **Characters that arrive closer together than this are one commit**, under
/// an input method whose commits the editor cannot see: a hand types one
/// character in no less than about 30 ms, a terminal writes a committed word
/// in one go.
pub const BURST: Duration = Duration::from_millis(15);

/// What a `:genda-…` command asks of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// `:genda` — open on the current buffer.
    Open,
    Next,
    Previous,
    Retry,
    Shuffle,
    /// The latest (or selected) record's score line to the clipboard.
    Copy,
    /// The whole history to the clipboard, as TSV.
    CopyHistory,
    /// The history view, or back to the typing.
    History,
    /// A segment length; `None` is 全文.
    Length(Option<usize>),
    /// `:genda-whole`: the whole article as one segment, or back to the
    /// length chosen.
    Whole(crate::command::Switch),
    /// Another article.
    Article(Choice),
    /// An article of the `N` most frequent 漢字 in a scope (`:genda-top`
    /// and its scopes); the core counts them and hands the article over.
    Top(crate::search_panel::Where, usize),
    /// `:genda-hint`: the next character's code beside the article.
    Hint(crate::command::Switch),
    /// `:genda-auto`: a finished segment brings up the next by itself.
    Auto(crate::command::Switch),
    /// One criterion of the goal, in hundredths; `0` takes it off.
    GoalSpeed(u32),
    GoalKps(u32),
    GoalAccuracy(u32),
    /// `:genda-goal-clean`: no errors.
    GoalClean(crate::command::Switch),
    /// `:genda-goal off`, every criterion off.
    GoalOff,
    /// `:genda-goal` with nothing after it: say the goal.
    GoalShow,
    /// A segment of the characters the last one got wrong.
    RetryWrong,
    /// A segment of the last one's slowest characters ([`SLOW`]).
    RetrySlow,
    /// `:q` while the page is open.
    Close,
}

/// **The page's switches, as the `:` menu says them**: the page holds them
/// and tells the core, which answers for every switch under `:`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Switches {
    pub hint: bool,
    pub auto: bool,
    pub clean: bool,
    pub whole: bool,
}

impl Default for Switches {
    fn default() -> Self {
        Switches { hint: false, auto: true, clean: false, whole: false }
    }
}

/// Which article `:genda-article` names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// The selection, or the whole buffer.
    Buffer,
    /// One band of 500 of Yume's most frequent characters: 0, 1 or 2.
    Common(usize),
}

/// The words `:genda-article` takes, in the order of [`Choice`].
pub const ARTICLE_WORDS: [&str; 4] = ["buffer", "chars-1-500", "chars-501-1000", "chars-1001-1500"];

impl Choice {
    /// The choice a word of [`ARTICLE_WORDS`] names.
    pub fn from_word(word: &str) -> Option<Choice> {
        match ARTICLE_WORDS.iter().position(|w| *w == word)? {
            0 => Some(Choice::Buffer),
            band => Some(Choice::Common(band - 1)),
        }
    }

    /// What the page calls a band of common characters.
    pub fn band_name(band: usize) -> String {
        match band {
            0 => say!("genda.article.common-1"),
            1 => say!("genda.article.common-2"),
            _ => say!("genda.article.common-3"),
        }
    }
}

/// `:genda-top` without a number.
pub const DEFAULT_TOP: usize = 500;

/// **A 漢字 that `:genda-top` counts**: the unified ideographs and every
/// extension block from A to J, block by block, and 〇. Not the
/// compatibility ideographs (each is another code point for a character
/// already counted) and not the radicals (parts, not characters).
pub fn counts_as_han(c: char) -> bool {
    matches!(c as u32,
        0x3007                  // 〇
        | 0x3400..=0x4DBF       // Ext A
        | 0x4E00..=0x9FFF       // unified
        | 0x20000..=0x2A6DF     // Ext B
        | 0x2A700..=0x2EE5F     // Ext C D E F I, one after another
        | 0x30000..=0x3347F     // Ext G H J
    )
}

/// **The `n` most frequent 漢字 in `text`**, most first; a tie goes to the
/// lower code point, so the same text always gives the same article.
pub fn top_chars(text: &str, n: usize) -> Vec<char> {
    let mut counts: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
    for c in text.chars().filter(|&c| counts_as_han(c)) {
        *counts.entry(c).or_default() += 1;
    }
    let mut ranked: Vec<(char, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    ranked.into_iter().take(n).map(|(c, _)| c).collect()
}

/// The text to type: line breaks and tabs taken out, everything else kept.
pub fn strip(text: &str) -> Vec<char> {
    text.chars().filter(|c| !matches!(c, '\n' | '\r' | '\t')).collect()
}

/// How many segments `total` characters make at `length` (`None`: 全文).
pub fn segment_count(total: usize, length: Option<usize>) -> usize {
    match length {
        _ if total == 0 => 0,
        None => 1,
        Some(n) => total.div_ceil(n.max(1)),
    }
}

/// The characters of segment `index` (from 0); the last one may be short.
pub fn segment(total: usize, length: Option<usize>, index: usize) -> std::ops::Range<usize> {
    let n = length.unwrap_or(total).max(1);
    let start = (index * n).min(total);
    start..(start + n).min(total)
}

/// **The article in another order**, the same for the same seed: a
/// Fisher–Yates walk over a xorshift stream, so a test can name the order.
pub fn shuffled(chars: &[char], seed: u64) -> Vec<char> {
    let mut out = chars.to_vec();
    let mut state = seed | 1;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for i in (1..out.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        out.swap(i, j);
    }
    out
}

/// One key that reached the pad, as the front end read it before handing it on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stroke {
    /// The built-in IME held the keys: every key passes through the loop
    /// before it, so the key fields can be measured.
    pub yume: bool,
    /// A code was being composed when the key came.
    pub composing: bool,
    /// The key was Backspace.
    pub backspace: bool,
    /// Which key, for the 當量 and the hands; `None` for one they do not
    /// count.
    pub key: Option<Pressed>,
}

/// **A key as the 當量 and the hands count it**, by the names typepad's key
/// heat map gives them: a letter is one key in either case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Pressed {
    /// A printing key; a letter in upper case.
    Char(char),
    Space,
    Backspace,
    Enter,
    Tab,
    /// A lone Shift tap: the terminal does not say which side, so it belongs
    /// to neither hand.
    Shift,
}

/// The keys of the left hand, Tab aside: the rows from `Q`, `A`, `Z` and
/// `1` to the fifth column, and the shifted symbols over them.
const LEFT_HAND: &str = "QWERTASDFGZXCVB12345`~!@#$%";

/// The keys of the right hand, Backspace and Enter aside. 6 is on the right,
/// as on a standard board.
const RIGHT_HAND: &str = "YUIOPHJKLNM67890-=[]\\;',./^&*()_+{}|:\"<>?";

impl Pressed {
    /// The key a [`crate::input::Key`] is, when it is one the pad counts.
    pub fn of(key: crate::input::Key) -> Option<Pressed> {
        use crate::input::Key;
        match key {
            Key::Char(' ') => Some(Pressed::Space),
            Key::Char(c) => Some(Pressed::Char(c.to_uppercase().next().unwrap_or(c))),
            Key::Backspace => Some(Pressed::Backspace),
            Key::Enter => Some(Pressed::Enter),
            Key::Tab => Some(Pressed::Tab),
            _ => None,
        }
    }

    /// Its name, as the history shows the most used key.
    pub fn name(self) -> String {
        match self {
            Pressed::Char(c) => c.to_string(),
            Pressed::Space => "Space".into(),
            Pressed::Backspace => "Backspace".into(),
            Pressed::Enter => "Enter".into(),
            Pressed::Tab => "Tab".into(),
            Pressed::Shift => "Shift".into(),
        }
    }

    /// `Some(true)` for the left hand, `Some(false)` for the right, `None`
    /// for a key of neither (Space, Shift).
    fn left_hand(self) -> Option<bool> {
        match self {
            Pressed::Tab => Some(true),
            Pressed::Backspace | Pressed::Enter => Some(false),
            Pressed::Char(c) if LEFT_HAND.contains(c) => Some(true),
            Pressed::Char(c) if RIGHT_HAND.contains(c) => Some(false),
            _ => None,
        }
    }

    /// What the 當量 table calls it: a letter, `;`, or `' '` for Space.
    fn in_table(self) -> Option<char> {
        match self {
            Pressed::Space => Some(' '),
            Pressed::Char(c) if c.is_ascii_alphabetic() || c == ';' => Some(c),
            _ => None,
        }
    }
}

/// **What a run's keys come to by the 當量 table and by the hands**, as
/// typepad's key heat map counts them, kept as sums so every rate is
/// derived on reading.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Strokes {
    /// Σ 當量 over the adjacent pairs the table has.
    pub equivalence: f64,
    /// Those pairs: 有效按鍵組合.
    pub pairs: usize,
    /// 互擊 and 同手 over the adjacent pairs; a pair with Space counts half
    /// to each, and a pair with a key of neither hand to none.
    pub alternating: f64,
    pub same_hand: f64,
    /// 最常用按鍵 and how often it was struck.
    pub top_key: String,
    pub top_count: usize,
}

impl Strokes {
    /// **Count a run of keys**, pair by adjacent pair.
    pub fn count(keys: &[Pressed]) -> Strokes {
        let mut out = Strokes::default();
        for pair in keys.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if let Some(value) = a.in_table().zip(b.in_table()).and_then(|(x, y)| crate::equivalence::of(x, y)) {
                out.equivalence += value;
                out.pairs += 1;
            }
            if a == Pressed::Space || b == Pressed::Space {
                out.alternating += 0.5;
                out.same_hand += 0.5;
                continue;
            }
            match (a.left_hand(), b.left_hand()) {
                (Some(x), Some(y)) if x != y => out.alternating += 1.0,
                (Some(_), Some(_)) => out.same_hand += 1.0,
                _ => {}
            }
        }
        let mut counts: std::collections::BTreeMap<Pressed, usize> = std::collections::BTreeMap::new();
        for &key in keys {
            *counts.entry(key).or_default() += 1;
        }
        // The first of the most struck in key order, so a tie always names the same key.
        if let Some((key, n)) = counts.into_iter().fold(None::<(Pressed, usize)>, |best, (k, n)| match best {
            Some((_, m)) if m >= n => best,
            _ => Some((k, n)),
        }) {
            out.top_key = key.name();
            out.top_count = n;
        }
        out
    }

    /// 按鍵組合平均當量: the mean over the pairs the table has.
    pub fn mean(&self) -> f64 {
        match self.pairs {
            0 => 0.0,
            n => self.equivalence / n as f64,
        }
    }

    /// 左右手互擊率 in percent.
    pub fn alternation_rate(&self) -> f64 {
        let all = self.alternating + self.same_hand;
        match all > 0.0 {
            true => self.alternating / all * 100.0,
            false => 0.0,
        }
    }
}

/// **峰值 is the best stretch of at least this many characters**, from the
/// end of one commit to the end of another: long enough that one quick word
/// is not a peak, short enough to find a fast run inside a 25-character
/// segment. A segment no longer than this has the whole segment for its peak.
pub const PEAK: usize = 10;

/// How many characters `:genda-retry-slow` takes at most.
pub const SLOW: usize = 10;

/// **A goal a segment must reach before the page moves on**; nothing set is
/// no goal.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Goal {
    /// 速度 at least this, after 錯一罰五.
    pub speed: Option<f64>,
    /// 擊鍵 at least this.
    pub kps: Option<f64>,
    /// 鍵準 at least this, in percent.
    pub accuracy: Option<f64>,
    /// No errors.
    pub clean: bool,
}

impl Goal {
    pub fn is_set(&self) -> bool {
        self.speed.is_some() || self.kps.is_some() || self.accuracy.is_some() || self.clean
    }

    /// **The criteria `record` falls short of**, as the goal reads them;
    /// empty when it is met. A key field the record could not measure
    /// (the system's input method) is passed over, not failed: a goal
    /// nobody could reach under that input method would hold the page on
    /// one segment for good.
    pub fn missed(&self, record: &Record) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(want) = self.speed.filter(|&want| record.penalised() < want) {
            out.push(say!("genda.goal.speed", number(want)));
        }
        if let (Some(want), Some(kps)) = (self.kps, record.keys_per_second()) {
            if kps < want {
                out.push(say!("genda.goal.kps", number(want)));
            }
        }
        if let (Some(want), Some(accuracy)) = (self.accuracy, record.key_accuracy()) {
            if accuracy < want {
                out.push(say!("genda.goal.accuracy", number(want)));
            }
        }
        if self.clean && record.errors > 0 {
            out.push(say!("genda.goal.clean"));
        }
        out
    }

    /// The goal as the 文章 box and the status line say it.
    pub fn text(&self) -> String {
        let mut parts = Vec::new();
        if let Some(v) = self.speed {
            parts.push(say!("genda.goal.speed", number(v)));
        }
        if let Some(v) = self.kps {
            parts.push(say!("genda.goal.kps", number(v)));
        }
        if let Some(v) = self.accuracy {
            parts.push(say!("genda.goal.accuracy", number(v)));
        }
        if self.clean {
            parts.push(say!("genda.goal.clean"));
        }
        match parts.is_empty() {
            true => say!("genda.goal.none"),
            false => parts.join(" "),
        }
    }
}

/// A goal's value as it was typed: `120`, `6.5`.
fn number(v: f64) -> String {
    let text = format!("{v:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// A hundredths value of [`Ask`], as a goal holds it; `0` is none.
pub fn from_hundredths(v: u32) -> Option<f64> {
    (v > 0).then(|| v as f64 / 100.0)
}

/// **A goal's value as typed**, in hundredths: `120` → 12000, `off` or `0` → 0.
pub fn hundredths(text: &str) -> Option<u32> {
    if text == "off" {
        return Some(0);
    }
    let v: f64 = text.parse().ok()?;
    (0.0..1e6).contains(&v).then(|| (v * 100.0).round() as u32)
}

/// One commit group still open, for 打詞率.
#[derive(Debug, Clone, Copy, Default)]
struct Group {
    /// 漢字 in it.
    han: usize,
    /// Characters in it that are not punctuation.
    words: usize,
    /// How many of its 漢字 are already counted as typed in words.
    counted: usize,
}

/// **One attempt at one segment.**
#[derive(Debug, Clone, Default)]
pub struct Run {
    target: Vec<char>,
    typed: Vec<char>,
    /// The clock when it last started running; `None` while it stands.
    running_since: Option<Duration>,
    /// Time run before the last pause.
    banked: Duration,
    started: bool,
    /// The elapsed time and the errors, once the segment is done.
    done: Option<(Duration, usize)>,
    keys: usize,
    backspaces: usize,
    picks: usize,
    corrections: usize,
    /// Every key so far came through the built-in IME.
    measured: bool,
    han: usize,
    han_in_words: usize,
    group: Group,
    last_commit: Option<Duration>,
    /// The keys, in order, while every one came through the built-in IME.
    pressed: Vec<Pressed>,
    /// For each typed position: the run's clock when it was committed, and
    /// which commit it came in (one burst is one commit).
    stamps: Vec<(Duration, u32)>,
    commits: u32,
    /// The run's clock at the last change to the text.
    last_change: Duration,
    /// 停留: the longest wait before a commit, the commit it was, and what
    /// came in it.
    stay: Option<(Duration, u32, String)>,
    /// Positions ever committed wrong, put right later or not.
    missed: std::collections::BTreeSet<usize>,
}

/// What a finished run counted, before anything is divided.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keys {
    /// Every key that reached the pad.
    pub keys: usize,
    /// Backspaces while a code was composed (退格).
    pub backspaces: usize,
    /// Commits that were not the first candidate (選重).
    pub picks: usize,
}

impl Run {
    /// A fresh attempt at `target`.
    pub fn new(target: Vec<char>) -> Run {
        Run { target, measured: true, ..Run::default() }
    }

    pub fn target(&self) -> &[char] {
        &self.target
    }

    /// The committed text as the run reads it.
    pub fn typed(&self) -> &[char] {
        &self.typed
    }

    /// The clock has started: a key reached the pad.
    pub fn started(&self) -> bool {
        self.started
    }

    pub fn finished(&self) -> bool {
        self.done.is_some()
    }

    /// The clock stands: paused, or not started, or done.
    pub fn paused(&self) -> bool {
        self.running_since.is_none()
    }

    /// Time on the clock at `now`.
    pub fn elapsed(&self, now: Duration) -> Duration {
        if let Some((took, _)) = self.done {
            return took;
        }
        self.banked + self.running_since.map_or(Duration::ZERO, |since| now.saturating_sub(since))
    }

    /// Stop the clock (Esc).
    pub fn pause(&mut self, now: Duration) {
        if let Some(since) = self.running_since.take() {
            self.banked += now.saturating_sub(since);
        }
    }

    /// Start it again (Enter); the next key would do the same.
    pub fn resume(&mut self, now: Duration) {
        if self.started && self.done.is_none() && self.running_since.is_none() {
            self.running_since = Some(now);
        }
    }

    /// **A key reached the pad**: the clock starts on the first, and the key
    /// fields count it while the built-in IME holds every key.
    pub fn key(&mut self, stroke: Stroke, now: Duration) {
        if self.done.is_some() {
            return;
        }
        self.started = true;
        if self.running_since.is_none() {
            self.running_since = Some(now);
        }
        self.measured &= stroke.yume;
        self.keys += 1;
        if stroke.backspace && stroke.composing {
            self.backspaces += 1;
        }
        if let Some(key) = stroke.key.filter(|_| self.measured) {
            self.pressed.push(key);
        }
    }

    /// The key just counted committed a candidate that was not the first.
    pub fn picked(&mut self) {
        self.picks += 1;
    }

    /// **The committed text after a key.** What was taken off the end by a
    /// Backspace outside a composition is 回改; what was added is a commit,
    /// joined to the one before when it came in a burst under an input
    /// method the editor cannot see. Gives whether the segment is done now.
    pub fn text_now(&mut self, text: &str, stroke: Stroke, now: Duration) -> bool {
        if self.done.is_some() {
            return true;
        }
        let new = strip(text);
        let same = self.typed.iter().zip(&new).take_while(|(a, b)| a == b).count();
        let gone = self.typed.len() - same;
        if gone > 0 && stroke.backspace && !stroke.composing {
            self.corrections += gone;
        }
        let clock = self.elapsed(now);
        self.stamps.truncate(same);
        let added = &new[same..];
        if !added.is_empty() {
            let join = !stroke.yume && gone == 0 && self.last_commit.is_some_and(|at| now.saturating_sub(at) < BURST);
            self.commit(added, join);
            self.last_commit = Some(now);
            if !join {
                self.commits += 1;
            }
            let id = self.commits;
            for (k, c) in added.iter().enumerate() {
                self.stamps.push((clock, id));
                if self.target.get(same + k) != Some(c) {
                    self.missed.insert(same + k);
                }
            }
            let wait = clock.saturating_sub(self.last_change);
            match &mut self.stay {
                Some((_, at, text)) if join && *at == id => text.extend(added),
                Some((longest, ..)) if join || wait <= *longest => {}
                stay => *stay = Some((wait, id, added.iter().collect())),
            }
        }
        if gone > 0 || !added.is_empty() {
            self.last_change = clock;
        }
        self.typed = new;
        self.settle(now)
    }

    fn commit(&mut self, added: &[char], join: bool) {
        if !join {
            self.group = Group::default();
        }
        for &c in added {
            if yumete_cjk::is_han(c) {
                self.han += 1;
                self.group.han += 1;
            }
            if c.is_alphanumeric() {
                self.group.words += 1;
            }
        }
        if self.group.words >= 2 {
            self.han_in_words += self.group.han - self.group.counted;
            self.group.counted = self.group.han;
        }
    }

    /// **The finish rule**: as long as the segment, and its last character
    /// typed right. Errors stand; what runs past the end is dropped.
    fn settle(&mut self, now: Duration) -> bool {
        let n = self.target.len();
        if n == 0 || self.typed.len() < n || self.typed[n - 1] != self.target[n - 1] {
            return false;
        }
        self.typed.truncate(n);
        self.stamps.truncate(n);
        let took = self.elapsed(now);
        self.running_since = None;
        self.done = Some((took, self.errors()));
        true
    }

    /// Positions typed and not matching the article.
    pub fn errors(&self) -> usize {
        self.typed.iter().zip(&self.target).filter(|(a, b)| a != b).count()
    }

    /// What each position of the segment shows.
    pub fn mark(&self, i: usize) -> Mark {
        match self.typed.get(i) {
            Some(c) if Some(c) == self.target.get(i) => Mark::Right,
            Some(_) => Mark::Wrong,
            None if i == self.typed.len() => Mark::Next,
            None => Mark::Untyped,
        }
    }

    /// **Which typed positions came in a word**: a 漢字 in a commit of two
    /// or more characters that are not punctuation, the same commits
    /// 打詞率 counts. Under an input method the editor cannot see, a commit
    /// is a burst of characters ([`BURST`]), so the marks are as near as
    /// that guess.
    pub fn word_marks(&self) -> Vec<bool> {
        let mut words: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
        for (c, (_, id)) in self.typed.iter().zip(&self.stamps) {
            if c.is_alphanumeric() {
                *words.entry(*id).or_default() += 1;
            }
        }
        self.typed
            .iter()
            .zip(&self.stamps)
            .map(|(&c, (_, id))| yumete_cjk::is_han(c) && words.get(id).is_some_and(|&n| n >= 2))
            .collect()
    }

    /// The commits of the typed text, in order: each one's first position,
    /// its length and the clock at its end.
    fn commit_spans(&self) -> Vec<(usize, usize, Duration)> {
        let mut spans: Vec<(usize, usize, Duration)> = Vec::new();
        for (i, &(at, id)) in self.stamps.iter().enumerate() {
            match spans.last_mut() {
                Some((start, len, end)) if self.stamps[*start].1 == id => {
                    *len += 1;
                    *end = at;
                }
                _ => spans.push((i, 1, at)),
            }
        }
        spans
    }

    /// **峰值**: the best stretch of at least [`PEAK`] characters, from the
    /// end of one commit (or the first key) to the end of another, as
    /// characters and time; the whole run when it is no longer than that.
    fn peak(&self) -> (usize, Duration) {
        let spans = self.commit_spans();
        let n = self.stamps.len();
        let whole = (n, self.stamps.last().map_or(Duration::ZERO, |s| s.0));
        if n <= PEAK {
            return whole;
        }
        let mut best: Option<(usize, Duration)> = None;
        for first in 0..spans.len() {
            let from = match first {
                0 => Duration::ZERO,
                _ => spans[first - 1].2,
            };
            let mut chars = 0;
            for &(_, len, end) in &spans[first..] {
                chars += len;
                if chars < PEAK {
                    continue;
                }
                let took = end.saturating_sub(from);
                let better = best.is_none_or(|(c, t)| chars as u128 * t.as_micros() > c as u128 * took.as_micros());
                if !took.is_zero() && better {
                    best = Some((chars, took));
                }
                break;
            }
        }
        best.unwrap_or(whole)
    }

    /// **The run's slowest characters**, at most `n` and each once, slowest
    /// first: a commit's wait shared out among the characters it brought,
    /// so a word typed slowly is not counted as that many slow characters.
    /// The article's characters, not what was typed in their place; spaces
    /// are not drilled.
    pub fn slowest(&self, n: usize) -> Vec<char> {
        let mut costs: Vec<(Duration, usize)> = Vec::new();
        let mut before = Duration::ZERO;
        for (start, len, end) in self.commit_spans() {
            let each = end.saturating_sub(before) / len as u32;
            costs.extend((start..start + len).map(|i| (each, i)));
            before = end;
        }
        costs.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        let mut out: Vec<char> = Vec::new();
        for (_, i) in costs {
            match self.target.get(i) {
                Some(&c) if !c.is_whitespace() && !out.contains(&c) => out.push(c),
                _ => {}
            }
            if out.len() == n {
                break;
            }
        }
        out
    }

    /// **The characters the run got wrong**, each once, in the article's
    /// order: every position ever committed wrong, put right after or not.
    pub fn missed(&self) -> Vec<char> {
        let mut out: Vec<char> = Vec::new();
        for &i in &self.missed {
            match self.target.get(i) {
                Some(&c) if !c.is_whitespace() && !out.contains(&c) => out.push(c),
                _ => {}
            }
        }
        out
    }

    /// The counts so far, as a record would keep them; `at` is the clock.
    pub fn record(&self, now: Duration) -> Record {
        let chars = match self.done {
            Some(_) => self.target.len(),
            None => self.typed.len(),
        };
        let (peak_chars, peak_took) = self.peak();
        let (stay, stay_text) = self.stay.as_ref().map_or((Duration::ZERO, String::new()), |(wait, _, text)| (*wait, text.clone()));
        Record {
            peak_chars,
            peak_millis: peak_took.as_millis() as u64,
            stay_millis: stay.as_millis() as u64,
            stay_text,
            strokes: (self.measured && self.started).then(|| Strokes::count(&self.pressed)),
            chars,
            errors: self.errors(),
            millis: self.elapsed(now).as_millis() as u64,
            corrections: self.corrections,
            han: self.han,
            han_in_words: self.han_in_words,
            keys: (self.measured && self.started).then_some(Keys {
                keys: self.keys,
                backspaces: self.backspaces,
                picks: self.picks,
            }),
            ..Record::default()
        }
    }
}

/// What one position of a segment shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    Right,
    Wrong,
    /// The one to type next.
    Next,
    Untyped,
}

/// **One finished segment, as kept**: raw counts only.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Record {
    /// Seconds since the Unix epoch.
    #[serde(default)]
    pub at: u64,
    /// 第幾段, from 1.
    #[serde(default)]
    pub segment: usize,
    /// 字數: the segment's length.
    pub chars: usize,
    pub errors: usize,
    pub millis: u64,
    /// 回改: characters a Backspace took back outside a composition.
    pub corrections: usize,
    /// 重打: attempts at this segment before this one.
    #[serde(default)]
    pub retry: usize,
    /// 漢字 committed, and those committed in words of two or more.
    #[serde(default)]
    pub han: usize,
    #[serde(default)]
    pub han_in_words: usize,
    /// Only with the built-in IME holding every key.
    #[serde(default)]
    pub keys: Option<Keys>,
    /// The 當量 and the hands, likewise.
    #[serde(default)]
    pub strokes: Option<Strokes>,
    /// 峰值: the characters and the time of the best stretch ([`PEAK`]).
    #[serde(default)]
    pub peak_chars: usize,
    #[serde(default)]
    pub peak_millis: u64,
    /// 停留: the longest wait before a commit, and what that commit brought.
    #[serde(default)]
    pub stay_millis: u64,
    #[serde(default)]
    pub stay_text: String,
    /// The scheme's name; empty under the system's input method.
    #[serde(default)]
    pub scheme: String,
    #[serde(default)]
    pub article: String,
    #[serde(default)]
    pub version: String,
}

impl Record {
    fn seconds(&self) -> f64 {
        self.millis as f64 / 1000.0
    }

    /// 速度, characters a minute.
    pub fn speed(&self) -> f64 {
        per_minute(self.chars as f64, self.seconds())
    }

    /// 速度 after 錯一罰五, never below nought.
    pub fn penalised(&self) -> f64 {
        let left = self.chars as f64 - 5.0 * self.errors as f64;
        per_minute(left.max(0.0), self.seconds())
    }

    /// 擊鍵, keys a second.
    pub fn keys_per_second(&self) -> Option<f64> {
        let keys = self.keys?;
        Some(match self.seconds() > 0.0 {
            true => keys.keys as f64 / self.seconds(),
            false => 0.0,
        })
    }

    /// 碼長, keys a character.
    pub fn code_length(&self) -> Option<f64> {
        let keys = self.keys?;
        Some(match self.chars {
            0 => 0.0,
            n => keys.keys as f64 / n as f64,
        })
    }

    /// 鍵準 in percent: (keys − 退格 − 回改 × 碼長) / keys.
    pub fn key_accuracy(&self) -> Option<f64> {
        let keys = self.keys?;
        if keys.keys == 0 {
            return Some(0.0);
        }
        let lost = keys.backspaces as f64 + self.corrections as f64 * self.code_length()?;
        Some(((keys.keys as f64 - lost) / keys.keys as f64 * 100.0).max(0.0))
    }

    /// 按鍵組合平均當量, only with the built-in IME holding every key.
    pub fn equivalence(&self) -> Option<f64> {
        self.strokes.as_ref().map(Strokes::mean)
    }

    /// 峰值, characters a minute over the best stretch.
    pub fn peak(&self) -> f64 {
        per_minute(self.peak_chars as f64, self.peak_millis as f64 / 1000.0)
    }

    /// 停留 as the score line writes it, `[字]1.23s`; empty before any commit.
    pub fn stay_text(&self) -> String {
        match self.stay_text.is_empty() {
            true => String::new(),
            false => format!("[{}]{:.2}s", self.stay_text, self.stay_millis as f64 / 1000.0),
        }
    }

    /// 打詞率 in percent.
    pub fn word_rate(&self) -> f64 {
        match self.han {
            0 => 0.0,
            n => self.han_in_words as f64 / n as f64 * 100.0,
        }
    }

    /// The speed as the score line and the panel show it: `罰後/原始` when
    /// there are errors.
    pub fn speed_text(&self) -> String {
        match self.errors {
            0 => format!("{:.2}", self.speed()),
            _ => format!("{:.2}/{:.2}", self.penalised(), self.speed()),
        }
    }

    /// The scheme's name, or the system's input method.
    pub fn ime_name(&self) -> String {
        match self.scheme.is_empty() {
            true => say!("genda.system-ime"),
            false => self.scheme.clone(),
        }
    }

    /// **The score line**, in the order the typing community pastes them; the
    /// key fields only when they were measured.
    pub fn score_line(&self) -> String {
        let mut parts = vec![say!("genda.line.segment", self.segment), say!("genda.line.speed", self.speed_text())];
        if let (Some(kps), Some(len)) = (self.keys_per_second(), self.code_length()) {
            parts.push(say!("genda.line.kps", format!("{kps:.2}")));
            parts.push(say!("genda.line.code-length", format!("{len:.2}")));
        }
        if let Some(equivalence) = self.equivalence() {
            parts.push(say!("genda.line.equivalence", format!("{equivalence:.2}")));
        }
        parts.push(say!("genda.line.chars", self.chars));
        parts.push(say!("genda.line.errors", self.errors));
        parts.push(say!("genda.line.time", clock_text(self.millis)));
        if self.peak_chars > 0 {
            parts.push(say!("genda.line.peak", format!("{:.2}", self.peak())));
        }
        if !self.stay_text.is_empty() {
            parts.push(say!("genda.line.stay", self.stay_text()));
        }
        parts.push(say!("genda.line.corrections", self.corrections));
        if let Some(accuracy) = self.key_accuracy() {
            parts.push(say!("genda.line.key-accuracy", format!("{accuracy:.2}")));
        }
        parts.push(say!("genda.line.word-rate", format!("{:.2}", self.word_rate())));
        if let Some(keys) = self.keys {
            parts.push(say!("genda.line.picks", keys.picks));
        }
        parts.push(match self.retry {
            0 => say!("genda.line.first"),
            n => say!("genda.line.retry", n),
        });
        // The details no community field holds, before the signature: a
        // pasted line carries all the history shows of the segment.
        if let Some(s) = &self.strokes {
            parts.push(say!("genda.line.pairs", s.pairs));
            parts.push(say!("genda.line.alternation-rate", format!("{:.2}", s.alternation_rate())));
            parts.push(say!("genda.line.alternating", half(s.alternating)));
            parts.push(say!("genda.line.same-hand", half(s.same_hand)));
            if s.top_count > 0 {
                parts.push(say!("genda.line.top-key", format!("{}×{}", s.top_key, s.top_count)));
            }
        }
        parts.push(say!("genda.line.ime", self.ime_name()));
        parts.push(format!("yumete v{}", self.version));
        parts.join(" ")
    }

    /// One row of the history, column for column with [`columns`]; a key
    /// field not measured is `—`.
    pub fn cells(&self) -> Vec<String> {
        let or_dash = |v: Option<f64>, f: fn(f64) -> String| v.map_or_else(|| "—".to_string(), f);
        let strokes = self.strokes.as_ref();
        let stroked = |f: &dyn Fn(&Strokes) -> String| strokes.map_or_else(|| "—".to_string(), f);
        vec![
            crate::clock::when(std::time::UNIX_EPOCH + Duration::from_secs(self.at)),
            self.segment.to_string(),
            self.speed_text(),
            or_dash(self.keys_per_second(), |v| format!("{v:.2}")),
            or_dash(self.code_length(), |v| format!("{v:.2}")),
            or_dash(self.equivalence(), |v| format!("{v:.2}")),
            self.chars.to_string(),
            self.errors.to_string(),
            clock_text(self.millis),
            format!("{:.2}", self.peak()),
            match self.stay_text.is_empty() {
                true => "—".to_string(),
                false => self.stay_text(),
            },
            self.corrections.to_string(),
            or_dash(self.key_accuracy(), |v| format!("{v:.2}%")),
            format!("{:.2}%", self.word_rate()),
            self.retry.to_string(),
            self.article.clone(),
            stroked(&|s| s.pairs.to_string()),
            stroked(&|s| format!("{:.2}%", s.alternation_rate())),
            stroked(&|s| half(s.alternating)),
            stroked(&|s| half(s.same_hand)),
            stroked(&|s| match s.top_count {
                0 => "—".to_string(),
                n => format!("{}×{n}", s.top_key),
            }),
        ]
    }

    /// **The details of the record**, label and value, for the history's
    /// foot: what the table is too narrow to hold.
    pub fn details(&self) -> Vec<(String, String)> {
        let heads = columns();
        let cells = self.cells();
        // 當量, 峰值, 停留, then the hands after the article.
        [5, 9, 10, 16, 17, 18, 19, 20].iter().map(|&i| (heads[i].clone(), cells[i].clone())).collect()
    }

    /// One JSON line.
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
}

/// The history's column heads, in the order of [`Record::cells`].
pub fn columns() -> Vec<String> {
    vec![
        say!("genda.label.time"),
        say!("genda.label.segment"),
        say!("genda.label.speed"),
        say!("genda.label.kps"),
        say!("genda.label.code-length"),
        say!("genda.label.equivalence"),
        say!("genda.label.chars"),
        say!("genda.label.errors"),
        say!("genda.label.elapsed"),
        say!("genda.label.peak"),
        say!("genda.label.stay"),
        say!("genda.label.corrections"),
        say!("genda.label.key-accuracy"),
        say!("genda.label.word-rate"),
        say!("genda.label.retry"),
        say!("genda.label.article"),
        say!("genda.label.pairs"),
        say!("genda.label.alternation-rate"),
        say!("genda.label.alternating"),
        say!("genda.label.same-hand"),
        say!("genda.label.top-key"),
    ]
}

/// A count that may hold a half (a pair with Space): `12`, `12.5`.
fn half(v: f64) -> String {
    match v.fract() == 0.0 {
        true => format!("{v:.0}"),
        false => format!("{v:.1}"),
    }
}

/// **The history day by day**, for the 累計 box and the history's title.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Daily {
    pub today_chars: usize,
    pub today_millis: u64,
    pub chars: usize,
    /// Days with at least one record.
    pub days: usize,
    /// Days in a row with a record, up to today, or up to yesterday while
    /// today has none yet: a streak is not broken before the day is out.
    pub streak: usize,
}

impl Daily {
    /// **Add up `records` by day**: `day_of` turns a record's seconds into a
    /// day number (the local calendar day, in the front end), and `today` is
    /// that number for now. A record with no time is counted in the total
    /// characters and on no day.
    pub fn of(records: &[Record], day_of: impl Fn(u64) -> i64, today: i64) -> Daily {
        let mut out = Daily::default();
        let mut days = std::collections::BTreeSet::new();
        for record in records {
            out.chars += record.chars;
            if record.at == 0 {
                continue;
            }
            let day = day_of(record.at);
            days.insert(day);
            if day == today {
                out.today_chars += record.chars;
                out.today_millis += record.millis;
            }
        }
        out.days = days.len();
        let mut day = match days.contains(&today) {
            true => today,
            false => today - 1,
        };
        while days.contains(&day) {
            out.streak += 1;
            day -= 1;
        }
        out
    }
}

/// The day number of a calendar date: days since 1970-01-01, by the
/// proleptic Gregorian calendar.
pub fn day_number(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// **The whole history as TSV**, a header row first: what a spreadsheet takes
/// from the clipboard. Every column of the history, the details too; a
/// field not measured is an empty cell, which a spreadsheet leaves out of
/// a sum or a mean, where the table's `—` would be text.
pub fn tsv(records: &[Record]) -> String {
    let clean = |s: &str| match s {
        "—" => String::new(),
        _ => s.replace(['\t', '\n', '\r'], " "),
    };
    let mut out = columns().join("\t");
    for record in records {
        out.push('\n');
        out.push_str(&record.cells().iter().map(|c| clean(c)).collect::<Vec<_>>().join("\t"));
    }
    out.push('\n');
    out
}

/// The records in a JSONL text; a line that does not read is passed over.
pub fn read_records(text: &str) -> Vec<Record> {
    text.lines().filter_map(|line| serde_json::from_str(line.trim()).ok()).collect()
}

/// **The records as the history file keeps them**: one JSON line each, in
/// order, what [`read_records`] reads back.
pub fn jsonl(records: &[Record]) -> String {
    records.iter().map(|r| r.to_line() + "\n").collect()
}

/// **The history file's text without `record`**: the first line that reads
/// as that record is dropped, and every other line stays as it was written —
/// records another yumete appended since this one read the file, and lines
/// that do not read, which are somebody's and not this one's to discard.
/// `None` when no line is that record.
pub fn without_record(text: &str, record: &Record) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let at = lines.iter().position(|line| serde_json::from_str::<Record>(line.trim()).ok().as_ref() == Some(record))?;
    Some(lines.iter().enumerate().filter(|(i, _)| *i != at).map(|(_, line)| format!("{line}\n")).collect())
}

/// **The history file's text with `record` back in it**: in front of
/// `next`, the record it stood before, when the file still holds that one;
/// else before the first record finished later than it, so the file stays
/// in the order the segments were typed. Every other line as it was written.
pub fn with_record(text: &str, record: &Record, next: Option<&Record>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let read = |line: &&str| serde_json::from_str::<Record>(line.trim()).ok();
    let at = next
        .and_then(|next| lines.iter().position(|line| read(line).as_ref() == Some(next)))
        .or_else(|| lines.iter().position(|line| read(line).is_some_and(|r| r.at > record.at)))
        .unwrap_or(lines.len());
    let mut out = String::with_capacity(text.len() + 256);
    for (i, line) in lines.iter().enumerate() {
        if i == at {
            out.push_str(&record.to_line());
            out.push('\n');
        }
        out.push_str(line);
        out.push('\n');
    }
    if at == lines.len() {
        out.push_str(&record.to_line());
        out.push('\n');
    }
    out
}

/// `m:ss.fff`.
pub fn clock_text(millis: u64) -> String {
    format!("{}:{:02}.{:03}", millis / 60_000, millis / 1000 % 60, millis % 1000)
}

fn per_minute(chars: f64, seconds: f64) -> f64 {
    match seconds > 0.0 {
        true => chars * 60.0 / seconds,
        false => 0.0,
    }
}

/// **This session's segments, added up**, for the 累計 box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Totals {
    pub segments: usize,
    pub chars: usize,
    pub millis: u64,
    /// Keys and their time, over the segments that measured them.
    pub keys: usize,
    pub keyed_chars: usize,
    pub keyed_millis: u64,
    pub fastest: f64,
}

impl Totals {
    /// The totals of `records`, as if each had been added in turn.
    pub fn of(records: &[Record]) -> Totals {
        let mut out = Totals::default();
        for record in records {
            out.add(record);
        }
        out
    }

    pub fn add(&mut self, record: &Record) {
        self.segments += 1;
        self.chars += record.chars;
        self.millis += record.millis;
        if let Some(keys) = record.keys {
            self.keys += keys.keys;
            self.keyed_chars += record.chars;
            self.keyed_millis += record.millis;
        }
        self.fastest = self.fastest.max(record.speed());
    }

    pub fn speed(&self) -> f64 {
        per_minute(self.chars as f64, self.millis as f64 / 1000.0)
    }

    pub fn keys_per_second(&self) -> Option<f64> {
        (self.keyed_millis > 0).then(|| self.keys as f64 * 1000.0 / self.keyed_millis as f64)
    }

    pub fn code_length(&self) -> Option<f64> {
        (self.keyed_chars > 0).then(|| self.keys as f64 / self.keyed_chars as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    const YUME: Stroke = Stroke { yume: true, composing: false, backspace: false, key: None };
    const CODE: Stroke = Stroke { yume: true, composing: true, backspace: false, key: None };
    const SYSTEM: Stroke = Stroke { yume: false, composing: false, backspace: false, key: None };

    fn run(target: &str) -> Run {
        Run::new(target.chars().collect())
    }

    #[test]
    fn a_segment_ends_on_its_length_with_the_last_character_right() {
        let mut r = run("天地人");
        r.key(YUME, ms(0));
        assert!(!r.text_now("天", YUME, ms(100)));
        assert!(!r.text_now("天他", YUME, ms(200)));
        assert!(r.text_now("天他人", YUME, ms(300)));
        assert_eq!(r.record(ms(900)).errors, 1);
        assert_eq!(r.record(ms(900)).millis, 300);
    }

    #[test]
    fn a_wrong_last_character_keeps_the_segment_open_until_it_is_fixed() {
        let mut r = run("天地人");
        r.key(YUME, ms(0));
        assert!(!r.text_now("天地入", YUME, ms(100)));
        assert!(!r.text_now("天地入和", YUME, ms(200)));
        let back = Stroke { backspace: true, ..YUME };
        assert!(!r.text_now("天地入", back, ms(300)));
        assert!(!r.text_now("天地", back, ms(400)));
        assert!(r.text_now("天地人", YUME, ms(500)));
        assert_eq!(r.record(ms(500)).corrections, 2);
        assert_eq!(r.record(ms(500)).errors, 0);
    }

    #[test]
    fn what_runs_past_the_end_is_dropped() {
        let mut r = run("天地");
        r.key(YUME, ms(0));
        assert!(r.text_now("天地人和", YUME, ms(100)));
        assert_eq!(r.typed(), &['天', '地']);
        assert_eq!(r.record(ms(100)).chars, 2);
    }

    #[test]
    fn one_wrong_costs_five() {
        let record = Record { chars: 25, errors: 2, millis: 15_000, ..Record::default() };
        assert_eq!(format!("{:.2}", record.speed()), "100.00");
        assert_eq!(format!("{:.2}", record.penalised()), "60.00");
        assert_eq!(record.speed_text(), "60.00/100.00");
        let floored = Record { chars: 10, errors: 3, millis: 6_000, ..Record::default() };
        assert_eq!(floored.penalised(), 0.0);
        assert_eq!(Record { errors: 0, ..floored }.speed_text(), "100.00");
    }

    #[test]
    fn a_backspace_in_a_code_is_tuige_and_outside_it_huigai() {
        let mut r = run("天地人");
        r.key(CODE, ms(0));
        r.key(Stroke { backspace: true, ..CODE }, ms(10));
        r.text_now("", Stroke { backspace: true, ..CODE }, ms(10));
        r.key(YUME, ms(20));
        r.text_now("天", YUME, ms(20));
        let bs = Stroke { backspace: true, ..YUME };
        r.key(bs, ms(30));
        r.text_now("", bs, ms(30));
        let record = r.record(ms(40));
        assert_eq!(record.keys.unwrap().backspaces, 1);
        assert_eq!(record.corrections, 1);
    }

    #[test]
    fn the_word_rate_counts_han_committed_two_or_more_at_a_time() {
        let mut r = run("天地人和。");
        r.text_now("天地", YUME, ms(0));
        r.text_now("天地人", YUME, ms(100));
        r.text_now("天地人和。", YUME, ms(200));
        let record = r.record(ms(200));
        assert_eq!((record.han, record.han_in_words), (4, 2));
        assert_eq!(format!("{:.2}", record.word_rate()), "50.00");
    }

    #[test]
    fn under_the_system_ime_a_burst_is_one_commit() {
        let mut r = run("天地人和");
        r.text_now("天", SYSTEM, ms(0));
        r.text_now("天地", SYSTEM, ms(3));
        r.text_now("天地人", SYSTEM, ms(400));
        r.text_now("天地人和", SYSTEM, ms(800));
        let record = r.record(ms(800));
        assert_eq!((record.han, record.han_in_words), (4, 2));
    }

    #[test]
    fn keys_give_the_rate_the_code_length_and_the_accuracy() {
        let mut r = run("天地");
        for i in 0..8 {
            r.key(CODE, ms(i * 250));
        }
        r.text_now("天地", YUME, ms(2000));
        let record = r.record(ms(2000));
        assert_eq!(record.keys_per_second(), Some(4.0));
        assert_eq!(record.code_length(), Some(4.0));
        assert_eq!(record.key_accuracy(), Some(100.0));
        let missed = Record { corrections: 1, keys: Some(Keys { keys: 8, backspaces: 2, picks: 0 }), ..record };
        assert_eq!(missed.key_accuracy(), Some(25.0));
    }

    #[test]
    fn under_the_system_ime_the_key_fields_are_empty_not_nought() {
        let mut r = run("天地");
        r.key(SYSTEM, ms(0));
        r.text_now("天地", SYSTEM, ms(1000));
        let record = r.record(ms(1000));
        assert_eq!(record.keys, None);
        assert_eq!(record.keys_per_second(), None);
        let line = record.score_line();
        assert!(!line.contains(&say!("genda.line.kps", "0.00")), "{line}");
        assert!(record.cells().iter().any(|c| c == "—"));
    }

    #[test]
    fn the_clock_starts_at_the_first_key_and_stands_while_paused() {
        let mut r = run("天地");
        assert_eq!(r.elapsed(ms(5000)), Duration::ZERO);
        r.key(YUME, ms(5000));
        r.pause(ms(6000));
        r.resume(ms(9000));
        assert!(r.text_now("天地", YUME, ms(9500)));
        assert_eq!(r.record(ms(9999)).millis, 1500);
    }

    #[test]
    fn the_presets_cut_the_article() {
        assert_eq!(segment_count(60, Some(25)), 3);
        assert_eq!(segment(60, Some(25), 2), 50..60);
        assert_eq!(segment_count(60, None), 1);
        assert_eq!(segment(60, None, 0), 0..60);
        assert_eq!(segment_count(0, Some(25)), 0);
        assert_eq!(strip("天\n地\t人\r和 。"), vec!['天', '地', '人', '和', ' ', '。']);
    }

    #[test]
    fn only_unified_ideographs_and_their_extensions_are_counted() {
        for c in ['〇', '\u{3400}', '\u{4DBF}', '一', '\u{9FFF}', '\u{20000}', '\u{2A700}', '\u{2EE5D}', '\u{30000}', '\u{323AF}', '\u{323B0}', '\u{3347F}'] {
            assert!(counts_as_han(c), "{:X}", c as u32);
        }
        // Compatibility ideographs, radicals, kana, punctuation, the gap after Ext B.
        for c in ['\u{F900}', '\u{2F800}', '⺀', '⼀', 'あ', '，', '々', 'a', '\u{2A6E0}', '\u{33480}'] {
            assert!(!counts_as_han(c), "{:X}", c as u32);
        }
    }

    #[test]
    fn the_top_characters_go_by_count_then_by_code_point() {
        let text = "天天天地地人人和〇〇。\u{F900}\u{F900}\u{F900}\u{F900}abc";
        // 人 (U+4EBA) and 地 (U+5730) tie at two; 〇 (U+3007) ties too and comes first.
        assert_eq!(top_chars(text, 10), vec!['天', '〇', '人', '地', '和']);
        assert_eq!(top_chars(text, 2), vec!['天', '〇']);
        assert_eq!(top_chars("abc，。", 5), Vec::<char>::new());
        let many: String = ('\u{4E00}'..='\u{4E00}').chain((0..600).map(|i| char::from_u32(0x4E00 + i).unwrap())).collect();
        assert_eq!(top_chars(&many, DEFAULT_TOP).len(), 500);
    }

    #[test]
    fn a_seed_names_one_order() {
        let chars: Vec<char> = "天地玄黄宇宙洪荒日月盈昃".chars().collect();
        let a = shuffled(&chars, 7);
        assert_eq!(a, shuffled(&chars, 7));
        assert_ne!(a, chars);
        assert_ne!(a, shuffled(&chars, 8));
        let mut sorted = a.clone();
        sorted.sort();
        let mut want = chars.clone();
        want.sort();
        assert_eq!(sorted, want);
    }

    #[test]
    fn a_record_reads_back_from_its_line() {
        let record = Record {
            at: 1_800_000_000,
            segment: 3,
            chars: 25,
            errors: 1,
            millis: 12_345,
            corrections: 2,
            retry: 1,
            han: 25,
            han_in_words: 10,
            keys: Some(Keys { keys: 80, backspaces: 3, picks: 1 }),
            strokes: Some(Strokes {
                equivalence: 92.5,
                pairs: 70,
                alternating: 40.5,
                same_hand: 29.5,
                top_key: "Space".into(),
                top_count: 20,
            }),
            peak_chars: 11,
            peak_millis: 3_300,
            stay_millis: 1_250,
            stay_text: "天地".into(),
            scheme: "靈明".into(),
            article: "一篇".into(),
            version: "0.22".into(),
        };
        let text = format!("{}\nnot json\n{}\n", record.to_line(), Record::default().to_line());
        let back = read_records(&text);
        assert_eq!(back, vec![record, Record::default()]);
    }

    #[test]
    fn the_score_line_and_the_tsv() {
        let record = Record {
            segment: 3,
            chars: 25,
            errors: 1,
            millis: 15_000,
            corrections: 2,
            han: 20,
            han_in_words: 10,
            keys: Some(Keys { keys: 75, backspaces: 1, picks: 2 }),
            strokes: Some(Strokes {
                equivalence: 111.0,
                pairs: 74,
                alternating: 40.5,
                same_hand: 29.5,
                top_key: "Space".into(),
                top_count: 20,
            }),
            peak_chars: 10,
            peak_millis: 4_000,
            stay_millis: 1_234,
            stay_text: "地".into(),
            scheme: "靈明".into(),
            version: "0.22".into(),
            ..Record::default()
        };
        let want = [
            say!("genda.line.segment", 3),
            say!("genda.line.speed", "80.00/100.00"),
            say!("genda.line.kps", "5.00"),
            say!("genda.line.code-length", "3.00"),
            say!("genda.line.equivalence", "1.50"),
            say!("genda.line.chars", 25),
            say!("genda.line.errors", 1),
            say!("genda.line.time", "0:15.000"),
            say!("genda.line.peak", "150.00"),
            say!("genda.line.stay", "[地]1.23s"),
            say!("genda.line.corrections", 2),
            say!("genda.line.key-accuracy", "90.67"),
            say!("genda.line.word-rate", "50.00"),
            say!("genda.line.picks", 2),
            say!("genda.line.first"),
            say!("genda.line.pairs", 74),
            say!("genda.line.alternation-rate", "57.86"),
            say!("genda.line.alternating", "40.5"),
            say!("genda.line.same-hand", "29.5"),
            say!("genda.line.top-key", "Space×20"),
            say!("genda.line.ime", "靈明"),
            "yumete v0.22".to_string(),
        ]
        .join(" ");
        assert_eq!(record.score_line(), want);
        // Whatever the history's foot shows of a record, its line carries.
        for (label, value) in record.details() {
            assert!(want.contains(&value), "{label} {value} not in {want}");
        }
        let again = Record { retry: 2, keys: None, strokes: None, scheme: String::new(), ..record.clone() };
        let line = again.score_line();
        assert!(line.contains(&say!("genda.line.retry", 2)), "{line}");
        assert!(line.contains(&say!("genda.line.ime", say!("genda.system-ime"))), "{line}");
        assert!(!line.contains(&say!("genda.line.picks", 2)), "{line}");
        assert!(!line.contains(&say!("genda.line.equivalence", "1.50")), "{line}");
        assert!(!line.contains(&say!("genda.line.pairs", 74)), "{line}");
        assert!(line.contains(&say!("genda.line.peak", "150.00")), "{line}");
        let table = tsv(&[record.clone(), again]);
        let rows: Vec<&str> = table.lines().collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], columns().join("\t"));
        assert!(rows.iter().all(|row| row.split('\t').count() == columns().len()));
        // Every detail has its column; one not measured is an empty cell.
        let heads: Vec<&str> = rows[0].split('\t').collect();
        let measured: Vec<&str> = rows[1].split('\t').collect();
        let unmeasured: Vec<&str> = rows[2].split('\t').collect();
        for (label, value) in record.details() {
            let at = heads.iter().position(|h| *h == label).unwrap_or_else(|| panic!("{label} has no column"));
            assert_eq!(measured[at], value, "{label}");
        }
        for label in [say!("genda.label.kps"), say!("genda.label.equivalence"), say!("genda.label.top-key")] {
            let at = heads.iter().position(|h| *h == label).unwrap();
            assert_eq!(unmeasured[at], "", "{label}");
        }
        assert_eq!(clock_text(83_004), "1:23.004");
    }

    fn keyed(key: Pressed) -> Stroke {
        Stroke { key: Some(key), ..CODE }
    }

    #[test]
    fn strokes_count_the_pairs_and_the_hands_as_typepad_does() {
        use Pressed::*;
        let s = Strokes::count(&[Char('D'), Char('K'), Space, Char('J'), Char('J'), Backspace, Shift]);
        // dk 1.1, k␣ 1.2, ␣j 1.4, jj 1.3; J–Backspace and Backspace–Shift are not in the table.
        assert_eq!(s.pairs, 4);
        assert_eq!(format!("{:.2}", s.mean()), "1.25");
        // D–K across; each pair with Space half and half; J–J and J–Backspace one hand; Shift neither.
        assert_eq!((s.alternating, s.same_hand), (2.0, 3.0));
        assert_eq!(format!("{:.2}", s.alternation_rate()), "40.00");
        assert_eq!((s.top_key.as_str(), s.top_count), ("J", 2));
        // Space twice is half and half too, and in no table; a tie names the first key.
        let s = Strokes::count(&[Space, Space]);
        assert_eq!((s.pairs, s.alternating, s.same_hand), (0, 0.5, 0.5));
        assert_eq!(Strokes::count(&[Char('B'), Char('A')]).top_key, "A");
        assert_eq!(Strokes::count(&[]), Strokes::default());
        assert_eq!(Pressed::of(crate::input::Key::Char('a')), Some(Char('A')));
        assert_eq!(Pressed::of(crate::input::Key::Char(' ')), Some(Space));
        assert_eq!(Pressed::of(crate::input::Key::Left), None);
    }

    #[test]
    fn a_run_keeps_its_keys_only_while_the_built_in_ime_holds_them() {
        let mut r = run("天地");
        for key in [Pressed::Char('D'), Pressed::Char('K'), Pressed::Space] {
            r.key(keyed(key), ms(0));
        }
        r.text_now("天地", YUME, ms(500));
        let record = r.record(ms(500));
        assert_eq!(record.strokes.as_ref().map(|s| s.pairs), Some(2));
        assert_eq!(record.equivalence().map(|v| format!("{v:.2}")), Some("1.15".into()));
        let mut r = run("天地");
        r.key(keyed(Pressed::Char('D')), ms(0));
        r.key(Stroke { key: Some(Pressed::Char('K')), ..SYSTEM }, ms(10));
        r.text_now("天地", SYSTEM, ms(500));
        assert_eq!(r.record(ms(500)).strokes, None);
    }

    /// 一二 slowly, then the other ten at 300 ms each.
    fn slow_start() -> Run {
        let mut r = run("一二三四五六七八九十百千");
        r.key(YUME, ms(0));
        let chars: Vec<char> = r.target().to_vec();
        let mut typed = String::new();
        for (i, c) in chars.iter().enumerate() {
            typed.push(*c);
            let at = match i {
                0 => 2000,
                n => 4500 + 300 * (n as u64 - 1),
            };
            r.text_now(&typed, YUME, ms(at));
        }
        r
    }

    #[test]
    fn the_peak_is_the_best_stretch_of_ten_from_one_commit_to_another() {
        let r = slow_start();
        assert!(r.finished());
        let record = r.record(ms(9999));
        // From 二 at 4.5 s to 千 at 7.5 s: ten characters in three seconds.
        assert_eq!((record.peak_chars, record.peak_millis), (10, 3000));
        assert_eq!(format!("{:.2}", record.peak()), "200.00");
        // A segment no longer than the stretch is its own peak.
        let mut short = run("天地");
        short.key(YUME, ms(0));
        short.text_now("天地", YUME, ms(600));
        assert_eq!(short.record(ms(600)).peak(), 200.0);
    }

    #[test]
    fn the_stay_is_the_longest_wait_before_a_commit_and_what_it_brought() {
        let record = slow_start().record(ms(9999));
        assert_eq!((record.stay_millis, record.stay_text.as_str()), (2500, "二"));
        assert_eq!(record.stay_text(), "[二]2.50s");
        // A word that waited is the stay as a whole, joined bursts and all.
        let mut r = run("天地人和");
        r.key(SYSTEM, ms(0));
        r.text_now("天", SYSTEM, ms(0));
        r.text_now("天地", SYSTEM, ms(900));
        r.text_now("天地人", SYSTEM, ms(905));
        r.text_now("天地人和", SYSTEM, ms(1000));
        assert_eq!(r.record(ms(1000)).stay_text, "地人");
    }

    #[test]
    fn the_slowest_share_a_commits_wait_and_the_missed_count_what_was_put_right() {
        assert_eq!(slow_start().slowest(3), vec!['二', '一', '三']);
        assert_eq!(slow_start().slowest(SLOW).len(), SLOW);
        let mut r = run("天地人和");
        r.key(YUME, ms(0));
        r.text_now("天地", YUME, ms(1000));
        r.text_now("天地入", YUME, ms(1100));
        let back = Stroke { backspace: true, ..YUME };
        r.text_now("天地", back, ms(1200));
        r.text_now("天地人", YUME, ms(1300));
        r.text_now("天地人和", YUME, ms(1400));
        assert_eq!(r.missed(), vec!['人']);
        // The word waited a second, 500 ms a character, against 300 for 人 and 100 for 和.
        assert_eq!(r.slowest(2), vec!['天', '地']);
        assert_eq!(r.word_marks(), vec![true, true, false, false]);
    }

    #[test]
    fn under_the_system_ime_a_burst_is_marked_a_word() {
        let mut r = run("天地人。");
        r.text_now("天", SYSTEM, ms(0));
        r.text_now("天地", SYSTEM, ms(3));
        r.text_now("天地人", SYSTEM, ms(400));
        r.text_now("天地人。", SYSTEM, ms(403));
        // 人 came with a punctuation mark only: one character, not a word.
        assert_eq!(r.word_marks(), vec![true, true, false, false]);
    }

    #[test]
    fn a_goal_names_what_a_record_misses() {
        let goal = Goal { speed: Some(120.0), kps: Some(6.0), accuracy: None, clean: true };
        assert!(goal.is_set() && !Goal::default().is_set());
        assert_eq!(goal.text(), [say!("genda.goal.speed", "120"), say!("genda.goal.kps", "6"), say!("genda.goal.clean")].join(" "));
        assert_eq!(Goal::default().text(), say!("genda.goal.none"));
        let record = Record { chars: 25, errors: 1, millis: 10_000, ..Record::default() };
        // 150 a minute, 120 after five off the one error; no keys measured, so 擊鍵 is passed over.
        assert_eq!(goal.missed(&record), vec![say!("genda.goal.clean")]);
        let slow = Record { millis: 15_000, ..record.clone() };
        assert_eq!(goal.missed(&slow), vec![say!("genda.goal.speed", "120"), say!("genda.goal.clean")]);
        let keyed = Record { errors: 0, keys: Some(Keys { keys: 50, ..Keys::default() }), ..record };
        assert_eq!(goal.missed(&keyed), vec![say!("genda.goal.kps", "6")]);
        assert_eq!(hundredths("6.5"), Some(650));
        assert_eq!(hundredths("off"), Some(0));
        assert_eq!(hundredths("0"), Some(0));
        assert_eq!(hundredths("-1"), None);
        assert_eq!(hundredths("fast"), None);
        assert_eq!(from_hundredths(650), Some(6.5));
        assert_eq!(from_hundredths(0), None);
    }

    #[test]
    fn the_days_add_up_by_calendar_day_with_a_streak_up_to_today() {
        let day = |d: u64, chars: usize| Record { at: d * 86_400 + 3600, chars, millis: 1000, ..Record::default() };
        let records = [day(7, 10), day(9, 20), day(10, 30), day(10, 40), Record { chars: 5, ..Record::default() }];
        let daily = Daily::of(&records, |s| (s / 86_400) as i64, 10);
        assert_eq!(daily, Daily { today_chars: 70, today_millis: 2000, chars: 105, days: 3, streak: 2 });
        // Tomorrow, before its first segment, the streak still stands.
        assert_eq!(Daily::of(&records, |s| (s / 86_400) as i64, 11).streak, 2);
        assert_eq!(Daily::of(&records, |s| (s / 86_400) as i64, 12).streak, 0);
        assert_eq!(day_number(1970, 1, 1), 0);
        assert_eq!(day_number(2000, 2, 29), 11_016);
        assert_eq!(day_number(2026, 11, 14), 20_771);
    }

    #[test]
    fn totals_add_up_the_session() {
        let mut totals = Totals::default();
        totals.add(&Record { chars: 25, millis: 15_000, keys: Some(Keys { keys: 75, ..Keys::default() }), ..Record::default() });
        totals.add(&Record { chars: 25, millis: 15_000, ..Record::default() });
        assert_eq!(totals.segments, 2);
        assert_eq!(totals.speed(), 100.0);
        assert_eq!(totals.keys_per_second(), Some(5.0));
        assert_eq!(totals.code_length(), Some(3.0));
        assert_eq!(totals.fastest, 100.0);
        let records = [Record { chars: 10, millis: 6_000, ..Record::default() }, Record { chars: 30, millis: 6_000, ..Record::default() }];
        let mut added = Totals::default();
        records.iter().for_each(|r| added.add(r));
        assert_eq!(Totals::of(&records), added);
        assert_eq!(Totals::of(&records[..1]).fastest, 100.0);
    }

    #[test]
    fn the_history_file_reads_back_record_for_record() {
        let records = vec![Record { segment: 1, chars: 5, ..Record::default() }, Record { segment: 2, chars: 7, ..Record::default() }];
        let text = jsonl(&records);
        assert_eq!(text.lines().count(), 2);
        assert_eq!(read_records(&text), records);
        assert_eq!(jsonl(&[]), "");
    }

    /// **Deleting a record touches that record only**: a record appended
    /// by another yumete since the file was read, and a line that does not
    /// read, both stay; putting it back puts it where its time says.
    #[test]
    fn a_record_comes_out_of_the_file_and_goes_back_by_itself() {
        let one = Record { at: 10, segment: 1, chars: 5, ..Record::default() };
        let two = Record { at: 20, segment: 2, chars: 7, ..Record::default() };
        let theirs = Record { at: 30, segment: 1, chars: 9, ..Record::default() };
        let text = format!("{}\nnot a record\n{}\n{}\n", one.to_line(), two.to_line(), theirs.to_line());
        let without = without_record(&text, &two).expect("the record is there");
        assert_eq!(without, format!("{}\nnot a record\n{}\n", one.to_line(), theirs.to_line()));
        assert_eq!(read_records(&without), vec![one.clone(), theirs.clone()]);
        assert_eq!(without_record(&without, &two), None, "not there twice");
        assert_eq!(with_record(&without, &two, None), text, "back where its time puts it");
        assert_eq!(with_record(&without, &two, Some(&theirs)), text, "back in front of the one it stood before");
        let same_time = Record { at: 30, ..two.clone() };
        let back = with_record(&without, &same_time, Some(&theirs));
        assert_eq!(read_records(&back), vec![one.clone(), same_time, theirs.clone()], "a tie goes by its neighbour");
        assert_eq!(with_record("", &one, None), format!("{}\n", one.to_line()));
    }
}
