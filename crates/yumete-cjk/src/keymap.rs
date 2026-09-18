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
                // ⚠️ **`dd` `dw` `cw` `yy` `cc` `yw` are not in this table
                // any more** (#429, 2026-09-18). They were six lines of a
                // grammar that has hundreds, and a table cannot spell the rest
                // — `d$`, `de`, `dG`, `df,`, `di(`. `d`, `c` and `y` are
                // operators now ([`VIM_MOTIONS`]), and all six fall out of the
                // general rule.
                ("^", "gs"),
                ("$", "gl"),
                ("0", "gh"),
                // 「this word, elsewhere」 — `g/` here, and the one vim key a
                // hand reaches for without thinking.
                ("*", "g/"),
                // ⚠️ **The ones that do something *else* here**, which is
                // worse than doing nothing: `%` selects the whole file (vim
                // jumps to the matching bracket), `D` and `C` cut the
                // selection (vim takes the rest of the line).
                //
                // ⚠️ **`J` is deliberately not among them** (作者 2026-09-18):
                // 「J 合併行我們和 helix 也不一樣，我覺得這個應該保持 gJ」.
                // `J`／`K` are half a page here — the most-pressed pair in a
                // novel, and not worth a chord — and joining is `gJ` in both
                // presets. A vim hand loses `J`; it keeps the pair it presses
                // a hundred times a day.
                ("%", "mm"),
                ("D", "d$"),
                ("C", "c$"),
                ("Y", "yy"),
                ("S", "cc"),
                // The two a hand presses on its way out of the editor.
                ("ZZ", ":x"),
                ("ZQ", ":quit!"),
            ],
        }
    }
}

/// **What may follow a vim operator** (`d`, `c`, `y`) — #429, 2026-09-18.
///
/// A translation table could give a vim hand `dw` and `dd`; it could not give
/// it `d$`, `de`, `dG` or `di(`, because those are an *operator waiting for a
/// motion* and a table has no way to wait. This is the list of motions that
/// wait is allowed to end with, and what this editor presses to do each one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VimMotion {
    /// The keys to play, `{n}` marking where the count goes. A `%` stands for
    /// the character the motion still has to be told (`df,`).
    pub keys: &'static str,
    /// Whether the operator takes **whole lines** — `dj` and `dG` do, `dw`
    /// does not. The selection is stretched to the line's bounds before the
    /// action, which is what `X` does here.
    pub linewise: bool,
    /// Whether the motion swallows one more character (`f`, `i`, `a`).
    pub wants: bool,
    /// Whether the motion **is** the selection rather than something to
    /// extend over — `mi(` selects the pair, so no 延伸模式 around it.
    pub whole: bool,
    /// vim's 「till」 (`t`, `T`): the same search as `f`, stopping one short of
    /// the character. This editor has no such motion — `t` is the table group
    /// — so it is `f` with the head pulled back one.
    pub till: bool,
    /// Which way that pulling back goes: `T` searches backwards, so its head
    /// moves forwards.
    pub back: bool,
}

const fn m(keys: &'static str) -> VimMotion {
    VimMotion { keys, linewise: false, wants: false, whole: false, till: false, back: false }
}

const fn line(keys: &'static str) -> VimMotion {
    VimMotion { keys, linewise: true, wants: false, whole: false, till: false, back: false }
}

const fn asks(keys: &'static str) -> VimMotion {
    VimMotion { keys, linewise: false, wants: true, whole: false, till: false, back: false }
}

const fn object(keys: &'static str) -> VimMotion {
    VimMotion { keys, linewise: false, wants: true, whole: true, till: false, back: false }
}

/// `t` and `T`: `f`'s search, one short of the character.
const fn till(keys: &'static str, back: bool) -> VimMotion {
    VimMotion { keys, linewise: false, wants: true, whole: false, till: true, back }
}

/// The table itself, keyed by what has been typed since the operator.
pub const VIM_MOTIONS: &[(&str, VimMotion)] = &[
    // ---- Words ----------------------------------------------------------
    ("w", m("{n}w")),
    ("W", m("{n}W")),
    ("b", m("{n}b")),
    ("B", m("{n}B")),
    ("e", m("{n}e")),
    ("E", m("{n}E")),
    // ---- The line -------------------------------------------------------
    ("$", m("gl")),
    ("0", m("gh")),
    ("^", m("gs")),
    // ---- Down the page --------------------------------------------------
    ("j", line("{n}j")),
    ("k", line("{n}k")),
    ("G", line("{n}G")),
    ("gg", line("{n}gg")),
    ("ge", line("ge")),
    ("}", m("{n}}")),
    ("{", m("{n}{")),
    ("H", m("{n}H")),
    ("L", m("{n}L")),
    // ---- Find, which is told a character --------------------------------
    ("f", asks("{n}f%")),
    ("F", asks("{n}F%")),
    ("t", till("{n}f%", false)),
    ("T", till("{n}F%", true)),
    // ---- Text objects, which are the selection --------------------------
    ("i", object("mi%")),
    ("a", object("ma%")),
];

/// The motion `typed` names, if it names one.
pub fn vim_motion(typed: &str) -> Option<VimMotion> {
    VIM_MOTIONS.iter().find(|(k, _)| *k == typed).map(|(_, v)| *v)
}

/// Whether `typed` is the start of a longer motion (`g` before `gg`).
pub fn vim_motion_ahead(typed: &str) -> bool {
    VIM_MOTIONS.iter().any(|(k, _)| k.len() > typed.len() && k.starts_with(typed))
}
