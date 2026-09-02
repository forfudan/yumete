//! `yumete-ime` — the built-in Yume IME session (Feature #27, #31, #32).
//!
//! yumete embeds the `yume-core` engine directly, with no FFI, since both are
//! Rust. This crate wraps a `yume-core` [`Engine`] in an [`ImeSession`] that the
//! editor drives per keystroke while composing CJK in Insert mode, and loads a
//! scheme's compiled data tables from the data directory (Feature #32). It stays
//! UI-agnostic: it returns candidate and preedit data for the TUI to render.
//!
//! **Which files make up a data set is not decided here.** `yume_core::data_manifest`
//! is the one place that says so, precisely so every frontend stops keeping a
//! copy that quietly rots — see yume's `docs/development.md` §4.1.1. This crate
//! walks that manifest, so when yume adds a table yumete picks it up with no
//! change beyond `scripts/build.sh`, which compiles the same list.
//!
//! Point yumete's data directory at the compiled artifacts (see
//! [`yumete_config::data_search_dirs`]); `scripts/build.sh` fills
//! `~/.local/share/yumete` from the sibling yume checkout. A data set built by an
//! older Yume will not load — the binary formats move with `yume-core` — so
//! recompile rather than copying an old `Resources` across. When the scheme's own
//! dictionary is missing the session still constructs but reports
//! [`ImeSession::available`] as `false`, so the editor falls back to plain input.

pub mod segment;

use std::path::PathBuf;
use std::sync::Arc;

use yume_core::data_manifest::{self, DataFile, DataKind};
use yume_core::division::DivisionTable;
use yume_core::key_bindings::{FuncKey, KeyAction};
use yume_core::lexicon::Lexicon;
use yume_core::zigen::ZigenTable;
use yume_core::{
    AnnotationTable, Charset, CodeTable, Engine, FluencyTable, UnigramTable, NAMED_CHARSETS,
};

pub use segment::YumeSegmenter;
pub use yume_core::DisplayMode;

/// One of yumete's five input schemes (方案).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scheme {
    /// 靈明 (Lingming) — the default shape scheme.
    Lingming,
    /// 星陳 (Xingchen) — shape scheme.
    Xingchen,
    /// 卿雲 (Qingyun) — shape scheme.
    Qingyun,
    /// 日月 (Riyue) — shape scheme.
    Riyue,
    /// 拼音 (Pinyin) — the phonetic, fluency-only scheme.
    Pinyin,
}

impl Scheme {
    /// All schemes, in menu order.
    pub const ALL: [Scheme; 5] = [
        Scheme::Lingming,
        Scheme::Xingchen,
        Scheme::Qingyun,
        Scheme::Riyue,
        Scheme::Pinyin,
    ];

    /// The canonical scheme tag understood by `yume-core`.
    pub fn tag(self) -> &'static str {
        match self {
            Scheme::Lingming => "lingming",
            Scheme::Xingchen => "xingchen",
            Scheme::Qingyun => "qingyun",
            Scheme::Riyue => "riyue",
            Scheme::Pinyin => "pinyin",
        }
    }

    /// Parse a scheme from a tag (canonical or a common legacy alias).
    pub fn from_tag(tag: &str) -> Option<Scheme> {
        match tag.trim().to_ascii_lowercase().as_str() {
            "lingming" | "ling" | "靈明" => Some(Scheme::Lingming),
            "xingchen" | "xing" | "星陳" => Some(Scheme::Xingchen),
            "qingyun" | "qing" | "卿雲" => Some(Scheme::Qingyun),
            "riyue" | "日月" => Some(Scheme::Riyue),
            "pinyin" | "拼音" => Some(Scheme::Pinyin),
            _ => None,
        }
    }

    /// The next scheme, cycling in menu order.
    pub fn next(self) -> Scheme {
        let idx = Scheme::ALL.iter().position(|&s| s == self).unwrap_or(0);
        Scheme::ALL[(idx + 1) % Scheme::ALL.len()]
    }
}

/// A candidate as shown in the panel: the text plus its inline annotations.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Candidate {
    /// The candidate text (漢字 or word).
    pub text: String,
    /// The remaining-letters 下標 (completion hint), including a `␣` mark when an
    /// explicit Space is needed. Empty for an exact match.
    pub completion: String,
    /// The 拆分 (chaifen) decomposition comment, when annotations are enabled.
    pub comment: String,
    /// The 陸臺港 (G/T/H) source tag, when applicable.
    pub source_tag: String,
    /// The 簡碼 (shortcut-code) hint (`=k` form), when applicable.
    pub simp_code: String,
}

/// A live IME session wrapping a `yume-core` [`Engine`].
pub struct ImeSession {
    engine: Engine,
    scheme: Scheme,
    data_dirs: Vec<PathBuf>,
    available: bool,
    /// Whether the 拆分 annotation is on (Feature #66). Off by default: it is
    /// a study aid, and it widens every candidate.
    annotations: bool,
    /// Whether the 碼表 in use is this binary's rather than one found on disk.
    /// Worth knowing: a writer who has just installed a newer 靈明 and is still
    /// seeing the old candidates deserves to be told which one is answering.
    builtin: bool,
}

impl ImeSession {
    /// Build a session for `scheme`, loading its data tables from `data_dirs`
    /// (the first directory containing a given file wins). [`available`] is
    /// `false` when the scheme's essential table could not be loaded, but the
    /// engine is still constructed so the editor can degrade gracefully.
    ///
    /// [`available`]: ImeSession::available
    pub fn new(scheme: Scheme, data_dirs: Vec<PathBuf>) -> Self {
        let (mut engine, mut available) = build_engine(scheme, &data_dirs);
        // Nothing installed. Rather than an editor that cannot type 漢字 until
        // somebody clones the 宇浩 source tree, 靈明's own 碼表 stands in when
        // this binary was built on a machine that had it.
        let mut builtin = false;
        if !available && scheme == Scheme::Lingming {
            available = load_builtin(&mut engine);
            builtin = available;
        }
        ImeSession {
            engine,
            scheme,
            data_dirs,
            available,
            annotations: false,
            builtin,
        }
    }

    /// A session using the 碼表 in the binary, whatever is installed.
    ///
    /// The escape hatch the fallback creates a need for: an installed table
    /// that is broken, or older than this binary's, or simply not the one you
    /// meant. Without it the only way back to a known 靈明 would be moving
    /// files about outside the editor.
    pub fn builtin_lingming() -> Self {
        let dirs = yumete_config::data_search_dirs();
        let mut engine = Engine::new(CodeTable::new());
        // The language layer still comes from disk where it is: it is not what
        // was being overridden, and it is what makes the candidates sensible.
        for file in data_manifest::shared() {
            if !matches!(file.kind, DataKind::Table | DataKind::Symbols) {
                load_data_file(&mut engine, &dirs, &file);
            }
        }
        let available = load_builtin(&mut engine);
        engine.set_scheme_by_tag(Scheme::Lingming.tag());
        ImeSession {
            engine,
            scheme: Scheme::Lingming,
            data_dirs: dirs,
            available,
            annotations: false,
            builtin: true,
        }
    }

    /// Whether the 碼表 answering is the one in the binary.
    pub fn is_builtin(&self) -> bool {
        self.builtin
    }

    /// Where the 碼表 in use came from, for `:yume` to say.
    pub fn table_source(&self) -> String {
        if !self.available {
            return "沒有碼表".to_string();
        }
        if self.builtin {
            return "出廠自帶".to_string();
        }
        // The manifest knows which file this scheme's 碼表 is; asking it beats
        // guessing at the name.
        data_manifest::for_scheme(self.scheme.tag())
            .into_iter()
            .find(|f| f.kind == DataKind::Table)
            .and_then(|f| find_file(&self.data_dirs, &f.file))
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "已安裝".to_string())
    }

    /// Build a session using [`yumete_config::data_search_dirs`].
    pub fn from_default_dirs(scheme: Scheme) -> Self {
        ImeSession::new(scheme, yumete_config::data_search_dirs())
    }

    /// Just the language model: the word frequencies and the 詞彙表.
    ///
    /// These two files are **scheme-independent** — they are the language, not
    /// the input method — and they are what `w`, `b` and `e` walk by. So they
    /// are worth having from the first keystroke even in a session that never
    /// types a 漢字, while the 碼表 that would let you *type* one is not.
    ///
    /// The session is not `available`: no code table, so Insert types ASCII.
    pub fn language_only(scheme: Scheme) -> Self {
        let dirs = yumete_config::data_search_dirs();
        let mut engine = Engine::new(CodeTable::new());
        for file in data_manifest::shared() {
            if matches!(file.kind, DataKind::Weights | DataKind::Lexicon) {
                load_data_file(&mut engine, &dirs, &file);
            }
        }
        ImeSession {
            engine,
            scheme,
            data_dirs: dirs,
            available: false,
            annotations: false,
            builtin: false,
        }
    }

    /// A session with no tables in it, and no intention of loading any here.
    ///
    /// What stands in while the real one is read on another thread. It is
    /// deliberately the *same* thing an uninstalled scheme produces — not
    /// available — so nothing downstream needs a third state to think about:
    /// Insert mode types plain ASCII, and that is all.
    pub fn empty(scheme: Scheme) -> Self {
        ImeSession {
            engine: Engine::new(CodeTable::new()),
            scheme,
            data_dirs: Vec::new(),
            available: false,
            annotations: false,
            builtin: false,
        }
    }

    /// Wrap a pre-built engine (used in tests and by callers assembling their own
    /// tables). The session is marked available.
    pub fn from_engine(engine: Engine, scheme: Scheme) -> Self {
        ImeSession {
            engine,
            scheme,
            data_dirs: Vec::new(),
            available: true,
            annotations: false,
            builtin: false,
        }
    }

    /// Build a session from an inline code table (`code text` / `text<TAB>code`
    /// lines), with no annotation or fluency data. Handy for a custom user table
    /// and for tests. The session is marked available.
    pub fn from_table_text(scheme: Scheme, table_text: &str) -> Self {
        let mut table = CodeTable::new();
        table.load_text(table_text);
        let mut engine = Engine::new(table);
        engine.set_scheme_by_tag(scheme.tag());
        ImeSession {
            engine,
            scheme,
            data_dirs: Vec::new(),
            available: true,
            annotations: false,
            builtin: false,
        }
    }

    /// Whether the scheme's data tables loaded successfully.
    pub fn available(&self) -> bool {
        self.available
    }

    /// The active scheme.
    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    /// The active scheme's display name (方案名, e.g. `靈明`).
    pub fn scheme_name(&self) -> &str {
        self.engine.scheme_name()
    }

    /// Switch to `scheme`, reloading its tables from the session's data
    /// directories. Returns [`available`](ImeSession::available) after the swap.
    pub fn set_scheme(&mut self, scheme: Scheme) -> bool {
        let (engine, available) = build_engine(scheme, &self.data_dirs);
        self.engine = engine;
        self.scheme = scheme;
        self.available = available;
        available
    }

    // ---- Per-keystroke input lifecycle ------------------------------------

    /// Feed one printable character to the engine.
    pub fn input(&mut self, ch: char) {
        self.engine.input(ch);
    }

    /// Space: commit the highlighted candidate (or the raw buffer if none).
    pub fn space(&mut self) {
        self.engine.space();
    }

    /// Enter: commit the raw buffer as-is.
    pub fn enter(&mut self) {
        self.engine.enter();
    }

    /// Backspace: remove the last buffer character. Returns `true` if the buffer
    /// was non-empty (so the host should swallow the key).
    pub fn backspace(&mut self) -> bool {
        self.engine.backspace()
    }

    /// Escape: clear the buffer and return to normal mode.
    pub fn escape(&mut self) {
        self.engine.escape();
    }

    /// Select the candidate at index `i` within the current page.
    pub fn select_in_page(&mut self, i: usize) {
        self.engine.select_in_page(i);
    }

    /// Press one of the keys the scheme binds to a function — `;` `'` `-` `=`
    /// — and say whether the engine took it.
    ///
    /// The binding table is yume's, not yumete's: 靈明 puts 選二 on `;` and 選三
    /// on `'`, which a 形碼 writer presses hundreds of times a day, and yumete
    /// used to send them straight to the engine's punctuation path — committing
    /// the first candidate and dropping a `；` into the manuscript. Asking
    /// `key_action` also hands us the scheme-deference rule for free: a custom
    /// 碼表 that uses `-` as a code key gets its `-` back.
    ///
    /// `false` means the key is the host's — pass it on untouched.
    pub fn press_func(&mut self, ch: char) -> bool {
        let key = match ch {
            ';' => FuncKey::Semicolon,
            '\'' => FuncKey::Quote,
            '-' => FuncKey::Minus,
            '=' => FuncKey::Equal,
            _ => return false,
        };
        match self.engine.key_action(key) {
            KeyAction::Native => false,
            KeyAction::Noop => true,
            KeyAction::SelectSecond => {
                self.engine.select_second();
                true
            }
            KeyAction::SelectThird => {
                self.engine.select_third();
                true
            }
            KeyAction::PageUp => {
                self.engine.page_up();
                true
            }
            KeyAction::PageDown => {
                self.engine.page_down();
                true
            }
            // Everything else this key can mean — punctuation, number mode, a
            // manual split — the engine already implements on the key itself.
            _ => {
                self.engine.input(ch);
                true
            }
        }
    }

    /// Whether the *nth* candidate (1-based) is actually on the page shown.
    ///
    /// The page holds `page_size` candidates, which is configurable, and the
    /// last page is usually short — so a digit key is only a selection when
    /// there is something under it. yume-core owns this rule; asking it is how
    /// the panel and the keyboard stay in agreement about what the reader can
    /// see.
    pub fn page_has(&self, n: usize) -> bool {
        self.engine.page_has_index(n)
    }

    /// Previous candidate page.
    pub fn page_up(&mut self) {
        self.engine.page_up();
    }

    /// Next candidate page.
    pub fn page_down(&mut self) {
        self.engine.page_down();
    }

    /// Move the highlight by a signed offset (wrapping across pages).
    pub fn move_highlight(&mut self, delta: i64) {
        self.engine.move_highlight(delta);
    }

    /// Toggle 中/英 (Chinese ⇄ ASCII).
    pub fn toggle_language(&mut self) {
        self.engine.toggle_language();
    }

    // ---- State & display --------------------------------------------------

    /// Whether the engine is in Chinese mode (vs. pass-through ASCII).
    pub fn is_chinese(&self) -> bool {
        self.engine.is_chinese()
    }

    /// Whether there is an in-progress composition (a non-empty buffer).
    pub fn is_composing(&self) -> bool {
        !self.engine.buffer().is_empty()
    }

    /// The raw composing buffer.
    pub fn buffer(&self) -> &str {
        self.engine.buffer()
    }

    /// The preedit to display (segmented 整句 with separators, or the raw buffer).
    pub fn display_buffer(&self) -> String {
        self.engine.display_buffer()
    }

    /// Take and clear any committed text produced by the last action.
    pub fn take_committed(&mut self) -> String {
        self.engine.take_committed()
    }

    /// The current display mode (Latin / Normal / Raw / Reverse / Sentence /
    /// Unicode / Number).
    pub fn display_mode(&self) -> DisplayMode {
        self.engine.display_mode()
    }

    /// Whether the engine is in number mode.
    pub fn is_number(&self) -> bool {
        self.engine.is_number()
    }

    /// Whether the buffer is a `/`-led special command.
    pub fn is_special(&self) -> bool {
        self.engine.is_special()
    }

    /// Whether the buffer is a `z`-led reverse lookup.
    pub fn is_reverse(&self) -> bool {
        self.engine.is_reverse()
    }

    /// Whether the engine is in 整句 (sentence) mode.
    pub fn is_sentence(&self) -> bool {
        self.engine.is_sentence()
    }

    /// The highlight index within the current page.
    pub fn highlight(&self) -> usize {
        self.engine.highlight()
    }

    /// The current page index (0-based).
    pub fn page(&self) -> usize {
        self.engine.page()
    }

    /// The total number of candidate pages.
    pub fn page_count(&self) -> usize {
        self.engine.page_count()
    }

    /// The candidates on the current page, with their inline annotations.
    pub fn page_candidates(&self) -> Vec<Candidate> {
        let texts = self.engine.page_candidates();
        let completions = self.engine.page_completions();
        let comments = self.engine.page_comments();
        let tags = self.engine.page_source_tags();
        let simps = self.engine.page_simp_codes();
        texts
            .into_iter()
            .enumerate()
            .map(|(i, text)| Candidate {
                text,
                completion: completions.get(i).cloned().unwrap_or_default(),
                comment: comments.get(i).cloned().unwrap_or_default(),
                source_tag: tags.get(i).cloned().unwrap_or_default(),
                simp_code: simps.get(i).cloned().unwrap_or_default(),
            })
            .collect()
    }

    /// Whether this scheme can annotate at all — 拼音 has no 拆分 layer.
    pub fn annotations_available(&self) -> bool {
        self.engine.comments_available()
    }

    /// Whether the 拆分 annotation is switched on.
    pub fn annotations_enabled(&self) -> bool {
        self.annotations
    }

    /// Switch the 拆分 annotation on or off, returning whether it is now on.
    ///
    /// The engine keeps a *table* of annotation profiles and an index into it.
    /// Profile 1 is 二重注解 — the 拆分 and its code, which is what `:chaifen`
    /// means. Profile 0 is 多重注解: 拆分 plus 分節碼 plus every reading plus the
    /// character sets plus the code point, some fifty cells wide, which in a
    /// terminal panel is not an annotation but a wall. A scheme that cannot
    /// render annotations stays off whatever is asked.
    pub fn set_annotations(&mut self, on: bool) -> bool {
        self.annotations = on && self.engine.comments_available();
        self.engine
            .set_comment_mode(if self.annotations { Some(1) } else { None });
        self.annotations
    }

    /// A word segmenter over this session's language model (Feature #63).
    ///
    /// The 詞頻表 and 詞彙表 are already loaded and reference-counted, so the
    /// segmenter shares them rather than holding a second copy of 1.25M entries.
    pub fn segmenter(&self) -> YumeSegmenter {
        YumeSegmenter::new(self.engine.unigram.clone(), self.engine.lexicon.clone())
    }

    /// The candidate page size (每頁候選數).
    pub fn page_size(&self) -> usize {
        self.engine.page_size
    }

    /// Set the candidate page size.
    pub fn set_page_size(&mut self, size: usize) {
        self.engine.page_size = size.max(1);
    }
}

/// Resolve one manifest entry's relative path (`charsets/common.ycs`) against
/// the search path, first directory wins.
fn find_file(dirs: &[PathBuf], relative: &str) -> Option<PathBuf> {
    for dir in dirs {
        let mut path = dir.clone();
        // Manifest paths always use forward slashes, whatever the host.
        for part in relative.split('/') {
            path.push(part);
        }
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// 靈明's 碼表, put here at build time by `build.rs` when the machine that
/// built this binary had it installed — `None` when it did not.
///
/// Everything else yume needs is data a writer installs; this one file is the
/// difference between "an editor that types Chinese" and "an editor that will
/// type Chinese once you have cloned another repository and run a script". It
/// is never committed: see `build.rs` for why, and for where it is found.
mod builtin {
    include!(concat!(env!("OUT_DIR"), "/builtin.rs"));
}

/// Put the built-in 靈明 tables into `engine`, reporting whether it can type.
fn load_builtin(engine: &mut Engine) -> bool {
    let Some(bytes) = builtin::BUILTIN_TABLE else {
        return false;
    };
    let mut table = CodeTable::new();
    if table.load_binary_bytes(bytes).is_err() {
        return false;
    }
    engine.set_table(Arc::new(table));
    if let Some(bytes) = builtin::BUILTIN_SYMBOLS {
        let mut symbols = CodeTable::new();
        if symbols.load_binary_bytes(bytes).is_ok() {
            engine.set_symbol_table(symbols);
        }
    }
    true
}

/// Whether this binary carries a 碼表 at all.
pub fn has_builtin_table() -> bool {
    builtin::BUILTIN_TABLE.is_some()
}

/// Load one entry of the factory data set into `engine`.
///
/// This is yumete's copy of the one dispatch every Yume frontend has — the
/// `kind` of a [`DataFile`] says which door of the engine it goes through. An
/// unrecognised `kind` is skipped rather than treated as an error, so a yumete
/// built against an older `yume-core` still starts against newer data.
fn load_data_file(engine: &mut Engine, dirs: &[PathBuf], file: &DataFile) -> bool {
    let Some(path) = find_file(dirs, &file.file) else {
        return false;
    };
    let Some(p) = path.to_str() else {
        return false;
    };
    match file.kind {
        DataKind::Table => {
            let mut table = CodeTable::new();
            if table.load_binary(p).is_err() {
                return false;
            }
            engine.set_table(Arc::new(table));
        }
        DataKind::Symbols => {
            let mut table = CodeTable::new();
            if table.load_binary(p).is_err() {
                return false;
            }
            engine.set_symbol_table(table);
        }
        DataKind::PinyinTable => {
            let mut fluency = FluencyTable::new();
            if fluency.load_binary(p).is_err() {
                return false;
            }
            engine.set_pinyin_table(Arc::new(fluency));
        }
        DataKind::Weights => {
            let mut unigram = UnigramTable::new();
            if unigram.load_binary(p).is_err() {
                return false;
            }
            engine.unigram = Arc::new(unigram);
        }
        DataKind::Lexicon => {
            let mut lexicon = Lexicon::new();
            if lexicon.load_binary(p).is_err() {
                return false;
            }
            engine.lexicon = Arc::new(lexicon);
        }
        DataKind::Annotations => {
            // 全息拆分表 plus this scheme's 字根表; the divisions are shared by
            // every scheme, and pinyin takes them with no 字根表 at all.
            let Ok(divisions) = std::fs::read(&path)
                .map_err(drop)
                .and_then(|b| DivisionTable::from_binary(&b).map_err(drop))
            else {
                return false;
            };
            let zigen = match find_file(dirs, &file.aux) {
                Some(aux) => match std::fs::read(&aux)
                    .map_err(drop)
                    .and_then(|b| ZigenTable::from_binary(&b).map_err(drop))
                {
                    Ok(z) => z,
                    Err(()) => return false,
                },
                None => ZigenTable::default(),
            };
            engine.set_annotations(AnnotationTable::with_tables(Arc::new(divisions), zigen));
        }
        DataKind::Charset => {
            let Some(&id) = usize::try_from(file.slot)
                .ok()
                .and_then(|i| NAMED_CHARSETS.get(i))
            else {
                return false;
            };
            let mut charset = Charset::new(id);
            if charset.load_binary(p).is_err() {
                return false;
            }
            return engine.set_named_charset(id, charset);
        }
        DataKind::Words => match yume_core::word_whitelist::load_binary(p) {
            Ok(words) => engine.set_word_whitelist(words),
            Err(_) => return false,
        },
        DataKind::Grammar => {
            if engine.load_grammar_binary(p).is_err() {
                return false;
            }
        }
        DataKind::SimpTrad => match std::fs::read_to_string(&path) {
            Ok(text) => engine.load_simp_trad_text(&text),
            Err(_) => return false,
        },
    }
    true
}

/// Assemble an engine for `scheme` from the compiled tables in `dirs`. Returns
/// the engine and whether the scheme can actually be typed with.
///
/// The list of files *is* `yume_core::data_manifest` — deliberately, because
/// that module exists to stop every frontend keeping its own copy that silently
/// rots, which is exactly what happened to the hand-written list this replaced.
///
/// Availability is yumete's own, lower bar, not the manifest's `required` flag:
/// composing needs the scheme's own dictionary and nothing else. Missing word
/// weights or charsets cost ranking and filtering, but an editor that can still
/// type 漢字 should not fall back to ASCII over them.
fn build_engine(scheme: Scheme, dirs: &[PathBuf]) -> (Engine, bool) {
    let mut engine = Engine::new(CodeTable::new());
    let mut dictionary = false;

    for file in data_manifest::shared()
        .into_iter()
        .chain(data_manifest::for_scheme(scheme.tag()))
    {
        let loaded = load_data_file(&mut engine, dirs, &file);
        // 拼音 decodes through the shared 音節表; the shape schemes need their
        // own 碼表.
        let essential = match scheme {
            Scheme::Pinyin => DataKind::PinyinTable,
            _ => DataKind::Table,
        };
        if loaded && file.kind == essential {
            dictionary = true;
        }
    }

    // Selecting the scheme sets fluency-only input and the commit strategy for
    // pinyin, so it comes after its tables are in place.
    engine.set_scheme_by_tag(scheme.tag());

    (engine, dictionary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yume_core::CodeTable;

    /// A tiny in-memory Lingming engine for driving the adapter without data
    /// files. Codes use format 2 (`code text`); `a` maps to 啊, `b` to 吧/八.
    fn synthetic_session() -> ImeSession {
        let mut table = CodeTable::new();
        table.load_text("a 啊\nb 吧 八\n");
        ImeSession::from_engine(Engine::new(table), Scheme::Lingming)
    }

    #[test]
    fn scheme_tags_round_trip() {
        for s in Scheme::ALL {
            assert_eq!(Scheme::from_tag(s.tag()), Some(s));
        }
        assert_eq!(Scheme::from_tag("ling"), Some(Scheme::Lingming));
        assert_eq!(Scheme::from_tag("拼音"), Some(Scheme::Pinyin));
        assert_eq!(Scheme::from_tag("nope"), None);
    }

    #[test]
    fn scheme_cycles_in_order() {
        assert_eq!(Scheme::Lingming.next(), Scheme::Xingchen);
        assert_eq!(Scheme::Pinyin.next(), Scheme::Lingming);
    }

    #[test]
    fn feeds_input_shows_candidates_and_commits() {
        let mut s = synthetic_session();
        assert!(s.is_chinese());
        s.input('b');
        assert!(s.is_composing());
        let cands = s.page_candidates();
        assert!(
            cands.iter().any(|c| c.text == "吧"),
            "expected 吧 among {cands:?}"
        );
        // Space commits the highlighted (first) candidate.
        s.space();
        assert_eq!(s.take_committed(), "吧");
        assert!(!s.is_composing());
    }

    #[test]
    fn select_in_page_picks_a_specific_candidate() {
        let mut s = synthetic_session();
        s.input('b');
        // Index 1 on the page is 八.
        s.select_in_page(1);
        assert_eq!(s.take_committed(), "八");
    }

    #[test]
    fn backspace_reports_whether_it_consumed() {
        let mut s = synthetic_session();
        s.input('b');
        assert!(s.backspace()); // consumed a buffer char
        assert!(!s.is_composing());
        assert!(!s.backspace()); // nothing left to delete
    }

    #[test]
    fn toggle_language_switches_between_chinese_and_ascii() {
        let mut s = synthetic_session();
        assert!(s.is_chinese());
        s.toggle_language();
        assert!(!s.is_chinese());
        s.toggle_language();
        assert!(s.is_chinese());
    }

    #[test]
    fn a_machine_with_nothing_installed_can_still_type() {
        let mut s = ImeSession::new(Scheme::Lingming, vec![PathBuf::from("/no/such/dir")]);
        // Whether it can depends on the machine that *built* this binary, and
        // both answers are correct — so the test is that the two facts agree,
        // not that either one holds.
        assert_eq!(s.available(), has_builtin_table());
        assert_eq!(s.is_builtin(), has_builtin_table());
        assert_eq!(s.scheme(), Scheme::Lingming);
        if has_builtin_table() {
            s.input('a');
            assert!(!s.page_candidates().is_empty(), "and it really answers");
            assert_eq!(s.table_source(), "出廠自帶");
        }

        // Only 靈明 — the others are installed, and without their tables the
        // session is honestly unavailable rather than silently 靈明.
        let other = ImeSession::new(Scheme::Riyue, vec![PathBuf::from("/no/such/dir")]);
        assert!(!other.available());
        assert_eq!(other.scheme(), Scheme::Riyue);
    }
}
