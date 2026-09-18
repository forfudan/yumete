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
                ("x", ";{n}D"),
                ("s", ";c"),
                ("V", "x"),
                ("dd", "{n}xd"),
                ("yy", "{n}xy"),
                ("cc", "{n}xc"),
                // ⚠️ **The word ones go through 延伸模式** (2026-09-18).
                // `3dw` in vim is three words; `3w` here is the *third* word,
                // because a motion is a selection and each step replaces the
                // last. `v` makes the steps add up, and the action closes it.
                // `{n}` puts the count on the `w` that is doing the extending
                // rather than on the `v` that opens it.
                ("dw", "v{n}wd"),
                ("cw", "v{n}ec"),
                ("yw", "v{n}wy"),
                ("^", "gs"),
                ("$", "gl"),
                ("0", "gh"),
            ],
        }
    }
}
