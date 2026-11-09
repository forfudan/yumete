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

/// **A keymap's behaviour, one switch each**: no `vim_keymap()` check in the
/// editor; a keymap is the set it picks, and each switch says what it
/// changes where it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    /// The keymap's own Normal and Insert tables are vim's (`VIM_NORMAL`,
    /// the vim rows of Insert mode and the groups), over helix's.
    pub vim_tables: bool,
    /// The caret never stands on a line's break, and `h`/`l` stop at the
    /// line's ends (vim's `whichwrap` empty).
    pub caret_off_newline: bool,
    /// `f`/`t` look on this line only; helix's look through the file.
    pub find_in_line: bool,
    /// `t` stands still when it is already against its character; helix's
    /// steps one past to find the next (「to enable repeated application」).
    pub till_stands_still: bool,
    /// `f`/`t` move the caret without selecting what they crossed.
    pub find_moves_caret: bool,
    /// Words, sentences and paragraphs move by vim's rules (an empty line
    /// is a paragraph's end, a word motion is a caret jump).
    pub vim_text_motions: bool,
    /// Text objects follow vim's rules: a pair found ahead on the line, no
    /// second quote outward for a count.
    pub vim_text_objects: bool,
    /// `"0` holds the last yank and `"1`–`"9` the last deletes of a line.
    pub numbered_registers: bool,
    /// Leaving Insert puts the caret back one.
    pub step_back_after_insert: bool,
    /// `r` replaces count characters from the caret, and `.` repeats the
    /// count; helix's `r` replaces each character of the selection.
    pub replace_from_caret: bool,
    /// `:r` puts the file under the cursor's line; helix's at the cursor.
    pub read_below_the_line: bool,
    /// `u` puts back every cursor (helix's); vim's one cursor has its own.
    pub undo_restores_every_cursor: bool,
    /// An undo point starts where typing began (helix's `A` comes back to
    /// the line's end).
    pub undo_at_typing_start: bool,
    /// `y` with several cursors copies every selection (helix's); vim's `y`
    /// waits for a motion.
    pub yank_every_selection: bool,
    /// A character delete stays on its line (vim's `x`).
    pub delete_stays_in_line: bool,
}

impl Rules {
    pub const HELIX: Rules = Rules {
        vim_tables: false,
        caret_off_newline: false,
        find_in_line: false,
        till_stands_still: false,
        find_moves_caret: false,
        vim_text_motions: false,
        vim_text_objects: false,
        numbered_registers: false,
        step_back_after_insert: false,
        replace_from_caret: false,
        read_below_the_line: false,
        undo_restores_every_cursor: true,
        undo_at_typing_start: true,
        yank_every_selection: true,
        delete_stays_in_line: false,
    };

    pub const VIM: Rules = Rules {
        vim_tables: true,
        caret_off_newline: true,
        find_in_line: true,
        till_stands_still: true,
        find_moves_caret: true,
        vim_text_motions: true,
        vim_text_objects: true,
        numbered_registers: true,
        step_back_after_insert: true,
        replace_from_caret: true,
        read_below_the_line: true,
        undo_restores_every_cursor: false,
        undo_at_typing_start: false,
        yank_every_selection: false,
        delete_stays_in_line: true,
    };
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

    /// The names the config writes.
    pub const NAMES: &'static [&'static str] = &["helix", "vim"];

    pub fn parse(word: &str) -> Option<KeyPreset> {
        match word.trim().to_ascii_lowercase().as_str() {
            "helix" => Some(KeyPreset::Helix),
            "vim" | "vi" => Some(KeyPreset::Vim),
            _ => None,
        }
    }

    /// The lines of [`Self::table`] that are for Normal mode only: in
    /// select mode (vim's Visual) the keymap answers those keys itself.
    pub fn normal_only(self) -> &'static [&'static str] {
        match self {
            KeyPreset::Helix => &[],
            KeyPreset::Vim => &["x", "s"],
        }
    }

    /// **What the preset's cursor, motions and registers do**, by name: the
    /// editor asks a rule, never 「is this vim?」, so a third keymap is a set
    /// of these and a table.
    pub fn rules(self) -> Rules {
        match self {
            KeyPreset::Helix => Rules::HELIX,
            KeyPreset::Vim => Rules::VIM,
        }
    }

    /// The preset's lines, in `[keys.normal]`'s own shape.
    ///
    /// Warning: **the keys carry their `@`, like anyone else's.** These
    /// go into the same map a reader's own bindings do and are dispatched by the
    /// same `run_binding`, so they are spelled the way a reader would have to
    /// spell them — `("D", "@d$")` is keys, `("ZZ", ":x")` is a command line.
    /// Exempting the table would mean the dispatcher had to guess again for one
    /// of its two callers, which is the whole thing `@` exists to avoid.
    pub fn table(self) -> &'static [(&'static str, &'static str)] {
        match self {
            KeyPreset::Helix => &[],
            // Warning: **`;` first wherever vim acts on one character**: here a
            // motion *is* a selection, so after `w` a bare `D` would cut the
            // word; vim's `x` after `w` cuts the one character under the
            // cursor.
            KeyPreset::Vim => &[
                ("x", "vim_delete_char_forward"),
                // Warning: **`s` 是一筆，不是「剪掉 ＋ 插入」兩筆。** 拼成 `;{n}Di`
                // 就是兩筆編輯、兩次撤銷，`2s` 的 redo 也成了兩步；vim 的 `s` 就是
                // `cl`（`:h s`：「synonym for `cl`」），而 `c` 在這一端是一個算子、
                // 一筆。
                //
                // 不能寫成 `;c`（`c` 是算子，`;c` 會停下來等一個動作）；`c{n}l`
                // 把動作一起給了它，不等。
                ("s", "@c{n}l"),
                // Warning: **`dd` `dw` `cw` `yy` `cc` `yw` are not in this
                // table** (#429). They are six lines of a grammar that has
                // hundreds, and a table cannot spell the rest — `d$`, `de`,
                // `dG`, `df,`, `di(`. `d`, `c` and `y` are operators
                // ([`VIM_MOTIONS`]), and all six fall out of the general rule.
                // **The three that mean 「next line, where its writing
                // begins」** (B4). `Enter` is the same motion and
                // cannot be spelled in this table — it has a key arm of its
                // own — but `+` and `-` are characters and belong here.
                ("+", "@jgs"),
                ("-", "@kgs"),
                ("^", "@gs"),
                ("$", "vim_goto_line_end"),
                ("0", "@gh"),
                // 「this word, elsewhere」 — `g/` here, and the one vim key a
                // hand reaches for without thinking.
                ("*", "search_selection"),
                // Warning: **The ones that do something *else* here**, which is
                // worse than doing nothing: `%` selects the whole file (vim
                // jumps to the matching bracket), `D` and `C` cut the
                // selection (vim takes the rest of the line).
                //
                // Warning: **`J`/`K` 在 vim 鍵位下讓給 vim**，雖然它們在這裏是翻
                // 半頁、讀一本小說按得最多的一對。代價只落在 vim 鍵位那一邊，而且
                // 不重：`C-f`/`C-b`/`C-d`/`C-u` 兩套鍵位下都綁着，一個明確選了 vim
                // 鍵位的人本來就用它們翻頁，在 vim 裏也從不用 `J`。helix 出廠那一套
                // 一個字不動。
                //
                // `K` 給的是「查光標下這個東西」——Neovim 掛着 LSP 時 `K` 就是
                // `vim.lsp.buf.hover()`（`lsp.txt`：「K is mapped to
                // vim.lsp.buf.hover()」），和 `空格 k` 是同一件事。
                // **`q` 錄、`Q` 播，和這個編輯器正好反着。** helix 的規矩是 `Q`
                // 錄、`q` 播，這個倉跟的是它；而一個 vim 的手按 `q` 是「開始錄」，
                // 照 helix 的鍵就會當場把上一個巨集放一遍——「一對調換了的鍵是最
                // 壞的一種分歧」（`keys.rs` 的 `Q` 那一段）。
                //
                // Warning: **展開出來的鍵不再過別名層**，所以這兩行不會互相咬。
                // `q{a-z}` waits for its letter (`vim_record_macro`, a row of
                // the vim table); `Q` plays the register last recorded into,
                // as Neovim's does.
                ("Q", "vim_replay_last_macro"),
                ("K", "hover"),
                ("%", "vim_match_brackets"),
                // **`(`/`)` 是句子**，不是 helix 的「換主選區」：vim 的手按的是
                // 上一句/下一句——這個編輯器把句子放在 `H`/`L` 上（#404）。換主選區在 vim 鍵位下没有鍵，多選區那一族本來
                // 就不是 vim 的東西。
                // **`m` 讓給設標記這件事不在這張表上**：寫成 `("m", "M")` 不行，
                // `M` 是「屏幕中間」，換出來的 `M` 又分不出是誰按的。所以是編輯器
                // 自己一支 vim 專用的 `m`（`keys.rs`）。
                ("D", "@d$"),
                ("C", "@c$"),
                // **`Y` ＝ `y$`，跟 nvim。**
                // nvim 0.6 起的出廠值，理由是和 `D`（`d$`）`C`（`c$`）成一套。
                // **一個鍵都没丟**：`yy` 一直是「複製整行」，兩種寫法都在。
                ("Y", "@y$"),
                ("S", "@cc"),
                // The two a hand presses on its way out of the editor.
                ("ZZ", ":x"),
                ("ZQ", ":quit!"),
            ],
        }
    }
}

/// **How a `[keys.<mode>]` table was written**, beside the runs
/// it reads into: which keys are groups (a table under a key), the order
/// the lines are in, and the menu text a command row gives itself. The
/// config reads it, and the editor lays its key tree from it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyShape {
    /// Each table that is a group, by the keys that open it as the config
    /// spells them (`<space>b`), in the file's order.
    pub groups: Vec<KeyGroup>,
    /// Every line's run and every group's, in the file's order.
    pub order: Vec<String>,
    /// A command row's own text (`d = { run = ":!lazygit", label = "lazygit" }`),
    /// by its run.
    pub says: std::collections::HashMap<String, String>,
    /// The runs and groups written inside a `[keys.<mode>.space]` table,
    /// which mean the Space group wherever it is opened; a flat
    /// `"<space>…"` is the Space key itself.
    pub in_space_table: std::collections::HashSet<String>,
}

/// One group a table makes: its keys, and the two words its table may set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyGroup {
    pub path: String,
    /// Stays open after each key until `Esc` (`sticky = true`).
    pub sticky: bool,
    /// `inherit = false`: a built-in group's own keys are not read.
    pub inherit: bool,
}

/// The shapes of one set of tables: Normal mode's, select mode's and a
/// table grid's (`[keys.normal]`, `[keys.select]`, `[keys.table]`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyShapes {
    pub normal: KeyShape,
    pub select: KeyShape,
    pub table: KeyShape,
}

/// **What may follow a vim operator** (`d`, `c`, `y`) — #429.
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
    // Warning: **`ge` is not among them.** vim's `ge` is 「the end of the
    // previous word」 — a short backward step — while this editor's `ge` goes
    // to the end of the **file**, so `dge` would delete the whole document
    // where vim takes back half a word. A motion that means two
    // wildly different things is worse under an operator than no motion at
    // all: `d` + an unknown key does nothing and says so.
    ("}", m("{n}}")),
    ("{", m("{n}{")),
    // vim 的句子動作是這一對。Warning: **`H`/`L` 不是句子**——它們在 vim 裏是
    // 屏幕的頂/中/底（§5.94：兼容第一），見下面那三行。
    //
    // 右邊這兩個按鍵串**没人讀**：算子後面那一格問的是 [`crate::vim::step_for`]，
    // 它自己認得 `(`/`)`；這張表只有兩個用處，一是算子提示那一行的鍵名
    // （`hint.rs` 的 `vim_motion_list`），二是 `linewise`/`wants` 這幾格。
    ("(", m("{n}(")),
    (")", m("{n})")),
    // 屏幕的頂/中/底，整行整行地取。
    ("H", line("{n}H")),
    ("M", line("{n}M")),
    ("L", line("{n}L")),
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

/// **vim 的 `whichwrap` 認得的字母**，照它的拼法：`b` 退格、`h`、`l`、`<` `>` 兩個
/// 方向鍵、`~`。
///
/// Warning: **`s` 不在裏面。** vim 出廠的 `b,s` 裏那個 `s` 是普通模式的空格，而空格
/// 在這個編輯器裏是菜單引導鍵。
pub const WHICH_WRAP: &str = "bhl<>~";

/// 讀一串 `whichwrap`：留下的字母，和說不通的那幾個。
///
/// 逗號和空格隨便加（`"<,>,b"` 和 `"<>b"` 一樣），重複的只留一次。回出來的次序
/// 照 [`WHICH_WRAP`]，所以同一組字母永遠印成同一個樣子。
///
/// Warning: **一份校驗，兩個讀者。** 配置檔那一邊和 `:which-wrap` 那一邊問的是同
/// 一句話；分兩處寫就是下一個「兩處記同一件事」。
pub fn read_which_wrap(written: &str) -> (String, Vec<char>) {
    let (mut kept, mut bad) = (String::new(), Vec::new());
    for c in written.chars() {
        match c {
            ',' | ' ' => {}
            c if WHICH_WRAP.contains(c) => {
                if !kept.contains(c) {
                    kept.push(c);
                }
            }
            c if !bad.contains(&c) => bad.push(c),
            _ => {}
        }
    }
    let kept = WHICH_WRAP.chars().filter(|c| kept.contains(*c)).collect();
    (kept, bad)
}
