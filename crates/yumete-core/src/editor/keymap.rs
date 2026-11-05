//! **The key tree** (2026-11-05): one per mode, laid in `lay_aliases`.
//!
//! The built-in groups (`groups.rs`) are its nodes, and a command that opens
//! a group is an edge to that node wherever it is bound: `Space w`, `C-w` and,
//! under vim, `\ w` reach one node, and so does a key the reader moves a
//! group to (`";" = "@ "`, `";" = "space_mode"`). The vim preset's lines and
//! the reader's are laid on it:
//!
//! - **A table is a group** (`[keys.normal.space.b]`). Over a built-in group
//!   it adds to it; anywhere else it takes its key, as helix's tables do, and
//!   a key the group does not have closes it, saying nothing.
//! - **A run written flat** (`"<space>bd"`, `"gggg"`, Insert's `jk`) walks
//!   the same path. Where it walks into a group it is a row of that group;
//!   past the last group it is held, and a key that does not go on with it
//!   lets the held keys act first, the reader's own binding for them
//!   included.
//!
//! Dispatch ([`Editor::on_node_key`]) and the which-key menu
//! ([`Editor::tree_menu`]) read the same tree, and so do the generated
//! pages: the ways into each group ([`ways_in`]) are found by walking its
//! edges from the top level, not by spelling them.

use super::groups::{describe, key_label, key_of, keys_label, opens, Context, Group, GROUPS, SPACE};
use super::keys::key_internal;
use super::*;
use crate::input::Key;
use std::collections::HashSet;
use yumete_cjk::actions::NormalCmd;

/// One mode's keys as a tree: the runs at the top level, and the reader's
/// keys laid over the groups and in groups of their own.
#[derive(Clone, Default)]
pub(crate) struct KeyTree {
    /// The runs that start at the top level, internal spelling: the
    /// preset's lines, and the reader's that do not walk into a group.
    pub(super) runs: HashMap<String, Vec<String>>,
    /// The ones of `runs` the preset wrote, which no menu lists.
    pub(super) preset: HashSet<String>,
    /// Where each of the reader's `runs` was written.
    order: HashMap<String, usize>,
    /// The names of runs held at the top level (`label` on a key that
    /// opens no group).
    pub(super) labels: HashMap<String, String>,
    /// Keys at the top level a table of the reader's took.
    pub(super) root: HashMap<String, usize>,
    pub(super) nodes: Vec<Node>,
    /// A built-in group, by its place in [`GROUPS`], to its node.
    over: HashMap<usize, usize>,
    /// What laying it found worth a line at start-up: a table that took a
    /// key from a command.
    pub(super) notes: Vec<String>,
}

/// **A group in the tree**: a built-in one with the reader's keys over it,
/// or one of the reader's own.
#[derive(Clone, Default)]
pub(super) struct Node {
    /// The built-in group under it, by its place in [`GROUPS`].
    group: Option<usize>,
    /// Its name (`label` in its table).
    label: Option<String>,
    /// The keys that reached it when it was laid, internal spelling.
    path: String,
    /// Open after each key until `Esc` (`sticky = true`).
    sticky: bool,
    /// `false`: the built-in group's own keys are not read (`inherit = false`).
    inherit: bool,
    /// The layer of the table that made it (see [`Lines`]).
    layer: usize,
    /// The reader's keys here, one each.
    rows: Vec<Mine>,
    /// Runs of keys from here that pass a key opening no group, held as
    /// the top level holds its runs, and where each was written.
    runs: HashMap<String, Vec<String>>,
    run_order: HashMap<String, usize>,
    /// The names of such runs' beginnings.
    labels: HashMap<String, String>,
}

/// One of the reader's keys in a group.
#[derive(Clone)]
struct Mine {
    key: String,
    to: To,
    /// Its own menu text (`{ run = …, label = … }`).
    says: Option<String>,
    /// Where it was written, for the menu's order.
    order: usize,
    layer: usize,
}

/// What a key of the reader's does: run commands, or open a group of theirs.
#[derive(Clone)]
enum To {
    Run(Vec<String>),
    Node(usize),
}

impl Node {
    fn row(&self, key: &str) -> Option<&Mine> {
        self.rows.iter().find(|row| row.key == key)
    }

    /// Whether a run from here is longer than `held` and begins with it.
    fn runs_on(&self, held: &str) -> bool {
        !held.is_empty() && self.runs.keys().any(|run| run.len() > held.len() && run.starts_with(held))
    }
}

/// **What one mode's tree is laid from**: the reader's lines and tables,
/// each with its layer (the shared tables first, then the keymap's own,
/// then select mode's or a table grid's, each over the ones before it) and
/// where in its file it was written.
#[derive(Default)]
pub(super) struct Lines {
    lines: Vec<Line>,
    groups: Vec<GroupAt>,
    says: HashMap<String, String>,
    labels: HashMap<String, String>,
    /// The layers added so far.
    layers: usize,
}

struct Line {
    run: String,
    commands: Vec<String>,
    layer: usize,
    order: usize,
}

struct GroupAt {
    path: String,
    sticky: bool,
    inherit: bool,
    layer: usize,
    order: usize,
}

/// A line no file placed (a binding made in code): after the written ones,
/// in the order of its keys.
const UNPLACED: usize = 1 << 20;

impl Lines {
    /// One more layer: its lines and how its tables were written, as the
    /// config spells them.
    pub(super) fn add(&mut self, lines: &HashMap<String, Vec<String>>, shape: &yumete_cjk::KeyShape) {
        let layer = self.layers;
        self.layers += 1;
        let at = |run: &str| (layer << 21) + shape.order.iter().position(|r| r == run).unwrap_or(UNPLACED);
        let written = yumete_cjk::actions::written;
        for (run, commands) in lines {
            self.lines.push(Line { run: written(run), commands: commands.clone(), layer, order: at(run) });
        }
        for group in &shape.groups {
            self.groups.push(GroupAt {
                path: written(&group.path),
                sticky: group.sticky,
                inherit: group.inherit,
                layer,
                order: at(&group.path),
            });
        }
        for (run, text) in &shape.says {
            self.says.insert(written(run), text.clone());
        }
    }

    /// The groups' names, internal spelling, the later layer's over the
    /// earlier.
    pub(super) fn name(&mut self, labels: &HashMap<String, String>) {
        self.labels.extend(labels.iter().map(|(k, v)| (k.clone(), v.clone())));
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
        self.nodes.push(Node { group: Some(group), path: path.to_string(), inherit: true, ..Node::default() });
        self.over.insert(group, self.nodes.len() - 1);
        self.nodes.len() - 1
    }

    /// The node over `group`, if the reader laid one.
    pub(super) fn node_of(&self, group: &Group) -> Option<usize> {
        self.over.get(&place_of(group)?).copied()
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

/// A group's place in [`GROUPS`].
fn place_of(group: &Group) -> Option<usize> {
    GROUPS.iter().position(|g| std::ptr::eq(*g, group))
}

/// How a menu row's line is held: a message of the group's own, which
/// merges with a neighbour saying the same, or text.
enum Said {
    Msg(&'static str),
    Text(String),
}

impl Editor {
    /// **One mode's tree**: the preset's `runs` at the top level, then the
    /// reader's tables and lines. `extend` says which mode's keys the groups
    /// are read in.
    pub(super) fn lay_tree(&mut self, preset: &HashMap<String, Vec<String>>, lines: &Lines, extend: bool) -> KeyTree {
        let was = (self.extend, self.expanding_alias, self.replaying_held);
        // The groups are opened by keys typed in that mode.
        (self.extend, self.expanding_alias, self.replaying_held) = (extend, false, false);
        let mut tree = KeyTree { runs: preset.clone(), preset: preset.keys().cloned().collect(), ..KeyTree::default() };
        // Tables first, shorter first: a run then walks into the groups.
        let mut groups: Vec<&GroupAt> = lines.groups.iter().collect();
        groups.sort_by(|a, b| (split_keys(&a.path).len(), a.order, &a.path).cmp(&(split_keys(&b.path).len(), b.order, &b.path)));
        for group in groups {
            self.lay_group(&mut tree, group);
        }
        // A later layer's line for the same run wins.
        let mut kept: HashMap<&str, &Line> = HashMap::new();
        for line in &lines.lines {
            match kept.get(line.run.as_str()) {
                Some(was) if was.layer > line.layer => {}
                _ => {
                    kept.insert(&line.run, line);
                }
            }
        }
        let mut sorted: Vec<&Line> = kept.into_values().filter(|l| !split_keys(&l.run).is_empty()).collect();
        sorted.sort_by(|a, b| (split_keys(&a.run).len(), a.order, &a.run).cmp(&(split_keys(&b.run).len(), b.order, &b.run)));
        for line in sorted {
            self.lay_line(&mut tree, line, lines.says.get(&line.run));
        }
        let mut names: Vec<(&String, &String)> = lines.labels.iter().collect();
        names.sort();
        for (path, label) in names {
            self.lay_label(&mut tree, path, label);
        }
        (self.extend, self.expanding_alias, self.replaying_held) = was;
        tree
    }

    /// **A table**: merged into the group its keys reach, or a group of the
    /// reader's that takes the key.
    fn lay_group(&self, tree: &mut KeyTree, group: &GroupAt) {
        let keys = split_keys(&group.path);
        let mut at: Option<usize> = None;
        let mut walked = String::new();
        for (i, key) in keys.iter().enumerate() {
            walked.push_str(key);
            let last = i + 1 == keys.len();
            if let Some(n) = self.step(tree, at, key, &walked) {
                if last {
                    let node = &mut tree.nodes[n];
                    node.sticky |= group.sticky;
                    node.inherit &= group.inherit;
                }
                at = Some(n);
                continue;
            }
            let took = self.what_ran(tree, at, key);
            let m = tree.nodes.len();
            tree.nodes.push(Node {
                path: walked.clone(),
                sticky: last && group.sticky,
                inherit: true,
                layer: group.layer,
                ..Node::default()
            });
            match at {
                None => {
                    tree.runs.retain(|run, _| !run.starts_with(key.as_str()));
                    tree.preset.retain(|run| !run.starts_with(key.as_str()));
                    tree.root.insert(key.clone(), m);
                }
                Some(n) => {
                    let row = Mine { key: key.clone(), to: To::Node(m), says: None, order: group.order, layer: group.layer };
                    let node = &mut tree.nodes[n];
                    match node.rows.iter_mut().find(|r| r.key == *key) {
                        Some(was) => *was = Mine { order: was.order, ..row },
                        None => node.rows.push(row),
                    }
                    node.runs.retain(|run, _| !run.starts_with(key.as_str()));
                }
            }
            if let Some(name) = took {
                tree.notes.push(say!("keys.table-takes-a-command", yumete_cjk::actions::spell(&walked), name));
            }
            at = Some(m);
        }
    }

    /// The command `key` ran at `at` before a table took it, if it ran one.
    fn what_ran(&self, tree: &KeyTree, at: Option<usize>, key: &str) -> Option<String> {
        let ran = match at {
            None => match tree.runs.get(key) {
                Some(commands) => Some(commands.join(", ")),
                None => {
                    let pressed = super::keys::pressed_key(key)?;
                    let cx = super::normal::Cx { key: pressed, count: 1, operator_count: None, continuing_zong: false };
                    self.normal_command_for(&cx).map(str::to_string)
                }
            },
            Some(n) => {
                let node = &tree.nodes[n];
                match node.row(key) {
                    Some(Mine { to: To::Run(commands), .. }) => Some(commands.join(", ")),
                    Some(_) => None,
                    None if node.inherit => {
                        let group = GROUPS[node.group?];
                        let bind = group.bind_for(super::keys::pressed_key(key)?, &self.context_in(group))?;
                        Some(bind.run.name().to_string())
                    }
                    None => None,
                }
            }
        };
        ran.filter(|name| name != NormalCmd::NoOp.name())
    }

    /// **A line**: one key at the top level is that key; a longer one walks
    /// the tree, a row of the group it ends in, or a run held past the last
    /// group it reaches.
    fn lay_line(&self, tree: &mut KeyTree, line: &Line, says: Option<&String>) {
        let keys = split_keys(&line.run);
        let first = &keys[0];
        let at_root = |tree: &mut KeyTree| {
            tree.runs.insert(line.run.clone(), line.commands.clone());
            tree.preset.remove(&line.run);
            tree.order.insert(line.run.clone(), line.order);
        };
        if keys.len() == 1 {
            match tree.root.get(first).copied() {
                // A table of a later layer keeps the key.
                Some(m) if tree.nodes[m].layer > line.layer => {}
                Some(_) => {
                    tree.root.remove(first);
                    at_root(tree);
                }
                None => at_root(tree),
            }
            return;
        }
        // A key the top level holds for a longer run stays the run's: the
        // run is what the key waits for (vim's `Z` of `ZZ`).
        let held = tree.runs.contains_key(&line.run)
            || tree.runs.keys().any(|r| r.len() > first.len() && r.starts_with(first.as_str()));
        if held && !tree.root.contains_key(first) {
            at_root(tree);
            return;
        }
        let mut at: Option<usize> = None;
        let mut walked = String::new();
        for (i, key) in keys.iter().enumerate() {
            if i + 1 == keys.len() {
                let Some(n) = at else { return };
                let row = Mine {
                    key: key.clone(),
                    to: To::Run(line.commands.clone()),
                    says: says.cloned(),
                    order: line.order,
                    layer: line.layer,
                };
                let node = &mut tree.nodes[n];
                match node.rows.iter_mut().find(|r| r.key == *key) {
                    // A group of a later layer keeps its key.
                    Some(was) if matches!(was.to, To::Node(_)) && was.layer > line.layer => {}
                    Some(was) => *was = Mine { order: was.order, ..row },
                    None => node.rows.push(row),
                }
                return;
            }
            walked.push_str(key);
            match self.step(tree, at, key, &walked) {
                Some(n) => at = Some(n),
                None => {
                    match at {
                        None => at_root(tree),
                        Some(n) => {
                            let rest = keys[i..].concat();
                            tree.nodes[n].run_order.insert(rest.clone(), line.order);
                            tree.nodes[n].runs.insert(rest, line.commands.clone());
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
            None => {
                if let Some(&m) = tree.root.get(key) {
                    return Some(m);
                }
                match tree.runs.get(key) {
                    _ if key == " " => Some(&SPACE),
                    Some(commands) => self.binding_opens(commands),
                    None => self.top_opens(key),
                }
            }
            Some(n) => {
                let node = &tree.nodes[n];
                match node.row(key) {
                    Some(Mine { to: To::Node(m), .. }) => return Some(*m),
                    Some(Mine { to: To::Run(commands), .. }) => self.binding_opens(commands),
                    None if node.inherit => {
                        let group = GROUPS[node.group?];
                        let bind = group.bind_for(super::keys::pressed_key(key)?, &self.context_in(group))?;
                        opens(bind.run)
                    }
                    None => None,
                }
            }
        }?;
        Some(tree.over(place_of(group)?, walked))
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

    /// **What laying the keys found worth saying at start-up**: each table
    /// that took a key from the command it ran.
    pub fn key_notes(&self) -> Vec<String> {
        self.normal_tree.notes.clone()
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
        match self.pending {
            Pending::Node(n) => Some(n as usize).filter(|n| *n < self.tree().nodes.len()),
            waiting => self.tree().node_of(waiting.group()?),
        }
    }

    /// What waits in node `n`.
    fn pending_for(&self, n: usize) -> Pending {
        match self.tree().nodes[n].group {
            Some(group) => GROUPS[group].pending,
            None => Pending::Node(n as u16),
        }
    }

    /// The node a key at the top level opens, when a table took the key.
    pub(super) fn root_node(&self, typed: &str) -> Option<usize> {
        self.reading_mine().then(|| self.tree().root.get(typed).copied()).flatten()
    }

    /// What the reader is holding in this group (`b` of `Space bd`).
    fn held_here(&self) -> &str {
        match &self.node_held {
            Some((at, held)) if *at == self.pending => held,
            _ => "",
        }
    }

    /// Whether a key in this group is the reader's: a group of theirs, a
    /// key of theirs here, a run of theirs begun or held. Its commands then
    /// reach the cursors themselves.
    pub(super) fn node_takes(&self, key: Key) -> bool {
        let Some(n) = self.node_here() else { return false };
        let node = &self.tree().nodes[n];
        let typed = key_internal(key);
        node.group.is_none()
            || !node.inherit
            || !self.held_here().is_empty()
            || node.row(&typed).is_some()
            || node.runs_on(&typed)
    }

    /// **A key arriving in a group**: Backspace back a level, the reader's
    /// keys there (a key of theirs, a run of theirs held), then the group's
    /// own rows. Keeps the trail of the groups it came through.
    pub(super) fn on_node_key(&mut self, key: Key, typed_before: &str, continuing_zong: bool) {
        let was = self.pending;
        let back = self.node_key(key, typed_before, continuing_zong);
        if self.pending == Pending::None {
            self.node_trail.clear();
        } else if !back && self.pending != was && self.pending.is_node() {
            self.node_trail.push(was);
        }
    }

    /// The key itself; `true` when it went back a level.
    fn node_key(&mut self, key: Key, typed_before: &str, continuing_zong: bool) -> bool {
        let group = self.pending.group();
        let node = self.node_here().map(|n| self.tree().nodes[n].clone());
        // **Backspace goes back a level** where the group has no Backspace of
        // its own (Space's `⌫` turns the page, and it is the first level).
        if key == Key::Backspace && self.held_here().is_empty() && !self.has_its_own_backspace(group, node.as_ref()) {
            if let Some(back) = self.node_trail.pop() {
                self.pending = back;
                self.node_held = None;
                self.z_digits = None;
                self.sequence = None;
                self.sort_keys.clear();
                self.alias_count = None;
                return true;
            }
        }
        let Some(node) = node else {
            match group {
                Some(group) => self.on_group_key(group, key, typed_before, continuing_zong, group.sticky),
                None => self.pending = Pending::None,
            }
            return false;
        };
        let typed = key_internal(key);
        let held = self.held_here().to_string();
        self.node_held = None;
        // A run played from a binding presses the keys it names.
        let may_hold = !self.expanding_alias;
        if !held.is_empty() {
            if key == Key::Esc {
                self.close_the_node();
                return false;
            }
            let longer = format!("{held}{typed}");
            // A digit inside a held run is a count (`Space b 3d`), unless a
            // run of the reader's goes on with it.
            if !node.runs.keys().any(|r| r.starts_with(longer.as_str())) && self.count_inside(key) {
                self.node_held = Some((self.pending, held));
                return false;
            }
            if !typed.is_empty() && node.runs_on(&longer) {
                self.node_held = Some((self.pending, longer));
                return false;
            }
            if let Some(commands) = node.runs.get(&longer).filter(|_| !typed.is_empty()).cloned() {
                self.run_mine(&commands, node.sticky);
                return false;
            }
            self.let_go_in(&node, &held);
            let waiting = self.pending;
            self.fan_out(waiting, key, false, &|e| e.on_normal_key(key));
            return false;
        }
        if may_hold && node.runs_on(&typed) {
            self.node_held = Some((self.pending, typed));
            return false;
        }
        if let Some(row) = node.row(&typed) {
            match &row.to {
                To::Run(commands) => self.run_mine(commands, node.sticky),
                To::Node(m) => self.pending = self.pending_for(*m),
            }
            return false;
        }
        let sticky = node.sticky || group.is_some_and(|g| g.sticky);
        match group.filter(|_| node.inherit) {
            Some(group) => self.on_group_key(group, key, typed_before, continuing_zong, sticky),
            // **A key a group of the reader's does not have closes it**,
            // saying nothing, as which-key's; a digit there is a count.
            None => {
                if key == Key::Esc || !self.count_inside(key) {
                    self.close_the_node();
                }
            }
        }
        false
    }

    /// A digit typed inside a run or a group of the reader's, as a count:
    /// `true` when `key` was one.
    fn count_inside(&mut self, key: Key) -> bool {
        let Key::Char(c @ '0'..='9') = key else { return false };
        if c == '0' && self.alias_count.is_none() {
            return false;
        }
        let n = self.alias_count.unwrap_or(0);
        self.alias_count = Some(n.saturating_mul(10).saturating_add(c as usize - '0' as usize).min(1_000_000));
        true
    }

    fn close_the_node(&mut self) {
        self.pending = Pending::None;
        self.alias_count = None;
        self.operator_count = None;
        self.group_count = None;
    }

    /// Whether Backspace is a key of this group's: a row of its own, a key
    /// of the reader's, or vim's `z{count}` taking a digit back.
    fn has_its_own_backspace(&self, group: Option<&'static Group>, node: Option<&Node>) -> bool {
        let typed = key_internal(Key::Backspace);
        if let Some(node) = node {
            if node.row(&typed).is_some() || node.runs_on(&typed) {
                return true;
            }
            if !node.inherit {
                return false;
            }
        }
        group.is_some_and(|g| {
            g.bind_for(Key::Backspace, &self.context_in(g)).is_some()
                || (g.opener == NormalCmd::ViewMode && self.z_digits.is_some())
        })
    }

    /// After a command at the top level: the count typed before it, kept
    /// for the reader's keys in the group it opened.
    pub(super) fn note_the_group_count(&mut self, before: Option<usize>) {
        self.group_count = self.pending.group().and(before);
    }

    /// Run one of the reader's keys in a group: the group closes, unless it
    /// is sticky, and the count typed before the group, and any typed inside
    /// it, goes to the commands.
    fn run_mine(&mut self, commands: &[String], sticky: bool) {
        let was = self.pending;
        self.pending = Pending::None;
        self.count = self.group_count.take().or(self.operator_count.take());
        self.operator_count = None;
        self.settle_alias_count();
        self.run_binding(commands);
        if sticky && self.pending == Pending::None && self.mode == Mode::Normal {
            self.pending = was;
        }
    }

    /// **What was held in a group acts first**: the longest start of it the
    /// reader bound there runs, and the rest is pressed as the editor's own
    /// keys; with none bound, all of it is.
    fn let_go_in(&mut self, node: &Node, held: &str) {
        let keys = split_keys(held);
        for cut in (1..=keys.len()).rev() {
            let start = keys[..cut].concat();
            let bound = match cut {
                1 => node.row(&start).and_then(|row| match &row.to {
                    To::Run(commands) => Some(commands.clone()),
                    To::Node(_) => None,
                }),
                _ => node.runs.get(&start).cloned(),
            };
            if let Some(commands) = bound {
                self.run_mine(&commands, false);
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
        let group = self.pending.group();
        let n = self.node_here();
        let node = n.map(|n| &self.tree().nodes[n]);
        if group.is_none() && node.is_none() {
            return None;
        }
        let held = self.held_here();
        if let Some(node) = node.filter(|_| !held.is_empty()) {
            let mine: Vec<(String, Vec<String>, usize)> = node
                .runs
                .iter()
                .map(|(k, v)| (k.clone(), v.clone(), node.run_order.get(k).copied().unwrap_or(UNPLACED)))
                .collect();
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
        Some(self.menu_of(group, node))
    }

    /// The menu of a run held at the top level: the reader's runs that go on
    /// from it, and, when letting go of it opens a group (vim's `Z` of
    /// `ZZ`), that group's keys besides.
    fn root_held_menu(&self, held: &str) -> Option<(String, KeyRows)> {
        let tree = self.tree();
        let mine: Vec<(String, Vec<String>, usize)> = tree
            .runs
            .iter()
            .filter(|(k, _)| !tree.preset.contains(*k))
            .map(|(k, v)| (k.clone(), v.clone(), tree.order.get(k).copied().unwrap_or(UNPLACED)))
            .collect();
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

    /// **A group's menu**: its own rows in their order, a key of the
    /// reader's in the place of the row it takes, then the reader's other
    /// keys in the order they were written. A row that opens a group says
    /// `+` and the group's name, and a group left with no keys is not
    /// listed. Titled with the reader's name for it, the group's, or its
    /// keys.
    fn menu_of(&self, group: Option<&'static Group>, node: Option<&Node>) -> (String, KeyRows) {
        let mine = node.map(|node| self.mine_rows(node)).unwrap_or_default();
        let mut rows: Vec<(Vec<String>, Said, bool)> = Vec::new();
        let mut placed: HashSet<String> = HashSet::new();
        if let Some(group) = group.filter(|_| node.is_none_or(|node| node.inherit)) {
            for (bind, name, says) in group.listed(&self.context_in(group)) {
                let Some(bind) = bind else {
                    rows.push((vec![name], Said::Msg(says), false));
                    continue;
                };
                let key = key_internal(bind.key);
                if let Some((_, text, _)) = mine.iter().find(|(k, _, _)| *k == key) {
                    if placed.insert(key) {
                        if let Some(text) = text {
                            rows.push((vec![name], Said::Text(text.clone()), false));
                        }
                    }
                    continue;
                }
                match opens(bind.run) {
                    Some(sub) => {
                        if self.group_is_empty(sub) {
                            continue;
                        }
                        let tree = self.tree();
                        let label = tree.node_of(sub).and_then(|n| tree.nodes[n].label.clone());
                        let label = label.unwrap_or_else(|| crate::messages::say(says, &[]));
                        rows.push((vec![name], Said::Text(format!("+{label}")), false));
                    }
                    None => match rows.last_mut() {
                        Some((keys, Said::Msg(said), true)) if *said == says => keys.push(name),
                        _ => rows.push((vec![name], Said::Msg(says), true)),
                    },
                }
            }
        }
        let mut rest: Vec<&(String, Option<String>, usize)> = mine.iter().filter(|(k, _, _)| !placed.contains(k)).collect();
        rest.sort_by(|a, b| (a.2, &a.0).cmp(&(b.2, &b.0)));
        for (key, text, _) in rest {
            if let Some(text) = text {
                rows.push((vec![key_label(key_of(key))], Said::Text(text.clone()), false));
            }
        }
        let rows: KeyRows = rows
            .into_iter()
            .map(|(keys, said, _)| {
                let said = match said {
                    Said::Msg(id) => crate::messages::say(id, &[]),
                    Said::Text(text) => text,
                };
                (keys_label(&keys).into(), said)
            })
            .collect();
        let title = match (node.and_then(|n| n.label.clone()), group) {
            (Some(label), _) => label,
            // The Space menu is titled by its key, and under vim that may be `\`.
            (None, Some(group)) if group.name == SPACE.name && self.space_sign() == "\\" => "\\".to_string(),
            (None, Some(group)) => crate::messages::say(group.title, &[]),
            (None, None) => yumete_cjk::actions::spell(node.map_or("", |n| n.path.as_str())),
        };
        (title, rows)
    }

    /// **The reader's keys in a group, as menu rows**: each key, its line
    /// (`None` for a key that leaves the menu: `no_op`, an empty group), and
    /// where it was written. A key that only begins runs held here says the
    /// name of what it begins, or `…`.
    fn mine_rows(&self, node: &Node) -> Vec<(String, Option<String>, usize)> {
        let tree = self.tree();
        let mut out: Vec<(String, Option<String>, usize)> = Vec::new();
        for row in &node.rows {
            let text = match &row.to {
                To::Run(commands) => row.says.clone().or_else(|| describe(commands)),
                To::Node(m) => match self.node_is_empty(*m) {
                    true => None,
                    false => Some(match &tree.nodes[*m].label {
                        Some(label) => format!("+{label}"),
                        None => format!("+{}", self.keys_in(*m)),
                    }),
                },
            };
            out.push((row.key.clone(), text, row.order));
        }
        let group = node.group.filter(|_| node.inherit).map(|g| GROUPS[g]);
        for run in node.runs.keys() {
            let Some(next) = split_keys(run).into_iter().next() else { continue };
            let order = node.run_order.get(run).copied().unwrap_or(UNPLACED);
            if let Some(row) = out.iter_mut().find(|(k, _, _)| *k == next) {
                row.2 = row.2.min(order);
                continue;
            }
            let said = match node.labels.get(&next) {
                Some(label) => label.clone(),
                // A group's key with the reader's runs under it keeps its own
                // line.
                None if group.is_some_and(|g| g.binds().any(|b| key_internal(b.key) == next)) => continue,
                None => "…".to_string(),
            };
            out.push((next, Some(said), order));
        }
        out
    }

    /// A group of the reader's with no name, on its parent's row: its keys.
    fn keys_in(&self, n: usize) -> String {
        let (_, rows) = self.menu_of(None, Some(&self.tree().nodes[n]));
        let mut keys: Vec<String> = rows.into_iter().take(4).map(|(k, _)| k.into_owned()).collect();
        if keys.len() == 4 {
            keys[3] = "…".to_string();
        }
        keys.join(" ")
    }

    /// Whether a node has nothing left on its menu.
    fn node_is_empty(&self, n: usize) -> bool {
        let node = &self.tree().nodes[n];
        let group = node.group.map(|g| GROUPS[g]);
        self.menu_of(group, Some(node)).1.is_empty()
    }

    /// Whether a built-in group has nothing left on its menu (the reader
    /// took every key of it).
    fn group_is_empty(&self, group: &Group) -> bool {
        self.tree().node_of(group).is_some_and(|n| self.node_is_empty(n))
    }

    /// The sign of a group of the reader's: the keys that opened it.
    pub(super) fn node_sign(&self) -> Option<String> {
        let Pending::Node(n) = self.pending else { return None };
        let node = self.tree().nodes.get(n as usize)?;
        Some(split_keys(&node.path).iter().map(|k| key_label(key_of(k))).collect())
    }

    /// **The key that opens the Space menu**, as a sign writes it: the
    /// first key at the top level that opens it, `␣` when that is Space.
    pub(super) fn space_sign(&self) -> &'static str {
        let at = self.group_context();
        let homes = home_paths(&Context { vim: at.vim, flavor: at.flavor, ..Context::default() });
        match place_of(&SPACE).and_then(|p| homes[p].as_ref()).and_then(|path| path.first()).map(String::as_str) {
            Some("\\") => "\\",
            _ => "␣",
        }
    }
}

/// **A menu's rows from the reader's runs**: the next key of each run that
/// goes on from `held`, as the command it runs or the name of what it
/// opens, after the rows of `group` the reader did not take, in the order
/// the runs were written.
fn held_rows(
    held: &str,
    mine: &[(String, Vec<String>, usize)],
    group: Option<&'static Group>,
    at: &Context,
    label_of: impl Fn(&str) -> Option<String>,
) -> KeyRows {
    let mut next_keys: Vec<(String, Option<String>, usize)> = Vec::new();
    for (left, commands, order) in mine {
        let Some(rest) = left.strip_prefix(held).filter(|r| !r.is_empty()) else { continue };
        let Some(next) = split_keys(rest).into_iter().next() else { continue };
        let exact = rest.len() == next.len();
        let said = match exact {
            true => describe(commands),
            false => match label_of(&format!("{held}{next}")) {
                Some(label) => Some(label),
                // A group's key with the reader's keys under it keeps its
                // own line.
                None if group.is_some_and(|g| g.binds().any(|b| key_internal(b.key) == next)) => continue,
                None => Some("…".to_string()),
            },
        };
        match next_keys.iter_mut().find(|(n, _, _)| *n == next) {
            Some(row) => {
                row.2 = row.2.min(*order);
                if exact {
                    row.1 = said;
                }
            }
            None => next_keys.push((next, said, *order)),
        }
    }
    next_keys.sort_by(|a, b| (a.2, &a.0).cmp(&(b.2, &b.0)));
    let mut rows: KeyRows = Vec::new();
    if let Some(group) = group {
        let taken = |key: Key| next_keys.iter().any(|(n, _, _)| *n == key_internal(key));
        for (keys, says) in group.menu_without(at, taken) {
            rows.push((keys.into(), crate::messages::say(says, &[])));
        }
    }
    for (next, said, _) in next_keys {
        let Some(said) = said else { continue };
        rows.push((key_label(key_of(&next)).into(), said));
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
    let place = |cmd: NormalCmd| opens(cmd).and_then(place_of);
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
