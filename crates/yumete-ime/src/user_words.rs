//! **The Yume app's word lists, read; yumete's own, kept.**
//!
//! A writer who types in the system Yume has taught it words (用户詞庫), pinned
//! candidates (固頂) and an order of their own (調頻). Typing the same scheme
//! in yumete should answer with the same words, so those files are read — and
//! only read: the app owns them, rewrites them whole, and a second writer
//! racing it would lose somebody's words.
//!
//! What yumete learns goes to files of its own, in the same formats, under
//! yumete's data directory, and is loaded **after** the app's, so on a key both
//! know yumete's record is the one in force:
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
//! `autowords.txt` (自動造詞's running counts) is the app's working state and is
//! not read: a word it has finished counting is already in the 用户詞庫.
//!
//! **The formats are yume-core's.** 調頻 and 固頂 go through
//! [`UserFreq`] and [`PinDict`] both ways. The 用户詞庫 is loaded by the
//! engine's own loader; the one thing taken out of it here is the word column,
//! for the whitelist that lets a word with no code (every 拼音 word) into the
//! sentence lattice — the core has no reader that returns it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use yume_core::pin_dict::PinDict;
use yume_core::user_freq::{Row, UserFreq};
use yume_core::user_records::{Kind, Record};
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

/// The non-empty, non-comment lines of a 用户詞庫.
fn entry_lines(text: &str) -> impl Iterator<Item = &str> {
    text.lines()
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
}

/// A 用户詞庫 line's word: everything before the first tab. A line with no tab
/// is a word with no code, and may hold a space.
fn word_of(line: &str) -> &str {
    line.split('\t').next().unwrap_or(line).trim()
}

fn append_line(text: &mut String, line: &str) {
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
}

/// Rows of a 調頻 record, by key.
type Rows = HashMap<(String, String), Row>;

fn rows_of(freq: &UserFreq) -> Rows {
    freq.rows().map(|(k, r)| (k.clone(), *r)).collect()
}

/// The rows of `now` that `base` does not hold as they are, as a record of
/// their own with the same clock.
fn changed(now: &UserFreq, base: &Rows) -> UserFreq {
    let mut out = UserFreq::new();
    for ((code, text), row) in now.rows() {
        if base.get(&(code.clone(), text.clone())) != Some(row) {
            out.absorb(code, text, *row, now.tick());
        }
    }
    out
}

/// Lay `own`'s rows over `into`'s: a key both hold is `own`'s.
fn overlay(into: &mut UserFreq, own: &UserFreq) {
    for ((code, text), row) in own.rows() {
        into.take(code, text);
        into.absorb(code, text, *row, own.tick());
    }
}

/// The two folders' files for one scheme, and what was read from them.
#[derive(Debug)]
pub(crate) struct UserWords {
    roots: UserWordRoots,
    tag: String,
    /// The shared 拼音用户詞典's name; empty when the scheme has none.
    shared: String,
    app_dict: String,
    app_shared_dict: String,
    own_dict: String,
    own_shared_dict: String,
    /// The engine's 調頻 right after the app's file went in, before yumete's
    /// own: what is different from this at save time is yumete's.
    freq_base: Rows,
    pinyin_base: Rows,
    pin_base: Vec<Record>,
    /// Every file this looks at, with the modification time it had when read
    /// (`None`: not there), so a change made by the app is noticed.
    stamps: Vec<(PathBuf, Option<SystemTime>)>,
    read: Vec<ReadFile>,
}

impl UserWords {
    /// Read both folders' files for `tag` into `engine`.
    pub(crate) fn load(engine: &mut Engine, roots: UserWordRoots, tag: &str) -> Self {
        let mut me = UserWords {
            roots,
            tag: tag.to_string(),
            shared: engine.pinyin_user_dict().to_string(),
            app_dict: String::new(),
            app_shared_dict: String::new(),
            own_dict: String::new(),
            own_shared_dict: String::new(),
            freq_base: Rows::new(),
            pinyin_base: Rows::new(),
            pin_base: Vec::new(),
            stamps: Vec::new(),
            read: Vec::new(),
        };
        me.read_all(engine);
        me
    }

    fn app_paths(&self) -> AppPaths {
        let Some(app) = &self.roots.app else {
            return AppPaths::default();
        };
        let shared = (!self.shared.is_empty()).then_some(self.shared.as_str());
        AppPaths {
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

    fn own_paths(&self) -> AppPaths {
        AppPaths {
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
        // moves rows keyed by reading into it as it is read.
        let app_shared_freq = read(app.shared_freq.as_deref());
        if !self.shared.is_empty() {
            engine.clear_pinyin_freq();
            if !app_shared_freq.is_empty() {
                engine.load_pinyin_freq(&app_shared_freq);
            }
        }
        engine.load_user_freq(&read(app.freq.as_deref()));
        self.freq_base = rows_of(engine.user_freq());
        self.pinyin_base = rows_of(engine.pinyin_freq());
        for (path, mine, shared) in [
            (&app.shared_freq, false, true),
            (&app.freq, false, false),
            (&own.shared_freq, true, true),
            (&own.freq, true, false),
        ] {
            let Some(path) = path.as_ref().filter(|p| p.is_file()) else {
                continue;
            };
            let mut record = UserFreq::new();
            record.load_text(&read(Some(path)));
            self.read.push(ReadFile { path: path.clone(), entries: record.len(), own: mine });
            if mine {
                match shared {
                    true => overlay(engine.pinyin_freq_mut(), &record),
                    false => overlay(engine.user_freq_mut(), &record),
                }
            }
        }
        engine.user_freq_mut().clear_dirty();
        engine.pinyin_freq_mut().clear_dirty();

        // 固頂: later rows win on a code, so the app's go first.
        let app_pins = read(app.pins.as_deref());
        let own_pins = read(own.pins.as_deref());
        engine.pin_dict_mut().load_text(&app_pins);
        self.pin_base = engine.records(Kind::Pin);
        let app_pin_count = self.pin_base.len();
        let mut both = app_pins;
        append_line(&mut both, "");
        both.push_str(&own_pins);
        engine.pin_dict_mut().load_text(&both);
        engine.pin_dict_mut().clear_dirty();
        if let Some(path) = app.pins.as_ref().filter(|p| p.is_file()) {
            self.read.push(ReadFile { path: path.clone(), entries: app_pin_count, own: false });
        }
        if let Some(path) = own.pins.as_ref().filter(|p| p.is_file()) {
            let mut own_record = PinDict::new();
            own_record.load_text(&own_pins);
            let entries = own_record.to_text().lines().count();
            self.read.push(ReadFile { path: path.clone(), entries, own: true });
        }
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

    /// The files read, for `:yume-where`.
    pub(crate) fn files_read(&self) -> &[ReadFile] {
        &self.read
    }

    /// **What the engine learned on the last commit**, written to yumete's own
    /// 用户詞庫 — new words from 快捷造詞 and 自動造詞, the shorter codes
    /// 動態碼長 found — and 固頂 changes. The queues are emptied either way, so
    /// they do not grow.
    pub(crate) fn collect(&mut self, engine: &mut Engine) {
        let codes = engine.take_new_codes();
        let words = engine.take_new_words();
        let pinyin_words = engine.take_new_pinyin_words();
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
        if engine.pin_dict().dirty() {
            self.save_pins(engine);
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

    fn restamp(&mut self, path: &Path) {
        let now = modified(path);
        if let Some(stamp) = self.stamps.iter_mut().find(|(p, _)| p == path) {
            stamp.1 = now;
        }
    }

    /// **Forget a word yumete learned**: every line of yumete's own 用户詞庫
    /// (the scheme's and the shared one) whose word is `word`. The app's files
    /// are not touched, so a word the app also has stays. Gives how many lines
    /// went.
    pub(crate) fn forget(&mut self, engine: &mut Engine, word: &str) -> usize {
        let mut gone = 0;
        for shared in [false, true] {
            let text = match shared {
                true => &mut self.own_shared_dict,
                false => &mut self.own_dict,
            };
            let before = entry_lines(text).count();
            let kept: Vec<&str> = text.lines().filter(|l| word_of(l) != word).collect();
            let mut out = kept.join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            let went = before - entry_lines(&out).count();
            if went > 0 {
                *text = out;
                gone += went;
                self.write_own(shared);
            }
        }
        if gone > 0 {
            self.install_words(engine);
        }
        gone
    }

    /// Write yumete's own 調頻 and 固頂: what differs from the app's.
    pub(crate) fn save(&mut self, engine: &mut Engine) {
        self.collect(engine);
        let paths = self.own_paths();
        for (shared, path) in [(false, paths.freq), (true, paths.shared_freq)] {
            let dirty = match shared {
                true => engine.pinyin_freq().dirty(),
                false => engine.user_freq().dirty(),
            };
            if !dirty {
                continue;
            }
            let mine = match shared {
                true => changed(engine.pinyin_freq(), &self.pinyin_base),
                false => changed(engine.user_freq(), &self.freq_base),
            };
            if let Some(path) = path.filter(|p| !mine.is_empty() || p.is_file()) {
                if write_atomically(&path, &mine.to_text()).is_ok() {
                    self.restamp(&path);
                }
            }
            match shared {
                true => engine.pinyin_freq_mut().clear_dirty(),
                false => engine.user_freq_mut().clear_dirty(),
            }
        }
    }

    fn save_pins(&mut self, engine: &mut Engine) {
        let mut mine = PinDict::new();
        for row in engine.records(Kind::Pin) {
            if !self.pin_base.contains(&row) {
                mine.set(&row.code, &row.text, usize::try_from(row.value).unwrap_or(0));
            }
        }
        if let Some(path) = self.own_paths().pins {
            if (!mine.is_empty() || path.is_file()) && write_atomically(&path, &mine.to_text()).is_ok() {
                self.restamp(&path);
            }
        }
        engine.pin_dict_mut().clear_dirty();
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
}

/// The five files of one folder for a scheme.
#[derive(Debug, Default)]
struct AppPaths {
    dict: Option<PathBuf>,
    shared_dict: Option<PathBuf>,
    freq: Option<PathBuf>,
    shared_freq: Option<PathBuf>,
    pins: Option<PathBuf>,
}

impl AppPaths {
    fn all(&self) -> impl Iterator<Item = &Path> {
        [&self.dict, &self.shared_dict, &self.freq, &self.shared_freq, &self.pins]
            .into_iter()
            .filter_map(|p| p.as_deref())
    }
}
