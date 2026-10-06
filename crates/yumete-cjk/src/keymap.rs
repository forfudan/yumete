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
            // Warning: **`;` first wherever vim acts on one character**: here a
            // motion *is* a selection, so after `w` a bare `D` would cut the
            // word; vim's `x` after `w` cuts the one character under the
            // cursor.
            KeyPreset::Vim => &[
                ("x", ";{n}D"),
                // Warning: **`s` cannot be 「collapse, then `c`」 any more**
                // (2026-09-19). It was written before #429 made `c` an
                // operator: now `;c` collapses and then *waits for a motion*,
                // so a hand typing `shello` got no substitution and an opened
                // line from the `o`. vim's `s` is 「cut this character and
                // start typing」, which is what `x` does plus `i`.
                ("s", ";{n}Di"),
                // Warning: **`dd` `dw` `cw` `yy` `cc` `yw` are not in this table
                // any more** (#429, 2026-09-18). They were six lines of a
                // grammar that has hundreds, and a table cannot spell the rest
                // — `d$`, `de`, `dG`, `df,`, `di(`. `d`, `c` and `y` are
                // operators now ([`VIM_MOTIONS`]), and all six fall out of the
                // general rule.
                // **The three that mean 「next line, where its writing
                // begins」** (B4, 2026-09-20). `Enter` is the same motion and
                // cannot be spelled in this table — it has a key arm of its
                // own — but `+` and `-` are characters and belong here.
                ("+", "jgs"),
                ("-", "kgs"),
                ("^", "gs"),
                ("$", "gl"),
                ("0", "gh"),
                // 「this word, elsewhere」 — `g/` here, and the one vim key a
                // hand reaches for without thinking.
                ("*", "g/"),
                // Warning: **The ones that do something *else* here**, which is
                // worse than doing nothing: `%` selects the whole file (vim
                // jumps to the matching bracket), `D` and `C` cut the
                // selection (vim takes the rest of the line).
                //
                // Warning: **`J`／`K` 2026-10-05 讓出來了，翻的是 2026-09-18 自己那
                // 一條**（原話：「J 合併行我們和 helix 也不一樣，我覺得這個應該保持
                // gJ」，理由是 `J`／`K` 在這裏是翻半頁，讀一本小說按得最多的一對）。
                //
                // **翻案的理由是那個代價只落在 helix 那一邊**：`C-f`／`C-b`／`C-d`／
                // `C-u` 兩套鍵位下都綁着，一個明確選了 vim 鍵位的人本來就用它們翻
                // 頁，在 vim 裏也從不用 `J`。helix 出廠那一套一個字不動。
                //
                // `K` 給的是「查光標下這個東西」——Neovim 掛着 LSP 時 `K` 就是
                // `vim.lsp.buf.hover()`（`lsp.txt`：「K is mapped to
                // vim.lsp.buf.hover()」），和 `空格 k` 是同一件事。
                // **`q` 錄、`Q` 播，和這個編輯器正好反着**（2026-10-06）。helix 的
                // 規矩是 `Q` 錄、`q` 播，這個倉跟的是它；而一個 vim 的手按 `q`
                // 本來是「開始錄」，在這裏當場把上一個巨集放了一遍——倉裏自己
                // 寫過「一對調換了的鍵是最壞的一種分歧」（`keys.rs` 的 `Q` 那一
                // 段），那句話正是說這個。
                //
                // Warning: **展開出來的鍵不再過別名層**，所以這兩行不會互相咬。
                ("q", "Q"),
                ("Q", "q"),
                ("K", " k"),
                ("%", "mm"),
                // **`(`／`)` 是句子**（2026-10-06）。這裏它們本來是「換主選區」，
                // 而 vim 的手按的是上一句／下一句——這個編輯器把句子放在 `H`／
                // `L` 上（#404）。換主選區在 vim 鍵位下沒有鍵，多選區那一族本來
                // 就不是 vim 的東西。
                // **`m` 讓給設標記這件事不在這張表上**（2026-10-06）：它從前寫成
                // `("m", "M")`，而 `M` 當天讓給了「屏幕中間」，換出來的 `M` 又分不出
                // 是誰按的。改成編輯器自己一支 vim 專用的 `m`（`keys.rs`）。
                ("D", "d$"),
                ("C", "c$"),
                // **`Y` ＝ `y$`，跟 nvim**（2026-10-06 定「我傾向 nvim」）。
                // nvim 0.6 起的出廠值，理由是和 `D`（`d$`）`C`（`c$`）成一套。
                // **一個鍵都没丟**：`yy` 一直是「複製整行」，兩種寫法都在。
                ("Y", "y$"),
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
    // Warning: **`ge` is not among them** (2026-09-19). vim's `ge` is 「the end of
    // the previous word」 — a short backward step — while this editor's `ge`
    // goes to the end of the **file**, so `dge` deleted the whole document
    // where vim would have taken back half a word. A motion that means two
    // wildly different things is worse under an operator than no motion at
    // all: `d` + an unknown key does nothing and says so.
    ("}", m("{n}}")),
    ("{", m("{n}{")),
    // vim 的句子動作是這一對。Warning: **`H`／`L` 2026-10-06 從這裏撤了**——它們在
    // vim 裏是屏幕的頂／中／底，不是句子（§5.94：兼容第一）。
    //
    // 右邊這兩個按鍵串**沒人讀**：算子後面那一格問的是 [`crate::vim::step_for`]，
    // 它自己認得 `(`／`)`；這張表今天只剩兩個用處，一是算子提示那一行的鍵名
    // （`hint.rs` 的 `vim_motion_list`），二是 `linewise`／`wants` 這幾格。
    ("(", m("{n}(")),
    (")", m("{n})")),
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
