//! **What a key row shows beside its meaning**: the value now of the
//! setting its key changes, so a menu of settings (`空格 ,`, `空格 z`) says
//! where each one stands before a key is pressed.
//!
//! A row is tied to its setting by the command it runs, in one table
//! ([`shows`]); every group's menu reads it, so a group gains values by
//! naming its commands there and nothing else. A row that opens a setting's
//! values, or flips a switch, shows the value now; a row that picks one value
//! is ticked while that value is the one.

use super::*;
use yumete_cjk::actions::NormalCmd;

/// A setting a key row can show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Setting {
    /// A switch, by the command line [`Editor::switch_state`] asks.
    Switch(&'static str),
    LineNumbers,
    Render,
    /// How many pages the page in use is cut into.
    Pages,
    /// One the front end holds (the theme's mood, the input method's
    /// dials), by the name it is handed over by ([`Editor::set_front_value`]).
    Front(&'static str),
}

enum Shows {
    /// The value now.
    Now(Setting),
    /// One value of the setting, by its word: ticked while it is the one.
    Is(Setting, &'static str),
}

/// The tick beside the value in force.
const TICK: &str = "✓";

/// **The setting `cmd`'s row shows**, when its key changes one.
fn shows(cmd: NormalCmd) -> Option<Shows> {
    use NormalCmd as N;
    use Setting as S;
    use Shows::{Is, Now};
    Some(match cmd {
        N::QuickVertical => Now(S::Switch(":vertical")),
        N::QuickFocus => Now(S::Switch(":view-focus")),
        N::QuickTypewriter => Now(S::Switch(":view-typewriter")),
        N::QuickWrap => Now(S::Switch(":view-wrap")),
        N::QuickZen => Now(S::Switch(":view-zen")),
        N::QuickLineNumbersMode => Now(S::LineNumbers),
        N::QuickLineNumbersAbsolute => Is(S::LineNumbers, "absolute"),
        N::QuickLineNumbersRelative => Is(S::LineNumbers, "relative"),
        N::QuickLineNumbersNone => Is(S::LineNumbers, "none"),
        N::QuickRenderMode => Now(S::Render),
        N::QuickRenderOff => Is(S::Render, "off"),
        N::QuickRenderBasic => Is(S::Render, "basic"),
        N::QuickRenderFull => Is(S::Render, "full"),
        N::QuickMoodMode => Now(S::Front("theme-mode")),
        N::QuickMoodSystem => Is(S::Front("theme-mode"), "system"),
        N::QuickMoodDark => Is(S::Front("theme-mode"), "dark"),
        N::QuickMoodLight => Is(S::Front("theme-mode"), "light"),
        N::QuickPages1 | N::QuickPages2 | N::QuickPages3 | N::QuickPages4 => Now(S::Pages),
        N::YumeCommitMode => Now(S::Front("commit")),
        N::YumeCommitDelayed => Is(S::Front("commit"), "delayed"),
        N::YumeCommitUnique => Is(S::Front("commit"), "unique"),
        N::YumeCommitFluency => Is(S::Front("commit"), "fluency"),
        N::YumePanelMode => Now(S::Front("panel")),
        N::YumePanelBox => Is(S::Front("panel"), "box"),
        N::YumePanelBottom => Is(S::Front("panel"), "bottom"),
        N::YumePanelInline => Is(S::Front("panel"), "inline"),
        N::YumePreeditMode => Now(S::Front("inline-preedit")),
        N::YumePreeditOff => Is(S::Front("inline-preedit"), "off"),
        N::YumePreeditComposition => Is(S::Front("inline-preedit"), "composition"),
        N::YumePreeditPreview => Is(S::Front("inline-preedit"), "preview"),
        N::YumeTogglePrediction => Now(S::Front("predict")),
        N::YumePickScheme => Now(S::Front("scheme")),
        N::YumeMenuSize3
        | N::YumeMenuSize4
        | N::YumeMenuSize5
        | N::YumeMenuSize6
        | N::YumeMenuSize7
        | N::YumeMenuSize8
        | N::YumeMenuSize9 => Now(S::Front("menu")),
        N::YumeCharsetMode => Now(S::Front("charset")),
        N::YumeCharsetAll => Is(S::Front("charset"), "all"),
        N::YumeCharsetCommon => Is(S::Front("charset"), "common"),
        N::YumeCharsetTonggui => Is(S::Front("charset"), "tonggui"),
        N::YumeCharsetHarmonic => Is(S::Front("charset"), "harmonic"),
        N::YumeSingleCharMode => Now(S::Front("single-char")),
        N::YumeSingleCharBoth => Is(S::Front("single-char"), "both"),
        N::YumeSingleCharPrimary => Is(S::Front("single-char"), "primary"),
        N::YumeSingleCharStrict => Is(S::Front("single-char"), "strict"),
        N::YumeSortMode => Now(S::Front("charset-sort")),
        N::YumeSortOriginal => Is(S::Front("charset-sort"), "original"),
        N::YumeSortCommon => Is(S::Front("charset-sort"), "common"),
        N::YumeSortTonggui => Is(S::Front("charset-sort"), "tonggui"),
        N::YumeSortHarmonic => Is(S::Front("charset-sort"), "harmonic"),
        N::YumeLanesMode => Now(S::Front("lanes")),
        N::YumeLaneTonggui => Is(S::Front("lanes"), "tonggui"),
        N::YumeLaneTongfan => Is(S::Front("lanes"), "tongfan"),
        N::YumeLaneTai => Is(S::Front("lanes"), "tai"),
        _ => return None,
    })
}

impl Editor {
    /// **What the row of `cmd` shows beside its meaning**: the value now, a
    /// tick, or nothing.
    pub(super) fn menu_value(&self, cmd: NormalCmd) -> Option<String> {
        match shows(cmd)? {
            Shows::Is(setting, word) => (self.setting_word(setting)? == word).then(|| TICK.to_string()),
            Shows::Now(setting) => {
                let word = self.setting_word(setting)?;
                Some(value_said(setting, &word))
            }
        }
    }

    /// Hand over the value now of a setting the front end holds, by the
    /// name [`shows`] reads it under and in the word its command takes
    /// (`theme-mode` → `dark`, `commit` → `fluency`).
    pub fn set_front_value(&mut self, name: &'static str, word: impl Into<String>) {
        self.front_values.insert(name, word.into());
    }

    /// The setting's value now, as the word its command takes.
    fn setting_word(&self, setting: Setting) -> Option<String> {
        let word = match setting {
            Setting::Switch(line) => match self.switch_state(line)? {
                true => "on",
                false => "off",
            },
            Setting::LineNumbers => match self.line_numbers() {
                LineNumbers::Absolute => "absolute",
                LineNumbers::Relative => "relative",
                LineNumbers::None => "none",
            },
            Setting::Render => match self.render() {
                Render::Off => "off",
                Render::Basic => "basic",
                Render::Full => "full",
            },
            Setting::Pages => return Some(self.pages_here().to_string()),
            Setting::Front(name) => return self.front_values.get(name).cloned(),
        };
        Some(word.to_string())
    }
}

/// **A value in words**: the line of the row that picks it, where a group
/// has one (`空格 , l` names 相對行號), else `on`/`off` said, else the word
/// itself (a number, a scheme's name).
fn value_said(setting: Setting, word: &str) -> String {
    for group in groups::GROUPS {
        for bind in group.binds() {
            if let Some(Shows::Is(of, value)) = shows(bind.run) {
                if of == setting && value == word {
                    return crate::messages::say(bind.says, &[]);
                }
            }
        }
    }
    match word {
        "on" => say!("label.on"),
        "off" => say!("label.off"),
        other => other.to_string(),
    }
}
