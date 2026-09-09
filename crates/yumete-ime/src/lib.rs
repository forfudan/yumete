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

pub mod reading;
pub mod segment;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use yume_core::commit_strategy::CommitOverrides;
use yume_core::data_manifest;
use yume_core::division::DivisionTable;
use yume_core::division_infer::CustomDivisions;
use yume_core::key_bindings::KeyAction;
use yume_core::lexicon::Lexicon;
use yume_core::zigen::ZigenTable;
use yume_core::{
    custom_scheme, scheme_slots, AnnotationTable, Charset, CodeTable, Engine, FluencyTable,
    UnigramTable, NAMED_CHARSETS,
};

pub use reading::YumeReader;
pub use segment::YumeSegmenter;
pub use yumete_config::PanelDisplay;
// A frontend that reads [`DataProblem`] has to be able to name its `kind`, and
// it has no yume-core of its own.
pub use yume_core::data_manifest::{DataFile, DataKind};
pub use yume_core::DisplayMode;
pub use yume_core::CommitStrategy;
// The key table and the lone-tap detector are yume's. Re-exported rather than
// re-modelled so the TUI drives the same state machine the other frontends do.
pub use yume_core::key_bindings::{FuncKey, ModifierTap};

/// One input scheme (方案), named by its tag.
///
/// **A tag, not a variant per scheme** (#169). The four 宇浩 schemes and 拼音
/// are what a build with no scheme files finds, but they are a fallback and not
/// a definition: a `schemes/<tag>.toml` in any data directory is a scheme too,
/// and yume-core has shipped 冰雪四拼 and 三拼 that way since 2026-09-04. The
/// tag is `&'static str` because the command table it feeds is `&'static`; a
/// found tag is leaked once, at discovery, and there are single digits of them.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Scheme(&'static str);

/// What [`discover`] found, or `None` while nothing has looked.
///
/// `None` and `Some(empty)` are different answers: the first means no directory
/// was ever scanned (a unit test, a `--help` run) and the built-in five stand;
/// the second means a directory was scanned and holds no scheme file, which is
/// every install today and also stands. Only a non-empty find replaces them.
static FOUND: std::sync::OnceLock<Vec<(&'static str, String)>> = std::sync::OnceLock::new();

/// One 自定義方案 the writer imported into yume, as [`discover_slots`] found it.
///
/// **Not a scheme file, and no manifest names it.** A 自定義方案 is compiled by
/// yume into a slot directory of its own — `custom.ytab`, `custom.yzg`, the
/// derived `custom.ycdv`, and `custom.yscm` for its parameters — and
/// `yume_core::data_manifest` answers nothing about it on purpose: the manifest
/// is the *factory* data set, and a slot is the writer's. So the tables are
/// named by absolute path (see [`slot_data_set`]) and the parameters come from
/// the slot's own manifest, the way yume's own frontends read them.
#[derive(Clone, Debug)]
struct Slot {
    /// `custom.a1b2c3d4` — `yume_core::scheme_slots::tag_for` decides the shape.
    tag: &'static str,
    /// The 方案名 the writer gave it, which is the only name it has: the tag is
    /// eight hex digits nobody chose.
    name: String,
    /// The slot directory, holding the four compiled artifacts.
    dir: PathBuf,
    /// The parameters `custom.yscm` holds — 最大碼長, 終止鍵, 反查引導鍵, the
    /// two 隱藏 ticks, and the rest. Kept from the scan rather than read again
    /// at load time: `scheme_slots::list` would not have reported the slot at
    /// all if this had not parsed, so a second read can only agree, and a
    /// scheme whose parameters went missing between the two would silently
    /// fall back to the engine's defaults — a 最大碼長 of the wrong length is
    /// not a thing a writer would ever guess at.
    manifest: custom_scheme::Manifest,
}

/// The 自定義方案 found, or `None` while nothing has looked.
static SLOTS: std::sync::OnceLock<Vec<Slot>> = std::sync::OnceLock::new();

/// Under a data directory, where yume keeps its slots.
///
/// Three, because yume has moved them: macOS puts them in `installed/` today,
/// Windows still writes `data/custom/`, and an installation older than 方案管理
/// has a bare `custom/` holding one scheme with no slot directory at all (that
/// last is `scheme_slots::migrate`'s business, and it is listed here only so a
/// yumete that meets it before yume does still finds nothing rather than
/// finding the wrong thing).
const SLOT_ROOTS: [&str; 3] = ["installed", "data/custom", "custom"];

/// The slots found, or an empty list while nothing has looked.
fn slots() -> &'static [Slot] {
    SLOTS.get().map(Vec::as_slice).unwrap_or(&[])
}

/// The slot a scheme names, if it is one.
fn slot_of(scheme: Scheme) -> Option<&'static Slot> {
    slots().iter().find(|s| s.tag == scheme.0)
}

/// The schemes a build knows when no scheme file has been found.
const BUILT_IN: [Scheme; 5] = [
    Scheme::LINGMING,
    Scheme::XINGCHEN,
    Scheme::QINGYUN,
    Scheme::RIYUE,
    Scheme::PINYIN,
];

impl Scheme {
    /// 靈明 — the default shape scheme.
    pub const LINGMING: Scheme = Scheme("lingming");
    /// 星陳 — shape scheme.
    pub const XINGCHEN: Scheme = Scheme("xingchen");
    /// 卿雲 — shape scheme.
    pub const QINGYUN: Scheme = Scheme("qingyun");
    /// 日月 — shape scheme.
    pub const RIYUE: Scheme = Scheme("riyue");
    /// 拼音 — the phonetic, fluency-only scheme.
    pub const PINYIN: Scheme = Scheme("pinyin");

    /// Every scheme, in menu order: what was shipped, then what the writer
    /// imported.
    ///
    /// **自定義方案 come last, always**, however many there are — the same order
    /// yume's own ⌃⇧N cycle uses. They are appended rather than merged because
    /// they are found a different way and sorted by a different key: a factory
    /// scheme's place is its author's 系列 and index, a slot's is the moment it
    /// was created.
    pub fn all() -> Vec<Scheme> {
        let mut all: Vec<Scheme> = match FOUND.get() {
            Some(found) if !found.is_empty() => found.iter().map(|(tag, _)| Scheme(tag)).collect(),
            _ => BUILT_IN.to_vec(),
        };
        all.extend(slots().iter().map(|s| Scheme(s.tag)));
        all
    }

    /// The canonical scheme tag understood by `yume-core`.
    pub fn tag(self) -> &'static str {
        self.0
    }

    /// This scheme's display name (方案名), when the scheme file gave one.
    ///
    /// Empty for a built-in: the name of a built-in comes from the engine once
    /// its tables are loaded ([`ImeSession::scheme_name`]), which is a better
    /// answer than any table here because it is the one the panel shows.
    pub fn found_name(self) -> &'static str {
        // A slot's name is **not** optional the way a built-in's is: its tag is
        // eight hex digits, so a menu that falls back to the tag shows the
        // writer a row they cannot read.
        if let Some(slot) = slot_of(self) {
            return slot.name.as_str();
        }
        FOUND
            .get()
            .into_iter()
            .flatten()
            .find(|(tag, _)| *tag == self.0)
            .map(|(_, name)| name.as_str())
            .unwrap_or("")
    }

    /// Whether this scheme decodes through a reading table rather than a 碼表.
    ///
    /// Asked of the tag rather than matched on, because a found 音碼 scheme —
    /// 冰雪四拼 is the first — is not a variant anything here can name. A
    /// scheme file says so itself; without one, 拼音 is the only phonetic
    /// scheme a build ships.
    pub fn is_phonetic(self) -> bool {
        // A found scheme says so itself: its file carries `kind`, and reading
        // that is one lookup rather than an inference from which files it
        // happens to ship. Only a scheme yume-core has no file for falls
        // through to the inference below.
        if let Some(kind) = data_manifest::factory_scheme_kind(self.0) {
            return kind == "yinma";
        }
        if self.0 == "pinyin" {
            return true;
        }
        let files = data_manifest::for_scheme(self.0);
        files.iter().all(|f| f.kind != DataKind::Table)
            && files.iter().any(|f| f.kind == DataKind::Reading)
    }

    /// Parse a scheme from a tag (canonical, found, or a common legacy alias).
    pub fn from_tag(tag: &str) -> Option<Scheme> {
        let tag = tag.trim().to_ascii_lowercase();
        let canonical = match tag.as_str() {
            "ling" | "靈明" => "lingming",
            "xing" | "星陳" => "xingchen",
            "qing" | "卿雲" => "qingyun",
            "日月" => "riyue",
            "拼音" => "pinyin",
            other => other,
        };
        // `canonical` is lower-cased and a found tag need not be: comparing
        // raw would refuse `Snow-Sipin` while accepting `snow-sipin`.
        Scheme::all()
            .into_iter()
            .find(|s| s.0.eq_ignore_ascii_case(canonical))
    }

    /// The scheme to fall back on when nobody named one that exists.
    ///
    /// The **first installed** scheme, not 靈明 — a build that ships only 冰雪
    /// has no 靈明 to fall back to, and naming one that is not there is how a
    /// startup ends up with a scheme the menu cannot even show. `all()` is
    /// never empty (it is the built-in five when nothing was found), so the
    /// last resort here is unreachable rather than a real answer.
    pub fn first() -> Scheme {
        Scheme::all().first().copied().unwrap_or(Scheme::LINGMING)
    }

    /// The next scheme, cycling in menu order.
    ///
    /// A scheme that is not on the list — the one a `:yume scheme` held across
    /// a directory that stopped shipping it — starts the cycle rather than
    /// continuing it, so ⌃⇧N lands on the first installed scheme instead of
    /// the second.
    pub fn next(self) -> Scheme {
        let all = Scheme::all();
        match all.iter().position(|&s| s == self) {
            Some(idx) => all[(idx + 1) % all.len()],
            None => all[0],
        }
    }
}

/// Scan `dirs` for scheme files and make what is there the scheme list (#169).
///
/// A scheme file is `<dir>/schemes/<tag>.toml`. Each is handed to yume-core,
/// which parses it, keeps the last one under a given tag (so a user directory
/// shadows the shipped file, the old Rime rule) and sorts the result into menu
/// order — 系列 first, then the scheme’s own index, then the name.
///
/// **Nothing found leaves everything alone**, deliberately: `factory_lists()`
/// in yume-core answers `Some(false)` for a tag that is *not* on a list that
/// exists, and refuses it. Registering a half-populated directory would take
/// away 拼音 from an install whose only scheme file happens to be 靈明's.
///
/// Called once, at startup, before the first line is drawn. Answers how many
/// were taken.
pub fn discover(dirs: &[PathBuf]) -> usize {
    if FOUND.get().is_some() {
        return FOUND.get().map(Vec::len).unwrap_or(0) + slots().len();
    }
    let mut files: Vec<PathBuf> = Vec::new();
    // Last directory first: `add_factory_scheme` lets a later file win, and the
    // search order runs from the most specific directory to the least.
    for dir in dirs.iter().rev() {
        let Ok(entries) = std::fs::read_dir(dir.join("schemes")) else {
            continue;
        };
        let mut here: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("toml"))
            })
            .collect();
        here.sort();
        files.extend(here);
    }
    let mut taken = 0;
    for file in &files {
        if let Ok(text) = std::fs::read_to_string(file) {
            if data_manifest::add_factory_scheme(&text) {
                taken += 1;
            }
        }
    }
    let list: Vec<(&'static str, String)> = match taken {
        0 => Vec::new(),
        _ => data_manifest::shipped_schemes()
            .into_iter()
            .map(|tag| {
                let name = data_manifest::factory_scheme_name(&tag).unwrap_or_default();
                (&*Box::leak(tag.into_boxed_str()), name)
            })
            .collect(),
    };
    // Nothing was taken — no file, or every file refused — so forget them
    // entirely rather than leaving yume-core with a list that half-answers.
    // A refused file is the dangerous half: yume-core would hold a list that
    // exists and is missing everything, and `factory_lists()` answers
    // `Some(false)` to every tag on it.
    if taken == 0 {
        data_manifest::reset_factory_schemes();
    }
    let len = list.len();
    let _ = FOUND.set(list);
    len + discover_slots(dirs)
}

/// Scan `dirs` for the 自定義方案 the writer imported into yume, and make what
/// is there part of the scheme list. Answers how many were found.
///
/// **A slot is not a scheme file, so the scan above cannot see it.** yume
/// compiles an imported 碼表 into a directory named by eight hex digits, under
/// one of [`SLOT_ROOTS`], and writes its parameters into a `custom.yscm` beside
/// the tables — not a `schemes/<tag>.toml`, and not in a form
/// `data_manifest::add_factory_scheme` would parse. So the two halves of yume's
/// scheme list are found two ways, and this is the second: `scheme_slots::list`
/// is the core's own answer to 「哪些槽位裝得起來」, including the test that a
/// half-written import stays invisible rather than showing as a nameless row.
///
/// Nothing is registered with yume-core. A slot's tag is deliberately *not* on
/// any factory list — `data_manifest::for_scheme` answers empty for it, which is
/// correct — and everything that would have come off the manifest comes off the
/// slot instead: its files from [`slot_data_set`], its parameters from its own
/// manifest at load time.
fn discover_slots(dirs: &[PathBuf]) -> usize {
    if SLOTS.get().is_some() {
        return slots().len();
    }
    let mut found: Vec<Slot> = Vec::new();
    for dir in dirs {
        for root in SLOT_ROOTS {
            let mut path = dir.clone();
            for part in root.split('/') {
                path.push(part);
            }
            for slot in scheme_slots::list(&path) {
                let tag = slot.tag();
                // The same slot reached twice — two data directories that are
                // the same place by different names, which `data_search_dirs`
                // already warns about — is one scheme, and the first way in
                // wins, because the search order runs from the most specific
                // directory to the least.
                if found.iter().any(|s| s.tag == tag) {
                    continue;
                }
                found.push(Slot {
                    tag: Box::leak(tag.into_boxed_str()),
                    name: slot.manifest.name.clone(),
                    dir: slot.dir,
                    manifest: slot.manifest,
                });
            }
        }
    }
    let len = found.len();
    let _ = SLOTS.set(found);
    len
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
    /// The file a table of your own was read from, if it was.
    table_file: Option<PathBuf>,
    /// Every entry of the data set that did not make it into the engine, and
    /// why (Feature #220). Kept rather than discarded because the interesting
    /// half of it is **silent**: a file that is there and that the core
    /// refuses looks exactly like a file that was never installed, and the
    /// writer sees neither — only annotations that stopped appearing.
    problems: Vec<DataProblem>,
    /// Whether the candidates get a panel of their own, or the page
    /// (Feature #211). A session's rather than a frame's, because it is the
    /// same question as [`Self::page_size`] — how this session offers what it
    /// has found — and the front end asks it once per frame.
    display: PanelDisplay,
    /// Whether yume has the keyboard at all — the **outer** switch, above
    /// 中/ABC (#290).
    ///
    /// 中文 and ABC are both yume holding the keys: one composes, the other
    /// passes ASCII through, and a lone-Shift tap crosses between them. This is
    /// the third state, where yume holds nothing and the keystrokes belong to
    /// whatever the system has — its own input method, most of all. The front
    /// end needs the distinction because the terminal flag that makes a bare
    /// Shift visible is the same flag that stops macOS's input method from
    /// composing: it can be held exactly while this is true.
    engaged: bool,
    /// Whether `Tab` has summoned the full panel for the composition in hand.
    ///
    /// It dies with that composition: [`Self::input`] clears it when a new one
    /// begins, so 「這一個詞我要看清楚」 never turns into a setting nobody
    /// remembers changing.
    summoned: bool,
}

impl ImeSession {
    /// Build a session for `scheme`, loading its data tables from `data_dirs`
    /// (the first directory containing a given file wins). [`available`] is
    /// `false` when the scheme's essential table could not be loaded, but the
    /// engine is still constructed so the editor can degrade gracefully.
    ///
    /// [`available`]: ImeSession::available
    pub fn new(scheme: Scheme, data_dirs: Vec<PathBuf>) -> Self {
        let (mut engine, mut available, problems) = build_engine(scheme, &data_dirs);
        // Nothing installed. Rather than an editor that cannot type 漢字 until
        // somebody clones the 宇浩 source tree, 靈明's own 碼表 stands in when
        // this binary was built on a machine that had it.
        let mut builtin = false;
        if !available && scheme == Scheme::LINGMING {
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
            table_file: None,
            problems,
            display: PanelDisplay::default(),
            summoned: false,
            engaged: true,
        }
    }

    /// A session over a code table read from a file of your own.
    ///
    /// **Any table yume-core can read**, which is more than yumete's own four:
    /// `load_text` auto-detects `text<TAB>code`, `code<SPACE>text` and their
    /// reverses, and strips a leading RIME YAML header — so a Rime
    /// `.dict.yaml` for 五筆, 倉頡, 粵拼 or 朙月拼音 works as it comes, and so
    /// does a table somebody typed by hand.
    ///
    /// The language layer still comes from the installed data where it is:
    /// this replaces the *spelling*, not the language.
    pub fn from_table_file(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut table = CodeTable::new();
        table.load_text(&text);
        if table.count() == 0 {
            return Err(format!("{}：讀不出碼表", path.display()));
        }
        let dirs = yumete_config::data_search_dirs();
        let mut engine = Engine::new(CodeTable::new());
        let mut problems = Vec::new();
        for file in data_manifest::shared() {
            if !matches!(file.kind, DataKind::Table) {
                if let Err(problem) = load_data_file(&mut engine, &dirs, &file) {
                    problems.push(problem);
                }
            }
        }
        engine.set_table(Arc::new(table));
        // 靈明's rules are the ones yumete knows; a foreign table is driven by
        // them until it says otherwise, which for a shape scheme is right.
        engine.set_scheme_by_tag(Scheme::LINGMING.tag());
        Ok(ImeSession {
            engine,
            scheme: Scheme::LINGMING,
            data_dirs: dirs,
            available: true,
            annotations: false,
            builtin: false,
            table_file: Some(path.to_path_buf()),
            problems,
            display: PanelDisplay::default(),
            summoned: false,
            engaged: true,
        })
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
        let mut problems = Vec::new();
        for file in data_manifest::shared() {
            if !matches!(file.kind, DataKind::Table | DataKind::Symbols) {
                if let Err(problem) = load_data_file(&mut engine, &dirs, &file) {
                    problems.push(problem);
                }
            }
        }
        let available = load_builtin(&mut engine);
        engine.set_scheme_by_tag(Scheme::LINGMING.tag());
        ImeSession {
            engine,
            scheme: Scheme::LINGMING,
            data_dirs: dirs,
            available,
            annotations: false,
            builtin: true,
            table_file: None,
            problems,
            display: PanelDisplay::default(),
            summoned: false,
            engaged: true,
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
        if let Some(path) = &self.table_file {
            return path.display().to_string();
        }
        if self.builtin {
            return match builtin_version() {
                Some(version) => format!("出廠自帶 {version}"),
                None => "出廠自帶".to_string(),
            };
        }
        // The manifest knows which file this scheme's 碼表 is — and for a
        // 自定義方案, which the manifest says nothing about, the slot does.
        // Either way, asking beats guessing at the name.
        own_data_set(self.scheme)
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
        let mut problems = Vec::new();
        for file in data_manifest::shared() {
            if matches!(file.kind, DataKind::Weights | DataKind::Lexicon) {
                if let Err(problem) = load_data_file(&mut engine, &dirs, &file) {
                    problems.push(problem);
                }
            }
        }
        ImeSession {
            engine,
            scheme,
            data_dirs: dirs,
            available: false,
            annotations: false,
            builtin: false,
            table_file: None,
            problems,
            display: PanelDisplay::default(),
            summoned: false,
            engaged: true,
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
            table_file: None,
            problems: Vec::new(),
            display: PanelDisplay::default(),
            summoned: false,
            engaged: true,
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
            table_file: None,
            problems: Vec::new(),
            display: PanelDisplay::default(),
            summoned: false,
            engaged: true,
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
            table_file: None,
            problems: Vec::new(),
            display: PanelDisplay::default(),
            summoned: false,
            engaged: true,
        }
    }

    /// Every entry of the data set that did not make it into the engine, and
    /// why (Feature #220).
    ///
    /// In the order they were tried, which is the manifest's — the shared
    /// files, then the scheme's own. That is a load order, not a ranking:
    /// `required` is a flag on the entry, not a place in the list. Most of it
    /// is [`DataFault::Missing`] on a normal install; see
    /// [`DataProblem::is_loud`] for the part worth showing.
    pub fn problems(&self) -> &[DataProblem] {
        &self.problems
    }

    /// Whether yume has the keyboard — see [`Self::engaged`] for the three
    /// states this is the outer half of.
    pub fn engaged(&self) -> bool {
        self.engaged
    }

    /// Take the keyboard, or hand it back.
    ///
    /// Handing it back does **not** change 中/ABC: coming back should come
    /// back to what you were typing in, and the language is what `:yume abc`
    /// and the lone-Shift tap are for.
    pub fn set_engaged(&mut self, engaged: bool) {
        self.engaged = engaged;
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
        // The 上屏方式 is the writer's, not the scheme's: switching 方案 builds a
        // new engine, and a preference that evaporated when you tried another
        // scheme would look like a setting that does not stick.
        let chosen = self.commit_override();
        let (engine, available, problems) = build_engine(scheme, &self.data_dirs);
        self.engine = engine;
        self.scheme = scheme;
        self.available = available;
        self.problems = problems;
        self.set_commit_strategy(chosen);
        available
    }

    // ---- Per-keystroke input lifecycle ------------------------------------

    /// Feed one printable character to the engine.
    pub fn input(&mut self, ch: char) {
        // A new composition starts with the panel the *setting* asks for:
        // `Tab` was asked for one word, not for the rest of the session.
        if !self.is_composing() {
            self.summoned = false;
        }
        // **A word landing is the event, not the buffer emptying.** Under
        // 頂功 and 整句 the engine commits the head of the buffer and goes on
        // composing the tail in the same call, so it is never *not* composing
        // between two words — and a panel summoned for 靈 stayed up for the
        // whole of a paragraph typed without a space.
        let landed = self.engine.committed().len();
        self.engine.input(ch);
        if self.engine.committed().len() != landed {
            self.summoned = false;
        }
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

    /// Press a modifier the user has bound — a lone Shift tap, today — and say
    /// whether the engine took it.
    ///
    /// The same two steps as [`Self::press_func`], and for the same reason:
    /// ask yume what the key *means* here, then let yume *do* it. Shift is not
    /// one thing. On an empty buffer its factory value is 中/英 toggle; while
    /// composing it is 「commit the raw code, then go English」, because the
    /// reason to reach for Shift mid-code is that this stretch is English —
    /// and the code already typed is text the writer meant. yumete used to
    /// call `set_chinese(false)` straight, whose job is to *clear* the buffer:
    /// a half-typed 拆分 disappeared with no commit and no undo entry.
    ///
    /// `false` means the action is the frontend's to do (a panel to open, or a
    /// key whose own character the action needs — a modifier has none).
    pub fn press_modifier(&mut self, key: FuncKey) -> bool {
        let action = self.engine.key_action(key);
        self.engine.perform(action)
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

    /// Everything the 拆分表 knows about one character — Feature #215.
    ///
    /// The candidate list can only afford one line, so what it shows is
    /// 拆分 and a code and nothing else. This is the same knowledge with the
    /// room to say all of it: 拆分、編碼、分節編碼、讀音、注釋、字集、統一碼碼位
    /// and the untruncated 全息拆分, as `(名, 值)` pairs a panel can draw.
    ///
    /// **A character with more than one 拆法 answers more than once.** 陸、臺、
    /// 港 disagree about a few thousand characters, and which one you are
    /// looking at is exactly the question this panel is opened to settle — so
    /// each source gets its own run of rows, headed by an empty value, rather
    /// than the first one winning silently.
    ///
    /// Empty when the scheme has no 拆分 layer at all (拼音), or the character
    /// is not in the table. The caller shows that as「查不到」rather than an
    /// empty panel, because those are different findings.
    pub fn glosses(&self, ch: char) -> Vec<(String, String)> {
        let table = self.engine.annotations();
        let text = ch.to_string();
        let sources = table.sources_of(&text);
        let found = table.annotations_for(&text);
        let mut out = Vec::new();
        for (i, a) in found.iter().enumerate() {
            // The heading only earns its row when there is something to tell
            // apart: one 拆法 is the ordinary case and a lone「陸」above it is
            // a row spent saying nothing.
            if found.len() > 1 {
                let label = sources
                    .get(i)
                    .map(|s| s.label())
                    .filter(|l| !l.is_empty())
                    .unwrap_or("陸");
                out.push((label.to_string(), String::new()));
            }
            for (name, value) in [
                ("拆分", &a.decomposition),
                ("編碼", &a.code),
                ("分節編碼", &a.segmented),
                ("讀音", &a.reading),
                ("注釋", &a.note),
                ("字集", &a.charset),
                ("全息拆分", &a.full_decomposition),
                ("方案拆分", &a.own_decomposition),
            ] {
                if !value.is_empty() {
                    out.push((name.to_string(), value.clone()));
                }
            }
            // The code point is the one field that is true of the character
            // rather than of a 拆法, so it is written the way a reader looks
            // it up elsewhere — `U+` and all — and only once.
            if i + 1 == found.len() && !a.unicode.is_empty() {
                out.push(("統一碼".to_string(), format!("U+{}", a.unicode)));
            }
        }
        out
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

    /// A reader over this session's 字料層 and 讀音表 (Feature #234).
    ///
    /// The sibling of [`segmenter`](Self::segmenter), and it takes the same
    /// stance: the tables are already loaded and reference-counted, so `:ruby
    /// auto` shares them rather than opening `chaifen.ydiv` a second time.
    pub fn reader(&self) -> YumeReader {
        YumeReader::new(
            self.engine.annotations().clone(),
            self.engine.reading_table.clone(),
        )
    }

    // ---- 上屏方式 (commit method) ------------------------------------------

    /// The 上屏方式 in force: **when** a finished code goes to the page.
    ///
    /// 延遲（頂字）waits and pushes the *previous* code on the keystroke that
    /// cannot continue it; 唯一 goes as soon as a code has one candidate; 整句
    /// never goes on its own, and the whole sentence is confirmed with Space.
    ///
    /// This is the answer for the status line, so it is what is *in force* —
    /// which for 拼音 is 整句 whether or not anybody chose it, because a scheme
    /// with no 碼表 to look a segment up in has nothing else it could be.
    pub fn commit_strategy(&self) -> CommitStrategy {
        match self.commit_override() {
            // 拼音 has no 碼表 to look a segment up in, so the core pins its
            // decoder to 整句 whatever is asked of it — and a status line that
            // answered 「延遲」 there would be describing something the engine
            // is not doing.
            Some(cs) if !self.engine.fluency_only() => cs,
            _ => self.engine.effective_commit_strategy(),
        }
    }

    /// What the writer *asked for*, which is not the same question: `None` is
    /// 「whatever this scheme's own default is」, and it has to stay tellable
    /// apart from a choice, or carrying the setting across a scheme switch
    /// would pin 拼音's 整句 onto 靈明.
    pub fn commit_override(&self) -> Option<CommitStrategy> {
        self.engine.commit_overrides().preset
    }

    /// Choose the 上屏方式, or hand the question back to the scheme (`None`).
    ///
    /// It goes in as yume's *user layer* (`CommitOverrides`), which is the one
    /// that wins over both the engine default and a scheme file — the three
    /// layers are merged in the core so that every front end gets the same
    /// answer.
    pub fn set_commit_strategy(&mut self, cs: Option<CommitStrategy>) {
        self.engine.set_commit_overrides(CommitOverrides {
            preset: cs,
            ..CommitOverrides::default()
        });
    }

    /// The candidate page size (每頁候選數).
    pub fn page_size(&self) -> usize {
        self.engine.page_size
    }

    /// Set the candidate page size.
    pub fn set_page_size(&mut self, size: usize) {
        self.engine.page_size = size.max(1);
    }

    /// Which way the candidates are offered — the setting, not this frame's
    /// answer. [`Self::panel_is_full`] is the question a renderer asks.
    pub fn panel_display(&self) -> PanelDisplay {
        self.display
    }

    /// Set it, forgetting any panel `Tab` had summoned.
    pub fn set_panel_display(&mut self, display: PanelDisplay) {
        self.display = display;
        self.summoned = false;
    }

    /// Whether the bordered panel is drawn on **this** frame.
    ///
    /// `bare` plus a `Tab`: the list comes up for the composition in hand and
    /// goes away with it. Asked of the session rather than worked out by the
    /// renderer because the same answer settles two things — whether to draw
    /// the panel, and whether the code needs somewhere else to be shown.
    pub fn panel_is_full(&self) -> bool {
        self.display == PanelDisplay::Full || (self.summoned && self.is_composing())
    }

    /// `Tab`: show me the whole list for this one word — and `Tab` again to
    /// put it away.
    ///
    /// A toggle, because a key that only goes one way is one you cannot undo:
    /// a `Tab` pressed by mistake would have covered the page until the word
    /// ended. Nothing when there is no composition to summon it for, so a
    /// `Tab` that fell through cannot leave the panel armed for whatever is
    /// typed next.
    pub fn summon_panel(&mut self) -> bool {
        if self.display == PanelDisplay::Full || !self.is_composing() {
            return false;
        }
        self.summoned = !self.summoned;
        true
    }

    /// The candidate that would land on the page if you pressed Space now.
    ///
    /// What `bare` draws into the sentence: the **highlighted** one, not
    /// literally the first, so that moving the highlight moves what you are
    /// reading. Empty when nothing is being composed, or when the engine has
    /// found nothing to offer.
    pub fn inline_candidate(&self) -> String {
        if !self.is_composing() {
            return String::new();
        }
        self.engine
            .page_candidates()
            .get(self.highlight())
            .cloned()
            .unwrap_or_default()
    }
}

/// Resolve one manifest entry's relative path (`charsets/common.ycs`) against
/// the search path, first directory wins.
fn find_file(dirs: &[PathBuf], relative: &str) -> Option<PathBuf> {
    // **An absolute name is already the answer.** A 自定義方案's tables are not
    // in the manifest and not under any data directory — they sit in the slot
    // yume allocated for them, wherever that is — so [`slot_data_set`] names
    // them in full, and there is nothing here to resolve.
    let named = Path::new(relative);
    if named.is_absolute() {
        return named.is_file().then(|| named.to_path_buf());
    }
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
    // Then flat. The manifest lays its files out in `data/` and `schemes/`
    // because that is how yume's own bundle is arranged — but a writer who
    // unzipped a release into one folder, or a build script older than the
    // split, has every one of them side by side, and the names are distinctive
    // enough (`chaifen.ydiv`, `lang.ywl`, `ling.ytab`) that the basename is a
    // safe second question. Nested wins, so a directory holding both is read
    // the way it was arranged.
    let name = relative.rsplit('/').next()?;
    if name != relative {
        for dir in dirs {
            let path = dir.join(name);
            if path.is_file() {
                return Some(path);
            }
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

/// Which build of yume the built-in 碼表 came from.
///
/// A table compiled into a binary is a *snapshot*, and a writer looking at a
/// candidate they do not recognise deserves to know how old the snapshot is
/// before they go looking for a bug in 宇浩.
pub fn builtin_version() -> Option<&'static str> {
    builtin::BUILTIN_VERSION
}

/// Why one entry of the factory data set is not in the engine (Feature #220).
///
/// It used to be a `bool`, and that made 「不在」 and 「在，而核心不收」 the
/// same answer. The second one is the one worth saying out loud: when a binary
/// format changes its magic — `.ydiv` did, on 2026-09-04 — an older data
/// directory keeps every file in place and quietly stops answering. Nothing in
/// the editor looked wrong; the 拆分 comments simply were not there any more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataFault {
    /// In none of the search directories. The ordinary case for optional data,
    /// and not worth interrupting anybody over.
    Missing,
    /// There, and the operating system would not hand it over.
    Unreadable(String),
    /// There and readable, and yume-core will not have it.
    Rejected {
        /// The core's own words — `bad division magic`, `truncated`, …
        reason: String,
        /// The 檔頭 this format expects against the one the file carries, when
        /// they differ and both are legible. This is the whole point of the
        /// feature: it names the *version* mismatch rather than the symptom.
        magic: Option<MagicMismatch>,
    },
    /// A `kind` this build of yumete was not compiled against — yume is newer
    /// than this binary. Skipped by design, and only a note.
    UnknownKind,
}

/// What a format's 檔頭 should say, and what this file's does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MagicMismatch {
    /// The magic this build of yume-core writes and reads.
    pub expected: String,
    /// The magic in the file, as far as it is printable.
    pub found: String,
}

/// One entry of the data set that did not make it into the engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataProblem {
    /// The manifest-relative name, which is what a writer will recognise —
    /// `chaifen.ydiv`, `schemes/ling.ytab`.
    pub file: String,
    /// Which door of the engine it would have gone through.
    pub kind: DataKind,
    /// What went wrong.
    pub fault: DataFault,
}

impl DataProblem {
    /// Whether this is something the writer should be told without asking.
    ///
    /// A missing file is not: half the manifest is optional and a normal
    /// install is missing several. A file that is **there and refused** is —
    /// somebody put it there on purpose and it is doing nothing.
    pub fn is_loud(&self) -> bool {
        matches!(
            self.fault,
            DataFault::Unreadable(_) | DataFault::Rejected { .. }
        )
    }
}

/// The three-variant `BinaryError` that yume-core declares once per format, in
/// words. Four identical enums in four modules and no shared trait upstream, so
/// the `match` is written once and stamped out four times.
macro_rules! binary_reasons {
    ($($name:ident => $module:ident),* $(,)?) => {
        $(
            fn $name(e: yume_core::$module::BinaryError) -> String {
                match e {
                    yume_core::$module::BinaryError::BadMagic => "bad magic".to_string(),
                    yume_core::$module::BinaryError::Truncated => "truncated".to_string(),
                    yume_core::$module::BinaryError::Io(e) => e.to_string(),
                }
            }
        )*
    };
}

binary_reasons!(
    table_reason => code_table,
    reading_reason => fluency_table,
    weights_reason => unigram_table,
    lexicon_reason => lexicon,
);

/// The data set a scheme is made of, in load order: the shared files, then the
/// scheme's own.
///
/// `yume_core::data_manifest` is the one place that says so, and this hands it
/// on to a frontend that has no yume-core of its own to ask. It is also what
/// [`build_engine`] walks, so what a caller inspects is exactly what was
/// loaded.
///
/// **An entry that appears twice is loaded once.** The two halves of the
/// manifest overlap on purpose: `shared()` carries `data/pinyin.yflb` and
/// `for_scheme` appends a reading table to every scheme that did not name one
/// of its own — which, for the 形碼 schemes, is that same file. Parsing 13 MB
/// twice to end at the same table is a second or so of startup for nothing.
/// Only an *identical* entry is dropped: a scheme with a reading table of its
/// own names a different file, and it still loads last and still wins.
pub fn data_set(scheme: Scheme) -> Vec<DataFile> {
    let mut seen: Vec<(DataKind, String, String, i32)> = Vec::new();
    data_manifest::shared()
        .into_iter()
        .chain(own_data_set(scheme))
        .filter(|f| {
            let key = (f.kind, f.file.clone(), f.aux.clone(), f.slot);
            let fresh = !seen.contains(&key);
            if fresh {
                seen.push(key);
            }
            fresh
        })
        .collect()
}

/// The files that are **this scheme's own**, as against the shared ones.
///
/// Two answers behind one question, because a scheme is found two ways: the
/// manifest for one yume shipped, the slot directory for one the writer
/// imported. Everything downstream — [`data_set`], [`build_engine`]'s test for
/// which file makes this scheme typable, `:yume` reporting where it looked —
/// asks here and never has to know which kind it is holding.
fn own_data_set(scheme: Scheme) -> Vec<DataFile> {
    match slot_of(scheme) {
        Some(slot) => slot_data_set(slot),
        None => data_manifest::for_scheme(scheme.tag()),
    }
}

/// A 自定義方案's own files, named the way the manifest names a factory
/// scheme's — kind by kind, so one loader serves both.
///
/// The paths are **absolute** (see [`find_file`]): a slot is not under a data
/// directory and there is no relative name that would find it. The three that
/// matter are yume's own three, in yume's own order:
///
/// * `custom.ytab` — the 碼表 the writer's table compiled to. Without it the
///   scheme cannot be typed, which is what makes it the essential file here.
/// * `data/chaifen.ydiv` — the shared 字料層, exactly as every factory scheme
///   takes it: 讀音・字義・字集 are the language's, not the scheme's.
/// * `custom.yzg` — this scheme's 字根表, as the annotation entry's `aux`, so
///   the 拆分 drawn beside a candidate is written in **this** scheme's roots
///   rather than 靈明's. Its derived `custom.ycdv`, when the compile produced
///   one, travels with it by name — see the annotation arm of
///   [`load_data_file`].
///
/// No reading table is named, so `with_reading`'s factory rule — a scheme that
/// names none reads 拼音's — holds here too by way of the shared set.
fn slot_data_set(slot: &Slot) -> Vec<DataFile> {
    let file = |ext: &str| {
        slot.dir
            .join(format!("{}.{ext}", custom_scheme::STEM))
            .to_string_lossy()
            .into_owned()
    };
    vec![
        DataFile {
            kind: DataKind::Table,
            file: file("ytab"),
            aux: String::new(),
            slot: -1,
            required: true,
        },
        DataFile {
            kind: DataKind::Annotations,
            file: "data/chaifen.ydiv".to_string(),
            aux: file("yzg"),
            slot: -1,
            required: true,
        },
    ]
}

/// The magic this kind of file should begin with, when the format has one.
///
/// **The core's own answer** (`DataKind::magic`, 2026-09-05), not a table here:
/// a copy would rot on exactly the day the constant changes, which is the day
/// it matters — the `.ydiv` move from `YDV20260828` to `YDV20260904` showed up
/// as the annotations silently vanishing. A text format has no head to check
/// and answers `None`, and the reason from the loader stands on its own.
///
/// This used to name each format's constant one by one, which meant the two
/// kinds that gained a magic later (a charset, a 固頂表) were never checked.
pub fn expected_magic(kind: DataKind) -> Option<&'static [u8]> {
    Some(kind.magic().as_bytes()).filter(|m| !m.is_empty())
}

/// Compare the head of `path` against what its format expects.
///
/// `None` when they agree, when the format has no constant to compare against,
/// or when the file's head is not printable — a mismatch nobody can read is
/// worse than no mismatch at all, because it looks like the answer.
fn magic_mismatch(path: &Path, kind: DataKind) -> Option<MagicMismatch> {
    let want = expected_magic(kind)?;
    // The head, not the file: some of these are tens of megabytes, and this
    // runs on the failing path of a load that has already read them once.
    let mut head = vec![0u8; want.len()];
    let mut file = std::fs::File::open(path).ok()?;
    std::io::Read::read_exact(&mut file, &mut head).ok()?;
    let head = &head[..];
    if head == want {
        return None;
    }
    if !head.iter().all(|b| b.is_ascii_graphic()) {
        return None;
    }
    Some(MagicMismatch {
        expected: String::from_utf8_lossy(want).into_owned(),
        found: String::from_utf8_lossy(head).into_owned(),
    })
}

/// Load one entry of the factory data set into `engine`.
///
/// This is yumete's copy of the one dispatch every Yume frontend has — the
/// `kind` of a [`DataFile`] says which door of the engine it goes through. An
/// unrecognised `kind` is skipped rather than treated as an error, so a yumete
/// built against an older `yume-core` still starts against newer data.
///
/// The error carries the core's own reason (Feature #220). yume-core has always
/// returned one; this function used to throw it away.
fn load_data_file(
    engine: &mut Engine,
    dirs: &[PathBuf],
    file: &DataFile,
) -> Result<(), DataProblem> {
    let at = |name: &str, fault: DataFault| DataProblem {
        file: name.to_string(),
        kind: file.kind,
        fault,
    };
    let Some(path) = find_file(dirs, &file.file) else {
        return Err(at(&file.file, DataFault::Missing));
    };
    let Some(p) = path.to_str() else {
        // A path this host cannot spell in UTF-8. The core's doors take `&str`,
        // so there is nothing to try.
        return Err(at(&file.file, DataFault::Unreadable("path is not UTF-8".into())));
    };
    // Building the mismatch costs a read of the file's head, so it is only
    // built on the failing path.
    let rejected = |path: &Path, reason: String| DataFault::Rejected {
        reason,
        magic: magic_mismatch(path, file.kind),
    };
    match file.kind {
        DataKind::Table => {
            let mut table = CodeTable::new();
            if let Err(e) = table.load_binary(p) {
                let why = table_reason(e);
                return Err(at(&file.file, rejected(&path, why)));
            }
            engine.set_table(Arc::new(table));
        }
        DataKind::Symbols => {
            let mut table = CodeTable::new();
            if let Err(e) = table.load_binary(p) {
                let why = table_reason(e);
                return Err(at(&file.file, rejected(&path, why)));
            }
            engine.set_symbol_table(table);
        }
        DataKind::Reading => {
            let mut fluency = FluencyTable::new();
            if let Err(e) = fluency.load_binary(p) {
                let why = reading_reason(e);
                return Err(at(&file.file, rejected(&path, why)));
            }
            engine.set_reading_table(Arc::new(fluency));
        }
        DataKind::Weights => {
            let mut unigram = UnigramTable::new();
            if let Err(e) = unigram.load_binary(p) {
                let why = weights_reason(e);
                return Err(at(&file.file, rejected(&path, why)));
            }
            engine.unigram = Arc::new(unigram);
        }
        DataKind::Lexicon => {
            let mut lexicon = Lexicon::new();
            if let Err(e) = lexicon.load_binary(p) {
                let why = lexicon_reason(e);
                return Err(at(&file.file, rejected(&path, why)));
            }
            engine.lexicon = Arc::new(lexicon);
        }
        DataKind::Annotations => {
            // 全息拆分表 plus this scheme's 字根表; the divisions are shared by
            // every scheme, and pinyin takes them with no 字根表 at all.
            let bytes = std::fs::read(&path)
                .map_err(|e| at(&file.file, DataFault::Unreadable(e.to_string())))?;
            let divisions = DivisionTable::from_binary(&bytes)
                .map_err(|e| at(&file.file, rejected(&path, e.to_string())))?;
            // The 字根表 is the *second* half of this entry, and losing it must
            // not cost the first: 拆分・讀音・字集 all come off the divisions,
            // and only 編碼 comes off the 字根表. So the annotations go in
            // either way, and a refused 字根表 is reported afterwards.
            let divisions = Arc::new(divisions);
            let zigen_path = find_file(dirs, &file.aux);
            let zigen = match zigen_path.as_ref() {
                Some(aux) => match std::fs::read(aux) {
                    // The 字根表 has no exported magic of its own, so the
                    // core's sentence is the whole answer here.
                    Ok(bytes) => ZigenTable::from_binary(&bytes).map_err(|e| {
                        at(
                            &file.aux,
                            DataFault::Rejected {
                                reason: e.to_string(),
                                magic: None,
                            },
                        )
                    }),
                    Err(e) => Err(at(&file.aux, DataFault::Unreadable(e.to_string()))),
                },
                None => Ok(ZigenTable::default()),
            };
            let (zigen, problem) = match zigen {
                Ok(zigen) => (zigen, None),
                Err(problem) => (ZigenTable::default(), Some(problem)),
            };
            let mut table = AnnotationTable::with_tables(divisions, zigen);
            // **A 拆分 of its own travels with the 字根表, by name.** A scheme
            // that is not written in 宇浩's roots — every 自定義方案 compiled
            // with 部分注解, and any factory scheme that ships one — has its
            // derived divisions in a `.ycdv` beside its `.yzg`, under the same
            // stem, because the root ids in it index the inventory that `.yzg`
            // was built from and it is meaningless anywhere else. So it has no
            // manifest entry of its own and is picked up here, exactly as
            // yume's own loader picks it up.
            if let Some(derived) = zigen_path.as_ref().map(|p| p.with_extension("ycdv")) {
                if derived.is_file() {
                    if let Ok(d) = CustomDivisions::load_binary(&derived.to_string_lossy()) {
                        table.attach_custom_divisions(d);
                    }
                }
            }
            engine.set_annotations(table);
            if let Some(problem) = problem {
                return Err(problem);
            }
        }
        DataKind::Charset => {
            let Some(&id) = usize::try_from(file.slot)
                .ok()
                .and_then(|i| NAMED_CHARSETS.get(i))
            else {
                let why = format!("no charset slot {}", file.slot);
                return Err(at(&file.file, rejected(&path, why)));
            };
            let mut charset = Charset::new(id);
            if let Err(e) = charset.load_binary(p) {
                return Err(at(&file.file, rejected(&path, e.to_string())));
            }
            if !engine.set_named_charset(id, charset) {
                let why = "the engine refused the charset".to_string();
                return Err(at(&file.file, rejected(&path, why)));
            }
        }
        DataKind::Words => match yume_core::word_whitelist::load_binary(p) {
            Ok(words) => engine.set_word_whitelist(words),
            Err(e) => return Err(at(&file.file, rejected(&path, e.to_string()))),
        },
        DataKind::Grammar => {
            if let Err(e) = engine.load_grammar_binary(p) {
                return Err(at(&file.file, rejected(&path, e.to_string())));
            }
        }
        DataKind::SimpTrad => match std::fs::read_to_string(&path) {
            Ok(text) => engine.load_simp_trad_text(&text),
            Err(e) => return Err(at(&file.file, DataFault::Unreadable(e.to_string()))),
        },
        // A kind this yumete was not built against. The doc comment above has
        // promised since the beginning that these are skipped rather than
        // treated as errors — but the match was exhaustive, so yume adding a
        // kind broke the *build* instead. Now it does what it said.
        _ => return Err(at(&file.file, DataFault::UnknownKind)),
    }
    Ok(())
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
fn build_engine(scheme: Scheme, dirs: &[PathBuf]) -> (Engine, bool, Vec<DataProblem>) {
    let mut engine = Engine::new(CodeTable::new());
    let mut dictionary = false;
    let mut problems = Vec::new();
    // Which file *is* this scheme: a 形碼 scheme is dead without its 碼表, a
    // 音碼 one decodes through its reading table and has no 碼表 at all. Asked
    // once — the answer walks the manifest, and the loop below is per file.
    let essential = match scheme.is_phonetic() {
        true => DataKind::Reading,
        false => DataKind::Table,
    };

    // **Its own table, not the shared one.** `data_set` is the shared files
    // plus this scheme's, and the shared set carries `data/pinyin.yflb` — so a
    // found 音碼 scheme that **names** a reading table of its own, and whose
    // file is not on disk, would be marked typable off 拼音's, and every
    // keystroke would decode as 拼音 under its name. That is the one this
    // closes; the schemes it matters for are the 冰雪 pair, which name
    // `schemes/snow/reading.yflb`.
    //
    // A scheme that names **no** reading is a different case and is left
    // alone: `data_manifest::for_scheme` ends in `with_reading`, which hands
    // that scheme `data/pinyin.yflb` on purpose — 拼音's readings *are* its
    // readings, which is how a 雙拼 scheme with only a syllable table of its
    // own is meant to work. So the file is in `own`, `is_own` is true, and the
    // scheme is typable, which is the right answer. 拼音 itself loses nothing
    // either: its manifest names that very file.
    let own = own_data_set(scheme);
    let is_own = |f: &DataFile| own.iter().any(|o| o.kind == f.kind && o.file == f.file);

    for file in data_set(scheme) {
        let loaded = match load_data_file(&mut engine, dirs, &file) {
            Ok(()) => true,
            Err(problem) => {
                problems.push(problem);
                false
            }
        };
        if loaded && file.kind == essential && is_own(&file) {
            dictionary = true;
        }
    }

    // Selecting the scheme sets fluency-only input and the commit strategy for
    // pinyin, so it comes after its tables are in place.
    match slot_of(scheme) {
        // **A slot is not on any factory list**, deliberately, so asking by tag
        // would answer `false` and leave the engine on the default 靈明 shape —
        // an imported scheme would type with 靈明's 最大碼長 and 終止鍵, which
        // is not a scheme anybody has. Its parameters come off its own manifest
        // instead, which is exactly what yume's own frontends do on 方案切換.
        Some(slot) => {
            let table = Arc::clone(&engine.table);
            // The compiled 碼表 is handed over so a manifest written before the
            // 反查引導鍵 was recorded can fill it in; it is already loaded, so
            // this costs a clone of an `Arc` and no read.
            let manifest = custom_scheme::load_manifest(&slot.dir, &table)
                .unwrap_or_else(|_| slot.manifest.clone());
            engine.set_scheme(manifest.schema());
            // 段界的裁判: `set_scheme` has just derived one from the 最大碼長
            // and the self-segmenting letters, and the 碼表's own answer — the
            // only one a 頂功 scheme can be read off — replaces it when the
            // compile learned one.
            if let Some(space) = manifest.code_space_for(&table) {
                engine.set_code_space(space);
            }
        }
        None => {
            engine.set_scheme_by_tag(scheme.tag());
        }
    }

    (engine, dictionary, problems)
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
        ImeSession::from_engine(Engine::new(table), Scheme::LINGMING)
    }

    #[test]
    fn scheme_tags_round_trip() {
        for s in Scheme::all() {
            assert_eq!(Scheme::from_tag(s.tag()), Some(s));
        }
        assert_eq!(Scheme::from_tag("ling"), Some(Scheme::LINGMING));
        assert_eq!(Scheme::from_tag("拼音"), Some(Scheme::PINYIN));
        assert_eq!(Scheme::from_tag("nope"), None);
    }

    /// #211: what `bare` draws is the candidate a Space would take, so moving
    /// the highlight moves what the sentence shows.
    #[test]
    fn the_inline_candidate_follows_the_highlight() {
        let mut ime = synthetic_session();
        assert_eq!(ime.inline_candidate(), "", "nothing composed, nothing shown");
        ime.input('b');
        assert_eq!(ime.inline_candidate(), "吧");
        ime.move_highlight(1);
        assert_eq!(ime.inline_candidate(), "八");
        ime.escape();
        assert_eq!(ime.inline_candidate(), "");
    }

    /// #211: `Tab` is for one word, and it cannot be armed for the next.
    #[test]
    fn a_summoned_panel_dies_with_the_word_it_was_summoned_for() {
        let mut ime = synthetic_session();
        ime.set_panel_display(PanelDisplay::Bare);
        // Nothing to summon it for: a `Tab` that fell through must not leave
        // the panel waiting for whatever is typed next.
        assert!(!ime.summon_panel());
        ime.input('b');
        assert!(ime.summon_panel());
        assert!(ime.panel_is_full());
        // A second `Tab` takes it back down — a key that cannot be un-pressed
        // is a trap.
        assert!(ime.summon_panel());
        assert!(!ime.panel_is_full(), "Tab again puts it away");
        assert!(ime.summon_panel());
        assert!(ime.panel_is_full(), "and again brings it back");
        ime.space();
        assert_eq!(ime.take_committed(), "吧");
        assert!(!ime.panel_is_full(), "gone with the word");
        ime.input('b');
        assert!(!ime.panel_is_full(), "and not inherited by the next one");
        // Under `full` there is nothing to summon, and the setting outranks it.
        ime.set_panel_display(PanelDisplay::Full);
        assert!(!ime.summon_panel());
        assert!(ime.panel_is_full());
    }

    #[test]
    fn scheme_cycles_in_order() {
        assert_eq!(Scheme::LINGMING.next(), Scheme::XINGCHEN);
        assert_eq!(Scheme::PINYIN.next(), Scheme::LINGMING);
    }

    #[test]
    fn the_commit_method_is_the_writers_and_it_changes_what_a_key_does() {
        // 唯一上屏: `a` is 啊 and nothing else, so the code is finished the
        // moment it is typed and the character goes without a Space. Under
        // 延遲 the same key waits (Feature #209).
        let mut s = synthetic_session();
        assert_eq!(s.commit_strategy(), CommitStrategy::Delayed);
        assert_eq!(s.commit_override(), None, "nobody has chosen yet");
        s.input('a');
        assert_eq!(s.take_committed(), "", "延遲 waits");
        s.escape();

        s.set_commit_strategy(Some(CommitStrategy::Unique));
        assert_eq!(s.commit_strategy(), CommitStrategy::Unique);
        s.input('a');
        assert_eq!(s.take_committed(), "啊", "唯一 goes on its own");
        // 吧/八 share `b`, so even 唯一 has to ask.
        s.input('b');
        assert_eq!(s.take_committed(), "");
        s.escape();

        // 整句 never goes on its own, whatever the code.
        s.set_commit_strategy(Some(CommitStrategy::Fluency));
        s.input('a');
        assert_eq!(s.take_committed(), "");
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
        let mut s = ImeSession::new(Scheme::LINGMING, vec![PathBuf::from("/no/such/dir")]);
        // Whether it can depends on the machine that *built* this binary, and
        // both answers are correct — so the test is that the two facts agree,
        // not that either one holds.
        assert_eq!(s.available(), has_builtin_table());
        assert_eq!(s.is_builtin(), has_builtin_table());
        assert_eq!(s.scheme(), Scheme::LINGMING);
        if has_builtin_table() {
            s.input('a');
            assert!(!s.page_candidates().is_empty(), "and it really answers");
            assert!(
                s.table_source().starts_with("出廠自帶"),
                "and says which one: {}",
                s.table_source()
            );
        }

        // Only 靈明 — the others are installed, and without their tables the
        // session is honestly unavailable rather than silently 靈明.
        let other = ImeSession::new(Scheme::RIYUE, vec![PathBuf::from("/no/such/dir")]);
        assert!(!other.available());
        assert_eq!(other.scheme(), Scheme::RIYUE);
    }
}

#[cfg(test)]
mod data_faults {
    use super::*;

    /// A data directory with one file in it, written by hand.
    fn dir_with(name: &str, bytes: &[u8], tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("yumete-fault-{tag}-{}", std::process::id()));
        let path = dir.join(name.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("fixture dir");
        }
        std::fs::write(&path, bytes).expect("fixture file");
        dir
    }

    /// The manifest entry for one kind, whatever it is called this month.
    ///
    /// 拆分 is per-scheme (the 字根表 is), so the search covers both halves of
    /// what a session loads rather than only the shared list.
    fn entry(kind: DataKind) -> DataFile {
        data_manifest::shared()
            .into_iter()
            .chain(data_manifest::for_scheme(Scheme::LINGMING.tag()))
            .find(|f| f.kind == kind)
            .expect("the manifest has one of these")
    }

    /// The whole point of #220: a file **left over from an older Yume** is
    /// present, is refused, and used to be indistinguishable from one that was
    /// never installed. `.ydiv` changed its magic on 2026-09-04 and every old
    /// data directory lost its 拆分 comments without a word.
    #[test]
    fn an_old_file_says_which_version_it_is() {
        let entry = entry(DataKind::Annotations);
        let mut old = b"YDV20260828".to_vec();
        old.extend_from_slice(&[0u8; 64]);
        let dir = dir_with(&entry.file, &old, "old");

        let mut engine = Engine::new(CodeTable::new());
        let problem = load_data_file(&mut engine, &[dir], &entry).expect_err("refused");
        assert_eq!(problem.file, entry.file);
        assert!(problem.is_loud(), "an installed file that does nothing is loud");
        let DataFault::Rejected { reason, magic } = &problem.fault else {
            panic!("expected a rejection, got {:?}", problem.fault);
        };
        assert!(!reason.is_empty(), "the core's own words are the reason");
        let magic = magic.as_ref().expect("both magics are printable ASCII");
        assert_eq!(magic.found, "YDV20260828");
        assert_eq!(
            magic.expected,
            String::from_utf8_lossy(yume_core::division::MAGIC)
        );
    }

    /// A file that is not there is **not** loud. Half the manifest is optional
    /// and an ordinary install is missing several; if those spoke up, the one
    /// line that matters would be buried.
    #[test]
    fn a_file_that_was_never_installed_is_quiet() {
        let entry = entry(DataKind::Annotations);
        let mut engine = Engine::new(CodeTable::new());
        let problem = load_data_file(&mut engine, &[PathBuf::from("/no/such/dir")], &entry)
            .expect_err("missing");
        assert_eq!(problem.fault, DataFault::Missing);
        assert!(!problem.is_loud());
    }

    /// Bytes that are not a header at all: there is a reason, and no mismatch
    /// to show. A 檔頭 nobody can read is worse than none, because it looks
    /// like the answer.
    #[test]
    fn unreadable_bytes_give_a_reason_and_no_magic() {
        let entry = entry(DataKind::Annotations);
        let dir = dir_with(&entry.file, &[0xff; 40], "junk");
        let mut engine = Engine::new(CodeTable::new());
        let problem = load_data_file(&mut engine, &[dir], &entry).expect_err("refused");
        let DataFault::Rejected { reason, magic } = &problem.fault else {
            panic!("expected a rejection, got {:?}", problem.fault);
        };
        assert!(!reason.is_empty());
        assert!(magic.is_none(), "not printable, so not shown");
    }

    /// The session keeps them, which is what `:yume` reads.
    #[test]
    fn a_session_remembers_what_did_not_load() {
        let entry = entry(DataKind::Annotations);
        let mut old = b"YDV20260828".to_vec();
        old.extend_from_slice(&[0u8; 64]);
        let dir = dir_with(&entry.file, &old, "session");
        let ime = ImeSession::new(Scheme::LINGMING, vec![dir]);
        let loud: Vec<&DataProblem> = ime.problems().iter().filter(|p| p.is_loud()).collect();
        assert_eq!(loud.len(), 1, "only the one that is there and refused");
        assert_eq!(loud[0].file, entry.file);
        // Everything else in that directory is simply absent, and the session
        // still carries those — quietly.
        assert!(ime.problems().len() > 1);
    }
    /// #220: the manifest's two halves overlap, and the overlap used to be
    /// loaded twice — 13 MB of 讀音表 parsed a second time to arrive at the
    /// same table.
    #[test]
    fn the_data_set_names_每一份_once() {
        for scheme in Scheme::all() {
            let set = data_set(scheme);
            let mut seen: Vec<(DataKind, &str, &str, i32)> = Vec::new();
            for f in &set {
                let key = (f.kind, f.file.as_str(), f.aux.as_str(), f.slot);
                assert!(!seen.contains(&key), "{scheme:?} names {} twice", f.file);
                seen.push(key);
            }
            // And the reason it can happen: `shared()` carries the 讀音表 and
            // `for_scheme` appends one to a scheme that has none of its own.
            assert!(
                set.iter().filter(|f| f.kind == DataKind::Reading).count() <= 1,
                "{scheme:?} loads more than one 讀音表"
            );
        }
    }

    /// A `.ydiv` yume-core accepts and a `.yzg` it refuses arrive as one
    /// manifest entry. Losing the second must not cost the first: 拆分・讀音・
    /// 字集 are all off the divisions, and only 編碼 is off the 字根表.
    #[test]
    fn a_refused_字根表_keeps_the_拆分表_that_was_good() {
        let dir = std::env::temp_dir().join(format!(
            "yumete-zigen-fault-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let entry = data_set(Scheme::LINGMING)
            .into_iter()
            .find(|f| f.kind == DataKind::Annotations)
            .expect("拆分 is in the manifest");
        let lay = |relative: &str, bytes: &[u8]| {
            let mut path = dir.clone();
            for part in relative.split('/') {
                path.push(part);
            }
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("fixture dir");
            std::fs::write(&path, bytes).expect("fixture file");
        };
        // The smallest 拆分表 the core will have: one 字根 and no entries.
        let mut ydiv = yume_core::division::MAGIC.to_vec();
        ydiv.extend_from_slice(&1u32.to_le_bytes()); // one root
        ydiv.extend_from_slice(&(u32::from('木')).to_le_bytes());
        for _ in 0..3 {
            ydiv.extend_from_slice(&0u32.to_le_bytes()); // labels, 讀音, 注釋
        }
        lay(&entry.file, &ydiv);
        lay(&entry.aux, "not a 字根表 at all".as_bytes());

        let mut engine = Engine::new(CodeTable::new());
        let problem = load_data_file(&mut engine, &[dir.clone()], &entry)
            .expect_err("the 字根表 is refused");
        let _ = std::fs::remove_dir_all(&dir);

        // The problem names the 字根表, not the 拆分表 …
        assert_eq!(problem.file, entry.aux);
        assert!(matches!(problem.fault, DataFault::Rejected { .. }));
        // … and the 拆分表 is in the engine anyway.
        assert_eq!(engine.annotations().divisions().roots(), &['木']);
    }
}

#[cfg(test)]
mod timing {
    use super::*;
    use std::time::Instant;

    /// Where a launch's time actually goes. A measurement, not an assertion:
    /// `cargo test -p yumete-ime --release what_startup_costs -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn what_startup_costs() {
        let t = Instant::now();
        let ime = ImeSession::language_only(Scheme::LINGMING);
        println!("language_only:      {:.1?}", t.elapsed());
        let t = Instant::now();
        let seg = ime.segmenter();
        println!("segmenter():        {:.1?}  (available: {})", t.elapsed(), seg.is_available());
        let t = Instant::now();
        let words = yumete_cjk::Segmenter::segment(&seg, "那年冬天下雪以後，岳復山聽到了如竹笛般清脆的聲音。");
        println!("first segment:      {:.1?}  ({} words)", t.elapsed(), words.len());
        let t = Instant::now();
        for _ in 0..100 {
            let _ = yumete_cjk::Segmenter::segment(&seg, "那年冬天下雪以後，岳復山聽到了如竹笛般清脆的聲音。");
        }
        println!("100 more segments:  {:.1?}", t.elapsed());
        let t = Instant::now();
        let mut full = ImeSession::from_default_dirs(Scheme::LINGMING);
        println!("碼表 (from_default_dirs): {:.1?} (available: {})", t.elapsed(), full.available());
        let _ = &mut full;
    }
}
