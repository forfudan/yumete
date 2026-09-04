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

/// How wide an East-Asian **Ambiguous** character is drawn.
///
/// Annex #11 leaves it to the environment, and the environment is the terminal
/// and its font: `—` `…` `“”` `·` `※` `▓` are one cell in a Latin font and two
/// in a CJK one. Nobody can work this out from the text — but the terminal can
/// be *asked*, which is what `Auto` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ambiguity {
    /// Ask the terminal at start-up, by printing one and reading back where the
    /// cursor landed. The default: it is the only answer that is about the
    /// terminal actually in front of the writer.
    #[default]
    Auto,
    /// Two cells, as a CJK font draws them.
    Wide,
    /// One cell, as a Latin font draws them.
    Narrow,
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
    /// Whether the line-number band carries a ground of its own.
    ///
    /// Off, and the same in both layouts: 縱書 painted its number band and
    /// 橫排 did not, which is one editor with two answers to one question.
    pub line_number_fill: bool,
    /// The shell line `:shot` runs to put a picture of the screen on the
    /// clipboard.
    ///
    /// A command rather than a built-in: what「截圖」 means is the window
    /// system's business, not the editor's, and every desktop answers it
    /// differently (`screencapture` on macOS, `grim` on Wayland, `import` on
    /// X11). The default is macOS's, and it takes the *frontmost window* —
    /// which, while the editor is running, is the terminal it is running in.
    pub screenshot: String,
    /// What is drawn in a paragraph's opening squares: `"none"` (default),
    /// `"color"`, `"symbol"`.
    pub indent_hint: String,
    /// The character the `symbol` hint draws in the first of them.
    pub indent_symbol: String,
    /// How a grid's columns are told apart (Feature #157).
    ///
    /// `"off"`, `"color"`, `"line"`, `"line dash"`, `"line double"`. A
    /// wide-columned table reads as a page and wants nothing between its
    /// columns; a 拆分表 of twenty-eight one-character columns reads as a grid
    /// and cannot do without. The file does not say which it is, so the reader
    /// does. Kept as the string the reader wrote — the core owns the meaning.
    pub table_rules: String,
    /// Whether the word-segmentation overlay is shown at start-up (Feature #24).
    /// On by default so the CJK word grouping is visible; toggle with
    /// `:word show off` or set `show_segmentation = false`.
    pub show_segmentation: bool,
    /// How readily characters join into words: `strict`, `balanced`, `full`
    /// (Feature #24), the same three `:word level` names.
    ///
    /// **Replaces `segmentation_threshold`**, which was a raw weight on a scale
    /// only the bundled list had — it said nothing to a reader and nothing at
    /// all to the language model, which most machines actually segment with.
    pub word_level: yumete_cjk::WordLevel,
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
    /// How many squares open a paragraph (首行縮進). 0 is none.
    pub indent: usize,
    /// How many bands the vertical page is divided into (段組). 1 is none.
    pub bands: usize,
    /// Whether `yumete` with no file opens again what was open last time.
    pub session: bool,
    /// What language the editor says things in: `"zh"` or `"en"`.
    pub language: String,
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
    /// How wide East-Asian Ambiguous characters — `—` `…` `“” ‘’` `·` — are
    /// drawn (Feature #81, #193). `"auto"` by default, which asks the terminal;
    /// `"wide"` and `"narrow"` say so outright.
    pub ambiguous_width: Ambiguity,
    /// How wide the detail panel is, in cells (Feature #187). Clamped to
    /// something readable, and never more than half the window.
    pub detail_width: usize,
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
            line_number_fill: false,
            screenshot: match cfg!(target_os = "macos") {
                true => "b=$(osascript -e 'tell application \"System Events\" to tell                      (first application process whose frontmost is true) to get                      {position, size} of front window' | tr -d ' ') &&                      screencapture -x -o -c -R\"$b\""
                    .to_string(),
                false => String::new(),
            },
            indent_hint: "none".to_string(),
            indent_symbol: "↵".to_string(),
            table_rules: "line dash".to_string(),
            show_segmentation: true,
            word_level: yumete_cjk::WordLevel::default(),
            layout: Layout::Horizontal,
            zong_length: 0,
            indent: 0,
            bands: 1,
            session: true,
            language: "zh".to_string(),
            zong_gap: DEFAULT_ZONG_GAP,
            show_chaifen: false,
            show_ruby: false,
            ruby_dialects: Vec::new(),
            tatechuyoko: false,
            hanging_punctuation: false,
            soft_wrap: true,
            autosave: true,
            ambiguous_width: Ambiguity::default(),
            detail_width: 30,
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

/// Whether the editor wears its own colours or the terminal's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ground {
    /// Paint the page: the theme owns both its ink and its ground.
    ///
    /// The default, because **a theme that does not own its ground cannot make
    /// any promise about contrast** — every tint would be measured against a
    /// colour the editor has never seen. Until this existed, 墨香 dressed the
    /// candidate panel and four other panes and left the manuscript itself in
    /// the terminal's own ink on the terminal's own ground.
    #[default]
    Paint,
    /// Leave the ground alone and set only the foregrounds, for a reader whose
    /// terminal palette is a decision they have already made.
    Terminal,
}

/// Which of a theme's two moods is in force.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// Ask the terminal what colour it is and follow it (the default).
    #[default]
    Auto,
    Dark,
    Light,
}

impl Mode {
    fn parse(name: &str) -> Option<Mode> {
        match name.trim().to_ascii_lowercase().as_str() {
            "auto" | "system" | "自動" | "自动" => Some(Mode::Auto),
            "dark" | "深" | "深色" => Some(Mode::Dark),
            "light" | "淺" | "浅" | "淺色" | "浅色" => Some(Mode::Light),
            _ => None,
        }
    }
}

/// A theme's two anchors and every shade between them.
///
/// **A theme is a few numbers, not a table of colours.** Yume's own themes are
/// defined this way — an ink and a paper, with every other shade interpolated
/// along a ladder between them — and it is what lets a skin be retuned by
/// editing a pair of values: the relationships between the shades stay right by
/// construction, and nothing can drift out of agreement with anything else.
///
/// Rungs run `0` (pure ink) to `1000` (pure paper). Mixed in **sRGB**, not in
/// linear light: measured over this ladder, sRGB gives a step of 5.8–7.4 L\*
/// across the whole ramp while linear light gives 3.2–16.8, piling nine tenths
/// of the rungs into the light half and then falling off a cliff. The even ramp
/// is the one you can place an interface on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ladder {
    pub ink: (u8, u8, u8),
    pub paper: (u8, u8, u8),
}

impl Ladder {
    /// One rung: `0` is the ink, `1000` the paper.
    pub fn step(self, t: u16) -> (u8, u8, u8) {
        let t = t.min(1000) as i64;
        let mix = |a: u8, b: u8| -> u8 {
            let (a, b) = (a as i64, b as i64);
            ((a * 1000 + (b - a) * t + 500) / 1000) as u8
        };
        (
            mix(self.ink.0, self.paper.0),
            mix(self.ink.1, self.paper.1),
            mix(self.ink.2, self.paper.2),
        )
    }
}

/// Where each part of the editor sits on the ladder.
///
/// **The rungs are not evenly useful, and that is what this list encodes.**
/// Measured against 墨香's own pair: at `t ≤ 350` a colour clears 4.5:1 as ink
/// on the paper; at `t ≥ 730` ink clears 4.5:1 drawn *on* it. The 38% between
/// carries nothing — too faint to read, too pale to write on — so nothing here
/// is placed there except the rules, which are neither.
pub mod rung {
    /// The writing, and anything that *is* the writing: a hung 句讀, the
    /// character a highlight covers.
    pub const TEXT: u16 = 0;
    /// One shade back: a reading beside its base, a 拆分 annotation, a
    /// candidate's number, the second line of anything.
    pub const QUIET: u16 = 300;
    /// Furniture you read once: line numbers, an unlit tab, a key's label, a
    /// 批注, a page's front matter.
    pub const FURNITURE: u16 = 400;
    /// The markup itself — `**`, `#`, `[]()`. Shown, and set back far enough
    /// that it is never read as a word.
    pub const MARKER: u16 = 450;
    /// A rule: a panel's ring, the sidebar's edge, the ruler's line, a 稿紙
    /// tick. Not text and not a ground, and the only thing that belongs in the
    /// middle of the ladder.
    pub const RULE: u16 = 550;
    /// Chrome: a sidebar, a tab bar, a table's gutter and header, a detail
    /// panel, the status line. **A hair off the page and no more** — the
    /// theme is 墨黑, and furniture that is grey rather than black flattens
    /// the whole screen toward the middle of the ladder. What separates a
    /// panel from the page is its rule and its 金墨, not a lighter ground.
    pub const CHROME: u16 = 970;
    /// A ground that must not shout: a table's alternating columns, its cursor
    /// row, a code fence, a callout, the tint past the measure.
    pub const BAND: u16 = 940;
    /// A band that must be **seen**, because position is not separating it
    /// from the text: the 縱書 number band sits in the text's own columns, and
    /// the lit tab sits among the unlit ones.
    pub const HEAD: u16 = 880;
    /// A selection: the loudest ground, and still only a ground — the ink on it
    /// is untouched, so a heading inside a selection is still a heading.
    pub const SELECTION: u16 = 800;
    /// The page.
    pub const PAPER: u16 = 1000;
}

/// Theme (colour) settings — 【墨香】 and anything shaped like it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeConfig {
    /// What the theme is called. 【墨香】 answers to both spellings.
    pub name: String,
    /// Dark or light, or ask the terminal.
    pub mode: Mode,
    /// Whether the page is painted.
    pub ground: Ground,
    /// 墨 and 紙, dark.
    pub dark: Ladder,
    /// 墨 and 紙, light. **Not the dark pair swapped**: the ground goes deeper
    /// and the ink dimmer in the dark, or the page glows at night.
    pub light: Ladder,
    /// 朱 — the red of the reader's brush, and the one colour that is not on
    /// the ladder.
    ///
    /// It says **這裏不對**: a row of the wrong width, a component with no row
    /// of its own, a footnote's mark. Never emphasis, never "here" — "here" is
    /// ink and paper changing places, which costs no colour and survives a
    /// light/dark flip. Warm, because the ground is a warm near-black and the
    /// ink a warm bone, and a cold accent on them reads as a terminal's rather
    /// than a manuscript's.
    pub mark_dark: (u8, u8, u8),
    pub mark_light: (u8, u8, u8),
    /// 金 — the warm one, for what is **not the prose**.
    ///
    /// The page's own ladder runs from a near-white ink to a cool near-black
    /// ground, and everything on it is therefore a grey: prose, a reading, a
    /// marker, a rule. That is what prose should look like, and it leaves the
    /// one thing greys cannot do — saying 「這不是正文」 without shouting. A
    /// heading, a table's header row, the label beside a value in a panel:
    /// warm, against a page that is not, so the eye finds them without
    /// reading. Never 朱, which is for what is *wrong*.
    pub gold_dark: (u8, u8, u8),
    pub gold_light: (u8, u8, u8),
}

impl ThemeConfig {
    /// The ladder in force.
    pub fn ladder(&self, dark: bool) -> Ladder {
        match dark {
            true => self.dark,
            false => self.light,
        }
    }

    /// 朱, in the mood in force.
    pub fn mark(&self, dark: bool) -> (u8, u8, u8) {
        match dark {
            true => self.mark_dark,
            false => self.mark_light,
        }
    }

    /// 金, in the mood in force.
    pub fn gold(&self, dark: bool) -> (u8, u8, u8) {
        match dark {
            true => self.gold_dark,
            false => self.gold_light,
        }
    }

    /// A theme by name, or `None` if nothing is called that.
    ///
    /// Two, and they are the two arguments a scheme can make: 【墨香】 says
    /// rank with warmth, 【黑白】 says it with nothing but weight. The second
    /// is not a lesser version of the first — it is what a scheme looks like
    /// when every colour has been taken away and the design has to hold
    /// anyway, which is also what a reader with a monochrome terminal, or with
    /// colour-blindness, is left with.
    /// **A theme's name is ASCII**, because a command line is: `:theme ink`
    /// has to be typeable with the IME off, which is where a reader who has
    /// just opened the editor is. The pinyin is an alias for the hand that
    /// thinks in the Chinese name, and the Chinese name itself lives in the
    /// comment beside it, which is where a name that cannot be typed belongs.
    pub fn named(name: &str) -> Option<ThemeConfig> {
        match name.trim().to_ascii_lowercase().as_str() {
            // 墨香 — warm ink on a deep ground.
            "ink" | "moxiang" => Some(ThemeConfig::default()),
            // 黑白 — the same design with every colour taken away.
            "bw" | "heibai" | "mono" => Some(ThemeConfig {
                name: "bw".to_string(),
                // A true neutral, and no accident of temperature anywhere: the
                // ink is off-white so it does not glare, the ground is off-
                // black so it is not a hole in the screen.
                dark: Ladder {
                    ink: (0xE6, 0xE6, 0xE6),
                    paper: (0x1E, 0x1E, 0x1E),
                },
                light: Ladder {
                    ink: (0x1E, 0x1E, 0x1E),
                    paper: (0xF6, 0xF6, 0xF6),
                },
                // 金 and 朱 have to go on saying what they said — 「不是正文」
                // and 「這裏不對」 — with no hue to say it in, so they say it
                // by *position*: 金 is brighter than the writing (nothing else
                // on the page is), 朱 is brighter still and is the only thing
                // that ever reaches the ends of the ladder.
                gold_dark: (0xFF, 0xFF, 0xFF),
                gold_light: (0x00, 0x00, 0x00),
                mark_dark: (0xB4, 0xB4, 0xB4),
                mark_light: (0x6E, 0x6E, 0x6E),
                ..ThemeConfig::default()
            }),
            _ => None,
        }
    }
}

impl Default for ThemeConfig {
    fn default() -> Self {
        ThemeConfig {
            name: "ink".to_string(),
            mode: Mode::Auto,
            ground: Ground::Paint,
            // **The page is cool and the writing is near-white.** 墨香's own
            // pair — a bone ink on a warm near-black — is the right skin for a
            // *panel*, where it has always been; laid over the whole page it
            // makes every grey on the ladder a warm grey, and then the prose,
            // the headings and the furniture are all the same colour and the
            // page has no ranks in it. So the page keeps the ink 白 and the
            // ground 墨, and the warmth goes to 金, below, which is worth
            // finding precisely because most of the screen is not warm.
            dark: Ladder {
                ink: (0xE8, 0xE4, 0xDA),
                paper: (0x24, 0x26, 0x2C),
            },
            // 墨 on paper is darker than 墨 on a screen — the light ladder's
            // ink is the dark ladder's *ground*, which is both true of the
            // material and what gives the light mood the range its top rungs
            // need to stay readable.
            light: Ladder {
                ink: (0x26, 0x2A, 0x27),
                paper: (0xF1, 0xEB, 0xD9),
            },
            // A seal's red on paper; lighter in the dark, for the same reason
            // the ink is dimmer there.
            mark_light: (0xA8, 0x30, 0x1C),
            mark_dark: (0xD2, 0x78, 0x5A),
            // 墨香's own bone, kept for the one job it is best at. On a light
            // page a bone would be invisible, so there it is the same warmth
            // taken the other way down: a dark gold on cream.
            gold_dark: (0xD8, 0xC9, 0x9A),
            gold_light: (0x6B, 0x54, 0x26),
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

/// **What a language can be told to run** (Feature #197).
///
/// `tinymist preview` was written into the front end in Rust, which is the
/// wrong place for it: whether a `.typ` is previewed by tinymist and a `.md` is
/// formatted by rumdl is a fact about the reader's machine, not about the
/// editor. So it is config:
///
/// ```toml
/// [language.typst]
/// preview = { run = "tinymist preview --no-open {file}", kind = "server" }
/// format  = { run = "typstfmt {file}" }
///
/// [language.markdown]
/// format = { run = "rumdl check --fix {file}" }
/// ```
///
/// **The verbs are language-independent** — `:preview`, `:format` — so one key
/// means one thing in every file and the config says how it is done here.
///
/// **A project may define these**, and the safety is in *how* they run: no
/// shell (so `;` and `$( )` are ordinary characters, not syntax), placeholders
/// substituted as **whole arguments** (so a file named `我的 稿;rm -rf ~.md`
/// cannot become a second command), and the line is checked when the config is
/// read — an unbalanced quote, an unknown `{placeholder}` or a shell
/// metacharacter is a config error that names the file, rather than a command
/// that quietly does something else.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Runner {
    /// The command line, as written.
    pub run: String,
    /// Whether it finishes (`once`), or runs until it is stopped (`server`),
    /// or takes the buffer on stdin and gives it back (`filter`).
    pub kind: RunKind,
}

/// How a language's command is run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RunKind {
    /// Runs, says what it said, and is done.
    #[default]
    Once,
    /// Runs until it is stopped, and its output is watched for an address.
    Server,
    /// Takes the buffer on stdin and replaces it with what comes back.
    Filter,
}

impl RunKind {
    fn parse(word: &str) -> Option<RunKind> {
        match word.trim() {
            "once" | "" => Some(RunKind::Once),
            "server" => Some(RunKind::Server),
            "filter" => Some(RunKind::Filter),
            _ => None,
        }
    }
}

impl Runner {
    /// The program and its arguments, with the placeholders filled in.
    ///
    /// **Never a shell.** The line is split on whitespace outside quotes, and
    /// each placeholder becomes one whole argument — so nothing in a file name
    /// can start a second command.
    pub fn argv(&self, file: &str) -> Option<Vec<String>> {
        let mut out = Vec::new();
        let mut word = String::new();
        let mut quote: Option<char> = None;
        let mut any = false;
        for c in self.run.chars() {
            match (quote, c) {
                (Some(q), c) if c == q => quote = None,
                (Some(_), c) => word.push(c),
                (None, '"') | (None, '\'') => quote = Some(c),
                (None, c) if c.is_whitespace() => {
                    if any {
                        out.push(std::mem::take(&mut word));
                        any = false;
                    }
                }
                (None, c) => word.push(c),
            }
            if !c.is_whitespace() || quote.is_some() {
                any = true;
            }
        }
        if quote.is_some() {
            return None;
        }
        if any {
            out.push(word);
        }
        // Whole arguments, one substitution each.
        let path = Path::new(file);
        let dir = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        // **One pass, not three.** Chained `replace`s substitute into what the
        // previous one just wrote: a manuscript called `{name}.md` came back as
        // `{name}.md.md`, and the program was handed a path that is not the
        // buffer's — which with `kind = "once"` means re-reading a file nobody
        // formatted.
        let fill = |word: &str| -> String {
            let mut out = String::with_capacity(word.len());
            let mut rest = word;
            while let Some(at) = rest.find('{') {
                out.push_str(&rest[..at]);
                let after = &rest[at..];
                let (name_of, len) = match after.find('}') {
                    Some(end) => (&after[1..end], end + 1),
                    None => break,
                };
                out.push_str(match name_of {
                    "file" => file,
                    "dir" => &dir,
                    "name" => &name,
                    // Checked at load; anything else is passed through as the
                    // text it is rather than guessed at.
                    _ => &after[..len],
                });
                rest = &after[len..];
            }
            out.push_str(rest);
            out
        };
        Some(out.iter().map(|w| fill(w)).collect())
    }

    /// What is wrong with this command line, if anything.
    ///
    /// Checked when the config is read, so a typo is a message about the config
    /// rather than a command that runs and does something else.
    pub fn fault(&self) -> Option<String> {
        if self.run.trim().is_empty() {
            return Some("空的命令".to_string());
        }
        if let Some(c) = self.run.chars().find(|c| "|;&<>`$".contains(*c)) {
            return Some(format!(
                "命令裏有 '{c}'——這裏不經過 shell，管道和重定向不會照你想的跑"
            ));
        }
        let mut rest = self.run.as_str();
        while let Some(at) = rest.find('{') {
            let after = &rest[at + 1..];
            let Some(end) = after.find('}') else {
                return Some("{ 沒有配對的 }".to_string());
            };
            let name = &after[..end];
            if !["file", "dir", "name"].contains(&name) {
                return Some(format!(
                    "不認識的佔位符 {{{name}}}——只有 {{file}} {{dir}} {{name}}"
                ));
            }
            rest = &after[end + 1..];
        }
        if self.argv("x").is_none() {
            return Some("引號沒有配對".to_string());
        }
        None
    }
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
    /// What each language can be told to run, by verb: `preview`, `format`, and
    /// whatever else a reader names.
    pub language: HashMap<String, HashMap<String, Runner>>,
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
                        Ok(mut parsed) => {
                            // **`screenshot` is the one setting that runs
                            // through a shell**, and a `.yumete/` directory
                            // travels with a manuscript — cloned, unzipped,
                            // handed over by a collaborator. Everything else a
                            // project may declare runs *without* a shell, with
                            // whole-argument placeholders; this one cannot, so
                            // a project does not get to set it. Said out loud
                            // rather than dropped, or a writer whose own line
                            // stopped working would never learn why.
                            if parsed.editor.screenshot.take().is_some() {
                                problems.push(format!(
                                    "{}：screenshot 只認全域設定——它是唯一經過 shell 的一條",
                                    local.display()
                                ));
                            }
                            raw.merge(parsed);
                        }
                        Err(err) => problems.push(Self::describe(&local, &err)),
                    }
                }
            }
        }

        // A setting that is gone is said out loud, not dropped: a reader whose
        // `segmentation_threshold = 50` stopped doing anything would have no
        // way to find out that the question is now spelled `word_level`.
        if raw.editor.segmentation_threshold.is_some() {
            problems.push(
                "segmentation_threshold 已經沒有了——改成 word_level = \"strict\"／\"balanced\"／\"full\""
                    .to_string(),
            );
        }

        // A command a language declares that will not do what it says is a
        // problem *about the config*, named here rather than discovered when
        // the key is pressed.
        for (language, verbs) in &raw.language {
            for (verb, runner) in verbs {
                let checked = Runner {
                    run: runner.run.clone(),
                    kind: RunKind::parse(runner.kind.as_deref().unwrap_or("")).unwrap_or_default(),
                };
                if RunKind::parse(runner.kind.as_deref().unwrap_or("")).is_none() {
                    problems.push(format!(
                        "[language.{language}] {verb}: kind 只能是 once、server 或 filter"
                    ));
                } else if let Some(fault) = checked.fault() {
                    problems.push(format!("[language.{language}] {verb}: {fault}"));
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
    #[serde(default)]
    language: HashMap<String, HashMap<String, RawRunner>>,
}

/// One command a language declares, as it is written in the file.
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawRunner {
    run: String,
    #[serde(default)]
    kind: Option<String>,
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
    line_number_fill: Option<bool>,
    screenshot: Option<String>,
    indent_hint: Option<String>,
    indent_symbol: Option<String>,
    table_rules: Option<String>,
    show_segmentation: Option<bool>,
    word_level: Option<String>,
    /// Retired. Kept so a config that still sets it is *told*, rather than
    /// refused by the unknown-key check with no idea what to write instead.
    segmentation_threshold: Option<i64>,
    layout: Option<String>,
    zong_length: Option<usize>,
    indent: Option<usize>,
    bands: Option<usize>,
    session: Option<bool>,
    language: Option<String>,
    zong_gap: Option<usize>,
    show_chaifen: Option<bool>,
    show_ruby: Option<bool>,
    ruby_dialects: Option<Vec<String>>,
    tatechuyoko: Option<bool>,
    hanging_punctuation: Option<bool>,
    soft_wrap: Option<bool>,
    autosave: Option<bool>,
    ambiguous_width: Option<String>,
    detail_width: Option<usize>,
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
    name: Option<String>,
    mode: Option<String>,
    ground: Option<String>,
    /// 墨 and 紙, as `"#RRGGBB"`. Two keys per mood, and every other shade in
    /// the editor is worked out from them.
    ink: Option<String>,
    paper: Option<String>,
    ink_light: Option<String>,
    paper_light: Option<String>,
    /// 朱.
    mark: Option<String>,
    mark_light: Option<String>,
    gold: Option<String>,
    gold_light: Option<String>,
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
        if other.editor.table_rules.is_some() {
            self.editor.table_rules = other.editor.table_rules.clone();
        }
        if other.editor.line_number_fill.is_some() {
            self.editor.line_number_fill = other.editor.line_number_fill;
        }
        if other.editor.screenshot.is_some() {
            self.editor.screenshot = other.editor.screenshot.clone();
        }
        if other.editor.indent_hint.is_some() {
            self.editor.indent_hint = other.editor.indent_hint.clone();
        }
        if other.editor.indent_symbol.is_some() {
            self.editor.indent_symbol = other.editor.indent_symbol.clone();
        }
        if other.editor.show_segmentation.is_some() {
            self.editor.show_segmentation = other.editor.show_segmentation;
        }
        if other.editor.word_level.is_some() {
            self.editor.word_level = other.editor.word_level.clone();
        }
        if other.editor.segmentation_threshold.is_some() {
            self.editor.segmentation_threshold = other.editor.segmentation_threshold;
        }
        if other.editor.layout.is_some() {
            self.editor.layout = other.editor.layout;
        }
        if other.editor.indent.is_some() {
            self.editor.indent = other.editor.indent;
        }
        if other.editor.bands.is_some() {
            self.editor.bands = other.editor.bands;
        }
        if other.editor.session.is_some() {
            self.editor.session = other.editor.session;
        }
        if other.editor.language.is_some() {
            self.editor.language = other.editor.language.clone();
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
        if other.theme.name.is_some() {
            self.theme.name = other.theme.name.clone();
        }
        if other.theme.mode.is_some() {
            self.theme.mode = other.theme.mode.clone();
        }
        if other.theme.ground.is_some() {
            self.theme.ground = other.theme.ground.clone();
        }
        for (from, to) in [
            (&other.theme.ink, &mut self.theme.ink),
            (&other.theme.paper, &mut self.theme.paper),
            (&other.theme.ink_light, &mut self.theme.ink_light),
            (&other.theme.paper_light, &mut self.theme.paper_light),
            (&other.theme.mark, &mut self.theme.mark),
            (&other.theme.mark_light, &mut self.theme.mark_light),
            (&other.theme.gold, &mut self.theme.gold),
            (&other.theme.gold_light, &mut self.theme.gold_light),
        ] {
            if from.is_some() {
                *to = from.clone();
            }
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
        if let Some(rules) = self.editor.table_rules {
            config.editor.table_rules = rules;
        }
        if let Some(on) = self.editor.line_number_fill {
            config.editor.line_number_fill = on;
        }
        if let Some(line) = self.editor.screenshot {
            config.editor.screenshot = line;
        }
        if let Some(hint) = self.editor.indent_hint {
            config.editor.indent_hint = hint;
        }
        if let Some(symbol) = self.editor.indent_symbol {
            config.editor.indent_symbol = symbol;
        }
        if let Some(on) = self.editor.show_segmentation {
            config.editor.show_segmentation = on;
        }
        if let Some(level) = self
            .editor
            .word_level
            .as_deref()
            .and_then(yumete_cjk::WordLevel::parse)
        {
            config.editor.word_level = level;
        }
        if let Some(layout) = self.editor.layout {
            // An unrecognised value keeps the default rather than refusing to
            // start, like every other setting here.
            if let Some(parsed) = Layout::parse(&layout) {
                config.editor.layout = parsed;
            }
        }
        if let Some(n) = self.editor.indent {
            // Two is the Chinese convention and eight is more than anyone
            // means; a number outside that is a typo, not a preference.
            config.editor.indent = n.min(8);
        }
        if let Some(n) = self.editor.bands {
            config.editor.bands = n.clamp(1, 4);
        }
        if let Some(on) = self.editor.session {
            config.editor.session = on;
        }
        if let Some(name) = &self.editor.language {
            config.editor.language = name.clone();
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
        if let Some(width) = self.editor.detail_width {
            config.editor.detail_width = width.clamp(12, 80);
        }
        if let Some(width) = self.editor.sidebar_width {
            config.editor.sidebar_width = width.clamp(12, 60);
        }
        if let Some(width) = self.editor.ambiguous_width {
            // An unknown value keeps the default rather than picking one: a
            // typo here shifts every line on the page.
            match width.trim().to_ascii_lowercase().as_str() {
                "wide" | "double" | "full" => config.editor.ambiguous_width = Ambiguity::Wide,
                "narrow" | "single" | "half" => config.editor.ambiguous_width = Ambiguity::Narrow,
                "auto" | "ask" | "terminal" => config.editor.ambiguous_width = Ambiguity::Auto,
                _ => {}
            }
        }
        // **What each language can be told to run.** A command that will not
        // do what it says is a config error, reported with the file, rather
        // than a program that runs and does something else — see [`Runner`].
        for (language, verbs) in self.language {
            let mut here: HashMap<String, Runner> = HashMap::new();
            for (verb, raw) in verbs {
                let Some(kind) = RunKind::parse(raw.kind.as_deref().unwrap_or("")) else {
                    continue;
                };
                let runner = Runner { run: raw.run, kind };
                if runner.fault().is_some() {
                    continue;
                }
                here.insert(verb, runner);
            }
            config.language.entry(language).or_default().extend(here);
        }
        if let Some(scheme) = self.ime.scheme {
            config.ime.scheme = scheme;
        }
        if let Some(start) = self.ime.start {
            config.ime.start = start;
        }
        config.syntax.by_name = self.syntax;
        if let Some(name) = self.theme.name {
            if !name.trim().is_empty() {
                // The name picks the whole set of anchors, and the keys below
                // then override whichever of them the reader has an opinion
                // about — so `name = "黑白"` plus `mark = "#…"` is a sentence.
                let mut named = ThemeConfig::named(&name).unwrap_or_else(|| config.theme.clone());
                named.mode = config.theme.mode;
                named.ground = config.theme.ground;
                if ThemeConfig::named(&name).is_some() {
                    config.theme = named;
                } else {
                    config.theme.name = name;
                }
            }
        }
        if let Some(mode) = self.theme.mode.as_deref().and_then(Mode::parse) {
            config.theme.mode = mode;
        }
        if let Some(ground) = self.theme.ground.as_deref() {
            match ground.trim().to_ascii_lowercase().as_str() {
                "paint" | "theme" | "自己" => config.theme.ground = Ground::Paint,
                "terminal" | "終端" | "终端" => config.theme.ground = Ground::Terminal,
                _ => {}
            }
        }
        for (hex, slot) in [
            (self.theme.ink, &mut config.theme.dark.ink),
            (self.theme.paper, &mut config.theme.dark.paper),
            (self.theme.ink_light, &mut config.theme.light.ink),
            (self.theme.paper_light, &mut config.theme.light.paper),
            (self.theme.mark, &mut config.theme.mark_dark),
            (self.theme.mark_light, &mut config.theme.mark_light),
            (self.theme.gold, &mut config.theme.gold_dark),
            (self.theme.gold_light, &mut config.theme.gold_light),
        ] {
            if let Some(rgb) = hex.as_deref().and_then(parse_hex) {
                *slot = rgb;
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
mod runner_tests {
    use super::*;

    #[test]
    fn a_placeholder_is_one_whole_argument() {
        let r = Runner { run: "tinymist preview --no-open {file}".into(), kind: RunKind::Server };
        // **Nothing in a file name can start a second command**: there is no
        // shell, and the substitution never re-splits.
        let argv = r.argv("/書/第一章;rm -rf ~.md").unwrap();
        assert_eq!(
            argv,
            ["tinymist", "preview", "--no-open", "/書/第一章;rm -rf ~.md"]
        );
        // A quoted argument stays one argument.
        let r = Runner { run: "fmt --style \"a b\" {file}".into(), kind: RunKind::Once };
        assert_eq!(r.argv("x.md").unwrap(), ["fmt", "--style", "a b", "x.md"]);
    }

    #[test]
    fn a_line_that_would_not_do_what_it_says_is_a_config_error() {
        for bad in [
            "rumdl {file} | tee log",
            "fmt {file} && echo done",
            "fmt {file} > out",
            "fmt $(echo {file})",
            "fmt {fil}",
            "fmt \"{file}",
            "   ",
        ] {
            let r = Runner { run: bad.into(), kind: RunKind::Once };
            assert!(r.fault().is_some(), "{bad:?} should be refused");
        }
        let good = Runner { run: "rumdl check --fix {file}".into(), kind: RunKind::Once };
        assert_eq!(good.fault(), None);
    }

    #[test]
    fn a_language_declares_its_commands() {
        let config = Config::from_toml(
            "[language.typst]\npreview = { run = \"tinymist preview {file}\", kind = \"server\" }\n\
             [language.markdown]\nformat = { run = \"rumdl check --fix {file}\" }\n",
        );
        let typst = config.language.get("typst").expect("typst");
        assert_eq!(typst["preview"].kind, RunKind::Server);
        assert_eq!(config.language["markdown"]["format"].kind, RunKind::Once);
    }
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
        assert_eq!(c.theme.name, "ink", "the name is ASCII; 墨香 is what it means");
        assert_eq!(c.theme.dark.ink, (0xE8, 0xE4, 0xDA));
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
            mode = "light"
            ink = "#204060"
            paper_light = "#FFFEF8"
            "##,
        );
        assert_eq!(c.editor.tab_width, 2);
        assert_eq!(c.editor.line_numbers, LineNumbers::Relative);
        assert_eq!(c.editor.scrolloff, 5);
        assert_eq!(c.theme.mode, Mode::Light);
        // Two anchors named, and every rung between them moves with the pair.
        assert_eq!(c.theme.dark.ink, (0x20, 0x40, 0x60));
        assert_eq!(c.theme.light.paper, (0xFF, 0xFE, 0xF8));
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
            word_level = "strict"
            "##,
        );
        assert!(c.editor.show_segmentation);
        assert_eq!(c.editor.word_level, yumete_cjk::WordLevel::Strict);
        // A word nobody defined keeps the default rather than refusing the file.
        let c = Config::from_toml("[editor]\nword_level = \"whatever\"\n");
        assert_eq!(c.editor.word_level, yumete_cjk::WordLevel::Balanced);
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
            mode = "wat"
            ink = "nothex"
            "#,
        );
        assert_eq!(c.editor.line_numbers, LineNumbers::Absolute);
        assert_eq!(c.theme.mode, Mode::Auto);
        assert_eq!(c.theme.dark.ink, ThemeConfig::default().dark.ink);
    }
}
