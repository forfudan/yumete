//! `yumete-config` — global and per-project configuration for yumete.
//!
//! Configuration is TOML, following the XDG convention:
//!
//! - **Global** (Feature #21): `$XDG_CONFIG_HOME/yumete/config.toml`
//!   (or `~/.config/yumete/config.toml`).
//! - **Per-project local override** (Feature #22): the nearest `.yumete/config.toml`
//!   (or `.yumete.toml`) found by walking up from the working directory. Its
//!   values override the global ones, key by key.
//! - **Keymap** (Feature #23): a `[keys.normal]` table remaps Normal-mode keys.
//!
//! The loader never fails: missing files and parse errors fall back to defaults,
//! so a broken config can't stop the editor from starting.

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub use yumete_cjk::{Layout, DEFAULT_ZONG_GAP, DEFAULT_ZONG_LENGTH};

/// How line numbers are displayed in the gutter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineNumbers {
    /// The absolute 1-based line number on every line.
    Absolute,
    /// The distance from the cursor line, with the cursor line absolute.
    Relative,
    /// No line-number gutter.
    None,
}

/// Editor behaviour settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditorConfig {
    /// Tab stop width in cells.
    pub tab_width: usize,
    /// Line-number display mode.
    pub line_numbers: LineNumbers,
    /// Minimum number of lines to keep above/below the cursor when scrolling.
    pub scrolloff: usize,
    /// Whether the word-segmentation overlay is shown at start-up (Feature #24).
    /// On by default so the CJK word grouping is visible; toggle with `:segment`
    /// or set `show_segmentation = false`.
    pub show_segmentation: bool,
    /// Minimum weight for a multi-character word to be joined by the dictionary
    /// segmenter (Feature #24). Zero joins every dictionary word.
    pub segmentation_threshold: i64,
    /// Horizontal (default) or vertical layout (Feature #61).
    pub layout: Layout,
    /// How many characters fit in one 縱 in vertical layout.
    ///
    /// `0` — the default — means **as many as the window allows**, which is the
    /// same rule the horizontal `measure` follows: how long a column should be
    /// is a decision about the book, and the editor has no business making one
    /// for you. A number here (or `:wrap n`) is that decision; it is clamped to
    /// 4–64, and the renderer lowers it further when the terminal is short.
    pub zong_length: usize,
    /// The gap between two 縱, in half-width cells (0–4).
    pub zong_gap: usize,
    /// Whether the 拆分 annotation is shown beside candidates (Feature #66).
    /// Off by default: it is a study aid, and it widens every candidate.
    pub show_chaifen: bool,
    /// Whether readings are laid out at all (Feature #65).
    ///
    /// **Off** by default, like every other kind of markup: what is in the file
    /// is what is on the page, until you ask otherwise. `:ruby-on` lays them
    /// out, and 所見即所得 mode lays them out along with everything else.
    pub show_ruby: bool,
    /// Extra ruby dialects to lay out beyond the one the file's extension
    /// implies — a document that mixes them names them all here.
    pub ruby_dialects: Vec<String>,
    /// Whether a pair of half-width characters shares one slot in vertical
    /// layout (縦中横). Off by default: one letter to a row, hung right.
    pub tatechuyoko: bool,
    /// Whether 句讀 hang in the margin beside the character they follow rather
    /// than taking a square each (標點旁置). Off by default; `:hanging` toggles.
    pub hanging_punctuation: bool,
    /// Whether a paragraph too wide for the terminal continues on the next
    /// screen row (Feature #77). On by default: a Chinese paragraph is one long
    /// line, and unwrapped most of it cannot be seen at all.
    pub soft_wrap: bool,
    /// Whether a recovery copy is kept beside each document while it has
    /// unsaved changes (Feature #79). On by default, and removed on save and on
    /// quit, so in an ordinary session it is never seen.
    pub autosave: bool,
    /// Whether East-Asian Ambiguous characters — `—` `…` `“” ‘’` `·` — are two
    /// cells wide (Feature #81). `"wide"` by default: this is an editor for
    /// 漢字 prose, and those characters are drawn wide by the CJK fonts such
    /// prose is read in. `"narrow"` for a Latin font.
    pub ambiguous_wide: bool,
    /// The measure a horizontal page is written to, in cells; `0` for none
    /// (Feature #101).
    ///
    /// Everything past it is tinted, which is information even with soft wrap
    /// on: it says this row has run past the length you want your sentences to
    /// be, which is what somebody breaking long sentences by hand is looking
    /// for. The *line* is only drawn with wrap off — with it on, the edge of
    /// the tint is already the line.
    pub ruler: usize,
    /// The width to write to, in columns; `0` for the width of the window.
    ///
    /// A measure rather than a mark: unlike `ruler`, which only says where the
    /// line is, this folds the rows there and leaves the rest of the window as
    /// margin. `:wrap 50` sets it for one session (Feature #113).
    pub measure: usize,
    /// Whether the status line names the character under the cursor
    /// (Feature #117).
    ///
    /// `冬 U+51AC · CJK Unified Ideographs`, at the right edge. For a writer
    /// setting rare 漢字 this is the difference between "my font is missing
    /// this" and "this is the wrong character"; for a 拆分表 it is the whole
    /// question. Given way to, right end first, when the line is crowded.
    pub char_info: bool,
    /// Whether a hint row sits above the status line (Feature #122).
    ///
    /// It costs one row of the window always — set vertically that is one 字
    /// off every 縱 — and buys the keys that finish a sequence you have begun,
    /// which is otherwise only in the manual.
    pub hints: bool,
    /// Whether the 縱書 page is packed as tight as a terminal allows.
    ///
    /// On by default: a terminal has few enough columns as it is, and the gap,
    /// the reading column, the hung margin and the ticks together cost about a
    /// third of them. `:dense off` gives them back for as long as you want
    /// them — it suppresses those things, it does not turn them off, so what
    /// the config says about readings and 句讀 is still what it says.
    pub dense: bool,
    /// A tick every `paper_ticks` characters down a 縱; `0` for none
    /// (Feature #102).
    ///
    /// 稿紙 is ruled, and a writer estimates length by it. The vertical page is
    /// already a grid of squares, so this costs one dim cell in a margin that
    /// is otherwise blank — and it is the one thing a horizontal editor has no
    /// equivalent of.
    ///
    /// Off by default, because switching it on gives *every* 縱 a margin —
    /// including the rightmost, which otherwise sits flush against the edge —
    /// and so moves the whole page a column. That is a change to what somebody
    /// is already looking at, and it should be asked for.
    pub paper_ticks: usize,
    /// Which markup the files here are written in, when their names do not say
    /// (Feature #106). Empty means: read the file and decide.
    ///
    /// This is what a per-project `.yumete/config.toml` is for. A novel written
    /// in Typst but filed as `.txt` — chapters pulled in by `#include` — cannot
    /// always be told from prose by reading it: a chapter that is nothing but
    /// writing has no Typst in it to find. One line in the directory settles
    /// the whole manuscript.
    pub syntax: String,
    /// When the tab bar is drawn (Feature #95).
    pub tabs: Tabs,
    /// How many columns the file sidebar takes when it is open (Feature #94).
    ///
    /// It costs columns, and set vertically it costs them by threes — a 縱 is
    /// two cells and a gap — so twenty-four is eight 縱 of page. Narrow enough
    /// to be worth the trade, wide enough for `卷二/驚蟄.md`.
    pub sidebar_width: usize,
}

impl Default for EditorConfig {
    fn default() -> Self {
        EditorConfig {
            tab_width: 4,
            line_numbers: LineNumbers::Absolute,
            scrolloff: 3,
            show_segmentation: true,
            segmentation_threshold: 0,
            layout: Layout::Horizontal,
            zong_length: 0,
            zong_gap: DEFAULT_ZONG_GAP,
            show_chaifen: false,
            show_ruby: false,
            ruby_dialects: Vec::new(),
            tatechuyoko: false,
            hanging_punctuation: false,
            soft_wrap: true,
            autosave: true,
            ambiguous_wide: true,
            syntax: String::new(),
            ruler: 0,
            measure: 0,
            char_info: true,
            hints: true,
            dense: true,
            paper_ticks: 0,
            tabs: Tabs::default(),
            sidebar_width: 24,
        }
    }
}

/// When the tab bar showing the open files is drawn (Feature #95).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tabs {
    /// Only when more than one file is open. A row is a row, and a single tab
    /// says nothing that the status line does not already say.
    #[default]
    Auto,
    /// Always, even for one file.
    Always,
    /// Never; the status line says which file this is.
    Never,
}

impl Tabs {
    /// Parse a config value.
    pub fn parse(value: &str) -> Option<Tabs> {
        match value.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(Tabs::Auto),
            "always" | "true" | "on" => Some(Tabs::Always),
            "never" | "false" | "off" => Some(Tabs::Never),
            _ => None,
        }
    }

    /// Whether to draw the bar with `open` files open.
    pub fn showing(self, open: usize) -> bool {
        match self {
            Tabs::Auto => open > 1,
            Tabs::Always => true,
            Tabs::Never => false,
        }
    }
}

/// The input method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImeConfig {
    /// Which scheme to start in (方案): `lingming`, `xingchen`, `qingyun`,
    /// `riyue`, `pinyin`. Only 靈明 ships with yumete; the others need their
    /// tables installed in the data directory.
    pub scheme: String,
    /// Whether that scheme's 碼表 is loaded at startup.
    ///
    /// Off by default. The 碼表 is a hundred milliseconds and is of use only to
    /// somebody who came to type 宇浩; the *language model* behind `w` and `b`
    /// is loaded either way, because that is about the words and not about how
    /// they are typed. `:yume scheme` starts typing whenever you want it.
    pub start: bool,
}

impl Default for ImeConfig {
    fn default() -> Self {
        ImeConfig {
            scheme: "lingming".to_string(),
            start: false,
        }
    }
}

/// The candidate panel's appearance.
///
/// The colours are **two** values, not thirteen: an ink and a ground, with the
/// shades between them interpolated. That is how Yume's own themes are defined
/// (`yume_core::themes::ink_ladder`) and it is the reason a skin can be changed
/// by editing a pair of numbers rather than a table — the relationships between
/// the shades stay right by construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PanelConfig {
    /// The characters candidates are numbered with, in order.
    ///
    /// Each has to be **two cells wide**, or the columns come apart: the
    /// circled Chinese numerals ㊀㊁㊂ are, while the circled Arabic ①②③ are
    /// East-Asian *ambiguous* and may be drawn either way. A value that is too
    /// short simply runs out and the rest fall back to plain digits.
    pub markers: String,
    /// 墨 — the colour candidates are drawn in.
    pub ink: (u8, u8, u8),
    /// 紙 — the panel's ground.
    pub paper: (u8, u8, u8),
    /// How many candidates a page offers.
    pub page_size: usize,
    /// Whether the panel's ring is rounded.
    pub rounded: bool,
}

impl Default for PanelConfig {
    fn default() -> Self {
        PanelConfig {
            // 帶圈中文數字, which are unambiguously wide.
            markers: "㊀㊁㊂㊃㊄㊅㊆㊇㊈".to_string(),
            // Yume's 墨香, dark: warm ink on a deep ground.
            ink: (0xCF, 0xC6, 0xA9),
            paper: (0x26, 0x2A, 0x27),
            page_size: 9,
            rounded: true,
        }
    }
}

/// Theme (colour) settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeConfig {
    /// The selection background, as an RGB triple.
    pub selection: (u8, u8, u8),
    /// The two alternating word-background tints for the segmentation overlay
    /// (Feature #24). Kept subtle so the overlay is not intrusive.
    pub segmentation: [(u8, u8, u8); 2],
    /// The ruler: the tint over what runs past the measure, and the line
    /// itself when there is one (Feature #101).
    pub ruler: (u8, u8, u8),
    /// The background of the paragraph-number band (Feature #89).
    ///
    /// Every other editor separates line numbers from the text by *position* —
    /// a gutter column the text can never enter — so a dim colour is enough.
    /// Set vertically the numbers sit above the 縱, in the same columns as the
    /// text, so position separates nothing and they read as digits somebody
    /// typed. Colour has to do the whole job.
    pub gutter: (u8, u8, u8),
}

impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig {
            selection: (60, 70, 100),
            segmentation: [(40, 44, 52), (52, 44, 40)],
            gutter: (36, 38, 44),
            ruler: (46, 48, 56),
        }
    }
}

/// Which markup a file is written in, by extension or by name.
///
/// The reliable answer for a manuscript whose files do not say: a novel written
/// in Typst with its chapters filed as `.txt` is one line here, and then every
/// chapter is read right — including the ones that are nothing but writing and
/// have no Typst in them to find.
///
/// ```toml
/// [syntax]
/// txt = "typst"                 # every .txt in this project
/// "筆記.txt" = "markdown"        # …except this one
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyntaxConfig {
    /// Extension (without the dot) or exact file name → language.
    pub by_name: HashMap<String, String>,
}

impl SyntaxConfig {
    /// What this config says about `name`, if anything.
    ///
    /// The exact name wins over the extension: a project can say "all my `.txt`
    /// are Typst, except that one".
    pub fn of(&self, name: &str) -> Option<&str> {
        if let Some(said) = self.by_name.get(name) {
            return Some(said);
        }
        let extension = name.rsplit_once('.')?.1;
        self.by_name.get(extension).map(String::as_str)
    }
}

/// Keymap overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyConfig {
    /// Normal-mode aliases: pressing the key on the left behaves as pressing
    /// **the keys** on the right.
    ///
    /// A sequence rather than a single key, because the defaults this editor
    /// chose on purpose — `J`/`K` paging a page of a book rather than joining
    /// lines — are exactly the ones a Vim reader wants back, and what they
    /// want back is `gJ`. One config line instead of leaving.
    pub normal: HashMap<char, String>,
}

/// The fully-resolved configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub editor: EditorConfig,
    pub theme: ThemeConfig,
    pub panel: PanelConfig,
    pub ime: ImeConfig,
    pub syntax: SyntaxConfig,
    pub keys: KeyConfig,
}

impl Config {
    /// Load the global config, then apply the nearest per-project override.
    pub fn load() -> Config {
        Config::load_reporting().0
    }

    /// Load the config, and say what went wrong while loading it.
    ///
    /// A config file that does not parse — a typo in a key name, a missing
    /// quote — used to be dropped in silence, and the only sign was that a
    /// setting did not take. Unknown keys are errors for the same reason:
    /// `zong_lenght = 24` must say so rather than look like a setting that does
    /// not work. Each problem is one line, ready for the status bar.
    pub fn load_reporting() -> (Config, Vec<String>) {
        let mut raw = RawConfig::default();
        let mut problems = Vec::new();

        let global = config_dir().join("config.toml");
        if let Ok(text) = fs::read_to_string(&global) {
            match toml::from_str::<RawConfig>(&text) {
                Ok(parsed) => raw.merge(parsed),
                Err(err) => problems.push(Self::describe(&global, &err)),
            }
        }

        if let Ok(cwd) = env::current_dir() {
            if let Some(local) = local_config_path(&cwd) {
                if let Ok(text) = fs::read_to_string(&local) {
                    match toml::from_str::<RawConfig>(&text) {
                        Ok(parsed) => raw.merge(parsed),
                        Err(err) => problems.push(Self::describe(&local, &err)),
                    }
                }
            }
        }

        (raw.into_config(), problems)
    }

    /// One line naming a config file and what is wrong with it.
    fn describe(path: &Path, err: &toml::de::Error) -> String {
        // toml's message names the offending key and what was expected instead;
        // its first line is the part a status bar has room for.
        let detail = err.message().lines().next().unwrap_or("could not be read");
        format!("{}: {detail}", path.display())
    }

    /// Parse a single TOML source (used for a one-file config or in tests).
    pub fn from_toml(source: &str) -> Config {
        toml::from_str::<RawConfig>(source)
            .unwrap_or_default()
            .into_config()
    }

    /// Merge a `global` then a `local` TOML source (used in tests).
    pub fn from_sources(global: Option<&str>, local: Option<&str>) -> Config {
        let mut raw = RawConfig::default();
        if let Some(g) = global {
            if let Ok(parsed) = toml::from_str::<RawConfig>(g) {
                raw.merge(parsed);
            }
        }
        if let Some(l) = local {
            if let Ok(parsed) = toml::from_str::<RawConfig>(l) {
                raw.merge(parsed);
            }
        }
        raw.into_config()
    }
}

/// The global config directory (`$XDG_CONFIG_HOME/yumete` or `~/.config/yumete`).
pub fn config_dir() -> PathBuf {
    if let Ok(dir) = env::var("XDG_CONFIG_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("yumete");
        }
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home).join(".config").join("yumete");
    }
    PathBuf::from(".config").join("yumete")
}

/// The user data directory (`$XDG_DATA_HOME/yumete` or `~/.local/share/yumete`).
///
/// This is where user-added scheme tables and 碼表 live; see the development
/// plan's data-management section for the full resolution order.
pub fn data_dir() -> PathBuf {
    if let Ok(dir) = env::var("XDG_DATA_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir).join("yumete");
        }
    }
    if let Ok(home) = env::var("HOME") {
        return PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("yumete");
    }
    PathBuf::from(".local").join("share").join("yumete")
}

/// The install-prefix data directory that ships alongside the binary.
///
/// Resolves `<prefix>/bin/yumete` to `<prefix>/share/yumete`, which is where a
/// Homebrew (or manual) install places the bundled scheme tables and fonts.
/// Returns `None` if the executable path can't be determined.
pub fn installed_data_dir() -> Option<PathBuf> {
    let exe = env::current_exe().ok()?;
    let prefix = exe.parent()?.parent()?;
    Some(prefix.join("share").join("yumete"))
}

/// The ordered list of directories to search for IME data (first found wins):
/// the user data dir, then the install-prefix data dir.
pub fn data_search_dirs() -> Vec<PathBuf> {
    let mut dirs = vec![data_dir()];
    if let Some(installed) = installed_data_dir() {
        dirs.push(installed);
    }
    dirs
}

/// Find the nearest per-project config by walking up from `start`.
pub fn local_config_path(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let nested = d.join(".yumete").join("config.toml");
        if nested.is_file() {
            return Some(nested);
        }
        let flat = d.join(".yumete.toml");
        if flat.is_file() {
            return Some(flat);
        }
        dir = d.parent();
    }
    None
}

// ---- Raw (as-parsed) config with per-field merge -------------------------

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    editor: RawEditor,
    #[serde(default)]
    theme: RawTheme,
    #[serde(default)]
    panel: RawPanel,
    #[serde(default)]
    ime: RawIme,
    #[serde(default)]
    syntax: HashMap<String, String>,
    #[serde(default)]
    keys: RawKeys,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawIme {
    scheme: Option<String>,
    start: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawPanel {
    markers: Option<String>,
    ink: Option<String>,
    paper: Option<String>,
    page_size: Option<usize>,
    rounded: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawEditor {
    tab_width: Option<usize>,
    line_numbers: Option<String>,
    scrolloff: Option<usize>,
    show_segmentation: Option<bool>,
    segmentation_threshold: Option<i64>,
    layout: Option<String>,
    zong_length: Option<usize>,
    zong_gap: Option<usize>,
    show_chaifen: Option<bool>,
    show_ruby: Option<bool>,
    ruby_dialects: Option<Vec<String>>,
    tatechuyoko: Option<bool>,
    hanging_punctuation: Option<bool>,
    soft_wrap: Option<bool>,
    autosave: Option<bool>,
    ambiguous_width: Option<String>,
    sidebar_width: Option<usize>,
    tabs: Option<String>,
    syntax: Option<String>,
    ruler: Option<usize>,
    measure: Option<usize>,
    char_info: Option<bool>,
    hints: Option<bool>,
    dense: Option<bool>,
    paper_ticks: Option<usize>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawTheme {
    selection: Option<String>,
    segmentation: Option<Vec<String>>,
    gutter: Option<String>,
    ruler: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawKeys {
    #[serde(default)]
    normal: HashMap<String, String>,
}

impl RawConfig {
    /// Overlay `other` onto `self`, field by field (later wins; keymaps extend).
    fn merge(&mut self, other: RawConfig) {
        if other.editor.tab_width.is_some() {
            self.editor.tab_width = other.editor.tab_width;
        }
        if other.editor.line_numbers.is_some() {
            self.editor.line_numbers = other.editor.line_numbers;
        }
        if other.editor.scrolloff.is_some() {
            self.editor.scrolloff = other.editor.scrolloff;
        }
        if other.editor.show_segmentation.is_some() {
            self.editor.show_segmentation = other.editor.show_segmentation;
        }
        if other.editor.segmentation_threshold.is_some() {
            self.editor.segmentation_threshold = other.editor.segmentation_threshold;
        }
        if other.editor.layout.is_some() {
            self.editor.layout = other.editor.layout;
        }
        if other.editor.zong_length.is_some() {
            self.editor.zong_length = other.editor.zong_length;
        }
        if other.editor.zong_gap.is_some() {
            self.editor.zong_gap = other.editor.zong_gap;
        }
        if other.editor.show_chaifen.is_some() {
            self.editor.show_chaifen = other.editor.show_chaifen;
        }
        if other.editor.show_ruby.is_some() {
            self.editor.show_ruby = other.editor.show_ruby;
        }
        if other.editor.ruby_dialects.is_some() {
            self.editor.ruby_dialects = other.editor.ruby_dialects.clone();
        }
        if other.editor.tatechuyoko.is_some() {
            self.editor.tatechuyoko = other.editor.tatechuyoko;
        }
        if other.editor.hanging_punctuation.is_some() {
            self.editor.hanging_punctuation = other.editor.hanging_punctuation;
        }
        if other.editor.soft_wrap.is_some() {
            self.editor.soft_wrap = other.editor.soft_wrap;
        }
        if other.editor.autosave.is_some() {
            self.editor.autosave = other.editor.autosave;
        }
        if other.editor.ambiguous_width.is_some() {
            self.editor.ambiguous_width = other.editor.ambiguous_width.clone();
        }
        if other.editor.sidebar_width.is_some() {
            self.editor.sidebar_width = other.editor.sidebar_width;
        }
        if other.editor.tabs.is_some() {
            self.editor.tabs = other.editor.tabs.clone();
        }
        if other.editor.syntax.is_some() {
            self.editor.syntax = other.editor.syntax.clone();
        }
        if other.editor.ruler.is_some() {
            self.editor.ruler = other.editor.ruler;
        }
        if other.editor.measure.is_some() {
            self.editor.measure = other.editor.measure;
        }
        if other.editor.char_info.is_some() {
            self.editor.char_info = other.editor.char_info;
        }
        if other.editor.hints.is_some() {
            self.editor.hints = other.editor.hints;
        }
        if other.editor.dense.is_some() {
            self.editor.dense = other.editor.dense;
        }
        if other.editor.paper_ticks.is_some() {
            self.editor.paper_ticks = other.editor.paper_ticks;
        }
        if other.theme.selection.is_some() {
            self.theme.selection = other.theme.selection;
        }
        if other.theme.segmentation.is_some() {
            self.theme.segmentation = other.theme.segmentation;
        }
        if other.theme.gutter.is_some() {
            self.theme.gutter = other.theme.gutter;
        }
        if other.theme.ruler.is_some() {
            self.theme.ruler = other.theme.ruler;
        }
        if other.ime.scheme.is_some() {
            self.ime.scheme = other.ime.scheme.clone();
        }
        if other.ime.start.is_some() {
            self.ime.start = other.ime.start;
        }
        // Merged entry by entry, so a project can add to what the global config
        // says rather than having to restate it.
        for (name, language) in &other.syntax {
            self.syntax.insert(name.clone(), language.clone());
        }
        if other.panel.markers.is_some() {
            self.panel.markers = other.panel.markers;
        }
        if other.panel.ink.is_some() {
            self.panel.ink = other.panel.ink;
        }
        if other.panel.paper.is_some() {
            self.panel.paper = other.panel.paper;
        }
        if other.panel.page_size.is_some() {
            self.panel.page_size = other.panel.page_size;
        }
        if other.panel.rounded.is_some() {
            self.panel.rounded = other.panel.rounded;
        }
        for (k, v) in other.keys.normal {
            self.keys.normal.insert(k, v);
        }
    }

    fn into_config(self) -> Config {
        let mut config = Config::default();
        if let Some(tab) = self.editor.tab_width {
            config.editor.tab_width = tab.max(1);
        }
        if let Some(mode) = self.editor.line_numbers {
            config.editor.line_numbers = parse_line_numbers(&mode);
        }
        if let Some(off) = self.editor.scrolloff {
            config.editor.scrolloff = off;
        }
        if let Some(on) = self.editor.show_segmentation {
            config.editor.show_segmentation = on;
        }
        if let Some(threshold) = self.editor.segmentation_threshold {
            config.editor.segmentation_threshold = threshold.max(0);
        }
        if let Some(layout) = self.editor.layout {
            // An unrecognised value keeps the default rather than refusing to
            // start, like every other setting here.
            if let Some(parsed) = Layout::parse(&layout) {
                config.editor.layout = parsed;
            }
        }
        if let Some(length) = self.editor.zong_length {
            // Below ~4 a 縱 stops being a column of text; above 64 no terminal
            // is tall enough and the eye loses the sweep anyway.
            // `0` is not a length, it is "as long as the window allows", so it
            // passes through rather than being clamped up to four.
            config.editor.zong_length = if length == 0 { 0 } else { length.clamp(4, 64) };
        }
        if let Some(gap) = self.editor.zong_gap {
            config.editor.zong_gap = gap.min(4);
        }
        if let Some(on) = self.editor.show_chaifen {
            config.editor.show_chaifen = on;
        }
        if let Some(on) = self.editor.show_ruby {
            config.editor.show_ruby = on;
        }
        if let Some(dialects) = self.editor.ruby_dialects {
            config.editor.ruby_dialects = dialects;
        }
        if let Some(on) = self.editor.tatechuyoko {
            config.editor.tatechuyoko = on;
        }
        if let Some(on) = self.editor.hanging_punctuation {
            config.editor.hanging_punctuation = on;
        }
        if let Some(on) = self.editor.soft_wrap {
            config.editor.soft_wrap = on;
        }
        if let Some(on) = self.editor.autosave {
            config.editor.autosave = on;
        }
        if let Some(ruler) = self.editor.ruler {
            config.editor.ruler = ruler.min(400);
        }
        if let Some(measure) = self.editor.measure {
            config.editor.measure = measure.min(400);
        }
        if let Some(on) = self.editor.char_info {
            config.editor.char_info = on;
        }
        if let Some(on) = self.editor.hints {
            config.editor.hints = on;
        }
        if let Some(on) = self.editor.dense {
            config.editor.dense = on;
        }
        if let Some(ticks) = self.editor.paper_ticks {
            config.editor.paper_ticks = ticks.min(64);
        }
        if let Some(syntax) = self.editor.syntax {
            config.editor.syntax = syntax;
        }
        if let Some(tabs) = self.editor.tabs {
            if let Some(parsed) = Tabs::parse(&tabs) {
                config.editor.tabs = parsed;
            }
        }
        if let Some(width) = self.editor.sidebar_width {
            config.editor.sidebar_width = width.clamp(12, 60);
        }
        if let Some(width) = self.editor.ambiguous_width {
            // An unknown value keeps the default rather than picking one: a
            // typo here shifts every line on the page.
            match width.trim().to_ascii_lowercase().as_str() {
                "wide" | "double" | "full" => config.editor.ambiguous_wide = true,
                "narrow" | "single" | "half" => config.editor.ambiguous_wide = false,
                _ => {}
            }
        }
        if let Some(scheme) = self.ime.scheme {
            config.ime.scheme = scheme;
        }
        if let Some(start) = self.ime.start {
            config.ime.start = start;
        }
        config.syntax.by_name = self.syntax;
        if let Some(hex) = self.theme.selection {
            if let Some(rgb) = parse_hex(&hex) {
                config.theme.selection = rgb;
            }
        }
        if let Some(colors) = self.theme.segmentation {
            for (slot, hex) in config.theme.segmentation.iter_mut().zip(colors.iter()) {
                if let Some(rgb) = parse_hex(hex) {
                    *slot = rgb;
                }
            }
        }
        if let Some(hex) = self.theme.gutter {
            if let Some(rgb) = parse_hex(&hex) {
                config.theme.gutter = rgb;
            }
        }
        if let Some(hex) = self.theme.ruler {
            if let Some(rgb) = parse_hex(&hex) {
                config.theme.ruler = rgb;
            }
        }
        if let Some(markers) = self.panel.markers {
            // An empty list would leave every candidate unnumbered; keep the
            // default rather than silently taking the numbers away.
            if !markers.is_empty() {
                config.panel.markers = markers;
            }
        }
        if let Some(rgb) = self.panel.ink.as_deref().and_then(parse_hex) {
            config.panel.ink = rgb;
        }
        if let Some(rgb) = self.panel.paper.as_deref().and_then(parse_hex) {
            config.panel.paper = rgb;
        }
        if let Some(size) = self.panel.page_size {
            // Below two there is nothing to choose between; above nine there is
            // no key left to choose with.
            config.panel.page_size = size.clamp(2, 9);
        }
        if let Some(rounded) = self.panel.rounded {
            config.panel.rounded = rounded;
        }
        for (k, v) in self.keys.normal {
            // The key on the left is one key — there is no key sequence to
            // *press* here, only one to be sent — and the right may be any
            // number of them.
            let mut ks = k.chars();
            if let Some(from) = ks.next() {
                if ks.next().is_none() && !v.is_empty() {
                    config.keys.normal.insert(from, v);
                }
            }
        }
        config
    }
}

fn parse_line_numbers(value: &str) -> LineNumbers {
    match value.trim().to_ascii_lowercase().as_str() {
        "relative" | "rel" => LineNumbers::Relative,
        "none" | "off" | "false" => LineNumbers::None,
        _ => LineNumbers::Absolute,
    }
}

fn parse_hex(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.trim().trim_start_matches('#');
    if hex.len() != 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some((r, g, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let c = Config::from_toml("");
        assert_eq!(c.editor.tab_width, 4);
        assert_eq!(c.editor.line_numbers, LineNumbers::Absolute);
        assert_eq!(c.editor.scrolloff, 3);
        assert_eq!(c.theme.selection, (60, 70, 100));
        assert!(c.keys.normal.is_empty());
    }

    #[test]
    fn parses_settings_and_theme() {
        let c = Config::from_toml(
            r##"
            [editor]
            tab_width = 2
            line_numbers = "relative"
            scrolloff = 5

            [theme]
            selection = "#204060"
            "##,
        );
        assert_eq!(c.editor.tab_width, 2);
        assert_eq!(c.editor.line_numbers, LineNumbers::Relative);
        assert_eq!(c.editor.scrolloff, 5);
        assert_eq!(c.theme.selection, (0x20, 0x40, 0x60));
    }

    #[test]
    fn parses_keymap_aliases() {
        let c = Config::from_toml(
            r#"
            [keys.normal]
            "j" = "h"
            "J" = "gJ"
            "toolong" = "x"
            "#,
        );
        assert_eq!(c.keys.normal.get(&'j').map(String::as_str), Some("h"));
        // The right-hand side may be a whole sequence: one config line puts
        // vi's join back on `J` without the editor keeping two spellings.
        assert_eq!(c.keys.normal.get(&'J').map(String::as_str), Some("gJ"));
        // Multi-character keys are ignored — there is no sequence to press.
        assert!(!c.keys.normal.contains_key(&'t'));
    }

    #[test]
    fn parses_segmentation_settings() {
        let c = Config::from_toml(
            r##"
            [editor]
            show_segmentation = true
            segmentation_threshold = 50

            [theme]
            segmentation = ["#101010", "#202020"]
            "##,
        );
        assert!(c.editor.show_segmentation);
        assert_eq!(c.editor.segmentation_threshold, 50);
        assert_eq!(
            c.theme.segmentation,
            [(0x10, 0x10, 0x10), (0x20, 0x20, 0x20)]
        );
    }

    #[test]
    fn parses_vertical_layout_settings() {
        let c = Config::from_toml(
            r#"
            [editor]
            layout = "vertical"
            zong_length = 24
            zong_gap = 2
            "#,
        );
        assert_eq!(c.editor.layout, Layout::Vertical);
        assert_eq!(c.editor.zong_length, 24);
        assert_eq!(c.editor.zong_gap, 2);
    }

    #[test]
    fn a_misspelled_key_is_reported_rather_than_dropped() {
        // The whole file is refused, and the message names the key — the shape
        // of a typo that used to look like a setting that simply did not work.
        let Err(err) = toml::from_str::<RawConfig>("[editor]\nzong_lenght = 24\n") else {
            panic!("a misspelled key parsed as if it were valid");
        };
        let line = Config::describe(Path::new("config.toml"), &err);
        assert!(line.contains("zong_lenght"), "{line}");
        assert!(line.starts_with("config.toml: "), "{line}");
    }

    #[test]
    fn clamps_zong_settings_and_ignores_an_unknown_layout() {
        let c = Config::from_toml(
            r#"
            [editor]
            layout = "sideways"
            zong_length = 500
            zong_gap = 99
            "#,
        );
        assert_eq!(c.editor.layout, Layout::Horizontal);
        assert_eq!(c.editor.zong_length, 64);
        assert_eq!(c.editor.zong_gap, 4);
    }

    #[test]
    fn a_panel_skin_is_two_colours_and_a_marker_list() {
        let c = Config::from_toml(
            r##"
            [panel]
            markers = "①②③"
            ink = "#112233"
            paper = "#445566"
            page_size = 5
            rounded = false
            "##,
        );
        assert_eq!(c.panel.markers, "①②③");
        assert_eq!(c.panel.ink, (0x11, 0x22, 0x33));
        assert_eq!(c.panel.paper, (0x44, 0x55, 0x66));
        assert_eq!(c.panel.page_size, 5);
        assert!(!c.panel.rounded);
    }

    #[test]
    fn the_panel_falls_back_rather_than_breaking() {
        let c = Config::from_toml(
            r##"
            [panel]
            markers = ""
            ink = "not a colour"
            page_size = 99
            "##,
        );
        // An empty list would leave every candidate unnumbered.
        assert_eq!(c.panel.markers, PanelConfig::default().markers);
        assert_eq!(c.panel.ink, PanelConfig::default().ink);
        // There is no tenth key to choose a tenth candidate with.
        assert_eq!(c.panel.page_size, 9);
    }

    #[test]
    fn local_overrides_global_field_by_field() {
        let global = r#"
            [editor]
            tab_width = 4
            line_numbers = "absolute"

            [keys.normal]
            "a" = "b"
        "#;
        let local = r#"
            [editor]
            tab_width = 8

            [keys.normal]
            "c" = "d"
        "#;
        let c = Config::from_sources(Some(global), Some(local));
        // Local overrides tab_width...
        assert_eq!(c.editor.tab_width, 8);
        // ...but keeps the global line_numbers it didn't touch...
        assert_eq!(c.editor.line_numbers, LineNumbers::Absolute);
        // ...and the keymaps are merged.
        assert_eq!(c.keys.normal.get(&'a').map(String::as_str), Some("b"));
        assert_eq!(c.keys.normal.get(&'c').map(String::as_str), Some("d"));
    }

    #[test]
    fn invalid_values_fall_back_to_defaults() {
        let c = Config::from_toml(
            r#"
            [editor]
            line_numbers = "wat"

            [theme]
            selection = "nothex"
            "#,
        );
        assert_eq!(c.editor.line_numbers, LineNumbers::Absolute);
        assert_eq!(c.theme.selection, (60, 70, 100));
    }
}
