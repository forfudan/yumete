//! `yumete-ime` — the built-in Yume IME session (Feature #27, #31, #32).
//!
//! yumete embeds the `yume-core` engine directly, with no FFI, since both are
//! Rust. This crate wraps a `yume-core` [`Engine`] in an [`ImeSession`] that the
//! editor drives per keystroke while composing CJK in Insert mode, and loads a
//! scheme's compiled data tables from the data directory (Feature #32). It stays
//! UI-agnostic: it returns candidate and preedit data for the TUI to render.
//!
//! Data tables are the compiled artifacts produced by the yume build
//! (`*.ytab`, `pinyin.yflb`, `pinyin.ywtb`, `chaifen_*.yann`, and optional
//! `charsets/*.ycs`). Point yumete's data directory at them (see
//! [`yumete_config::data_search_dirs`]) — for example by copying a Yume install's
//! `Resources` into `~/.local/share/yumete`. When the tables are absent the
//! session still constructs but reports [`ImeSession::available`] as `false`, so
//! the editor can fall back to plain input.

use std::path::PathBuf;
use std::sync::Arc;

use yume_core::{AnnotationTable, Charset, CodeTable, Engine, FluencyTable, WeightTable};

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

    /// The compiled code-table file for a shape scheme, or `None` for the
    /// fluency-only pinyin scheme (which has no shape code table).
    fn table_file(self) -> Option<&'static str> {
        match self {
            Scheme::Lingming => Some("ling.ytab"),
            Scheme::Xingchen => Some("xing.ytab"),
            Scheme::Qingyun => Some("qing.ytab"),
            Scheme::Riyue => Some("riyue.ytab"),
            Scheme::Pinyin => None,
        }
    }

    /// The compiled 拆分 (chaifen) annotation file for a shape scheme.
    fn annotation_file(self) -> Option<&'static str> {
        match self {
            // Lingming's annotation binary is named `chaifen.yann` (no suffix);
            // the sibling schemes carry a scheme suffix.
            Scheme::Lingming => Some("chaifen.yann"),
            Scheme::Xingchen => Some("chaifen_xing.yann"),
            Scheme::Qingyun => Some("chaifen_qing.yann"),
            Scheme::Riyue => Some("chaifen_riyue.yann"),
            Scheme::Pinyin => None,
        }
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
}

impl ImeSession {
    /// Build a session for `scheme`, loading its data tables from `data_dirs`
    /// (the first directory containing a given file wins). [`available`] is
    /// `false` when the scheme's essential table could not be loaded, but the
    /// engine is still constructed so the editor can degrade gracefully.
    ///
    /// [`available`]: ImeSession::available
    pub fn new(scheme: Scheme, data_dirs: Vec<PathBuf>) -> Self {
        let (engine, available) = build_engine(scheme, &data_dirs);
        ImeSession {
            engine,
            scheme,
            data_dirs,
            available,
        }
    }

    /// Build a session using [`yumete_config::data_search_dirs`].
    pub fn from_default_dirs(scheme: Scheme) -> Self {
        ImeSession::new(scheme, yumete_config::data_search_dirs())
    }

    /// Wrap a pre-built engine (used in tests and by callers assembling their own
    /// tables). The session is marked available.
    pub fn from_engine(engine: Engine, scheme: Scheme) -> Self {
        ImeSession {
            engine,
            scheme,
            data_dirs: Vec::new(),
            available: true,
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

    /// The candidate page size (每頁候選數).
    pub fn page_size(&self) -> usize {
        self.engine.page_size
    }

    /// Set the candidate page size.
    pub fn set_page_size(&mut self, size: usize) {
        self.engine.page_size = size.max(1);
    }
}

/// The first directory in `dirs` that contains `name`, either directly or under
/// a `subdir` (used for `charsets/`). Returns the full path.
fn find_file(dirs: &[PathBuf], name: &str, subdir: Option<&str>) -> Option<PathBuf> {
    for dir in dirs {
        let direct = dir.join(name);
        if direct.is_file() {
            return Some(direct);
        }
        if let Some(sub) = subdir {
            let nested = dir.join(sub).join(name);
            if nested.is_file() {
                return Some(nested);
            }
        }
    }
    None
}

/// Assemble an engine for `scheme` from the compiled tables in `dirs`, mirroring
/// the load order the macOS frontend uses. Returns the engine and whether its
/// essential table loaded.
fn build_engine(scheme: Scheme, dirs: &[PathBuf]) -> (Engine, bool) {
    // 1. Shape code table (empty for the fluency-only pinyin scheme).
    let mut table = CodeTable::new();
    let mut shape_ok = false;
    if let Some(file) = scheme.table_file() {
        if let Some(path) = find_file(dirs, file, None) {
            if let Some(p) = path.to_str() {
                shape_ok = table.load_binary(p).is_ok();
            }
        }
    }
    let mut engine = Engine::new(table);

    // 2. Reverse/fluency table + 3. word weights (shared pinyin.* — used by the
    // pinyin scheme and by `z` reverse lookup in the shape schemes).
    let mut fluency_ok = false;
    if let Some(path) = find_file(dirs, "pinyin.yflb", None) {
        if let Some(p) = path.to_str() {
            let mut fluency = FluencyTable::new();
            if fluency.load_binary(p).is_ok() {
                engine.reverse_lookup = Arc::new(fluency);
                fluency_ok = true;
            }
        }
    }
    if let Some(path) = find_file(dirs, "pinyin.ywtb", None) {
        if let Some(p) = path.to_str() {
            let mut weights = WeightTable::new();
            if weights.load_binary(p).is_ok() {
                engine.word_weights = Arc::new(weights);
            }
        }
    }

    // 4. Annotations (拆分), for the shape schemes.
    if let Some(file) = scheme.annotation_file() {
        if let Some(path) = find_file(dirs, file, None) {
            if let Some(p) = path.to_str() {
                let mut annotations = AnnotationTable::new();
                if annotations.load_binary(p).is_ok() {
                    engine.attach_annotations(annotations);
                }
            }
        }
    }

    // 5. Charsets (字集), optional. Looked up flat or under `charsets/`.
    if let Some(common) = load_charset(dirs, "common.ycs") {
        engine.set_common_charset(common);
    }
    if let Some(tonggui) = load_charset(dirs, "tonggui.ycs") {
        engine.set_tonggui_charset(tonggui);
    }
    if let Some(harmonic) = load_charset(dirs, "harmonic.ycs") {
        engine.set_harmonic_charset(harmonic);
    }

    // 6. Select the scheme (sets fluency-only / commit strategy for pinyin, etc.).
    engine.set_scheme_by_tag(scheme.tag());

    // The scheme is usable when its essential data loaded: a shape table for the
    // shape schemes, or the fluency table for pinyin.
    let available = match scheme {
        Scheme::Pinyin => fluency_ok,
        _ => shape_ok,
    };
    (engine, available)
}

/// Load a compiled charset by file name (flat or under `charsets/`).
fn load_charset(dirs: &[PathBuf], name: &str) -> Option<Charset> {
    let path = find_file(dirs, name, Some("charsets"))?;
    let mut charset = Charset::new("");
    if charset.load_binary(path.to_str()?).is_ok() {
        Some(charset)
    } else {
        None
    }
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
    fn missing_data_dir_is_unavailable_but_usable() {
        let s = ImeSession::new(Scheme::Lingming, vec![PathBuf::from("/no/such/dir")]);
        assert!(!s.available());
        // The scheme name still resolves from the engine.
        assert_eq!(s.scheme(), Scheme::Lingming);
    }
}
