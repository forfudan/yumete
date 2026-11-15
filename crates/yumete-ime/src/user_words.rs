//! **The Yume app's word lists, read; yumete's own, kept apart.**
//!
//! A writer who types in the system Yume has taught it words (用户詞庫), pinned
//! candidates (固頂) and an order of their own (調頻). Typing the same scheme
//! in yumete should answer with the same words, so those files are read — and
//! only read: the app owns them, rewrites them whole, and a second writer
//! racing it would lose somebody's words.
//!
//! **Two layers, never entangled.** The first is the app's files, loaded as
//! they are. The second is what yumete itself learned — and only that: a word
//! it made, a commit it counted, a pin set while typing in yumete. It is kept
//! in files of its own, in the same formats, under yumete's data directory:
//!
//! ```text
//! <app>/data/<tag>_user_dict.txt         用户詞庫 (old stems ling/xing/qing too)
//! <app>/data/<shared>_user_dict.txt      the shared 拼音用户詞典 (冰雪's snow-pinyin)
//! <app>/schemes/<tag>/freq.txt           調頻
//! <app>/schemes/<shared>/freq.txt        調頻 kept by reading, shared
//! <app>/schemes/<tag>/pins.txt           固頂
//!
//! <own>/<tag>/user_dict.txt, freq.txt, pins.txt
//! <own>/<shared>/user_dict.txt, freq.txt
//! ```
//!
//! At typing time the two are **combined, neither overwriting the other**:
//!
//! - 用户詞庫: one table, the union of both.
//! - 調頻: the engine holds one record per scheme, so the two records are
//!   summed into it key by key ([`UserFreq::absorb`]: the counts add, the
//!   heat of each decays to now and adds). A key both know counts both
//!   writers' choices — the app's 2 and yumete's 5 make 7, and taking
//!   yumete's away leaves the app's 2. Summing rather than choosing is what
//!   lets either file change without the other going stale: a commit made in
//!   the app shows on the next read, whatever yumete learned on that key.
//! - 固頂: the app's rows, then yumete's. A pin is an action on a slot, so the
//!   later one is the one in force, as in the app's own file.
//!
//! What a commit taught the engine is told apart by **what changed in its
//! record since it was last looked at** — the rows whose count went up — and
//! only that much is written to yumete's own record. So yumete's files never
//! hold a row because the app has it, and with them empty yumete answers
//! exactly as the app's files alone.
//!
//! **Learning is a switch** (`[ime] learn`, off by default): off, nothing is
//! written and the engine's queues of new words are emptied unread, though the
//! engine may still reorder in memory for the session. yumete's own files are
//! read either way: turning learning off stops new learning, it does not hide
//! what was learned — that is what the words page deletes from.
//!
//! `autowords.txt` (自動造詞's running counts) is the app's working state and is
//! not read: a word it has finished counting is already in the 用户詞庫.
//!
//! **The formats are yume-core's.** 調頻 and 固頂 go through
//! [`UserFreq`] and [`PinDict`] both ways. The 用户詞庫 is loaded by the
//! engine's own loader; the one thing taken out of it here is the word column,
//! for the whitelist that lets a word with no code (every 拼音 word) into the
//! sentence lattice — the core has no reader that returns it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use yume_core::pin_dict::PinDict;
use yume_core::user_freq::{Row, UserFreq};
use yume_core::user_records::{Kind, Record, UserRecords};
use yume_core::Engine;

/// Where the two sets of files live. Either may be absent: with no app folder
/// there is nothing to read, and with no own folder what is learned stays in
/// memory for the session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserWordRoots {
    /// The Yume app's user folder (`~/Library/Application Support/Yume`,
    /// `%APPDATA%\Yume`, `$XDG_DATA_HOME/yume`). Read only.
    pub app: Option<PathBuf>,
    /// yumete's own folder for what it learns (`<data dir>/yume`).
    pub own: Option<PathBuf>,
}

impl UserWordRoots {
    /// This machine's: the app's folder when it exists, and yumete's own.
    pub fn of_this_machine() -> Self {
        UserWordRoots {
            app: yumete_config::yume_user_dir().filter(|dir| dir.is_dir()),
            own: Some(yumete_config::data_dir().join("yume")),
        }
    }
}

/// One file that was read, for `:yume-where`: where, how many entries, and
/// whether it is yumete's own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadFile {
    pub path: PathBuf,
    pub entries: usize,
    pub own: bool,
}

/// **Which of a scheme's lists**, as the words page shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WordList {
    /// 用户詞庫: words with their codes.
    Words,
    /// 調頻 kept by code.
    Freq,
    /// 調頻 kept by reading, in the shared 拼音用户詞典.
    PinyinFreq,
    /// 固頂.
    Pins,
}

/// Which layer holds an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Held {
    /// yumete's own files only.
    Own,
    /// The Yume app's only: read, never deleted here.
    App,
    /// Both: deleting takes yumete's copy and the app's stays.
    Both,
}

/// **One row of a list**, both layers' say about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WordRow {
    /// The word (上屏的字詞).
    pub word: String,
    /// The code: for 用户詞庫 every code yumete keeps for the word, by a space
    /// (the app's when only the app has it); else the one code.
    pub code: String,
    /// The app's codes for a 用户詞庫 word both have.
    pub app_code: String,
    /// yumete's number: 調頻's count, 固頂's place. `None` for 用户詞庫.
    pub own: Option<i64>,
    /// The app's number on the same key.
    pub app: Option<i64>,
    pub held: Held,
    /// yumete's file for it, and for a 用户詞庫 word the line (from 1).
    pub file: Option<PathBuf>,
    pub line: Option<usize>,
    /// The app's file for it.
    pub app_file: Option<PathBuf>,
}

/// **What a delete took**, to put back. Belongs to the scheme it was taken
/// from; another scheme's lists refuse it.
#[derive(Debug, Clone)]
pub struct Deleted {
    tag: String,
    list: WordList,
    gone: Gone,
}

#[derive(Debug, Clone)]
enum Gone {
    /// 用户詞庫 lines: (the shared file, the line's index, the line).
    Lines(Vec<(bool, usize, String)>),
    Freq { shared: bool, code: String, text: String, row: Row, tick: u64 },
    Pin { at: usize, record: Record },
}

impl Deleted {
    /// The scheme it was taken from.
    pub fn tag(&self) -> &str {
        &self.tag
    }

    pub fn list(&self) -> WordList {
        self.list
    }
}

/// The stems a scheme's 用户詞庫 has had: the tag, then the short name the
/// three oldest schemes were saved under.
fn stems(tag: &str) -> Vec<&str> {
    let old = match tag {
        "lingming" => Some("ling"),
        "xingchen" => Some("xing"),
        "qingyun" => Some("qing"),
        _ => None,
    };
    std::iter::once(tag).chain(old).collect()
}

/// The first of `paths` that is a file, else the first (where a new one goes).
fn first_present(paths: Vec<PathBuf>) -> Option<PathBuf> {
    paths.iter().find(|p| p.is_file()).cloned().or_else(|| paths.into_iter().next())
}

fn app_dict(app: &Path, tag: &str) -> Option<PathBuf> {
    first_present(stems(tag).iter().map(|s| app.join("data").join(format!("{s}_user_dict.txt"))).collect())
}

fn app_scheme_file(app: &Path, tag: &str, name: &str) -> Option<PathBuf> {
    first_present(stems(tag).iter().map(|s| app.join("schemes").join(s).join(name)).collect())
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn read(path: Option<&Path>) -> String {
    path.and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default()
}

/// Write by a temporary file and a rename, so a reader never sees half of it.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("txt.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

fn is_entry(line: &str) -> bool {
    !line.trim().is_empty() && !line.starts_with('#')
}

/// The lines of a file, a `\r` before the newline taken off.
fn lines_of(text: &str) -> impl Iterator<Item = &str> {
    text.lines().map(|l| l.strip_suffix('\r').unwrap_or(l))
}

/// The non-empty, non-comment lines of a 用户詞庫.
fn entry_lines(text: &str) -> impl Iterator<Item = &str> {
    lines_of(text).filter(|l| is_entry(l))
}

/// A 用户詞庫 line's word: everything before the first tab. A line with no tab
/// is a word with no code, and may hold a space.
fn word_of(line: &str) -> &str {
    line.split('\t').next().unwrap_or(line).trim()
}

/// A 用户詞庫 line's code: the second column, empty when there is none.
fn code_of(line: &str) -> &str {
    line.split('\t').nth(1).unwrap_or("").trim()
}

fn append_line(text: &mut String, line: &str) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
}

fn joined(lines: &[&str]) -> String {
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    out
}

/// Rows of a 調頻 record, by key.
type Rows = HashMap<(String, String), Row>;

/// **The engine's 調頻 as last looked at**: what a later look compares with to
/// find what commits added.
#[derive(Debug, Default)]
struct Seen {
    rows: Rows,
    tick: u64,
}

impl Seen {
    fn of(freq: &UserFreq) -> Self {
        Seen { rows: freq.rows().map(|(k, r)| (k.clone(), *r)).collect(), tick: freq.tick() }
    }

    /// The keys chosen since the last look, with how many times each. Every
    /// choice moves the record's clock and stamps the row with it, so only
    /// rows stamped after the last look are asked.
    fn learned(&mut self, now: &UserFreq) -> Vec<(String, String, u32)> {
        if now.tick() == self.tick {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (key, row) in now.rows().filter(|(_, r)| r.tick > self.tick) {
            let before = self.rows.get(key).map_or(0, |r| r.commits);
            let added = row.commits.saturating_sub(before);
            if added > 0 {
                out.push((key.0.clone(), key.1.clone(), added));
            }
            self.rows.insert(key.clone(), *row);
        }
        self.tick = now.tick();
        out
    }
}

/// `into` with `add`'s rows summed in, key by key.
fn summed(into: &UserFreq, add: &UserFreq) -> UserFreq {
    let mut out = into.clone();
    for ((code, text), row) in add.rows() {
        out.absorb(code, text, *row, add.tick());
    }
    out.clear_dirty();
    out
}

fn place(value: i64) -> usize {
    usize::try_from(value).unwrap_or(0)
}

/// The two folders' files for one scheme, and what was read from them.
#[derive(Debug)]
pub(crate) struct UserWords {
    roots: UserWordRoots,
    tag: String,
    /// The shared 拼音用户詞典's name; empty when the scheme has none.
    shared: String,
    /// Whether what is learned is written (`[ime] learn`).
    learn: bool,
    app_dict: String,
    app_shared_dict: String,
    own_dict: String,
    own_shared_dict: String,
    /// The app's 調頻 as the engine read it, keys settled.
    app_freq: UserFreq,
    app_pinyin: UserFreq,
    /// yumete's own 調頻: only what yumete counted.
    own_freq: UserFreq,
    own_pinyin: UserFreq,
    app_pins: PinDict,
    own_pins: PinDict,
    seen_freq: Seen,
    seen_pinyin: Seen,
    seen_pins: Vec<Record>,
    /// yumete's own 調頻 holds what its file does not yet.
    freq_unsaved: bool,
    pinyin_unsaved: bool,
    /// Every file this looks at, with the modification time it had when read
    /// (`None`: not there), so a change made by the app is noticed.
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    read: Vec<ReadFile>,
}

impl UserWords {
    /// Read both folders' files for `tag` into `engine`.
    pub(crate) fn load(engine: &mut Engine, roots: UserWordRoots, tag: &str, learn: bool) -> Self {
        let mut me = UserWords {
            roots,
            tag: tag.to_string(),
            shared: engine.pinyin_user_dict().to_string(),
            learn,
            app_dict: String::new(),
            app_shared_dict: String::new(),
            own_dict: String::new(),
            own_shared_dict: String::new(),
            app_freq: UserFreq::new(),
            app_pinyin: UserFreq::new(),
            own_freq: UserFreq::new(),
            own_pinyin: UserFreq::new(),
            app_pins: PinDict::new(),
            own_pins: PinDict::new(),
            seen_freq: Seen::default(),
            seen_pinyin: Seen::default(),
            seen_pins: Vec::new(),
            freq_unsaved: false,
            pinyin_unsaved: false,
            stamps: Vec::new(),
            read: Vec::new(),
        };
        me.read_all(engine);
        me
    }

    fn app_paths(&self) -> Paths {
        let Some(app) = &self.roots.app else {
            return Paths::default();
        };
        let shared = (!self.shared.is_empty()).then_some(self.shared.as_str());
        Paths {
            dict: app_dict(app, &self.tag),
            shared_dict: shared.and_then(|s| app_dict(app, s)),
            freq: app_scheme_file(app, &self.tag, "freq.txt"),
            shared_freq: shared.and_then(|s| app_scheme_file(app, s, "freq.txt")),
            pins: app_scheme_file(app, &self.tag, "pins.txt"),
        }
    }

    fn own_path(&self, stem: &str, name: &str) -> Option<PathBuf> {
        match stem.is_empty() {
            true => None,
            false => self.roots.own.as_ref().map(|own| own.join(stem).join(name)),
        }
    }

    fn own_paths(&self) -> Paths {
        Paths {
            dict: self.own_path(&self.tag, "user_dict.txt"),
            shared_dict: self.own_path(&self.shared, "user_dict.txt"),
            freq: self.own_path(&self.tag, "freq.txt"),
            shared_freq: self.own_path(&self.shared, "freq.txt"),
            pins: self.own_path(&self.tag, "pins.txt"),
        }
    }

    fn read_all(&mut self, engine: &mut Engine) {
        let app = self.app_paths();
        let own = self.own_paths();
        self.read.clear();
        self.stamps =
            app.all().chain(own.all()).map(|p| (p.to_path_buf(), modified(p))).collect();

        self.app_dict = read(app.dict.as_deref());
        self.app_shared_dict = read(app.shared_dict.as_deref());
        self.own_dict = read(own.dict.as_deref());
        self.own_shared_dict = read(own.shared_dict.as_deref());
        for (path, text, mine) in [
            (&app.dict, &self.app_dict, false),
            (&app.shared_dict, &self.app_shared_dict, false),
            (&own.dict, &self.own_dict, true),
            (&own.shared_dict, &self.own_shared_dict, true),
        ] {
            if let Some(path) = path.as_ref().filter(|p| p.is_file()) {
                let entries = entry_lines(text).count();
                self.read.push(ReadFile { path: path.clone(), entries, own: mine });
            }
        }
        self.install_words(engine);

        // 調頻: the shared one first, as the app does — the scheme's own file
        // moves rows keyed by reading into it as it is read. What the engine
        // holds then is the app's layer, keys settled.
        let app_shared_freq = read(app.shared_freq.as_deref());
        if !self.shared.is_empty() {
            engine.clear_pinyin_freq();
            if !app_shared_freq.is_empty() {
                engine.load_pinyin_freq(&app_shared_freq);
            }
        }
        engine.load_user_freq(&read(app.freq.as_deref()));
        self.app_freq = engine.user_freq().clone();
        self.app_pinyin = engine.pinyin_freq().clone();
        self.own_freq.load_text(&read(own.freq.as_deref()));
        self.own_pinyin.load_text(&read(own.shared_freq.as_deref()));
        self.freq_unsaved = false;
        self.pinyin_unsaved = false;
        for (path, mine) in
            [(&app.shared_freq, false), (&app.freq, false), (&own.shared_freq, true), (&own.freq, true)]
        {
            let Some(path) = path.as_ref().filter(|p| p.is_file()) else {
                continue;
            };
            let mut record = UserFreq::new();
            record.load_text(&read(Some(path)));
            self.read.push(ReadFile { path: path.clone(), entries: record.len(), own: mine });
        }
        self.install_freq(engine);

        // 固頂.
        self.app_pins.load_text(&read(app.pins.as_deref()));
        self.own_pins.load_text(&read(own.pins.as_deref()));
        for (path, pins, mine) in [(&app.pins, &self.app_pins, false), (&own.pins, &self.own_pins, true)] {
            if let Some(path) = path.as_ref().filter(|p| p.is_file()) {
                self.read.push(ReadFile { path: path.clone(), entries: pins.records().len(), own: mine });
            }
        }
        self.install_pins(engine);
    }

    /// Hand the four 用户詞庫 texts to the engine, the app's first.
    pub(crate) fn install_words(&self, engine: &mut Engine) {
        let mut text = String::new();
        for part in [&self.app_dict, &self.app_shared_dict, &self.own_dict, &self.own_shared_dict] {
            if !part.is_empty() {
                append_line(&mut text, part.trim_end_matches('\n'));
            }
        }
        engine.load_user_table_text(&text);
        let words: Vec<&str> =
            entry_lines(&text).map(word_of).filter(|w| w.chars().count() > 1).collect();
        engine.set_user_word_whitelist_str(&words.join("\n"));
    }

    /// The engine's 調頻: the app's record and yumete's, summed.
    fn install_freq(&mut self, engine: &mut Engine) {
        *engine.user_freq_mut() = summed(&self.app_freq, &self.own_freq);
        if !self.shared.is_empty() {
            *engine.pinyin_freq_mut() = summed(&self.app_pinyin, &self.own_pinyin);
        }
        self.seen_freq = Seen::of(engine.user_freq());
        self.seen_pinyin = Seen::of(engine.pinyin_freq());
    }

    /// The engine's 固頂: the app's rows, then yumete's, so yumete's action on
    /// a slot is the later one.
    fn install_pins(&mut self, engine: &mut Engine) {
        let mut text = self.app_pins.to_text();
        text.push_str(&self.own_pins.to_text());
        engine.pin_dict_mut().load_text(&text);
        engine.pin_dict_mut().clear_dirty();
        self.seen_pins = engine.records(Kind::Pin);
    }

    /// The files read, for `:yume-where`.
    pub(crate) fn files_read(&self) -> &[ReadFile] {
        &self.read
    }

    /// Whether what is learned is written.
    pub(crate) fn set_learn(&mut self, engine: &mut Engine, on: bool) {
        if self.learn && !on {
            self.save(engine);
        }
        self.learn = on;
    }

    /// **What the engine learned on the last commit**: new words from 快捷造詞
    /// and 自動造詞 and the shorter codes 動態碼長 found, written to yumete's own
    /// 用户詞庫; choices counted, kept for [`Self::save`]; 固頂 changes,
    /// written. Only while learning is on — the queues are emptied either way,
    /// so they do not grow.
    pub(crate) fn collect(&mut self, engine: &mut Engine) {
        let codes = engine.take_new_codes();
        let words = engine.take_new_words();
        let pinyin_words = engine.take_new_pinyin_words();
        self.track_freq(engine);
        self.track_pins(engine);
        if !self.learn {
            return;
        }
        let mut own_changed = false;
        let mut shared_changed = false;
        for (code, word) in codes {
            own_changed |= self.add_line(false, &format!("{word}\t{code}"));
        }
        for word in words {
            own_changed |= self.add_word(engine, false, &word);
        }
        for word in pinyin_words {
            // Words kept by reading go to the shared 用户詞典, with no code:
            // the codes are each scheme's own and the reading is synthesised.
            let shared = !self.shared.is_empty();
            match shared {
                true => shared_changed |= self.add_line(true, &word),
                false => own_changed |= self.add_word(engine, false, &word),
            }
        }
        if own_changed {
            self.write_own(false);
        }
        if shared_changed {
            self.write_own(true);
        }
        if own_changed || shared_changed {
            self.install_words(engine);
        }
    }

    /// The choices counted since the last look, added to yumete's own 調頻.
    fn track_freq(&mut self, engine: &Engine) {
        let learned = self.seen_freq.learned(engine.user_freq());
        let learned_pinyin = match self.shared.is_empty() {
            true => Vec::new(),
            false => self.seen_pinyin.learned(engine.pinyin_freq()),
        };
        if !self.learn {
            return;
        }
        for (code, text, times) in &learned {
            (0..*times).for_each(|_| self.own_freq.note(code, text));
        }
        for (code, text, times) in &learned_pinyin {
            (0..*times).for_each(|_| self.own_pinyin.note(code, text));
        }
        self.freq_unsaved |= !learned.is_empty();
        self.pinyin_unsaved |= !learned_pinyin.is_empty();
    }

    /// **固頂 set or taken away in yumete**, recorded as yumete's own rows. A
    /// pin of the app's taken away cannot leave the app's file, so it is
    /// recorded as yumete's row at place 0 — 「not pinned here」 — which comes
    /// later and so is the one in force.
    fn track_pins(&mut self, engine: &mut Engine) {
        if !engine.pin_dict().dirty() {
            return;
        }
        let now = engine.records(Kind::Pin);
        engine.pin_dict_mut().clear_dirty();
        let seen = std::mem::replace(&mut self.seen_pins, now.clone());
        if !self.learn {
            return;
        }
        let same = |a: &Record, b: &Record| a.code == b.code && a.text == b.text;
        let mut changed = false;
        for row in now.iter().filter(|r| !seen.contains(r)) {
            self.own_pins.set(&row.code, &row.text, place(row.value));
            changed = true;
        }
        let app_rows = self.app_pins.records();
        for row in seen.iter().filter(|r| !now.iter().any(|n| same(n, r))) {
            match app_rows.iter().any(|a| same(a, row) && a.value > 0) {
                true => self.own_pins.set(&row.code, &row.text, 0),
                false => self.own_pins.remove(&row.code, &row.text),
            }
            changed = true;
        }
        if changed {
            self.write_pins();
        }
    }

    /// A new word with the codes the scheme's 構詞法 gives it, or a bare line
    /// when it gives none.
    fn add_word(&mut self, engine: &Engine, shared: bool, word: &str) -> bool {
        let codes = engine.auto_codes_for_text(word);
        if codes.is_empty() {
            return self.add_line(shared, word);
        }
        let mut any = false;
        for code in codes {
            any |= self.add_line(shared, &format!("{word}\t{code}"));
        }
        any
    }

    /// Add `line` to yumete's own 用户詞庫 unless either folder already has it.
    pub(crate) fn add_line(&mut self, shared: bool, line: &str) -> bool {
        let (app, own) = match shared {
            true => (&self.app_shared_dict, &mut self.own_shared_dict),
            false => (&self.app_dict, &mut self.own_dict),
        };
        if entry_lines(app).chain(entry_lines(own)).any(|l| l == line) {
            return false;
        }
        append_line(own, line);
        true
    }

    pub(crate) fn write_own(&mut self, shared: bool) {
        let paths = self.own_paths();
        let (path, text) = match shared {
            true => (paths.shared_dict, &self.own_shared_dict),
            false => (paths.dict, &self.own_dict),
        };
        if let Some(path) = path {
            if write_atomically(&path, text).is_ok() {
                self.restamp(&path);
            }
        }
    }

    fn write_freq(&mut self, shared: bool) {
        let paths = self.own_paths();
        let (path, record) = match shared {
            true => (paths.shared_freq, &self.own_pinyin),
            false => (paths.freq, &self.own_freq),
        };
        let Some(path) = path.filter(|p| !record.is_empty() || p.is_file()) else {
            return;
        };
        if write_atomically(&path, &record.to_text()).is_ok() {
            self.restamp(&path);
            match shared {
                true => self.pinyin_unsaved = false,
                false => self.freq_unsaved = false,
            }
        }
    }

    fn write_pins(&mut self) {
        let Some(path) = self.own_paths().pins else {
            return;
        };
        if (!self.own_pins.is_empty() || path.is_file()) && write_atomically(&path, &self.own_pins.to_text()).is_ok() {
            self.restamp(&path);
        }
    }

    fn restamp(&mut self, path: &Path) {
        let now = modified(path);
        if let Some(stamp) = self.stamps.iter_mut().find(|(p, _)| p == path) {
            stamp.1 = now;
        }
    }

    /// **Take every line of yumete's own 用户詞庫 whose word is `word`** — the
    /// scheme's and the shared one — and write what is left. The app's files
    /// are not touched, so a word the app also has stays.
    fn take_lines(&mut self, engine: &mut Engine, word: &str) -> Vec<(bool, usize, String)> {
        let mut gone = Vec::new();
        for shared in [false, true] {
            let text = match shared {
                true => &mut self.own_shared_dict,
                false => &mut self.own_dict,
            };
            let mut kept = Vec::new();
            let mut taken = Vec::new();
            for (at, line) in lines_of(text).enumerate() {
                match is_entry(line) && word_of(line) == word {
                    true => taken.push((shared, at, line.to_string())),
                    false => kept.push(line),
                }
            }
            if !taken.is_empty() {
                *text = joined(&kept);
                gone.extend(taken);
                self.write_own(shared);
            }
        }
        if !gone.is_empty() {
            self.install_words(engine);
        }
        gone
    }

    /// **Forget a word yumete learned**: every line of its own 用户詞庫 for
    /// `word`. Gives how many lines went.
    pub(crate) fn forget(&mut self, engine: &mut Engine, word: &str) -> usize {
        self.take_lines(engine, word).len()
    }

    /// Write yumete's own 調頻 and 固頂, while learning is on.
    pub(crate) fn save(&mut self, engine: &mut Engine) {
        self.collect(engine);
        if !self.learn {
            return;
        }
        if self.freq_unsaved {
            self.write_freq(false);
        }
        if self.pinyin_unsaved {
            self.write_freq(true);
        }
    }

    /// **Read again when a file changed** since it was read — the app saved,
    /// or somebody edited one by hand. What yumete learned in the meantime is
    /// saved first, so the re-read does not drop it. Gives whether it read.
    pub(crate) fn refresh(&mut self, engine: &mut Engine) -> bool {
        if self.stamps.iter().all(|(path, when)| modified(path) == *when) {
            return false;
        }
        self.save(engine);
        self.read_all(engine);
        true
    }

    // ---- The words page ------------------------------------------------

    /// The lists this scheme has: 拼音調頻 only with a shared 拼音用户詞典.
    pub(crate) fn lists(&self) -> Vec<WordList> {
        let mut out = vec![WordList::Words, WordList::Freq];
        if !self.shared.is_empty() {
            out.push(WordList::PinyinFreq);
        }
        out.push(WordList::Pins);
        out
    }

    /// yumete's own file for a list (for 用户詞庫, the scheme's own one).
    pub(crate) fn own_file(&self, list: WordList) -> Option<PathBuf> {
        let paths = self.own_paths();
        match list {
            WordList::Words => paths.dict,
            WordList::Freq => paths.freq,
            WordList::PinyinFreq => paths.shared_freq,
            WordList::Pins => paths.pins,
        }
    }

    /// **Every row of a list**, both layers: yumete's first (a word both have
    /// says so), then the app's alone.
    pub(crate) fn rows(&self, list: WordList) -> Vec<WordRow> {
        match list {
            WordList::Words => self.word_rows(),
            WordList::Freq => self.freq_rows(false),
            WordList::PinyinFreq => self.freq_rows(true),
            WordList::Pins => self.pin_rows(),
        }
    }

    fn word_rows(&self) -> Vec<WordRow> {
        let own = self.own_paths();
        let app = self.app_paths();
        // Words in file order, each with its codes and its first line.
        type Grouped = Vec<(String, Vec<String>, Option<PathBuf>, usize)>;
        let group = |parts: [(&str, &Option<PathBuf>); 2]| -> Grouped {
            let mut out: Grouped = Vec::new();
            for (text, path) in parts {
                for (at, line) in lines_of(text).enumerate().filter(|(_, l)| is_entry(l)) {
                    let (word, code) = (word_of(line), code_of(line));
                    match out.iter_mut().find(|(w, ..)| w == word) {
                        Some((_, codes, ..)) => {
                            if !code.is_empty() && !codes.iter().any(|c| c == code) {
                                codes.push(code.to_string());
                            }
                        }
                        None => {
                            let codes = (!code.is_empty()).then(|| code.to_string()).into_iter().collect();
                            out.push((word.to_string(), codes, path.clone(), at + 1));
                        }
                    }
                }
            }
            out
        };
        let mine = group([(&self.own_dict, &own.dict), (&self.own_shared_dict, &own.shared_dict)]);
        let theirs = group([(&self.app_dict, &app.dict), (&self.app_shared_dict, &app.shared_dict)]);
        let mut out = Vec::new();
        for (word, codes, file, line) in &mine {
            let other = theirs.iter().find(|(w, ..)| w == word);
            out.push(WordRow {
                word: word.clone(),
                code: codes.join(" "),
                app_code: other.map(|(_, c, ..)| c.join(" ")).unwrap_or_default(),
                own: None,
                app: None,
                held: if other.is_some() { Held::Both } else { Held::Own },
                file: file.clone(),
                line: Some(*line),
                app_file: other.and_then(|(_, _, f, _)| f.clone()),
            });
        }
        let ours: HashSet<&str> = mine.iter().map(|(w, ..)| w.as_str()).collect();
        for (word, codes, file, _) in theirs.iter().filter(|(w, ..)| !ours.contains(w.as_str())) {
            out.push(WordRow {
                word: word.clone(),
                code: codes.join(" "),
                app_code: codes.join(" "),
                own: None,
                app: None,
                held: Held::App,
                file: None,
                line: None,
                app_file: file.clone(),
            });
        }
        out
    }

    fn freq_rows(&self, shared: bool) -> Vec<WordRow> {
        let (mine, theirs) = match shared {
            true => (&self.own_pinyin, &self.app_pinyin),
            false => (&self.own_freq, &self.app_freq),
        };
        let (file, app_file) = match shared {
            true => (self.own_paths().shared_freq, self.app_paths().shared_freq),
            false => (self.own_paths().freq, self.app_paths().freq),
        };
        let app_count: HashMap<(String, String), i64> =
            theirs.records().into_iter().map(|r| ((r.code, r.text), r.value)).collect();
        let mut out = Vec::new();
        let mut ours = HashSet::new();
        for r in mine.records() {
            let app = app_count.get(&(r.code.clone(), r.text.clone())).copied();
            ours.insert((r.code.clone(), r.text.clone()));
            out.push(WordRow {
                word: r.text,
                code: r.code,
                app_code: String::new(),
                own: Some(r.value),
                app,
                held: if app.is_some() { Held::Both } else { Held::Own },
                file: file.clone(),
                line: None,
                app_file: app.and(app_file.clone()),
            });
        }
        for r in theirs.records().into_iter().filter(|r| !ours.contains(&(r.code.clone(), r.text.clone()))) {
            out.push(WordRow {
                word: r.text,
                code: r.code,
                app_code: String::new(),
                own: None,
                app: Some(r.value),
                held: Held::App,
                file: None,
                line: None,
                app_file: app_file.clone(),
            });
        }
        // By code, as the candidates are found; on a code yumete's first, then
        // the most chosen.
        let rank = |h: Held| match h {
            Held::App => 1,
            _ => 0,
        };
        out.sort_by(|a, b| {
            a.code
                .cmp(&b.code)
                .then(rank(a.held).cmp(&rank(b.held)))
                .then(b.own.or(b.app).cmp(&a.own.or(a.app)))
                .then(a.word.cmp(&b.word))
        });
        out
    }

    fn pin_rows(&self) -> Vec<WordRow> {
        let file = self.own_paths().pins;
        let app_file = self.app_paths().pins;
        let theirs = self.app_pins.records();
        let same = |a: &Record, b: &Record| a.code == b.code && a.text == b.text;
        let mine = self.own_pins.records();
        let mut out = Vec::new();
        for r in &mine {
            let app = theirs.iter().find(|t| same(t, r)).map(|t| t.value);
            out.push(WordRow {
                word: r.text.clone(),
                code: r.code.clone(),
                app_code: String::new(),
                own: Some(r.value),
                app,
                held: if app.is_some() { Held::Both } else { Held::Own },
                file: file.clone(),
                line: None,
                app_file: app.and(app_file.clone()),
            });
        }
        for r in theirs.iter().filter(|t| !mine.iter().any(|m| same(m, t))) {
            out.push(WordRow {
                word: r.text.clone(),
                code: r.code.clone(),
                app_code: String::new(),
                own: None,
                app: Some(r.value),
                held: Held::App,
                file: None,
                line: None,
                app_file: app_file.clone(),
            });
        }
        out
    }

    /// **Delete yumete's copy of a row**: a 用户詞庫 word with all its codes,
    /// or one 調頻 or 固頂 row. Written at once and in the engine at once; the
    /// app's files are never touched, so a row both have stays as the app's.
    /// `None` when yumete holds nothing for it.
    pub(crate) fn delete(&mut self, engine: &mut Engine, list: WordList, row: &WordRow) -> Option<Deleted> {
        if row.held == Held::App {
            return None;
        }
        // What the last commits taught goes in first, so it is not lost when
        // the engine's tables are laid again.
        self.collect(engine);
        let gone = match list {
            WordList::Words => {
                let lines = self.take_lines(engine, &row.word);
                (!lines.is_empty()).then_some(Gone::Lines(lines))?
            }
            WordList::Freq | WordList::PinyinFreq => {
                let shared = list == WordList::PinyinFreq;
                let record = match shared {
                    true => &mut self.own_pinyin,
                    false => &mut self.own_freq,
                };
                let tick = record.tick();
                let taken = record.take(&row.code, &row.word)?;
                self.write_freq(shared);
                self.install_freq(engine);
                Gone::Freq { shared, code: row.code.clone(), text: row.word.clone(), row: taken, tick }
            }
            WordList::Pins => {
                let mut records = self.own_pins.records();
                let at = records.iter().position(|r| r.code == row.code && r.text == row.word)?;
                let record = records.remove(at);
                self.own_pins.set_records(&records);
                self.write_pins();
                self.install_pins(engine);
                Gone::Pin { at, record }
            }
        };
        Some(Deleted { tag: self.tag.clone(), list, gone })
    }

    /// **Put back what a delete took**, where it stood. `false` when it was
    /// taken from another scheme's lists.
    pub(crate) fn restore(&mut self, engine: &mut Engine, deleted: Deleted) -> bool {
        if deleted.tag != self.tag {
            return false;
        }
        self.collect(engine);
        match deleted.gone {
            Gone::Lines(lines) => {
                for shared in [false, true] {
                    let mut back: Vec<&(bool, usize, String)> = lines.iter().filter(|l| l.0 == shared).collect();
                    if back.is_empty() {
                        continue;
                    }
                    back.sort_by_key(|l| l.1);
                    let text = match shared {
                        true => &mut self.own_shared_dict,
                        false => &mut self.own_dict,
                    };
                    let mut all: Vec<String> = lines_of(text).map(str::to_string).collect();
                    for (_, at, line) in back {
                        all.insert((*at).min(all.len()), line.clone());
                    }
                    *text = joined(&all.iter().map(String::as_str).collect::<Vec<_>>());
                    self.write_own(shared);
                }
                self.install_words(engine);
            }
            Gone::Freq { shared, code, text, row, tick } => {
                match shared {
                    true => self.own_pinyin.absorb(&code, &text, row, tick),
                    false => self.own_freq.absorb(&code, &text, row, tick),
                }
                self.write_freq(shared);
                self.install_freq(engine);
            }
            Gone::Pin { at, record } => {
                let mut records = self.own_pins.records();
                records.insert(at.min(records.len()), record);
                self.own_pins.set_records(&records);
                self.write_pins();
                self.install_pins(engine);
            }
        }
        true
    }
}

/// The five files of one folder for a scheme.
#[derive(Debug, Default)]
struct Paths {
    dict: Option<PathBuf>,
    shared_dict: Option<PathBuf>,
    freq: Option<PathBuf>,
    shared_freq: Option<PathBuf>,
    pins: Option<PathBuf>,
}

impl Paths {
    fn all(&self) -> impl Iterator<Item = &Path> {
        [&self.dict, &self.shared_dict, &self.freq, &self.shared_freq, &self.pins]
            .into_iter()
            .filter_map(|p| p.as_deref())
    }
}
