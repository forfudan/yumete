//! **The keymap pages, read off the keymap** (`ye --docgen`, 2026-11-02).
//!
//! The site has a page for helix's keymap, one for vim's and one for the
//! flavour. Each page's tables come from an editor laid with that keymap and
//! that flavour, asked what each key runs, so a key on a page is a key the
//! editor has, and a key the flavour adds is on the flavour's page only.

use super::groups::{Context, Group, GROUPS};
use super::keys::key_internal;
use super::normal::{Cx, FLAVOR_NORMAL, HELIX_NORMAL, SELECT_NORMAL, VIM_NORMAL};
use yumete_cjk::actions::NormalCmd;
use super::{Bounds, Editor, FindKind};
use crate::input::Key;

/// One key on a keymap page: what is pressed, one press each in the
/// internal spelling, and what it runs: a command's name, `@` and keys, or
/// `:` and a command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocKey {
    pub keys: Vec<String>,
    pub run: String,
}

/// One menu on a keymap page: its title message, every way in (one press
/// each, internal spelling), and its rows as (keys, message), as the
/// which-key panel draws them.
#[derive(Debug, Clone)]
pub struct DocKeyMenu {
    pub title: &'static str,
    pub opened_by: Vec<Vec<String>>,
    pub rows: Vec<(String, &'static str)>,
}

/// **A keymap under a flavour**: the keys of Normal mode, the keys that do
/// something else while a selection is extended, Insert mode's keys, and
/// the menus the keys open.
#[derive(Debug, Clone)]
pub struct DocKeymap {
    pub normal: Vec<DocKey>,
    pub select: Vec<DocKey>,
    pub insert: Vec<DocKey>,
    pub menus: Vec<DocKeyMenu>,
}

/// The command that opens a group, which is how the pages find its keys.
fn opener(group: &Group) -> Option<NormalCmd> {
    Some(match group.name {
        "goto" => NormalCmd::GotoMode,
        "space" => NormalCmd::SpaceMode,
        "right_bracket" => NormalCmd::RightBracketMode,
        "left_bracket" => NormalCmd::LeftBracketMode,
        "view" => NormalCmd::ViewMode,
        "match" => NormalCmd::MatchMode,
        "window" => NormalCmd::WindowMode,
        "window_new" => NormalCmd::WindowNewMode,
        "case" => NormalCmd::CaseMode,
        "conflict" => NormalCmd::ConflictMode,
        "open" => NormalCmd::OpenRecentMode,
        "yume" => NormalCmd::YumeMode,
        "table" => NormalCmd::TableMode,
        "table_convert" => NormalCmd::TableConvertMode,
        // `Z` is `z` that stays open, and the three value lists under
        // `Space z` are one line each on that menu.
        _ => return None,
    })
}

/// The order a page lists keys in: letters as `a A b B`, then digits and
/// the other characters, then Ctrl, Alt and the named keys.
fn rank(key: &Key) -> (u8, u32, u8) {
    match *key {
        Key::Char(c) if c.is_ascii_alphabetic() => (0, c.to_ascii_lowercase() as u32, c.is_ascii_uppercase() as u8),
        Key::Char(c) if c.is_ascii_digit() => (1, c as u32, 0),
        Key::Char(c) => (2, c as u32, 0),
        Key::Ctrl(c) => (3, c.to_ascii_lowercase() as u32, c.is_ascii_uppercase() as u8),
        Key::Alt(c) => (4, c.to_ascii_lowercase() as u32, c.is_ascii_uppercase() as u8),
        named => {
            let order = [
                Key::Esc,
                Key::Enter,
                Key::Tab,
                Key::BackTab,
                Key::Backspace,
                Key::Delete,
                Key::Left,
                Key::Right,
                Key::Up,
                Key::Down,
                Key::Home,
                Key::End,
                Key::PageUp,
                Key::PageDown,
            ];
            (5, order.iter().position(|k| *k == named).unwrap_or(order.len()) as u32, 0)
        }
    }
}

impl Editor {
    /// **The keys of `preset` under `flavor`**, for the generated keymap
    /// and flavour pages.
    pub fn keymap_for_docs(preset: yumete_cjk::KeyPreset, flavor: yumete_cjk::Flavor) -> DocKeymap {
        let mut ed = Editor::new();
        ed.set_key_preset(preset);
        ed.set_flavor(flavor);
        // vim's `;` and `,` repeat an `f`; with none to repeat they do
        // nothing, and a page lists what they are for.
        ed.last_find = Some((FindKind::Forward, 'a'));
        let normal = ed.top_keys_for_docs();
        ed.extend = true;
        let selecting = ed.top_keys_for_docs();
        ed.extend = false;
        let select = selecting.into_iter().filter(|key| !normal.contains(key)).collect();
        let mut insert: Vec<DocKey> = Vec::new();
        for (key, _, _) in super::insert::INSERT_KEYS {
            let keys = vec![key_internal(*key)];
            if insert.iter().any(|k| k.keys == keys) {
                continue;
            }
            if let Some(name) = ed.insert_command_for(*key) {
                insert.push(DocKey { keys, run: name.to_string() });
            }
        }
        let menus = ed.menus_for_docs(&normal);
        DocKeymap { normal, select, insert, menus }
    }

    /// Every key at the top level and what it runs here, in [`rank`]'s
    /// order: the keymap's bindings first, then the command tables.
    fn top_keys_for_docs(&self) -> Vec<DocKey> {
        let mut keys: Vec<Key> = Vec::new();
        let tables = HELIX_NORMAL
            .iter()
            .map(|(k, _)| *k)
            .chain(VIM_NORMAL.iter().map(|(k, _, _)| *k))
            .chain(SELECT_NORMAL.iter().map(|(k, _)| *k))
            .chain(FLAVOR_NORMAL.iter().map(|(k, _, _, _)| *k));
        let aliases = self.aliases();
        // The keymap's own one-key bindings (vim's `$`, `+`, `-`).
        let bound = aliases.keys().filter_map(|typed| super::keys::pressed_key(typed));
        for key in tables.chain(bound) {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys.sort_by_key(rank);
        let mut out: Vec<DocKey> = Vec::new();
        for key in keys {
            // `C-^` has no internal spelling of its own; the page writes it out.
            let typed = match (key_internal(key), key) {
                (typed, Key::Ctrl(c)) if typed.is_empty() => format!("C-{c}"),
                (typed, _) if typed.is_empty() => continue,
                (typed, _) => typed,
            };
            let run = match aliases.get(&typed) {
                Some(commands) => commands.join(", "),
                None => {
                    let cx = Cx { key, count: 1, operator_count: None, continuing_zong: false };
                    match self.normal_command_for(&cx) {
                        Some(name) => name.to_string(),
                        None => continue,
                    }
                }
            };
            if run != NormalCmd::NoOp.name() {
                out.push(DocKey { keys: vec![typed], run });
            }
        }
        // A binding of two keys or more (vim's `ZZ`), after the key it starts with.
        let mut longer: Vec<(&String, &Vec<String>)> =
            aliases.iter().filter(|(typed, _)| super::keys::pressed_key(typed).is_none()).collect();
        longer.sort();
        for (typed, commands) in longer {
            let presses: Vec<String> = typed.chars().map(|c| c.to_string()).collect();
            let at = out.iter().rposition(|k| k.keys[0] == presses[0]).map_or(out.len(), |n| n + 1);
            out.insert(at, DocKey { keys: presses, run: commands.join(", ") });
        }
        out
    }

    /// Every menu the keys reach, each with every way in: a key at the top
    /// level that opens it, or a key in a menu reached already.
    fn menus_for_docs(&self, normal: &[DocKey]) -> Vec<DocKeyMenu> {
        let at = Context { code: true, ..self.group_context() };
        let mut opened: Vec<Vec<Vec<String>>> = GROUPS
            .iter()
            .map(|group| {
                let Some(cmd) = opener(group) else { return Vec::new() };
                normal.iter().filter(|k| k.run == cmd.name()).map(|k| k.keys.clone()).collect()
            })
            .collect();
        // A menu opened from a menu: as many rounds as there are levels.
        for _ in 0..GROUPS.len() {
            let mut grew = false;
            for (from, group) in GROUPS.iter().enumerate() {
                for bind in group.binds().filter(|b| at.reaches(b)) {
                    let Some(to) = GROUPS.iter().position(|g| opener(g) == Some(bind.run)) else { continue };
                    for path in opened[from].clone() {
                        let mut path = path;
                        path.push(key_internal(bind.key));
                        if !opened[to].contains(&path) {
                            opened[to].push(path);
                            grew = true;
                        }
                    }
                }
            }
            if !grew {
                break;
            }
        }
        let mut out = Vec::new();
        for (group, opened_by) in GROUPS.iter().zip(opened) {
            if opened_by.is_empty() {
                continue;
            }
            let rows = match group.name {
                // Each kind of table has its own keys: every one, the way in first.
                "table" => {
                    let mut all: Vec<(String, &'static str)> = Vec::new();
                    for (inside, pane) in [(Some(Bounds::Md), true), (Some(Bounds::WholeFile), false), (Some(Bounds::Block), false)] {
                        for (keys, what) in group.menu(&Context { pane, table: inside, ..at }) {
                            let taken = all.iter().any(|(k, _)| k.split(' ').any(|one| keys.split(' ').any(|two| one == two)));
                            if !taken {
                                all.push((keys, what));
                            }
                        }
                    }
                    all
                }
                _ => group.menu(&at),
            };
            out.push(DocKeyMenu { title: group.title, opened_by, rows });
        }
        out
    }
}
