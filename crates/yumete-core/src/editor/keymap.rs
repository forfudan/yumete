//! **The key tree** (2026-11-05): one per mode, laid in `lay_aliases`.
//!
//! The built-in groups (`groups.rs`) are its nodes, and a command that opens
//! a group is an edge to that node wherever it is bound: `Space w`, `C-w` and,
//! under vim, `\ w` reach one node, and so does a key the reader moves a
//! group to (`";" = "@ "`, `";" = "space_mode"`). The vim preset's lines and
//! the reader's are laid on it. A line that walks into a group is a row of
//! that group, so the group has it however it is opened; the rest of a run
//! past the last group is held, as a run at the top level is, and a key
//! that does not continue it lets the held keys act first.
//!
//! Dispatch ([`Editor::on_node_key`]) and the which-key menu
//! ([`Editor::tree_menu`]) read the same tree, and so do the generated
//! pages: the ways into each group ([`ways_in`]) are found by walking its
//! edges from the top level, not by spelling them.

use super::groups::{opens, Context, Group, GROUPS, SPACE};
use super::keys::key_internal;
use super::*;
use crate::input::Key;
use std::collections::HashSet;
use yumete_cjk::actions::NormalCmd;

/// One mode's keys as a tree: the runs at the top level, and the reader's
/// keys laid over the groups.
#[derive(Clone, Default)]
pub(crate) struct KeyTree {
    /// The runs that start at the top level, internal spelling: the
    /// preset's lines, and the reader's that do not walk into a group.
    pub(super) runs: HashMap<String, Vec<String>>,
    /// The ones of `runs` the preset wrote, which no menu lists.
    pub(super) preset: HashSet<String>,
    /// The names of runs held at the top level (`label` on a key that
    /// opens no group).
    pub(super) labels: HashMap<String, String>,
    /// The reader's keys over the groups.
    pub(super) nodes: Vec<Node>,
    /// A group, by its place in [`GROUPS`], to its node.
    over: HashMap<usize, usize>,
}

/// **The reader's keys in one group.**
#[derive(Clone, Default)]
pub(super) struct Node {
    /// The group under it, by its place in [`GROUPS`].
    pub(super) group: Option<usize>,
    /// Its name (`label` in its table).
    pub(super) label: Option<String>,
    /// The keys that reached it when it was laid, internal spelling.
    pub(super) path: String,
    /// The reader's keys, one each, in the order laid.
    pub(super) rows: Vec<Mine>,
    /// Runs of keys from here that pass a key opening no group, held as
    /// the top level holds its runs.
    pub(super) runs: HashMap<String, Vec<String>>,
    /// The names of such runs' beginnings.
    pub(super) labels: HashMap<String, String>,
}

/// One of the reader's keys in a group, and the commands it runs.
#[derive(Clone)]
pub(super) struct Mine {
    pub(super) key: String,
    pub(super) run: Vec<String>,
}

impl Node {
    /// Whether the reader put anything here.
    pub(super) fn has_keys(&self) -> bool {
        !self.rows.is_empty() || !self.runs.is_empty()
    }

    fn row(&self, key: &str) -> Option<&Mine> {
        self.rows.iter().find(|row| row.key == key)
    }

    /// Whether a run from here is longer than `held` and begins with it.
    fn runs_on(&self, held: &str) -> bool {
        !held.is_empty() && self.runs.keys().any(|run| run.len() > held.len() && run.starts_with(held))
    }
}

/// A run of keys in the internal spelling, one press each (Alt and its key
/// are two `char`s).
pub(super) fn split_keys(run: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = run.chars();
    while let Some(c) = chars.next() {
        match c {
            yumete_cjk::actions::ALT => match chars.next() {
                Some(next) => out.push(format!("{c}{next}")),
                None => break,
            },
            c => out.push(c.to_string()),
        }
    }
    out
}

impl KeyTree {
    /// The node over `group`, made when it is first asked for.
    fn over(&mut self, group: usize, path: &str) -> usize {
        if let Some(&n) = self.over.get(&group) {
            return n;
        }
        self.nodes.push(Node { group: Some(group), path: path.to_string(), ..Node::default() });
        self.over.insert(group, self.nodes.len() - 1);
        self.nodes.len() - 1
    }

    /// The node over `group`, if the reader laid one.
    pub(super) fn node_of(&self, group: &Group) -> Option<usize> {
        let at = GROUPS.iter().position(|g| std::ptr::eq(*g, group))?;
        self.over.get(&at).copied()
    }

    /// Whether a run at the top level takes these keys (vim's `ZZ`): the
    /// keys then never reach a group row of the same spelling.
    pub(super) fn takes(&self, path: &str) -> bool {
        self.runs.contains_key(path)
    }

    /// The vim preset's tree, or helix's, with nothing of the reader's.
    pub(super) fn of_preset(preset: yumete_cjk::KeyPreset) -> KeyTree {
        let runs: HashMap<String, Vec<String>> = preset
            .table()
            .iter()
            .map(|(k, v)| (yumete_cjk::actions::written(k), vec![v.to_string()]))
            .collect();
        KeyTree { preset: runs.keys().cloned().collect(), runs, ..KeyTree::default() }
    }
}

impl Editor {
    /// **One mode's tree**: the preset's `runs` at the top level, then the
    /// reader's `lines` and group names (`labels`), all in the internal
    /// spelling. `extend` says which mode's keys the groups are read in.
    pub(super) fn lay_tree(
        &mut self,
        preset: &HashMap<String, Vec<String>>,
        lines: &[(String, Vec<String>)],
        labels: &HashMap<String, String>,
        extend: bool,
    ) -> KeyTree {
        let was = (self.extend, self.expanding_alias, self.replaying_held);
        // The groups are opened by keys typed in that mode.
        (self.extend, self.expanding_alias, self.replaying_held) = (extend, false, false);
        let mut tree = KeyTree { runs: preset.clone(), preset: preset.keys().cloned().collect(), ..KeyTree::default() };
        // One key at the top level is that key; the rest walk the tree,
        // shorter first, so a run walks through the keys laid before it.
        let mut longer: Vec<(usize, &String, &Vec<String>)> = Vec::new();
        for (run, commands) in lines {
            match split_keys(run).len() {
                0 => {}
                1 => {
                    tree.runs.insert(run.clone(), commands.clone());
                    tree.preset.remove(run);
                }
                n => longer.push((n, run, commands)),
            }
        }
        longer.sort();
        for (_, run, commands) in longer {
            self.lay_line(&mut tree, run, commands);
        }
        let mut names: Vec<(&String, &String)> = labels.iter().collect();
        names.sort();
        for (path, label) in names {
            self.lay_label(&mut tree, path, label);
        }
        (self.extend, self.expanding_alias, self.replaying_held) = was;
        tree
    }

    /// Walk one line of two keys or more into the tree.
    fn lay_line(&self, tree: &mut KeyTree, run: &str, commands: &[String]) {
        let keys = split_keys(run);
        // A key the top level holds for a longer run stays the run's: the
        // run is what the key waits for (vim's `Z` of `ZZ`).
        let first = &keys[0];
        if tree.runs.contains_key(run) || tree.runs.keys().any(|r| r.len() > first.len() && r.starts_with(first.as_str())) {
            tree.runs.insert(run.to_string(), commands.to_vec());
            tree.preset.remove(run);
            return;
        }
        let mut at: Option<usize> = None;
        let mut walked = String::new();
        for (i, key) in keys.iter().enumerate() {
            if i + 1 == keys.len() {
                if let Some(n) = at {
                    let node = &mut tree.nodes[n];
                    match node.rows.iter_mut().find(|row| row.key == *key) {
                        Some(row) => row.run = commands.to_vec(),
                        None => node.rows.push(Mine { key: key.clone(), run: commands.to_vec() }),
                    }
                }
                return;
            }
            walked.push_str(key);
            match self.step(tree, at, key, &walked) {
                Some(n) => at = Some(n),
                None => {
                    match at {
                        None => {
                            tree.runs.insert(run.to_string(), commands.to_vec());
                            tree.preset.remove(run);
                        }
                        Some(n) => {
                            tree.nodes[n].runs.insert(keys[i..].concat(), commands.to_vec());
                        }
                    }
                    return;
                }
            }
        }
    }

    /// Name the group `path` reaches, or the run it begins.
    fn lay_label(&self, tree: &mut KeyTree, path: &str, label: &str) {
        let keys = split_keys(path);
        let mut at: Option<usize> = None;
        let mut walked = String::new();
        for (i, key) in keys.iter().enumerate() {
            walked.push_str(key);
            match self.step(tree, at, key, &walked) {
                Some(n) => at = Some(n),
                None => {
                    match at {
                        None => {
                            tree.labels.insert(path.to_string(), label.to_string());
                        }
                        Some(n) => {
                            tree.nodes[n].labels.insert(keys[i..].concat(), label.to_string());
                        }
                    }
                    return;
                }
            }
        }
        if let Some(n) = at {
            tree.nodes[n].label = Some(label.to_string());
        }
    }

    /// **The node `key` opens from `at`** (`None`: the top level), when it
    /// opens one. At the top level a run that begins with Space is the Space
    /// group's, wherever that group is opened (`[keys.normal.space]`); the
    /// Space key itself is a line of one key.
    fn step(&self, tree: &mut KeyTree, at: Option<usize>, key: &str, walked: &str) -> Option<usize> {
        let group = match at {
            None if key == " " => Some(&SPACE),
            None => match tree.runs.get(key) {
                Some(commands) => self.binding_opens(commands),
                None => self.top_opens(key),
            },
            Some(n) => {
                let node = &tree.nodes[n];
                match node.row(key) {
                    Some(row) => self.binding_opens(&row.run),
                    None => {
                        let group = GROUPS[node.group?];
                        let bind = group.bind_for(super::keys::pressed_key(key)?, &self.context_in(group))?;
                        opens(bind.run)
                    }
                }
            }
        }?;
        let at = GROUPS.iter().position(|g| std::ptr::eq(*g, group))?;
        Some(tree.over(at, walked))
    }

    /// The group a key opens at the top level by the keymap's own tables.
    fn top_opens(&self, key: &str) -> Option<&'static Group> {
        let key = super::keys::pressed_key(key)?;
        let cx = super::normal::Cx { key, count: 1, operator_count: None, continuing_zong: false };
        opens(NormalCmd::from_name(self.normal_command_for(&cx)?)?)
    }

    /// The group a binding opens: an action that opens one (`space_mode`),
    /// or one key of the editor's own that does (`"@ "`).
    fn binding_opens(&self, commands: &[String]) -> Option<&'static Group> {
        let [one] = commands else { return None };
        match yumete_cjk::actions::bound(one)? {
            yumete_cjk::actions::Bound::Named(action) => opens(NormalCmd::from_name(action.name)?),
            yumete_cjk::actions::Bound::Keys(keys) => {
                let keys = yumete_cjk::actions::written(keys);
                (split_keys(&keys).len() == 1).then(|| self.top_opens(&keys)).flatten()
            }
            yumete_cjk::actions::Bound::Command(_) => None,
        }
    }

    /// **The tree in force**: select mode's while a selection is being
    /// extended, Normal mode's otherwise, each with `[keys.table]` over it
    /// in a table's grid.
    pub(super) fn tree(&self) -> &KeyTree {
        // Asked only when the reader wrote some, `table_here` being a lookup.
        let grid = !self.user_table.is_empty() && self.table_here();
        match (self.extend, grid) {
            (true, false) => &self.select_tree,
            (false, false) => &self.normal_tree,
            (true, true) => &self.table_select_tree,
            (false, true) => &self.table_tree,
        }
    }

    /// Whether the reader's keys are read now: not while a run of keys on
    /// the right of a binding plays, which always means the editor's own.
    fn reading_mine(&self) -> bool {
        !self.expanding_alias || self.replaying_held
    }

    /// The node of the group this is waiting in, when the reader laid one.
    pub(super) fn node_here(&self) -> Option<usize> {
        if !self.reading_mine() {
            return None;
        }
        self.tree().node_of(self.pending.group()?)
    }

    /// What the reader is holding in this group (`b` of `Space bd`).
    fn held_here(&self) -> &str {
        match &self.node_held {
            Some((at, held)) if *at == self.pending => held,
            _ => "",
        }
    }

    /// Whether a key in this group is the reader's: it is a key of theirs
    /// here, begins a run of theirs, or goes on with one held. Its commands
    /// then reach the cursors themselves.
    pub(super) fn node_takes(&self, key: Key) -> bool {
        let Some(n) = self.node_here() else { return false };
        let node = &self.tree().nodes[n];
        let typed = key_internal(key);
        !self.held_here().is_empty() || node.row(&typed).is_some() || node.runs_on(&typed)
    }

    /// **A key arriving in a group**: the reader's keys there first (a key
    /// of theirs, a run of theirs held), then the group's own rows.
    pub(super) fn on_node_key(&mut self, key: Key, typed_before: &str, continuing_zong: bool) {
        let group = self.pending.group();
        let Some(n) = self.node_here() else {
            if let Some(group) = group {
                self.on_group_key(group, key, typed_before, continuing_zong);
            }
            return;
        };
        let node = self.tree().nodes[n].clone();
        let typed = key_internal(key);
        let held = self.held_here().to_string();
        self.node_held = None;
        // A run played from a binding presses the keys it names.
        let may_hold = !self.expanding_alias;
        if !held.is_empty() {
            if key == Key::Esc {
                self.pending = Pending::None;
                self.alias_count = None;
                self.operator_count = None;
                return;
            }
            let longer = format!("{held}{typed}");
            // A digit inside a held run is a count (`Space b 3d`), unless a
            // run of the reader's goes on with it.
            if let Some(c) = typed.chars().next().filter(|c| typed.len() == 1 && c.is_ascii_digit()) {
                if !node.runs.keys().any(|r| r.starts_with(longer.as_str())) && (c != '0' || self.alias_count.is_some()) {
                    let n = self.alias_count.unwrap_or(0);
                    self.alias_count =
                        Some(n.saturating_mul(10).saturating_add(c.to_digit(10).unwrap_or(0) as usize).min(1_000_000));
                    self.node_held = Some((self.pending, held));
                    return;
                }
            }
            if !typed.is_empty() && node.runs_on(&longer) {
                self.node_held = Some((self.pending, longer));
                return;
            }
            if let Some(commands) = node.runs.get(&longer).filter(|_| !typed.is_empty()).cloned() {
                self.run_mine(&commands);
                return;
            }
            self.let_go_in(&node, &held);
            let waiting = self.pending;
            self.fan_out(waiting, key, false, &|e| e.on_normal_key(key));
            return;
        }
        if may_hold && node.runs_on(&typed) {
            self.node_held = Some((self.pending, typed));
            return;
        }
        if let Some(row) = node.row(&typed) {
            self.run_mine(&row.run.clone());
            return;
        }
        if let Some(group) = group {
            self.on_group_key(group, key, typed_before, continuing_zong);
        }
    }

    /// After a command at the top level: the count typed before it, kept
    /// for the reader's keys in the group it opened.
    pub(super) fn note_the_group_count(&mut self, before: Option<usize>) {
        self.group_count = self.pending.group().and(before);
    }

    /// Run one of the reader's keys in a group: the group closes, and the
    /// count typed before the group, and any typed inside a run held there,
    /// goes to the commands.
    fn run_mine(&mut self, commands: &[String]) {
        self.pending = Pending::None;
        self.count = self.group_count.take().or(self.operator_count.take());
        self.operator_count = None;
        self.settle_alias_count();
        self.run_binding(commands);
    }

    /// **What was held in a group acts first**: the longest start of it the
    /// reader bound there runs, and the rest is pressed as the editor's own
    /// keys; with none bound, all of it is.
    fn let_go_in(&mut self, node: &Node, held: &str) {
        let keys = split_keys(held);
        for cut in (1..=keys.len()).rev() {
            let start = keys[..cut].concat();
            let bound = match cut {
                1 => node.row(&start).map(|row| row.run.clone()),
                _ => node.runs.get(&start).cloned(),
            };
            if let Some(commands) = bound {
                self.run_mine(&commands);
                let rest = keys[cut..].concat();
                if !rest.is_empty() {
                    self.play_held(&rest);
                }
                return;
            }
        }
        if let Some(n) = self.alias_count.take() {
            self.operator_count = Some(n.saturating_mul(self.operator_count.unwrap_or(1)));
        }
        self.play_held(held);
    }

    /// **What a run held at the top level does when the next key goes on
    /// with none**: the longest start of it bound runs (the reader's own
    /// binding for the prefix: `"<space>b"` beside `"<space>bd"`), and the
    /// rest is pressed as the editor's own keys. `false` when no start of it
    /// is bound.
    pub(super) fn let_go_of_the_root(&mut self, held: &str) -> bool {
        let keys = split_keys(held);
        for cut in (1..=keys.len()).rev() {
            let start = keys[..cut].concat();
            let Some(commands) = self.tree().runs.get(&start).cloned() else { continue };
            self.settle_alias_count();
            self.run_binding(&commands);
            let rest = keys[cut..].concat();
            if !rest.is_empty() {
                self.play_held(&rest);
            }
            return true;
        }
        false
    }

    /// **The which-key menu of where the keys are in the tree**: a run held
    /// at the top level, a group, or a run held in a group.
    pub(super) fn tree_menu(&self) -> Option<(String, KeyRows)> {
        if self.pending == Pending::None {
            let held = self.alias_held.as_str();
            if held.is_empty() || self.mode != Mode::Normal {
                return None;
            }
            return self.root_held_menu(held);
        }
        let group = self.pending.group()?;
        let Some(n) = self.node_here() else { return Some(self.group_menu(group)) };
        let node = &self.tree().nodes[n];
        let held = self.held_here();
        if !held.is_empty() {
            let mine: Vec<(String, Vec<String>)> = node.runs.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let rows = held_rows(held, &mine, None, &self.group_context(), |keys| node.labels.get(keys).cloned());
            if rows.is_empty() {
                return None;
            }
            let title = match node.labels.get(held) {
                Some(label) => label.clone(),
                None => yumete_cjk::actions::spell(&format!("{}{held}", node.path)),
            };
            return Some((title, rows));
        }
        if !node.has_keys() {
            return Some(self.group_menu(group));
        }
        let mine: Vec<(String, Vec<String>)> = node
            .rows
            .iter()
            .map(|row| (row.key.clone(), row.run.clone()))
            .chain(node.runs.iter().map(|(k, v)| (k.clone(), v.clone())))
            .collect();
        let rows = held_rows("", &mine, Some(group), &self.context_in(group), |keys| node.labels.get(keys).cloned());
        let title = match &node.label {
            Some(label) => label.clone(),
            None => self.group_menu(group).0,
        };
        Some((title, rows))
    }

    /// The menu of a run held at the top level: the reader's runs that go on
    /// from it, and, when letting go of it opens a group (vim's `Z` of
    /// `ZZ`), that group's keys besides.
    fn root_held_menu(&self, held: &str) -> Option<(String, KeyRows)> {
        let tree = self.tree();
        let mine: Vec<(String, Vec<String>)> =
            tree.runs.iter().filter(|(k, _)| !tree.preset.contains(*k)).map(|(k, v)| (k.clone(), v.clone())).collect();
        let group = (split_keys(held).len() == 1).then(|| self.top_opens(held)).flatten();
        let rows = held_rows(held, &mine, group, &self.group_context(), |keys| tree.labels.get(keys).cloned());
        if rows.is_empty() {
            return None;
        }
        let title = match (tree.labels.get(held), group) {
            (Some(label), _) => label.clone(),
            (None, Some(group)) => crate::messages::say(group.title, &[]),
            (None, None) => yumete_cjk::actions::spell(held),
        };
        Some((title, rows))
    }

    /// **The which-key menu of a group as it ships.**
    pub(super) fn group_menu(&self, group: &'static Group) -> (String, KeyRows) {
        let rows = group
            .menu(&self.context_in(group))
            .into_iter()
            .map(|(keys, says)| (keys.into(), crate::messages::say(says, &[])))
            .collect();
        // The Space menu is titled by its key, and under vim that may be `\`.
        let title = match (group.name, self.space_sign()) {
            ("space", "\\") => "\\".to_string(),
            _ => crate::messages::say(group.title, &[]),
        };
        (title, rows)
    }

    /// **The key that opens the Space menu**, as a sign writes it: the
    /// first key at the top level that opens it, `␣` when that is Space.
    pub(super) fn space_sign(&self) -> &'static str {
        let at = self.group_context();
        match home_paths(&Context { vim: at.vim, flavor: at.flavor, ..Context::default() })
            [GROUPS.iter().position(|g| g.name == SPACE.name).unwrap_or(0)]
        .as_ref()
        .and_then(|path| path.first())
        .map(String::as_str)
        {
            Some("\\") => "\\",
            _ => "␣",
        }
    }
}

/// **A menu's rows from the reader's runs**: the next key of each run that
/// goes on from `held`, as the command it runs or the name of what it
/// opens, after the rows of `group` the reader did not take. Today's order:
/// the group's rows as it ships, then the reader's by key.
fn held_rows(
    held: &str,
    mine: &[(String, Vec<String>)],
    group: Option<&'static Group>,
    at: &Context,
    label_of: impl Fn(&str) -> Option<String>,
) -> KeyRows {
    let mut next_keys: Vec<(String, Option<String>)> = Vec::new();
    for (left, commands) in mine {
        let Some(rest) = left.strip_prefix(held).filter(|r| !r.is_empty()) else { continue };
        let Some(next) = split_keys(rest).into_iter().next() else { continue };
        let exact = rest.len() == next.len();
        let said = match exact {
            true => super::groups::describe(commands),
            false => match label_of(&format!("{held}{next}")) {
                Some(label) => Some(label),
                // A group's key with the reader's keys under it keeps its
                // own line.
                None if group.is_some_and(|g| g.binds().any(|b| key_internal(b.key) == next)) => continue,
                None => Some("…".to_string()),
            },
        };
        match next_keys.iter_mut().find(|(n, _)| *n == next) {
            Some(row) if exact => row.1 = said,
            Some(_) => {}
            None => next_keys.push((next, said)),
        }
    }
    next_keys.sort_by(|a, b| a.0.cmp(&b.0));
    let mut rows: KeyRows = Vec::new();
    if let Some(group) = group {
        let taken = |key: Key| next_keys.iter().any(|(n, _)| *n == key_internal(key));
        for (keys, says) in group.menu_without(at, taken) {
            rows.push((keys.into(), crate::messages::say(says, &[])));
        }
    }
    for (next, said) in next_keys {
        let Some(said) = said else { continue };
        rows.push((super::groups::key_label(super::groups::key_of(&next)).into(), said));
    }
    rows
}

/// The order keys are listed in: letters as `a A b B`, then digits and the
/// other characters, then Ctrl, Alt and the named keys.
pub(super) fn rank(key: &Key) -> (u8, u32, u8) {
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

/// **Every way into each group from `roots`** (keys at the top level and
/// the command each runs), one press each in the internal spelling, by the
/// group's place in [`GROUPS`]: a key at the top level that opens it, or a
/// key in a group reached already whose row opens it, in the order found.
pub(super) fn ways_in(roots: &[(Vec<String>, NormalCmd)], at: &Context) -> Vec<Vec<Vec<String>>> {
    let place = |cmd: NormalCmd| opens(cmd).and_then(|g| GROUPS.iter().position(|one| std::ptr::eq(*one, g)));
    let mut opened: Vec<Vec<Vec<String>>> = vec![Vec::new(); GROUPS.len()];
    for (keys, cmd) in roots {
        if let Some(to) = place(*cmd) {
            if !opened[to].contains(keys) {
                opened[to].push(keys.clone());
            }
        }
    }
    // A group opened from a group: as many rounds as there are levels.
    for _ in 0..GROUPS.len() {
        let mut grew = false;
        for (from, group) in GROUPS.iter().enumerate() {
            for bind in group.binds().filter(|b| at.reaches(b)) {
                let Some(to) = place(bind.run) else { continue };
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
    opened
}

/// **The keys that open each group under a keymap and flavour** (`at`'s
/// `vim` and `flavor`), the first way in found from the keys at the top
/// level in [`rank`]'s order; `None` for a group no key reaches (vim's `m`
/// sets a mark). Kept per keymap and flavour, as the tables never change.
pub(super) fn home_paths(at: &Context) -> std::sync::Arc<Vec<Option<Vec<String>>>> {
    use std::sync::{Arc, Mutex, OnceLock};
    type Homes = HashMap<(bool, u64), Arc<Vec<Option<Vec<String>>>>>;
    static KEPT: OnceLock<Mutex<Homes>> = OnceLock::new();
    let kept = KEPT.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(found) = kept.lock().ok().and_then(|k| k.get(&(at.vim, at.flavor)).cloned()) {
        return found;
    }
    use super::normal::{top_command, FLAVOR_NORMAL, HELIX_NORMAL, VIM_NORMAL};
    let at = Context { vim: at.vim, flavor: at.flavor, ..Context::default() };
    let mut keys: Vec<Key> = Vec::new();
    for key in FLAVOR_NORMAL
        .iter()
        .map(|(k, _, _, _)| *k)
        .chain(VIM_NORMAL.iter().map(|(k, _, _)| *k))
        .chain(HELIX_NORMAL.iter().map(|(k, _)| *k))
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys.sort_by_key(rank);
    let roots: Vec<(Vec<String>, NormalCmd)> = keys
        .into_iter()
        .filter_map(|key| top_command(key, &at, &|_| true).map(|cmd| (vec![key_internal(key)], cmd)))
        .collect();
    let homes: Arc<Vec<Option<Vec<String>>>> =
        Arc::new(ways_in(&roots, &at).into_iter().map(|ways| ways.into_iter().next()).collect());
    if let Ok(mut k) = kept.lock() {
        k.insert((at.vim, at.flavor), homes.clone());
    }
    homes
}
