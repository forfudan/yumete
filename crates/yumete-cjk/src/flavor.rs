//! **The yumete flavour** — what yumete adds to or changes in
//! a keymap, laid over whichever one is in use. The helix and vim keymaps
//! are their upstreams on every key those editors bind; everything that is
//! yumete's own is an item here, so it can be named, listed, and switched
//! off one at a time.
//!
//! Precedence, top first: `[keys.normal]`, then the flavour, then the base
//! keymap.
//!
//! An item belongs here only when it is needed, improves the experience, or
//! adds a feature. A key that simply does not match its upstream is a bug in
//! the base keymap, not an item (`ge`).

use std::collections::BTreeMap;

use crate::KeyPreset;

/// How much of the flavour is on.
///
/// `Basic` is what you see the moment you press it: a menu, a new key, a
/// command on a key upstream leaves free. `Full` adds the items that make a
/// familiar key quietly do something else (`d` not filling the register),
/// which nobody should meet without having asked for it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FlavorLevel {
    Off,
    #[default]
    Basic,
    Full,
}

impl FlavorLevel {
    pub const ALL: [FlavorLevel; 3] = [FlavorLevel::Off, FlavorLevel::Basic, FlavorLevel::Full];

    /// The words the config and `:keymap-yumete-flavor` write.
    pub const NAMES: &'static [&'static str] = &["off", "basic", "full"];

    pub fn name(self) -> &'static str {
        Self::NAMES[self as usize]
    }

    pub fn parse(word: &str) -> Option<FlavorLevel> {
        let word = word.trim().to_ascii_lowercase();
        Self::ALL.into_iter().find(|level| level.name() == word)
    }
}

/// What one item is: the name the config writes, the level that turns it
/// on, the keymaps it is laid over, and the line that names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemInfo {
    /// `[keys.flavor_items] <id> = false`.
    pub id: &'static str,
    pub level: FlavorLevel,
    /// The base keymaps it changes. A vim hand has vim's own answer for some
    /// of them (`"_d` for a delete that keeps the register), and an item
    /// that would only repeat it stays off there.
    pub keymaps: &'static [KeyPreset],
    /// The keys it changes, as `[keys.normal]` spells them (`C-we` is `C-w`
    /// then `e`): a line there for one of them wins over the item, and the
    /// report says so.
    pub keys: &'static [&'static str],
    /// The keys under the vim keymap, when they are not [`Self::keys`]
    /// (the case menu is `` g` `` there, the labels `gW`).
    pub vim_keys: Option<&'static [&'static str]>,
    /// Its keys are Insert mode's, so `[keys.insert]` is what can take them.
    pub insert: bool,
    /// A message tag: what it does, in a few words.
    pub label: &'static str,
    /// A message tag: the line the settings panel says about it, with what
    /// the key does when the item is off.
    pub hint: &'static str,
}

macro_rules! flavor_items {
    ($( $(#[$doc:meta])* $item:ident => $info:expr, )*) => {
        /// One thing the flavour changes.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum FlavorItem {
            $( $(#[$doc])* $item, )*
        }

        impl FlavorItem {
            /// Every item, in the order lists show them.
            pub const ALL: &'static [FlavorItem] = &[$( FlavorItem::$item, )*];

            pub fn info(self) -> ItemInfo {
                match self {
                    $( FlavorItem::$item => $info, )*
                }
            }
        }
    };
}

flavor_items! {
    /// `` ` `` opens the case and conversion menu (繁簡, regions, width); under vim it is `` g` ``, since `` ` `` is vim's jump to a mark. Off, helix's `` ` `` lowercases and `` A-` `` uppercases.
    CaseMenu => ItemInfo {
        id: "case_menu",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &["`"],
        vim_keys: Some(&["g`"]),
        insert: false,
        label: "keys.flavor.item.case-menu",
        hint: "keys.flavor.item.case-menu.hint",
    },
    /// `H` and `L` walk sentences, keys helix leaves free.
    SentencesOnHL => ItemInfo {
        id: "sentences_on_h_l",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix],
        keys: &["H", "L"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.sentences-on-h-l",
        hint: "keys.flavor.item.sentences-on-h-l.hint",
    },
    /// `M` sets a mark and `'` goes to it, across files.
    MarksOnMQuote => ItemInfo {
        id: "marks_on_m_quote",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix],
        keys: &["M", "'"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.marks-on-m-quote",
        hint: "keys.flavor.item.marks-on-m-quote.hint",
    },
    /// `C-n` and `C-p` turn two-thirds of a page.
    TwoThirdsPage => ItemInfo {
        id: "two_thirds_page",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix],
        keys: &["C-n", "C-p"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.two-thirds-page",
        hint: "keys.flavor.item.two-thirds-page.hint",
    },
    /// `Space t`, the table menu. The grid's own keys are the table feature's, not an item.
    Tables => ItemInfo {
        id: "tables",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &[" t"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.tables",
        hint: "keys.flavor.item.tables.hint",
    },
    /// `Space m` resolves a merge conflict, `]m` and `[m` walk them.
    MergeConflicts => ItemInfo {
        id: "merge_conflicts",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &[" m", "]m", "[m"],
        vim_keys: Some(&[" m"]),
        insert: false,
        label: "keys.flavor.item.merge-conflicts",
        hint: "keys.flavor.item.merge-conflicts.hint",
    },
    /// `Space z`, the input method's menu.
    InputMethodMenu => ItemInfo {
        id: "input_method_menu",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &[" z"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.input-method-menu",
        hint: "keys.flavor.item.input-method-menu.hint",
    },
    /// `Space o`, recent files and projects.
    RecentFiles => ItemInfo {
        id: "recent_files",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &[" o"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.recent-files",
        hint: "keys.flavor.item.recent-files.hint",
    },
    /// `Space "`, the registers' menu.
    RegistersMenu => ItemInfo {
        id: "registers_menu",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &[" \""],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.registers-menu",
        hint: "keys.flavor.item.registers-menu.hint",
    },
    /// `Space n` and `Space N`, the dictionary as a popup or in the sidebar.
    Dictionary => ItemInfo {
        id: "dictionary",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &[" n", " N"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.dictionary",
        hint: "keys.flavor.item.dictionary.hint",
    },
    /// The popup and sidebar pairs: `Space K` puts helix's hover in the sidebar; `Space l` `L` a diagnostic, `Space x` `X` a change, `Space i` `I` a table record.
    PopupPanelPairs => ItemInfo {
        id: "popup_panel_pairs",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &[" K", " l", " L", " x", " X", " i", " I"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.popup-panel-pairs",
        hint: "keys.flavor.item.popup-panel-pairs.hint",
    },
    /// `Space E`, the file explorer from this file's directory.
    ExplorerHere => ItemInfo {
        id: "explorer_here",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &[" E"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.explorer-here",
        hint: "keys.flavor.item.explorer-here.hint",
    },
    /// `Space Space` and `Space Backspace` turn a page.
    SpacePaging => ItemInfo {
        id: "space_paging",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &["  ", " <backspace>"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.space-paging",
        hint: "keys.flavor.item.space-paging.hint",
    },
    /// `C-w e` `i` show or hide the sidebars, `C-w E` `I` go into them.
    Sidebars => ItemInfo {
        id: "sidebars",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &["C-we", "C-wi", "C-wE", "C-wI"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.sidebars",
        hint: "keys.flavor.item.sidebars.hint",
    },
    /// The `g` group's jumps (every key free in vim, Neovim and helix): `gz` jumps to a word by its first letters (z as in
    /// easymotion), `gZ` to a Chinese character by its pinyin (Z for 中),
    /// `g/` searches the word, `gL` follows any link (it hands the work to
    /// `gx`, `gf` or `gd`, whichever the link is). Under helix `gx` opens a
    /// link, as vim's does; under vim `gW` is the jump labels, since vim's
    /// `gw` formats.
    GotoExtras => ItemInfo {
        id: "goto_extras",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &["gz", "gZ", "g/", "gL", "gx"],
        vim_keys: Some(&["gz", "gZ", "g/", "gL", "gW"]),
        insert: false,
        label: "keys.flavor.item.goto-extras",
        hint: "keys.flavor.item.goto-extras.hint",
    },
    /// A new window on a chosen side from the `C-w n` group: `h` `j` `k` `l`
    /// blank, `H` `J` `K` `L` with the buffer picker open in it. Free in
    /// helix's group; vim's `C-w n` opens below at once and keeps it.
    WindowSides => ItemInfo {
        id: "window_sides",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix],
        keys: &["C-wnh", "C-wnj", "C-wnk", "C-wnl", "C-wnH", "C-wnJ", "C-wnK", "C-wnL"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.window-sides",
        hint: "keys.flavor.item.window-sides.hint",
    },
    /// mini.splitjoin's `gS`: the brackets round the cursor,
    /// one item per line, or back onto one line. Free in helix and vim.
    SplitJoin => ItemInfo {
        id: "split_join",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &["gS"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.split-join",
        hint: "keys.flavor.item.split-join.hint",
    },
    /// Insert mode's `C-a` and `C-e` to the line's ends, `C-n` for completion, `C-g` an undo checkpoint (helix's `C-s`, which a terminal takes).
    InsertExtras => ItemInfo {
        id: "insert_extras",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix],
        keys: &["C-a", "C-e", "C-n", "C-g"],
        vim_keys: None,
        insert: true,
        label: "keys.flavor.item.insert-extras",
        hint: "keys.flavor.item.insert-extras.hint",
    },
    /// Insert mode's `C-c` leaves Insert, as `Esc` does.
    InsertCtrlC => ItemInfo {
        id: "insert_ctrl_c",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix],
        keys: &["C-c"],
        vim_keys: None,
        insert: true,
        label: "keys.flavor.item.insert-ctrl-c",
        hint: "keys.flavor.item.insert-ctrl-c.hint",
    },
    /// `X` selects the line above, as `x` the line below: Shift reverses, as `f`/`F` do.
    XSelectsUpward => ItemInfo {
        id: "x_selects_upward",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix],
        keys: &["X"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.x-selects-upward",
        hint: "keys.flavor.item.x-selects-upward.hint",
    },
    /// `G` with no count goes to the last line's start; with one, to that line, as helix's.
    BareGLastLine => ItemInfo {
        id: "bare_g_last_line",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix],
        keys: &["G"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.bare-g-last-line",
        hint: "keys.flavor.item.bare-g-last-line.hint",
    },
    /// `A-j` and `A-k` move the selected lines (a paragraph, in prose) down or up.
    MoveLines => ItemInfo {
        id: "move_lines",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &["A-j", "A-k"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.move-lines",
        hint: "keys.flavor.item.move-lines.hint",
    },
    /// `0` `^`, the line's start and its first character, as vim's; both
    /// free in helix. `$` is helix's (`shell_keep_pipe`), and the line's end
    /// is helix's `gl`.
    LineStartKeys => ItemInfo {
        id: "line_start_keys",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix],
        keys: &["0", "^"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.line-start-keys",
        hint: "keys.flavor.item.line-start-keys.hint",
    },
    /// `D` deletes to the line's end and `A-D` does without filling the register; `delete_keeps_register` swaps them as it does `d` and `A-d`.
    DToLineEnd => ItemInfo {
        id: "d_to_line_end",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix],
        keys: &["D", "A-D"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.d-to-line-end",
        hint: "keys.flavor.item.d-to-line-end.hint",
    },
    /// vim's `za` `zR` `zM` `zf` under helix, on `z` keys helix
    /// leaves free: helix has no folds, and the heading folds have no keys
    /// there otherwise. `zf` folds the selection's lines, helix's way round.
    /// `zc` `zm` `zj` `zk` stay helix's; `zo` is not bound, as `za` does it.
    HelixFoldKeys => ItemInfo {
        id: "helix_fold_keys",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Helix],
        keys: &["za", "zR", "zM", "zf"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.helix-fold-keys",
        hint: "keys.flavor.item.helix-fold-keys.hint",
    },
    /// vim's `gc{motion}`, `gcc` and Visual `gc`, as vim 9.1's comment package
    /// and Neovim have them: the lines, commented as
    /// `Space A-c` comments them. Off, vim has no comment key but `\c`.
    VimCommentKeys => ItemInfo {
        id: "vim_comment_keys",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Vim],
        keys: &["gc"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.vim-comment-keys",
        hint: "keys.flavor.item.vim-comment-keys.hint",
    },
    /// The Space menu under vim, on vim's `Space`; off, `Space`
    /// moves right as vim's and the menu is on `\`, vim's leader.
    VimSpaceMenu => ItemInfo {
        id: "vim_space_menu",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Vim],
        keys: &[" "],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.vim-space-menu",
        hint: "keys.flavor.item.vim-space-menu.hint",
    },
    /// helix's keys under vim, on keys vim and Neovim leave free (measured
    /// against `index.txt` and Neovim's default mappings): the Alt
    /// keys, `C-s`, `Z`'s sticky view, `gb` `gy` `g.`, `]g` `[g`, `;` `,`
    /// with no `f` to repeat, Insert `C-k` and `A-d`. Off, vim has only
    /// vim's. Under helix they are helix's own, so this item is always on
    /// there. `ga` `gp` `gr` `gc` `]f` `]t` are vim's or Neovim's, and stay
    /// theirs.
    HelixKeysUnderVim => ItemInfo {
        id: "helix_keys_under_vim",
        level: FlavorLevel::Basic,
        keymaps: &[KeyPreset::Vim],
        keys: &["g.", ";", ",", "A-C", "A-,", "A--", "A-_", "A-:", "A-x", "A-s", "A-(", "A-)", "A-K", "A-d", "A-c", "A-u", "A-U", "A-!", "A-|", "A-J", "A-;", "A-*", "A-.", "A-`", "A-o", "A-i", "A-I", "A-p", "A-n", "A-e", "A-b", "A-a", "C-s", "Z", "gb", "gy", "]g", "[g", "]e", "[e", "]x", "[x"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.helix-keys-under-vim",
        hint: "keys.flavor.item.helix-keys-under-vim.hint",
    },
    /// `d` and `c` delete without filling the register, and `A-d`/`A-c` are the pair that cut — in the text and in a table's grid. helix only: vim has `"_d`, and that is what vim users write.
    DeleteKeepsRegister => ItemInfo {
        id: "delete_keeps_register",
        level: FlavorLevel::Full,
        keymaps: &[KeyPreset::Helix],
        keys: &["d", "c", "A-d", "A-c"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.delete-keeps-register",
        hint: "keys.flavor.item.delete-keeps-register.hint",
    },
    /// `Esc` in Normal mode also collapses the selections to the primary cursor; leaving select mode still keeps the selection.
    EscCollapses => ItemInfo {
        id: "esc_collapses",
        level: FlavorLevel::Full,
        keymaps: &[KeyPreset::Helix],
        keys: &["Esc"],
        vim_keys: None,
        insert: false,
        label: "keys.flavor.item.esc-collapses",
        hint: "keys.flavor.item.esc-collapses.hint",
    },
    /// Insert mode's `S-Tab` types the other kind of indent (a tab where the file uses spaces); helix's types the file's own.
    ShiftTabOtherIndent => ItemInfo {
        id: "shift_tab_other_indent",
        level: FlavorLevel::Full,
        keymaps: &[KeyPreset::Helix, KeyPreset::Vim],
        keys: &["<S-tab>"],
        vim_keys: None,
        insert: true,
        label: "keys.flavor.item.shift-tab-other-indent",
        hint: "keys.flavor.item.shift-tab-other-indent.hint",
    },
}

impl FlavorItem {
    pub fn id(self) -> &'static str {
        self.info().id
    }

    pub fn parse(id: &str) -> Option<FlavorItem> {
        Self::ALL.iter().copied().find(|item| item.id() == id.trim())
    }

    /// Its bit in a set of items, as a key table asks (see `Flavor::mask`).
    pub fn bit(self) -> u64 {
        1 << (self as u32)
    }

    /// The keys it changes under `keymap`.
    pub fn keys_under(self, keymap: KeyPreset) -> &'static [&'static str] {
        let info = self.info();
        match (keymap, info.vim_keys) {
            (KeyPreset::Vim, Some(keys)) => keys,
            _ => info.keys,
        }
    }

    /// The ids, for a complaint about one that is not.
    pub fn ids() -> Vec<&'static str> {
        Self::ALL.iter().map(|item| item.id()).collect()
    }
}

/// The flavour in force: a level, and the items switched apart from it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flavor {
    pub level: FlavorLevel,
    /// `[keys.flavor_items]`: an item written here is what it says, whatever
    /// the level.
    pub items: BTreeMap<FlavorItem, bool>,
}

impl Flavor {
    /// Whether `item` is on under `keymap`.
    pub fn on(&self, item: FlavorItem, keymap: KeyPreset) -> bool {
        // helix's keys are helix's own under helix: what this item switches
        // is whether vim borrows them.
        if item == FlavorItem::HelixKeysUnderVim && keymap == KeyPreset::Helix {
            return true;
        }
        let info = item.info();
        info.keymaps.contains(&keymap)
            && self.items.get(&item).copied().unwrap_or(info.level <= self.level && self.level != FlavorLevel::Off)
    }

    /// The items on under `keymap`, one bit each ([`FlavorItem::bit`]).
    pub fn mask(&self, keymap: KeyPreset) -> u64 {
        FlavorItem::ALL.iter().filter(|item| self.on(**item, keymap)).fold(0, |mask, item| mask | item.bit())
    }

    /// Whether `item` was switched by name rather than by the level.
    pub fn set_by_name(&self, item: FlavorItem) -> bool {
        self.items.contains_key(&item)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_turns_on_its_items_and_the_ones_below() {
        let at = |level| Flavor { level, items: BTreeMap::new() };
        let item = FlavorItem::DeleteKeepsRegister;
        assert!(!at(FlavorLevel::Off).on(item, KeyPreset::Helix));
        assert!(!at(FlavorLevel::Basic).on(item, KeyPreset::Helix));
        assert!(at(FlavorLevel::Basic).on(FlavorItem::CaseMenu, KeyPreset::Helix));
        assert!(at(FlavorLevel::Full).on(item, KeyPreset::Helix));
        assert!(!at(FlavorLevel::Full).on(item, KeyPreset::Vim), "helix only");
    }

    #[test]
    fn an_item_written_by_name_beats_the_level() {
        let item = FlavorItem::DeleteKeepsRegister;
        let on = Flavor { level: FlavorLevel::Off, items: BTreeMap::from([(item, true)]) };
        assert!(on.on(item, KeyPreset::Helix));
        let off = Flavor { level: FlavorLevel::Full, items: BTreeMap::from([(item, false)]) };
        assert!(!off.on(item, KeyPreset::Helix));
    }

    #[test]
    fn every_id_is_its_own_and_reads_back() {
        let ids = FlavorItem::ids();
        let mut seen = ids.clone();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), ids.len());
        assert!(FlavorItem::ALL.len() <= 64, "a mask holds them");
        for item in FlavorItem::ALL {
            assert_eq!(FlavorItem::parse(item.id()), Some(*item));
            assert_ne!(item.info().level, FlavorLevel::Off, "an item is on at some level");
        }
        for level in FlavorLevel::ALL {
            assert_eq!(FlavorLevel::parse(level.name()), Some(level));
        }
    }
}
