//! Shipped keymaps (#428) — data only, so the config that names one and the
//! editor that switches to one read the same table.

/// A shipped keymap, laid under `[keys.normal]` (#428).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum KeyPreset {
    /// The editor's own keys, which are helix's.
    #[default]
    Helix,
    /// vim's muscle memory, **translated** into keys this editor already has
    /// — not a second grammar. Only what does harm when a vim hand types it
    /// here: `x` and `s` and `dd` delete or change something else, `^` `$` `0`
    /// go nowhere.
    Vim,
}

impl KeyPreset {
    /// Both, for `:keymap`'s menu.
    pub const ALL: [KeyPreset; 2] = [KeyPreset::Helix, KeyPreset::Vim];

    /// Its name, as the config and `:keymap` spell it.
    pub fn name(self) -> &'static str {
        match self {
            KeyPreset::Helix => "helix",
            KeyPreset::Vim => "vim",
        }
    }

    pub fn parse(word: &str) -> Option<KeyPreset> {
        match word.trim().to_ascii_lowercase().as_str() {
            "helix" => Some(KeyPreset::Helix),
            "vim" | "vi" => Some(KeyPreset::Vim),
            _ => None,
        }
    }

    /// The preset's lines, in `[keys.normal]`'s own shape.
    pub fn table(self) -> &'static [(&'static str, &'static str)] {
        match self {
            KeyPreset::Helix => &[],
            // ⚠️ **`;` first wherever vim acts on one character**: here a
            // motion *is* a selection, so after `w` a bare `D` would cut the
            // word; vim's `x` after `w` cuts the one character under the
            // cursor.
            KeyPreset::Vim => &[
                ("x", ";D"),
                ("s", ";c"),
                ("V", "x"),
                ("dd", "xd"),
                ("yy", "xy"),
                ("cc", "xc"),
                ("dw", "wd"),
                ("cw", "ec"),
                ("yw", "wy"),
                ("^", "gs"),
                ("$", "gl"),
                ("0", "gh"),
            ],
        }
    }
}
