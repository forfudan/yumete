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
    /// Another article.
    Article(Choice),
    /// An article of the `N` most frequent 漢字 in a scope (`:genda-top`
    /// and its scopes); the core counts them and hands the article over.
    Top(crate::search_panel::Where, usize),
    /// `:q` while the page is open.
    Close,
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
        let added = &new[same..];
        if !added.is_empty() {
            let join = !stroke.yume && gone == 0 && self.last_commit.is_some_and(|at| now.saturating_sub(at) < BURST);
            self.commit(added, join);
            self.last_commit = Some(now);
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

    /// The counts so far, as a record would keep them; `at` is the clock.
    pub fn record(&self, now: Duration) -> Record {
        let chars = match self.done {
            Some(_) => self.target.len(),
            None => self.typed.len(),
        };
        Record {
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
        parts.push(say!("genda.line.chars", self.chars));
        parts.push(say!("genda.line.errors", self.errors));
        parts.push(say!("genda.line.time", clock_text(self.millis)));
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
        parts.push(say!("genda.line.ime", self.ime_name()));
        parts.push(format!("yumete v{}", self.version));
        parts.join(" ")
    }

    /// One row of the history, column for column with [`columns`]; a key
    /// field not measured is `—`.
    pub fn cells(&self) -> Vec<String> {
        let or_dash = |v: Option<f64>, f: fn(f64) -> String| v.map_or_else(|| "—".to_string(), f);
        vec![
            crate::clock::when(std::time::UNIX_EPOCH + Duration::from_secs(self.at)),
            self.segment.to_string(),
            self.speed_text(),
            or_dash(self.keys_per_second(), |v| format!("{v:.2}")),
            or_dash(self.code_length(), |v| format!("{v:.2}")),
            self.chars.to_string(),
            self.errors.to_string(),
            clock_text(self.millis),
            self.corrections.to_string(),
            or_dash(self.key_accuracy(), |v| format!("{v:.2}%")),
            format!("{:.2}%", self.word_rate()),
            self.retry.to_string(),
            self.article.clone(),
        ]
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
        say!("genda.label.chars"),
        say!("genda.label.errors"),
        say!("genda.label.elapsed"),
        say!("genda.label.corrections"),
        say!("genda.label.key-accuracy"),
        say!("genda.label.word-rate"),
        say!("genda.label.retry"),
        say!("genda.label.article"),
    ]
}

/// **The whole history as TSV**, a header row first: what a spreadsheet takes
/// from the clipboard.
pub fn tsv(records: &[Record]) -> String {
    let clean = |s: &str| s.replace(['\t', '\n', '\r'], " ");
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

    const YUME: Stroke = Stroke { yume: true, composing: false, backspace: false };
    const CODE: Stroke = Stroke { yume: true, composing: true, backspace: false };
    const SYSTEM: Stroke = Stroke { yume: false, composing: false, backspace: false };

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
            scheme: "靈明".into(),
            version: "0.22".into(),
            ..Record::default()
        };
        let want = [
            say!("genda.line.segment", 3),
            say!("genda.line.speed", "80.00/100.00"),
            say!("genda.line.kps", "5.00"),
            say!("genda.line.code-length", "3.00"),
            say!("genda.line.chars", 25),
            say!("genda.line.errors", 1),
            say!("genda.line.time", "0:15.000"),
            say!("genda.line.corrections", 2),
            say!("genda.line.key-accuracy", "90.67"),
            say!("genda.line.word-rate", "50.00"),
            say!("genda.line.picks", 2),
            say!("genda.line.first"),
            say!("genda.line.ime", "靈明"),
            "yumete v0.22".to_string(),
        ]
        .join(" ");
        assert_eq!(record.score_line(), want);
        let again = Record { retry: 2, keys: None, scheme: String::new(), ..record.clone() };
        let line = again.score_line();
        assert!(line.contains(&say!("genda.line.retry", 2)), "{line}");
        assert!(line.contains(&say!("genda.line.ime", say!("genda.system-ime"))), "{line}");
        assert!(!line.contains(&say!("genda.line.picks", 2)), "{line}");
        let table = tsv(&[record, again]);
        let rows: Vec<&str> = table.lines().collect();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], columns().join("\t"));
        assert!(rows.iter().all(|row| row.split('\t').count() == columns().len()));
        assert_eq!(clock_text(83_004), "1:23.004");
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
    }
}
