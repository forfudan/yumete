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
    /// How many characters fit in one 縱 in vertical layout. Clamped to 4–64;
    /// the renderer lowers it further when the terminal is too short.
    pub zong_length: usize,
    /// The gap between two 縱, in half-width cells (0–4).
    pub zong_gap: usize,
    /// Whether the 拆分 annotation is shown beside candidates (Feature #66).
    /// Off by default: it is a study aid, and it widens every candidate.
    pub show_chaifen: bool,
    /// Whether readings are laid out at all (Feature #65). On by default;
    /// `:ruby-off` turns it off.
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
            zong_length: DEFAULT_ZONG_LENGTH,
            zong_gap: DEFAULT_ZONG_GAP,
            show_chaifen: false,
            show_ruby: true,
            ruby_dialects: Vec::new(),
            tatechuyoko: false,
            hanging_punctuation: false,
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
}

impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig {
            selection: (60, 70, 100),
            segmentation: [(40, 44, 52), (52, 44, 40)],
        }
    }
}

/// Keymap overrides.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyConfig {
    /// Normal-mode single-key aliases: pressing the key on the left behaves as
    /// pressing the key on the right.
    pub normal: HashMap<char, char>,
}

/// The fully-resolved configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Config {
    pub editor: EditorConfig,
    pub theme: ThemeConfig,
    pub panel: PanelConfig,
    pub keys: KeyConfig,
}

impl Config {
    /// Load the global config, then apply the nearest per-project override.
    pub fn load() -> Config {
        let mut raw = RawConfig::default();

        let global = config_dir().join("config.toml");
        if let Ok(text) = fs::read_to_string(&global) {
            if let Ok(parsed) = toml::from_str::<RawConfig>(&text) {
                raw.merge(parsed);
            }
        }

        if let Ok(cwd) = env::current_dir() {
            if let Some(local) = local_config_path(&cwd) {
                if let Ok(text) = fs::read_to_string(&local) {
                    if let Ok(parsed) = toml::from_str::<RawConfig>(&text) {
                        raw.merge(parsed);
                    }
                }
            }
        }

        raw.into_config()
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
struct RawConfig {
    #[serde(default)]
    editor: RawEditor,
    #[serde(default)]
    theme: RawTheme,
    #[serde(default)]
    panel: RawPanel,
    #[serde(default)]
    keys: RawKeys,
}

#[derive(Deserialize, Default)]
struct RawPanel {
    markers: Option<String>,
    ink: Option<String>,
    paper: Option<String>,
    page_size: Option<usize>,
    rounded: Option<bool>,
}

#[derive(Deserialize, Default)]
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
}

#[derive(Deserialize, Default)]
struct RawTheme {
    selection: Option<String>,
    segmentation: Option<Vec<String>>,
}

#[derive(Deserialize, Default)]
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
        if other.theme.selection.is_some() {
            self.theme.selection = other.theme.selection;
        }
        if other.theme.segmentation.is_some() {
            self.theme.segmentation = other.theme.segmentation;
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
            config.editor.zong_length = length.clamp(4, 64);
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
            // Only single-character aliases are meaningful here.
            let mut ks = k.chars();
            let mut vs = v.chars();
            if let (Some(from), Some(to)) = (ks.next(), vs.next()) {
                if ks.next().is_none() && vs.next().is_none() {
                    config.keys.normal.insert(from, to);
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
            "toolong" = "x"
            "#,
        );
        assert_eq!(c.keys.normal.get(&'j'), Some(&'h'));
        // Multi-character keys are ignored.
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
        assert_eq!(c.keys.normal.get(&'a'), Some(&'b'));
        assert_eq!(c.keys.normal.get(&'c'), Some(&'d'));
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
