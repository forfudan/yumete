//! **The yumete flavour** (2026-11-01) — what yumete adds to or changes in
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
//! the base keymap, not an item (`ge`, 2026-11-01).

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
    /// The keys it changes, as `[keys.normal]` spells them: a line there for
    /// one of them wins over the item, and the report says so.
    pub keys: &'static [&'static str],
    /// A message tag: what it does, in a few words.
    pub label: &'static str,
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
    /// `d` and `c` delete without filling the register, and `A-d`/`A-c`
    /// are the pair that cut — in the text and in a table's grid. helix only:
    /// vim has `"_d` (2026-11-01, after looking at what vim users write).
    DeleteKeepsRegister => ItemInfo {
        id: "delete_keeps_register",
        level: FlavorLevel::Full,
        keymaps: &[KeyPreset::Helix],
        keys: &["d", "c", "A-d", "A-c"],
        label: "keys.flavor.item.delete-keeps-register",
    },
}

impl FlavorItem {
    pub fn id(self) -> &'static str {
        self.info().id
    }

    pub fn parse(id: &str) -> Option<FlavorItem> {
        Self::ALL.iter().copied().find(|item| item.id() == id.trim())
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
        let info = item.info();
        info.keymaps.contains(&keymap)
            && self.items.get(&item).copied().unwrap_or(info.level <= self.level && self.level != FlavorLevel::Off)
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
        for item in FlavorItem::ALL {
            assert_eq!(FlavorItem::parse(item.id()), Some(*item));
            assert_ne!(item.info().level, FlavorLevel::Off, "an item is on at some level");
        }
        for level in FlavorLevel::ALL {
            assert_eq!(FlavorLevel::parse(level.name()), Some(level));
        }
    }
}
