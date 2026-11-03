//! **The key groups, as nodes of the keymap** (2026-10-31, step 3 of the
//! keymap plan).
//!
//! A group is what a key such as `g`, `Space` or `C-w` opens: a table of
//! the keys that may follow, each naming a command, with the line its menu
//! shows. The table is the one place a group is written. The key that
//! arrives is looked up in it ([`Editor::on_group_key`]), and the which-key
//! menu, the key lists the documents are checked against and the generated
//! keymap page are all read off it, so a menu cannot list a key the group
//! does not have.
//!
//! A row may hold only where something is true ([`Only`]): under one preset,
//! on a vertical page, in a code file, in one kind of table. Two rows may
//! give one key, and the first that holds is the one that answers, as the
//! top-level tables in `normal.rs` do. A row's menu line can hold more
//! narrowly than its key (`listed`), for a key that works anywhere and is
//! only worth offering in some places (`]f` finds a function in any file,
//! and is offered in code). Rows next to each other with the same line merge
//! into one, their keys side by side (`h l  行首/行尾`), as helix's menus do.

use super::normal::Cx;
use super::*;
use yumete_cjk::actions::NormalCmd;
use yumete_cjk::FlavorItem;
use crate::input::Key;

/// **Where a row holds.** `Always` and the presets are the common ones; the
/// rest each answer one group's question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Only {
    Always,
    Vim,
    /// helix's keymap only: a key whose vim meaning yumete does not have.
    Helix,
    /// `g h` / `g l` / `g s` / `g t`: helix's keys, which the vim preset
    /// reaches only when a binding or the preset plays them (`0`, `$` and
    /// `^` are its own; vim's `gs` sleeps and its `gt` is the next tab).
    GLetter,
    /// A vertical page under the vim preset, or under helix's.
    VimVertical,
    HelixVertical,
    /// A page laid out across, under the vim preset, or under either.
    FlatVim,
    Flat,
    /// The file is code (functions, classes, comments to walk to).
    Code,
    /// A selection is being extended (select mode, or vim's visual).
    Selecting,
    /// A table has the window.
    Pane,
    /// The cursor is in a fenced block's table, a Markdown table, or a
    /// delimited file.
    Block,
    Md,
    File,
}

/// What a group's row is answered by.
#[derive(Debug, Clone, Copy)]
pub(super) struct Bind {
    pub key: Key,
    /// The command it runs, by name.
    pub run: NormalCmd,
    /// The menu line; empty for a key that is not on the menu (the arrow
    /// keys beside `hjkl`, the `C-` spellings in the window group).
    pub says: &'static str,
    /// Where the key does this.
    pub when: Only,
    /// Where the menu offers it, when the key does it.
    pub listed: Only,
    /// The flavour item it belongs to, if it is one of yumete's own keys:
    /// it holds only while that item is on.
    pub flavor: Option<FlavorItem>,
}

/// One row of a group: a key, or a line that explains rather than binds
/// (`<N>g`, the sort keys a column number comes before).
#[derive(Debug, Clone, Copy)]
pub(super) enum Row {
    Bind(Bind),
    Note { keys: &'static str, says: &'static str, listed: Only },
}

impl Row {
    const fn when(mut self, only: Only) -> Self {
        if let Row::Bind(bind) = &mut self {
            bind.when = only;
        }
        self
    }

    /// One of yumete's own keys, there while `item` is on.
    const fn flavor(mut self, item: FlavorItem) -> Self {
        if let Row::Bind(bind) = &mut self {
            bind.flavor = Some(item);
        }
        self
    }

    const fn listed(mut self, only: Only) -> Self {
        match &mut self {
            Row::Bind(bind) => bind.listed = only,
            Row::Note { listed, .. } => *listed = only,
        }
        self
    }
}

const fn key(key: Key, run: NormalCmd, says: &'static str) -> Row {
    Row::Bind(Bind { key, run, says, when: Only::Always, listed: Only::Always, flavor: None })
}

const fn ch(c: char, run: NormalCmd, says: &'static str) -> Row {
    key(Key::Char(c), run, says)
}

/// A key that is not on the menu.
const fn quiet(key_: Key, run: NormalCmd) -> Row {
    key(key_, run, "")
}

const fn note(keys: &'static str, says: &'static str) -> Row {
    Row::Note { keys, says, listed: Only::Always }
}

/// What a key the group does not have does.
#[derive(Debug, Clone, Copy)]
pub(super) enum Unknown {
    /// Nothing; the group closes.
    Silent,
    /// 「`gq` 爲無效按鍵組合」.
    NoSuchCombination,
    /// The keys the group has, in this message.
    SayKeys(&'static str),
    /// The group's own answer.
    Hand(fn(&mut Editor, Key)),
}

/// **A group**: what its menu is titled, its rows, and the few things a
/// group does around them.
pub(super) struct Group {
    /// For the docs and the tests: `goto`, `space`.
    pub name: &'static str,
    /// What the editor waits in while the group is open.
    pub pending: Pending,
    /// The menu's title.
    pub title: &'static str,
    /// The keys that open it, in a binding's spelling (`g`, `" t"`).
    pub path: &'static str,
    pub rows: &'static [Row],
    pub unknown: Unknown,
    /// Stays open until `Esc` (helix's `Z`).
    pub sticky: bool,
    /// `C-h` is `h` here (the window group, after vim's `C-w C-h`).
    pub ctrl_is_plain: bool,
    /// Runs on each key before it is looked up; `true` when it took the key
    /// (a count inside the group: `g30g`, `Space t 2s`).
    pub before: Option<fn(&mut Editor, Key) -> bool>,
    /// Runs after any key but one `before` took.
    pub after: Option<fn(&mut Editor)>,
}

impl Group {
    /// The rows that are keys.
    pub(super) fn binds(&self) -> impl Iterator<Item = &'static Bind> {
        self.rows.iter().filter_map(|row| match row {
            Row::Bind(bind) => Some(bind),
            Row::Note { .. } => None,
        })
    }

    /// **The bind `key` reaches here**: the first whose `when` holds.
    pub(super) fn bind_for(&self, key: Key, at: &Context) -> Option<&'static Bind> {
        self.binds().find(|bind| bind.key == key && at.reaches(bind))
    }

    /// **The menu, as (keys, message) rows**: each key the group answers
    /// here and offers here, notes in their places, neighbours that say the
    /// same thing on one row.
    pub(super) fn menu(&'static self, at: &Context) -> Vec<(String, &'static str)> {
        self.menu_without(at, |_| false)
    }

    /// The menu, leaving out the keys `taken` says the reader has bound
    /// to something else.
    pub(super) fn menu_without(&'static self, at: &Context, taken: impl Fn(Key) -> bool) -> Vec<(String, &'static str)> {
        let mut out: Vec<(Vec<String>, &'static str, bool)> = Vec::new();
        for (nth, row) in self.rows.iter().enumerate() {
            match row {
                Row::Bind(bind) => {
                    if bind.says.is_empty() || !at.reaches(bind) || !at.holds(bind.listed) || taken(bind.key) {
                        continue;
                    }
                    // Only a row the key reaches, once: a key with a vim row
                    // and a helix row is listed as what it does here, and a
                    // key offered in several kinds of table as that kind's.
                    let hidden = self.rows[..nth].iter().any(|earlier| match earlier {
                        Row::Bind(b) if b.key == bind.key && at.reaches(b) => {
                            b.run != bind.run || (!b.says.is_empty() && at.holds(b.listed))
                        }
                        _ => false,
                    });
                    if hidden {
                        continue;
                    }
                    let name = key_label(bind.key);
                    match out.last_mut() {
                        Some((keys, says, true)) if *says == bind.says => keys.push(name),
                        _ => out.push((vec![name], bind.says, true)),
                    }
                }
                Row::Note { keys, says, listed } if at.holds(*listed) => {
                    out.push((vec![keys.to_string()], says, false));
                }
                Row::Note { .. } => {}
            }
        }
        out.into_iter().map(|(keys, says, _)| (keys_label(&keys), says)).collect()
    }

    /// Every key the group has anywhere, each once, for the test that asks
    /// whether a key the documents print is one the editor has. Keys are
    /// spelled as themselves (the space bar is `" "`, Backspace `"\u{8}"`),
    /// which is how that test reads `Space Space` off a page.
    pub(super) fn every_key(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for row in self.rows {
            let names: Vec<String> = match row {
                Row::Bind(bind) if !bind.says.is_empty() => vec![match bind.key {
                    Key::Char(c) => c.to_string(),
                    Key::Backspace => "\u{8}".to_string(),
                    other => key_label(other),
                }],
                Row::Bind(_) => continue,
                Row::Note { keys, .. } => keys.split_whitespace().map(str::to_string).collect(),
            };
            for name in names {
                if !out.contains(&name) {
                    out.push(name);
                }
            }
        }
        out
    }
}

/// **What a binding does, in a menu line**: a named command's description,
/// a command line or a run of keys as written, a list's parts in order.
/// `None` for `no_op`, which leaves the menu.
fn describe(commands: &[String]) -> Option<String> {
    if commands.iter().all(|one| one == NormalCmd::NoOp.name()) {
        return None;
    }
    let parts: Vec<String> = commands
        .iter()
        .map(|one| match yumete_cjk::actions::bound(one) {
            Some(yumete_cjk::actions::Bound::Named(action)) => crate::messages::say(action.help, &[]),
            Some(yumete_cjk::actions::Bound::Command(line)) => format!(":{line}"),
            Some(yumete_cjk::actions::Bound::Keys(keys)) => format!("@{keys}"),
            None => one.clone(),
        })
        .collect();
    Some(parts.join(", "))
}

/// One key in the internal spelling (`d`, Alt and a letter), as a [`Key`].
fn key_of(next: &str) -> Key {
    let mut chars = next.chars();
    match (chars.next(), chars.next()) {
        (Some(yumete_cjk::actions::ALT), Some(c)) => Key::Alt(c),
        (Some(c), _) => super::keys::pressed(c),
        _ => Key::Esc,
    }
}

/// A key as a menu writes it: `␣` for the space bar, `⌫` for Backspace.
fn key_label(key: Key) -> String {
    match key {
        Key::Char(c) => Editor::key_name(c).into_owned(),
        Key::Backspace => "⌫".to_string(),
        Key::Ctrl(c) => format!("C-{c}"),
        Key::Alt(c) => format!("A-{c}"),
        other => yumete_cjk::actions::spell(&super::keys::key_internal(other)),
    }
}

/// A row's keys, side by side. Three or more characters in a run are written
/// as the run (`3-9`), as a reader would.
fn keys_label(keys: &[String]) -> String {
    let chars: Option<Vec<char>> = keys
        .iter()
        .map(|k| {
            let mut cs = k.chars();
            match (cs.next(), cs.next()) {
                (Some(c), None) if c.is_ascii_alphanumeric() => Some(c),
                _ => None,
            }
        })
        .collect();
    if let Some(chars) = chars.filter(|cs| cs.len() >= 3) {
        if chars.windows(2).all(|w| w[1] as u32 == w[0] as u32 + 1) {
            return format!("{}-{}", chars[0], chars[chars.len() - 1]);
        }
    }
    keys.join(" ")
}

/// **What a row's [`Only`] is asked against**: the few facts about the
/// editor that change a group.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Context {
    pub vim: bool,
    pub vertical: bool,
    /// A binding is playing keys (`g h` is pressable then under vim too).
    pub alias: bool,
    pub code: bool,
    /// Select mode.
    pub extend: bool,
    pub pane: bool,
    pub table: Option<Bounds>,
    /// The flavour items on, one bit each (`yumete_cjk::Flavor::mask`).
    pub flavor: u64,
}

impl Context {
    pub(super) fn holds(&self, only: Only) -> bool {
        match only {
            Only::Always => true,
            Only::Vim => self.vim,
            Only::Helix => !self.vim,
            Only::GLetter => !self.vim || self.alias,
            Only::VimVertical => self.vim && self.vertical,
            Only::HelixVertical => !self.vim && self.vertical,
            Only::FlatVim => self.vim && !self.vertical,
            Only::Flat => !self.vertical,
            Only::Code => self.code,
            Only::Selecting => self.extend,
            Only::Pane => self.pane,
            Only::Block => self.table == Some(Bounds::Block),
            Only::Md => self.table == Some(Bounds::Md),
            Only::File => self.table == Some(Bounds::WholeFile),
        }
    }

    /// Whether a flavour item is on (always, for a row that is not one).
    pub(super) fn has(&self, item: Option<FlavorItem>) -> bool {
        item.is_none_or(|item| self.flavor & item.bit() != 0)
    }

    /// Whether a group's row answers here: its condition, and its item.
    pub(super) fn reaches(&self, bind: &Bind) -> bool {
        self.holds(bind.when) && self.has(bind.flavor)
    }

    /// The context the generated keymap page is written for: helix's keys
    /// under the factory flavour, a page laid out across, a code file (so
    /// every bracket key is listed).
    pub(super) fn for_docs() -> Self {
        let flavor = yumete_cjk::Flavor::default().mask(yumete_cjk::KeyPreset::Helix);
        Context { code: true, flavor, ..Context::default() }
    }
}

impl Pending {
    /// **The group this is waiting in**, when it is one.
    pub(super) fn group(self) -> Option<&'static Group> {
        Some(match self {
            Pending::Goto => &GOTO,
            Pending::Space => &SPACE,
            Pending::Hop { forward: true } => &RIGHT_BRACKET,
            Pending::Hop { forward: false } => &LEFT_BRACKET,
            Pending::Aim => &VIEW,
            Pending::AimStuck => &STICKY_VIEW,
            Pending::Match => &MATCH,
            Pending::Region => &WINDOW,
            Pending::RegionNew => &WINDOW_NEW,
            Pending::Case => &CASE,
            Pending::Conflict => &CONFLICT,
            Pending::Open => &OPEN,
            Pending::Yume => &YUME,
            Pending::YumeValue('m') => &YUME_COMMIT,
            Pending::YumeValue('p') => &YUME_PANEL,
            Pending::YumeValue(_) => &YUME_PREEDIT,
            Pending::Table => &TABLE,
            Pending::TableConvert => &TABLE_CONVERT,
            _ => return None,
        })
    }
}

/// Every group, for the docs and the tests.
pub(super) const GROUPS: &[&Group] = &[
    &SPACE,
    &GOTO,
    &MATCH,
    &RIGHT_BRACKET,
    &LEFT_BRACKET,
    &VIEW,
    &STICKY_VIEW,
    &WINDOW,
    &WINDOW_NEW,
    &TABLE,
    &TABLE_CONVERT,
    &CASE,
    &CONFLICT,
    &OPEN,
    &YUME,
    &YUME_COMMIT,
    &YUME_PANEL,
    &YUME_PREEDIT,
];

impl Editor {
    /// What this editor's groups are asked against now.
    pub(super) fn group_context(&self) -> Context {
        // **Which list is a question about the cursor, not the mode.** It
        // used to be `md_region().is_none()`, which is *also* true of a
        // Markdown table nobody has opened yet — so standing in one of
        // 手冊's own tables offered the delimited file's keys.
        let table = match self.table.as_ref().map(|v| v.bounds) {
            Some(Bounds::Block) if self.block_region().is_some() => Some(Bounds::Block),
            Some(Bounds::Md) if self.md_region().is_some() => Some(Bounds::Md),
            Some(Bounds::WholeFile) => Some(Bounds::WholeFile),
            _ => None,
        };
        Context {
            vim: self.vim_keymap(),
            vertical: self.layout() == crate::zong::Layout::Vertical,
            alias: self.expanding_alias,
            code: self.writes_code(),
            extend: self.extend,
            pane: self.the_pane_is_on(),
            table,
            flavor: self.flavor().mask(self.key_preset()),
        }
    }

    /// **A key arriving in a group**: the row it reaches runs its command.
    ///
    /// The count typed before the group's key (`3]d`, `5gt`) is handed to
    /// the command in its [`Cx`] and left where it was, for the commands
    /// that pass it on (`mi` keeps it for the pair it waits for).
    pub(super) fn on_group_key(&mut self, group: &'static Group, key: Key, typed_before: &str, continuing_zong: bool) {
        if let Some(before) = group.before {
            if before(self, key) {
                return;
            }
        }
        let was = self.pending;
        self.pending = Pending::None;
        if group.sticky && key != Key::Esc {
            self.pending = was;
        }
        let key = match (group.ctrl_is_plain, key) {
            (true, Key::Ctrl(c)) => Key::Char(c),
            (_, key) => key,
        };
        let at = self.group_context();
        match group.bind_for(key, &at) {
            Some(bind) => {
                if let Some(&(_, run, _)) = super::normal::normal_cmd(bind.run) {
                    let operator_count = self.operator_count;
                    let count = operator_count.unwrap_or(1).max(1);
                    // `g h` in a table's grid is the record's first cell.
                    if !self.takes_by_name(bind.run, count) {
                        run(self, Cx { key, count, operator_count, continuing_zong });
                    }
                }
            }
            None if key == Key::Esc => {}
            None => match group.unknown {
                Unknown::Silent => {}
                Unknown::NoSuchCombination => self.no_such_combination(typed_before, key),
                Unknown::SayKeys(message) => {
                    let keys: Vec<String> = group.menu(&at).into_iter().map(|(k, _)| k).collect();
                    self.status = crate::messages::say(message, &[&keys.join(" ")]);
                }
                Unknown::Hand(hand) => hand(self, key),
            },
        }
        if let Some(after) = group.after {
            after(self);
        }
    }

    /// **The menu of a run of keys the reader's bindings are holding**
    /// (2026-10-31, step 5 of the keymap plan): their own keys that may come
    /// next, and when the run is the way into one of the groups (`Space`, once
    /// `[keys.normal.space]` adds a key to it), that group's keys besides.
    /// Titled with the name the reader gave it (`label`), the group's, or the
    /// keys themselves. `None` when the run is the vim preset's own (`Z` of
    /// `ZZ`), which never had a menu.
    pub(super) fn held_menu(&self) -> Option<(String, KeyRows)> {
        let held = self.alias_held.as_str();
        if held.is_empty() || self.mode != Mode::Normal {
            return None;
        }
        let written = |k: &String| yumete_cjk::actions::written(k);
        let mut mine: Vec<(String, &Vec<String>)> = self.user_aliases.iter().map(|(k, v)| (written(k), v)).collect();
        if self.extend {
            for (k, v) in &self.user_select {
                let k = written(k);
                mine.retain(|(m, _)| *m != k);
                mine.push((k, v));
            }
        }
        let group = GROUPS.iter().find(|g| g.path == held).copied();
        let at = self.group_context();
        // The reader's keys after the run: each the command it runs, or the
        // name of the group it opens.
        let mut next_keys: Vec<(String, Option<String>)> = Vec::new();
        for (left, commands) in &mine {
            let Some(rest) = left.strip_prefix(held).filter(|r| !r.is_empty()) else { continue };
            let next: String = match rest.starts_with(yumete_cjk::actions::ALT) {
                true => rest.chars().take(2).collect(),
                false => rest.chars().take(1).collect(),
            };
            let exact = rest.len() == next.len();
            let opens = format!("{held}{next}");
            let said = match exact {
                true => describe(commands),
                false => match self.key_labels.get(&opens) {
                    Some(label) => Some(label.clone()),
                    // A default group's key with the reader's keys under it
                    // keeps its own line.
                    None if group.is_some_and(|g| g.binds().any(|b| super::keys::key_internal(b.key) == next)) => continue,
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
            let taken = |key: Key| next_keys.iter().any(|(n, _)| *n == super::keys::key_internal(key));
            for (keys, says) in group.menu_without(&at, taken) {
                rows.push((keys.into(), crate::messages::say(says, &[])));
            }
        }
        for (next, said) in next_keys {
            let Some(said) = said else { continue };
            rows.push((key_label(key_of(&next)).into(), said));
        }
        if rows.is_empty() {
            return None;
        }
        let title = match (self.key_labels.get(held), group) {
            (Some(label), _) => label.clone(),
            (None, Some(group)) => crate::messages::say(group.title, &[]),
            (None, None) => yumete_cjk::actions::spell(held),
        };
        Some((title, rows))
    }

    /// **The which-key menu of the group this is waiting in.**
    pub(super) fn group_menu(&self, group: &'static Group) -> (String, KeyRows) {
        let rows = group
            .menu(&self.group_context())
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

    /// **The key that opens the Space menu**, as a sign writes it: `␣`, or
    /// `\` under vim while the flavour's `vim_space_menu` leaves Space to vim.
    pub(super) fn space_sign(&self) -> &'static str {
        match self.vim_keymap() && !self.flavor_on(FlavorItem::VimSpaceMenu) {
            true => "\\",
            false => "␣",
        }
    }

    /// The keys `Space t` offers given what the cursor is standing in and
    /// whether a table has the window, as (keys, message).
    pub(super) fn table_keys(inside: Option<Bounds>, pane: bool) -> Vec<(String, &'static str)> {
        TABLE.menu(&Context { pane, table: inside, ..Context::default() })
    }

    /// 「`空格 t` 後面可以按這些」, from the same rows.
    pub(super) fn table_keys_say(inside: Option<Bounds>, pane: bool) -> String {
        let keys: Vec<String> = Self::table_keys(inside, pane).into_iter().map(|(k, _)| k).collect();
        say!("hint.table.after-t", keys.join(" "))
    }

    /// **Every key that may follow `leader`, in any context.**
    ///
    /// The menus stay context-sensitive on purpose (`Space t` inside a
    /// Markdown table offers what a Markdown table can do), so this does not
    /// replace them. It is their union, and it has one reader: the test that
    /// asks whether a key sequence the documents print is one the editor
    /// has.
    ///
    /// `None` means **any character follows**: `f`, `r`, `"`, `M`, `'` and
    /// the `mi`/`ma`/`ms`/`mr` pairs take a letter or a bracket the writer
    /// chooses, so there is no list for a document to be checked against.
    /// `t` is vi's till, one of those, since the table group moved to
    /// `Space t` (2026-09-21).
    pub fn keys_after(leader: char) -> Option<Vec<String>> {
        Some(match leader {
            // 空 is the first character of how the documents spell it: `空格 f`.
            ' ' | '空' => SPACE.every_key(),
            'g' => GOTO.every_key(),
            'm' => MATCH.every_key(),
            '`' => CASE.every_key(),
            ']' => RIGHT_BRACKET.every_key(),
            '[' => LEFT_BRACKET.every_key(),
            _ => return None,
        })
    }
}

/// **One command as the generated reference lists it** (`ye --docgen`,
/// step 7 of the keymap plan): its name, its description's message, and
/// the keys that reach it under helix's keymap, under vim's, and in Insert
/// mode. Each key is the run of keypresses, in the internal spelling.
pub type DocCommand = (&'static str, &'static str, Vec<Vec<String>>, Vec<Vec<String>>, Vec<Vec<String>>);

impl Only {
    /// Whether a row holds somewhere under a preset (the docs list every
    /// key a preset has, on any page).
    fn under(self, vim: bool) -> bool {
        match self {
            Only::Vim | Only::VimVertical | Only::FlatVim => vim,
            Only::GLetter | Only::HelixVertical | Only::Helix => !vim,
            _ => true,
        }
    }
}

/// The keys that open a group under a preset, one per keypress. vim's `m`
/// sets a mark, so the match group has no key there; its case group is
/// `` g` ``.
fn group_opened_by(group: &Group, vim: bool) -> Option<Vec<String>> {
    match (vim, group.name) {
        (true, "match") => None,
        (true, "case") => Some(vec!["g".to_string(), "`".to_string()]),
        _ => Some(group.path.chars().map(|c| c.to_string()).collect()),
    }
}

impl Editor {
    /// **Every bindable command, with every key that reaches it** under
    /// each preset and in Insert mode: the generated reference's rows, read
    /// off the same tables the keys run on, so a key the editor has is a key
    /// the site lists.
    pub fn commands_for_docs() -> Vec<DocCommand> {
        use super::normal::{FLAVOR_NORMAL, HELIX_NORMAL, SELECT_NORMAL, VIM_NORMAL};
        let one = |key: Key| vec![super::keys::key_internal(key)];
        // The factory flavour's keys are listed as each keymap's own: what a
        // reader who changed nothing presses. The flavour page lists them all.
        let factory = |vim: bool| {
            let keymap = match vim {
                true => yumete_cjk::KeyPreset::Vim,
                false => yumete_cjk::KeyPreset::Helix,
            };
            Context { vim, flavor: yumete_cjk::Flavor::default().mask(keymap), ..Context::default() }
        };
        let mut out = Vec::new();
        for action in yumete_cjk::actions::ALL {
            if action.how != yumete_cjk::actions::How::Normal {
                continue;
            }
            let name = action.name;
            let mut helix: Vec<Vec<String>> = Vec::new();
            let mut vim: Vec<Vec<String>> = Vec::new();
            for (key, n, item, only) in FLAVOR_NORMAL {
                if n.name() != name {
                    continue;
                }
                for (is_vim, keys) in [(false, &mut helix), (true, &mut vim)] {
                    if factory(is_vim).has(Some(*item)) && only.under(is_vim) {
                        keys.push(one(*key));
                    }
                }
            }
            // A key the factory flavour gives to something else is not the
            // keymap's own key for this command.
            let flavor_took = |key: &Key, is_vim: bool| {
                FLAVOR_NORMAL.iter().any(|(k, _, item, only)| k == key && factory(is_vim).has(Some(*item)) && only.under(is_vim))
            };
            // A key the vim preset spells for itself (`x` `s` `%` `K` `Q`…) is
            // vim's, not the helix command's (found in review, 2026-11-02).
            let preset_took = |key: &Key| {
                let written = super::keys::key_internal(*key);
                yumete_cjk::KeyPreset::Vim.table().iter().any(|(k, _)| yumete_cjk::actions::written(k) == written)
            };
            for (key, n) in HELIX_NORMAL.iter().chain(SELECT_NORMAL) {
                if n.name() != name || flavor_took(key, false) {
                    continue;
                }
                helix.push(one(*key));
                // The vim preset keeps a helix key it has no entry of its own for.
                if !VIM_NORMAL.iter().any(|(k, _, _)| k == key) && !flavor_took(key, true) && !preset_took(key) {
                    vim.push(one(*key));
                }
            }
            // The preset's keys that name this command (`x` is
            // `vim_delete_char_forward`, `K` is `hover`).
            for (key, run) in yumete_cjk::KeyPreset::Vim.table() {
                if *run == name {
                    let written = vec![yumete_cjk::actions::written(key)];
                    if !vim.contains(&written) {
                        vim.push(written);
                    }
                }
            }
            for (key, n, _) in VIM_NORMAL {
                if n.name() == name {
                    vim.push(one(*key));
                }
            }
            for group in GROUPS {
                // Keys off the menu too (`z j`, the sort keys a column
                // number comes before): a key the editor has is listed.
                for bind in group.binds().filter(|b| b.run.name() == name) {
                    for (is_vim, keys) in [(false, &mut helix), (true, &mut vim)] {
                        let Some(mut path) = group_opened_by(group, is_vim)
                            .filter(|_| bind.when.under(is_vim) && factory(is_vim).has(bind.flavor))
                        else {
                            continue;
                        };
                        path.push(super::keys::key_internal(bind.key));
                        if !keys.contains(&path) {
                            keys.push(path);
                        }
                    }
                }
            }
            // The vim preset's keys that play one of these runs (`$` is `gl`).
            for (key, run) in yumete_cjk::KeyPreset::Vim.table() {
                let Some(run) = run.strip_prefix('@') else { continue };
                let written = yumete_cjk::actions::written(run);
                if helix.iter().any(|keys| keys.concat() == written) {
                    vim.push(vec![yumete_cjk::actions::written(key)]);
                }
            }
            let insert: Vec<Vec<String>> = super::insert::INSERT_KEYS
                .iter()
                .filter(|(_, n, _)| n.name() == name)
                .map(|(key, _, _)| one(*key))
                .collect();
            out.push((name, action.help, helix, vim, insert));
        }
        out
    }

    /// The vim preset's keys that play a run of other keys rather than one
    /// command (`x` is `;` then `D`), as (key, run), internal spelling.
    pub fn vim_runs_for_docs() -> Vec<(String, String)> {
        let commands = Self::commands_for_docs();
        yumete_cjk::KeyPreset::Vim
            .table()
            .iter()
            .filter(|(key, _)| {
                let key = yumete_cjk::actions::written(key);
                !commands.iter().any(|(_, _, _, vim, _)| vim.iter().any(|k| k.concat() == key))
            })
            .map(|(key, run)| (yumete_cjk::actions::written(key), run.to_string()))
            .collect()
    }
}

// ── The groups ─────────────────────────────────────────────────────────────

/// **`g` — goto.**
///
/// **`g` 那一層的四個字母一個都不轉**（2026-10-13 定）：竪排時 `hjkl` 跟着版面
/// 轉，而 `gh gj gk gl` 不轉。原話：「竖排视觉方案只影响 hjkl 四个键……gh gj gk gl
/// 的逻辑性更大，所以不需要更改」。方向鍵仍舊轉：它們永遠按螢幕走，而竪排的「往
/// 下」就是順着這一行讀下去，也就是它的末尾。
///
/// The vim preset's own rows come before helix's for the keys they share
/// (`ge` is vim's word-end-back), and its keys helix has no use for come at
/// the end.
pub(super) static GOTO: Group = Group {
    name: "goto",
    pending: Pending::Goto,
    title: "hint.goto.title",
    path: "g",
    rows: &[
        // Select mode's own (helix's select keymap): the same places,
        // reached by extending.
        ch('g', NormalCmd::ExtendToFileStart, "hint.goto.start-of-file").when(Only::Selecting),
        ch('g', NormalCmd::GotoFileStart, "hint.goto.start-of-file"),
        note("<N>g", "hint.goto.line-n"),
        ch('e', NormalCmd::JumpPrevWordEnd, "action.jump-prev-word-end").when(Only::Vim),
        ch('E', NormalCmd::JumpPrevLongWordEnd, "action.jump-prev-long-word-end").when(Only::Vim),
        // helix's `ge` is the start of the last line (2026-11-01; it went to
        // the end of the file).
        ch('e', NormalCmd::ExtendToLastLine, "action.goto-last-line").when(Only::Selecting),
        ch('e', NormalCmd::GotoLastLine, "action.goto-last-line"),
        ch('h', NormalCmd::GotoLineStart, "hint.goto.line-start-or-end").when(Only::GLetter),
        ch('l', NormalCmd::GotoLineEnd, "hint.goto.line-start-or-end").when(Only::GLetter),
        quiet(Key::Up, NormalCmd::GotoLineStart).when(Only::HelixVertical),
        quiet(Key::Down, NormalCmd::GotoLineEnd).when(Only::HelixVertical),
        // vim's `gs` sleeps and its `gt` is the next tab: under vim these are
        // reached only when the preset plays them (`^` is `gs` there).
        ch('s', NormalCmd::GotoFirstNonwhitespace, "hint.goto.first-non-blank").when(Only::GLetter),
        ch('f', NormalCmd::GotoFile, "hint.goto.open-this-file"),
        ch('L', NormalCmd::FollowLink, "hint.goto.follow-link").flavor(FlavorItem::GotoExtras),
        // vim's `gw` formats, as `gq` with the cursor kept; the jump labels
        // are `gW` there, a flavour key (2026-11-02).
        ch('w', NormalCmd::OperatorFillKeepCursor, "action.operator-fill-keep-cursor").when(Only::Vim),
        ch('W', NormalCmd::GotoWord, "hint.goto.jump-labels").when(Only::Vim).flavor(FlavorItem::GotoExtras),
        ch('w', NormalCmd::ExtendToWord, "hint.goto.jump-labels").when(Only::Selecting),
        ch('w', NormalCmd::GotoWord, "hint.goto.jump-labels"),
        // `gx` opens the link with the system, as vim's: vim's own key, and a
        // flavour key under helix, which leaves it free.
        ch('x', NormalCmd::OpenLink, "hint.goto.open-link").when(Only::Vim),
        ch('x', NormalCmd::OpenLink, "hint.goto.open-link").flavor(FlavorItem::GotoExtras),
        // The two letter jumps (2026-11-02): `gz` by first letters (z as in
        // easymotion), `gZ` by pinyin (Z for 中). Both free in vim and helix.
        ch('z', NormalCmd::GotoWordByLetters, "hint.goto.seek-written").flavor(FlavorItem::GotoExtras),
        ch('Z', NormalCmd::GotoCharByPinyin, "hint.goto.seek-reading").flavor(FlavorItem::GotoExtras),
        // `ga` `gp` `gr` `gc` are vim's or Neovim's own (`index.txt`: ga, gp,
        // gr; Neovim's defaults: gc, gr*), so helix's are helix's alone.
        ch('a', NormalCmd::GotoLastAccessedFile, "hint.goto.last-file").when(Only::Helix),
        ch('n', NormalCmd::SearchNext, "hint.goto.next-match").when(Only::Vim),
        ch('N', NormalCmd::SearchPrev, "hint.goto.next-match").when(Only::Vim),
        ch('n', NormalCmd::GotoNextBuffer, "hint.goto.next-or-previous-buffer"),
        ch('p', NormalCmd::GotoPreviousBuffer, "hint.goto.next-or-previous-buffer").when(Only::Helix),
        ch('d', NormalCmd::GotoDefinition, "hint.goto.follow-note"),
        // helix's `gD` `gy` `gi` `gm` (2026-11-02). vim's `gD` is its global
        // declaration, the same question; its `gi` and `gm` are its own
        // (the vim `gi` row is above), and `gy` is a key vim leaves free.
        ch('D', NormalCmd::GotoDeclaration, "hint.goto.declaration"),
        ch('y', NormalCmd::GotoTypeDefinition, "hint.goto.type-definition").flavor(FlavorItem::HelixKeysUnderVim),
        ch('i', NormalCmd::GotoImplementation, "hint.goto.implementation").when(Only::Helix),
        ch('m', NormalCmd::GotoLastModifiedFile, "hint.goto.last-modified-file").when(Only::Helix),
        ch('r', NormalCmd::GotoReference, "hint.goto.references").when(Only::Helix),
        ch('/', NormalCmd::SearchSelection, "hint.goto.search-word").flavor(FlavorItem::GotoExtras),
        ch('j', NormalCmd::MoveNextZong, "hint.goto.by-file-line-vertical").when(Only::VimVertical),
        ch('k', NormalCmd::MovePrevZong, "hint.goto.by-file-line-vertical").when(Only::VimVertical),
        ch('j', NormalCmd::MoveVisualLineDown, "action.move-visual-line-down").when(Only::Vim),
        ch('k', NormalCmd::MoveVisualLineUp, "action.move-visual-line-up").when(Only::Vim),
        ch('j', NormalCmd::MoveLineDown, "hint.goto.by-file-line-vertical").when(Only::HelixVertical),
        ch('k', NormalCmd::MoveLineUp, "hint.goto.by-file-line-vertical").when(Only::HelixVertical),
        ch('j', NormalCmd::ExtendLineDown, "hint.goto.by-file-line").when(Only::Selecting),
        ch('k', NormalCmd::ExtendLineUp, "hint.goto.by-file-line").when(Only::Selecting),
        ch('j', NormalCmd::MoveLineDown, "hint.goto.by-file-line"),
        ch('k', NormalCmd::MoveLineUp, "hint.goto.by-file-line"),
        quiet(Key::Down, NormalCmd::MoveVisualLineDown).when(Only::FlatVim),
        quiet(Key::Up, NormalCmd::MoveVisualLineUp).when(Only::FlatVim),
        quiet(Key::Down, NormalCmd::MoveLineDown).when(Only::Flat),
        quiet(Key::Up, NormalCmd::MoveLineUp).when(Only::Flat),
        ch('.', NormalCmd::GotoLastModification, "hint.goto.last-change").flavor(FlavorItem::HelixKeysUnderVim),
        ch('t', NormalCmd::GotoWindowTop, "hint.goto.screen-top-middle-bottom").when(Only::GLetter),
        ch('c', NormalCmd::GotoWindowCenter, "hint.goto.screen-top-middle-bottom").when(Only::Helix),
        // vim 9.1's comment package and Neovim's (the flavour's `vim_comment_keys`).
        ch('c', NormalCmd::OperatorComment, "action.operator-comment").when(Only::Vim).flavor(FlavorItem::VimCommentKeys),
        ch('b', NormalCmd::GotoWindowBottom, "hint.goto.screen-top-middle-bottom").flavor(FlavorItem::HelixKeysUnderVim),
        // vim's own: none of these is a helix command.
        ch('v', NormalCmd::ReselectLastSelection, "hint.goto.reselect").when(Only::Vim),
        ch('i', NormalCmd::InsertAtLastInsert, "hint.goto.insert-again").when(Only::Vim),
        ch(';', NormalCmd::GotoOlderChange, "hint.goto.walk-changes").when(Only::Vim),
        ch(',', NormalCmd::GotoNewerChange, "hint.goto.walk-changes").when(Only::Vim),
        ch('u', NormalCmd::OperatorLowercase, "hint.goto.case-operators").when(Only::Vim),
        ch('U', NormalCmd::OperatorUppercase, "hint.goto.case-operators").when(Only::Vim),
        ch('~', NormalCmd::OperatorSwitchCase, "hint.goto.case-operators").when(Only::Vim),
        ch('q', NormalCmd::OperatorFill, "hint.goto.fill").when(Only::Vim),
        ch('J', NormalCmd::JoinSelectionsNoSpace, "hint.goto.join-without-a-space").when(Only::Vim),
        ch('`', NormalCmd::CaseMode, "action.case-mode").when(Only::Vim).flavor(FlavorItem::CaseMenu),
        // …and vim's own `` g` `` with the case menu off: to a mark.
        ch('`', NormalCmd::GotoMark, "action.goto-mark").when(Only::Vim),
    ],
    unknown: Unknown::Silent,
    sticky: false,
    ctrl_is_plain: false,
    before: Some(|e, key| e.take_sequence_argument(key)),
    after: Some(|e| {
        e.operator_count = None;
        e.sequence = None;
    }),
};

/// **`Space`** — helix's menu of the things that are not motions.
///
/// **空格 空格 / 空格 退格 翻頁**（2026-10-18 要的）。A reader holding a book
/// with one hand reaches for the space bar, which is what every reader on
/// every platform pages with; here the bar is this menu's own leader, so the
/// page turn is the bar pressed twice. The whole page, not the half `J`/`K`
/// take: `C-f`/`C-b` and `PageDown`/`PageUp` are the same turn, and these
/// two are the ones a hand finds without leaving the home row.
pub(super) static SPACE: Group = Group {
    name: "space",
    pending: Pending::Space,
    title: "hint.space.title",
    path: " ",
    rows: &[
        ch('f', NormalCmd::FilePicker, "hint.space.open-file"),
        ch('F', NormalCmd::FilePickerInCurrentDirectory, "hint.space.open-file-here"),
        ch('e', NormalCmd::FileExplorer, "hint.space.explorer"),
        ch('E', NormalCmd::FileExplorerInCurrentBufferDirectory, "hint.space.explorer-here").flavor(FlavorItem::ExplorerHere),
        ch('b', NormalCmd::BufferPicker, "hint.space.switch-buffer"),
        ch('j', NormalCmd::JumplistPicker, "hint.space.jumplist"),
        ch('\'', NormalCmd::LastPicker, "hint.space.last-picker"),
        ch('g', NormalCmd::ChangedFilePicker, "hint.space.changed-files"),
        ch('y', NormalCmd::YankToClipboard, "hint.space.copy-to-clipboard"),
        ch('Y', NormalCmd::YankMainSelectionToClipboard, "hint.space.copy-main-to-clipboard"),
        ch('p', NormalCmd::PasteClipboardAfter, "hint.space.paste-from-clipboard"),
        ch('P', NormalCmd::PasteClipboardBefore, "hint.space.paste-before"),
        ch('R', NormalCmd::ReplaceSelectionsWithClipboard, "hint.space.replace-from-clipboard"),
        ch('s', NormalCmd::LspOrSyntaxSymbolPicker, "hint.space.symbols"),
        ch('S', NormalCmd::LspOrSyntaxWorkspaceSymbolPicker, "hint.space.project-symbols"),
        ch('n', NormalCmd::Dictionary, "hint.space.dictionary").flavor(FlavorItem::Dictionary),
        ch('N', NormalCmd::DictionaryPanel, "hint.space.dictionary-panel").flavor(FlavorItem::Dictionary),
        ch('k', NormalCmd::Hover, "hint.space.what-is-this"),
        ch('K', NormalCmd::HoverPanel, "hint.space.what-is-this-panel").flavor(FlavorItem::PopupPanelPairs),
        ch('d', NormalCmd::DiagnosticsPicker, "hint.space.diagnostics"),
        ch('D', NormalCmd::WorkspaceDiagnosticsPicker, "hint.space.project-diagnostics"),
        ch('l', NormalCmd::ShowDiagnostic, "hint.space.problems").flavor(FlavorItem::PopupPanelPairs),
        ch('L', NormalCmd::ShowDiagnosticPanel, "hint.space.problems-panel").flavor(FlavorItem::PopupPanelPairs),
        ch('x', NormalCmd::ShowChange, "hint.space.change").flavor(FlavorItem::PopupPanelPairs),
        ch('X', NormalCmd::ShowChangePanel, "hint.space.change-panel").flavor(FlavorItem::PopupPanelPairs),
        ch('r', NormalCmd::RenameSymbol, "hint.space.rename"),
        ch('h', NormalCmd::SelectReferencesToSymbolUnderCursor, "hint.space.select-references"),
        ch('a', NormalCmd::CodeAction, "hint.space.actions"),
        ch('i', NormalCmd::ShowRecord, "hint.space.record").flavor(FlavorItem::PopupPanelPairs),
        ch('I', NormalCmd::ShowRecordPanel, "hint.space.record-panel").flavor(FlavorItem::PopupPanelPairs),
        ch('w', NormalCmd::WindowMode, "hint.region.title"),
        ch('t', NormalCmd::TableMode, "hint.space.table").flavor(FlavorItem::Tables),
        ch('"', NormalCmd::PasteMenu, "menu.paste.title").flavor(FlavorItem::RegistersMenu),
        ch('c', NormalCmd::ToggleComments, "hint.space.comment-line"),
        ch('C', NormalCmd::ToggleBlockComments, "hint.space.comment-block"),
        key(Key::Alt('c'), NormalCmd::ToggleLineComments, "hint.space.comment-lines-only"),
        ch('m', NormalCmd::ConflictMode, "hint.conflict.title").flavor(FlavorItem::MergeConflicts),
        ch('o', NormalCmd::OpenRecentMode, "hint.space.open-recent").flavor(FlavorItem::RecentFiles),
        ch('z', NormalCmd::YumeMode, "hint.space.yume").flavor(FlavorItem::InputMethodMenu),
        ch('/', NormalCmd::GlobalSearch, "hint.space.advanced-search"),
        ch('?', NormalCmd::CommandPalette, "hint.space.all-commands"),
        ch(' ', NormalCmd::PageDown, "hint.space.page-down").flavor(FlavorItem::SpacePaging),
        key(Key::Backspace, NormalCmd::PageUp, "hint.space.page-up").flavor(FlavorItem::SpacePaging),
        ch('q', NormalCmd::Quit, "hint.space.quit").flavor(FlavorItem::SpaceQuit),
    ],
    unknown: Unknown::Silent,
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

/// **`]` and `[` — 「the next one of these」**, and which way. Helix keeps
/// the walks between things a document *has* on the brackets.
///
/// Warning: **a code file's three are offered in code only** (2026-10-06,
/// seen in a drawn frame). Offered everywhere, a code file's menu showed
/// `c` twice and a manuscript's offered an `f` that never finds anything.
/// They still answer anywhere; there is just nothing to find.
pub(super) static RIGHT_BRACKET: Group = Group {
    name: "right_bracket",
    pending: Pending::Hop { forward: true },
    title: "hint.hop.next",
    path: "]",
    rows: &[
        // vim's `]m` `[m` are its member motions: under vim the conflicts
        // are on `Space m` only.
        ch('m', NormalCmd::GotoNextConflict, "hint.hop.conflict").flavor(FlavorItem::MergeConflicts).when(Only::Helix),
        ch('g', NormalCmd::GotoNextChange, "hint.hop.change-ends").flavor(FlavorItem::HelixKeysUnderVim),
        ch('G', NormalCmd::GotoLastChange, "hint.hop.change-ends").flavor(FlavorItem::HelixKeysUnderVim),
        ch('d', NormalCmd::GotoNextDiag, "hint.hop.problem"),
        ch('D', NormalCmd::GotoLastDiag, "hint.hop.problem"),
        // vim's `]f` is `gf`, Neovim's `]t` a tag: helix's alone.
        ch('f', NormalCmd::GotoNextFunction, "hint.hop.function").listed(Only::Code).when(Only::Helix),
        ch('t', NormalCmd::GotoNextClass, "hint.hop.class").listed(Only::Code).when(Only::Helix),
        // vim's `]c` is the next change, which yumete has (helix's `]g`);
        // helix's comment is helix's only.
        ch('c', NormalCmd::GotoNextChange, "hint.hop.change").when(Only::Vim),
        ch('c', NormalCmd::GotoNextComment, "hint.hop.comment").listed(Only::Code),
        // helix's (2026-11-02). Neovim's `]a` is the argument list and its
        // `]T` the last tag: those two are helix's alone; vim borrows `]e`
        // `]x` with the rest of helix's free keys.
        ch('a', NormalCmd::GotoNextParameter, "hint.hop.parameter").listed(Only::Code).when(Only::Helix),
        ch('e', NormalCmd::GotoNextEntry, "hint.hop.entry").listed(Only::Code).flavor(FlavorItem::HelixKeysUnderVim),
        ch('T', NormalCmd::GotoNextTest, "hint.hop.test").listed(Only::Code).when(Only::Helix),
        ch('x', NormalCmd::GotoNextXmlElement, "hint.hop.element").listed(Only::Code).flavor(FlavorItem::HelixKeysUnderVim),
        // Warning: **段落那一行借的是 `:keymap-actions` 的名字**：同一件事在兩張
        // 表上，寫成兩句話只會讓它們慢慢說岔。
        // vim's `]p` pastes with the indent adjusted; `]P` is `[p`.
        ch('p', NormalCmd::VimPasteAfterIndented, "action.vim-paste-after-indented").when(Only::Vim),
        ch('P', NormalCmd::VimPasteBeforeIndented, "action.vim-paste-before-indented").when(Only::Vim),
        ch('p', NormalCmd::GotoNextParagraph, "action.goto-next-paragraph"),
        ch('z', NormalCmd::GotoFoldEnd, "hint.fold.end").when(Only::Vim),
        ch(' ', NormalCmd::AddNewlineBelow, "hint.hop.blank-below"),
    ],
    unknown: Unknown::NoSuchCombination,
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

pub(super) static LEFT_BRACKET: Group = Group {
    name: "left_bracket",
    pending: Pending::Hop { forward: false },
    title: "hint.hop.previous",
    path: "[",
    rows: &[
        ch('m', NormalCmd::GotoPrevConflict, "hint.hop.conflict").flavor(FlavorItem::MergeConflicts).when(Only::Helix),
        ch('g', NormalCmd::GotoPrevChange, "hint.hop.change-ends").flavor(FlavorItem::HelixKeysUnderVim),
        ch('G', NormalCmd::GotoFirstChange, "hint.hop.change-ends").flavor(FlavorItem::HelixKeysUnderVim),
        ch('d', NormalCmd::GotoPrevDiag, "hint.hop.problem"),
        ch('D', NormalCmd::GotoFirstDiag, "hint.hop.problem"),
        ch('f', NormalCmd::GotoPrevFunction, "hint.hop.function").listed(Only::Code).when(Only::Helix),
        ch('t', NormalCmd::GotoPrevClass, "hint.hop.class").listed(Only::Code).when(Only::Helix),
        // vim's `]c` is the next change, which yumete has (helix's `]g`);
        // helix's comment is helix's only.
        ch('c', NormalCmd::GotoPrevChange, "hint.hop.change").when(Only::Vim),
        ch('c', NormalCmd::GotoPrevComment, "hint.hop.comment").listed(Only::Code),
        ch('a', NormalCmd::GotoPrevParameter, "hint.hop.parameter").listed(Only::Code).when(Only::Helix),
        ch('e', NormalCmd::GotoPrevEntry, "hint.hop.entry").listed(Only::Code).flavor(FlavorItem::HelixKeysUnderVim),
        ch('T', NormalCmd::GotoPrevTest, "hint.hop.test").listed(Only::Code).when(Only::Helix),
        ch('x', NormalCmd::GotoPrevXmlElement, "hint.hop.element").listed(Only::Code).flavor(FlavorItem::HelixKeysUnderVim),
        ch('p', NormalCmd::VimPasteBeforeIndented, "action.vim-paste-before-indented").when(Only::Vim),
        ch('P', NormalCmd::VimPasteBeforeIndented, "action.vim-paste-before-indented").when(Only::Vim),
        ch('p', NormalCmd::GotoPrevParagraph, "action.goto-prev-paragraph"),
        ch('z', NormalCmd::GotoFoldStart, "hint.fold.start").when(Only::Vim),
        ch(' ', NormalCmd::AddNewlineAbove, "hint.hop.blank-above"),
    ],
    unknown: Unknown::NoSuchCombination,
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

/// **`z` — where the cursor's line goes on the screen**, and helix's other
/// view keys. The menu lists the three this group is for; the scrolling and
/// paging keys are helix's (`z j`, `z C-d`…) and answer without a line.
const VIEW_ROWS: &[Row] = &[
    ch('z', NormalCmd::AlignViewCenter, "hint.aim.middle"),
    // vim's folds (2026-11-03, measured in nvim); `zc` `zj` `zk` are
    // helix's view keys under helix.
    ch('f', NormalCmd::CreateFold, "hint.fold.create").when(Only::Vim),
    ch('F', NormalCmd::CreateFoldLines, "hint.fold.create-lines").when(Only::Vim),
    ch('o', NormalCmd::OpenFold, "hint.fold.open").when(Only::Vim),
    ch('O', NormalCmd::OpenFoldsRecursively, "hint.fold.open-all-here").when(Only::Vim),
    ch('c', NormalCmd::CloseFold, "hint.fold.close").when(Only::Vim),
    ch('C', NormalCmd::CloseFoldsRecursively, "hint.fold.close-all-here").when(Only::Vim),
    ch('a', NormalCmd::ToggleFold, "hint.fold.toggle").when(Only::Vim),
    ch('A', NormalCmd::ToggleFoldsRecursively, "hint.fold.toggle-all-here").when(Only::Vim),
    ch('v', NormalCmd::OpenFoldsToCursor, "hint.fold.view").when(Only::Vim),
    ch('R', NormalCmd::OpenAllFolds, "hint.fold.open-every").when(Only::Vim),
    ch('M', NormalCmd::CloseAllFolds, "hint.fold.close-every").when(Only::Vim),
    ch('d', NormalCmd::DeleteFold, "hint.fold.delete").when(Only::Vim),
    ch('D', NormalCmd::DeleteFoldsRecursively, "hint.fold.delete-all-here").when(Only::Vim),
    ch('E', NormalCmd::DeleteAllFolds, "hint.fold.delete-every").when(Only::Vim),
    ch('j', NormalCmd::GotoNextFold, "hint.fold.next").when(Only::Vim),
    ch('k', NormalCmd::GotoPrevFold, "hint.fold.prev").when(Only::Vim),
    ch('n', NormalCmd::DisableFolds, "hint.fold.off").when(Only::Vim),
    ch('N', NormalCmd::EnableFolds, "hint.fold.on").when(Only::Vim),
    ch('i', NormalCmd::ToggleFoldsEnabled, "hint.fold.on-off").when(Only::Vim),
    quiet(Key::Char('c'), NormalCmd::AlignViewCenter).when(Only::Helix),
    ch('t', NormalCmd::AlignViewTop, "hint.aim.top"),
    ch('b', NormalCmd::AlignViewBottom, "hint.aim.bottom"),
    quiet(Key::Char('j'), NormalCmd::ScrollDown).when(Only::Helix),
    quiet(Key::Down, NormalCmd::ScrollDown),
    quiet(Key::Char('k'), NormalCmd::ScrollUp).when(Only::Helix),
    quiet(Key::Up, NormalCmd::ScrollUp),
    quiet(Key::Ctrl('d'), NormalCmd::PageCursorHalfDown),
    quiet(Key::Char(' '), NormalCmd::PageCursorHalfDown),
    quiet(Key::Ctrl('u'), NormalCmd::PageCursorHalfUp),
    quiet(Key::Backspace, NormalCmd::PageCursorHalfUp),
    quiet(Key::Ctrl('f'), NormalCmd::PageDown),
    quiet(Key::PageDown, NormalCmd::PageDown),
    quiet(Key::Ctrl('b'), NormalCmd::PageUp),
    quiet(Key::PageUp, NormalCmd::PageUp),
    quiet(Key::Char('/'), NormalCmd::Search),
    quiet(Key::Char('?'), NormalCmd::Rsearch),
    quiet(Key::Char('n'), NormalCmd::SearchNext).when(Only::Helix),
    quiet(Key::Char('N'), NormalCmd::SearchPrev).when(Only::Helix),
];

pub(super) static VIEW: Group = Group {
    name: "view",
    pending: Pending::Aim,
    title: "hint.aim.title",
    path: "z",
    rows: VIEW_ROWS,
    unknown: Unknown::NoSuchCombination,
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

/// `Z` — the same group, open until `Esc` (helix's sticky view mode).
pub(super) static STICKY_VIEW: Group = Group {
    name: "sticky_view",
    pending: Pending::AimStuck,
    title: "hint.aim.title",
    path: "Z",
    rows: VIEW_ROWS,
    unknown: Unknown::NoSuchCombination,
    sticky: true,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

/// **`m` — 「這一對」**, and what to do with it. Every key after the first
/// waits for a character, a pair or an object, and lists its own.
pub(super) static MATCH: Group = Group {
    name: "match",
    pending: Pending::Match,
    title: "hint.match.title",
    path: "m",
    rows: &[
        ch('m', NormalCmd::MatchBrackets, "hint.match.pair"),
        ch('i', NormalCmd::SelectTextobjectInner, "hint.match.inside"),
        ch('a', NormalCmd::SelectTextobjectAround, "hint.match.around"),
        ch('s', NormalCmd::SurroundAdd, "hint.match.surround"),
        ch('d', NormalCmd::SurroundDelete, "hint.match.take-off"),
        ch('r', NormalCmd::SurroundReplace, "hint.change"),
    ],
    unknown: Unknown::NoSuchCombination,
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

/// **`C-w` / `Space w` — the regions** (2026-09-30, helix's
/// `keymap/default.rs:193` and `:260`, one group behind two doors).
///
/// 原話：「space+w is the parent command for 『region』 operations (helix
/// uses 『window』)。」`e`/`i`/`E`/`I` are this editor's own: helix has no
/// sidebars.
pub(super) static WINDOW: Group = Group {
    name: "window",
    pending: Pending::Region,
    title: "hint.region.title",
    path: "\u{17}",
    rows: &[
        ch('w', NormalCmd::RotateView, "hint.region.next"),
        // vim's (2026-11-03, measured in nvim): `W` `t` `b` `p` go to a
        // window; `x` `r` `R` and `H` `J` `K` `L` rearrange them; `c`
        // closes all but the last; `S` `F` `^` `:` as vim has them.
        ch('W', NormalCmd::RotateViewReverse, "hint.region.previous").when(Only::Vim),
        ch('t', NormalCmd::JumpViewFirst, "hint.region.first").when(Only::Vim),
        ch('b', NormalCmd::JumpViewLast, "hint.region.last").when(Only::Vim),
        ch('p', NormalCmd::JumpViewLastAccessed, "hint.region.last-accessed").when(Only::Vim),
        ch('x', NormalCmd::ExchangeView, "hint.region.exchange").when(Only::Vim),
        ch('r', NormalCmd::RotateViewsForward, "hint.region.rotate").when(Only::Vim),
        ch('R', NormalCmd::RotateViewsBackward, "hint.region.rotate-back").when(Only::Vim),
        ch('H', NormalCmd::MoveViewFarLeft, "hint.region.to-edge").when(Only::Vim),
        ch('J', NormalCmd::MoveViewFarDown, "hint.region.to-edge").when(Only::Vim),
        ch('K', NormalCmd::MoveViewFarUp, "hint.region.to-edge").when(Only::Vim),
        ch('L', NormalCmd::MoveViewFarRight, "hint.region.to-edge").when(Only::Vim),
        ch('c', NormalCmd::VimCloseWindow, "hint.region.close-not-last").when(Only::Vim),
        ch('S', NormalCmd::Hsplit, "hint.region.split").when(Only::Vim),
        ch('F', NormalCmd::GotoFileHsplit, "hint.region.file-below").when(Only::Vim),
        ch('^', NormalCmd::GotoLastAccessedFileHsplit, "hint.region.last-file-below").when(Only::Vim),
        ch(':', NormalCmd::CommandMode, "action.command-mode").when(Only::Vim),
        ch('h', NormalCmd::JumpViewLeft, "hint.region.go"),
        ch('j', NormalCmd::JumpViewDown, "hint.region.go"),
        ch('k', NormalCmd::JumpViewUp, "hint.region.go"),
        ch('l', NormalCmd::JumpViewRight, "hint.region.go"),
        quiet(Key::Left, NormalCmd::JumpViewLeft),
        quiet(Key::Down, NormalCmd::JumpViewDown),
        quiet(Key::Up, NormalCmd::JumpViewUp),
        quiet(Key::Right, NormalCmd::JumpViewRight),
        ch('e', NormalCmd::ToggleLeftSidebar, "hint.region.toggle-bar").flavor(FlavorItem::Sidebars),
        ch('i', NormalCmd::ToggleRightSidebar, "hint.region.toggle-bar").flavor(FlavorItem::Sidebars),
        ch('E', NormalCmd::GotoLeftSidebar, "hint.region.into-bar").flavor(FlavorItem::Sidebars),
        ch('I', NormalCmd::GotoRightSidebar, "hint.region.into-bar").flavor(FlavorItem::Sidebars),
        ch('s', NormalCmd::Hsplit, "hint.region.split"),
        ch('v', NormalCmd::Vsplit, "hint.region.split-beside"),
        // vim's `C-w n` is the blank window below; helix's is a group.
        ch('n', NormalCmd::HsplitNew, "hint.region.new").when(Only::Vim),
        ch('n', NormalCmd::WindowNewMode, "hint.region.new"),
        ch('q', NormalCmd::Wclose, "hint.region.close"),
        ch('o', NormalCmd::Wonly, "hint.region.only"),
        // helix's (2026-11-02). vim's `C-w t` is the top window and its
        // `H` `J` `K` `L` move a window to an edge, so these are helix's;
        // `C-w f` is the same in both.
        ch('t', NormalCmd::TransposeView, "hint.region.transpose").when(Only::Helix),
        ch('H', NormalCmd::SwapViewLeft, "hint.region.swap").when(Only::Helix),
        ch('J', NormalCmd::SwapViewDown, "hint.region.swap").when(Only::Helix),
        ch('K', NormalCmd::SwapViewUp, "hint.region.swap").when(Only::Helix),
        ch('L', NormalCmd::SwapViewRight, "hint.region.swap").when(Only::Helix),
        ch('f', NormalCmd::GotoFileHsplit, "hint.region.file-below"),
        ch('F', NormalCmd::GotoFileVsplit, "hint.region.file-beside").when(Only::Helix),
    ],
    unknown: Unknown::Silent,
    sticky: false,
    ctrl_is_plain: true,
    before: None,
    after: None,
};

/// `C-w n`, helix's group of the same letter.
pub(super) static WINDOW_NEW: Group = Group {
    name: "window_new",
    pending: Pending::RegionNew,
    title: "hint.region.new",
    path: "\u{17}n",
    rows: &[
        ch('s', NormalCmd::HsplitNew, "hint.region.new-below"),
        ch('v', NormalCmd::VsplitNew, "hint.region.new-beside"),
    ],
    unknown: Unknown::Silent,
    sticky: false,
    ctrl_is_plain: true,
    before: None,
    after: None,
};

/// **`` ` `` — 「不改它說什麼，只改它長什麼樣」** (§5.2.3 ②): case, width, and
/// which 繁 or 簡.
///
/// The last line is `hint.vi.backtick` — 「vi 的記號：M 記、`'` 回去」. It is a
/// line in a menu the reader opened, not a guess about why a key was
/// pressed, which is why it outlived the phrasebook (2026-10-08).
pub(super) static CASE: Group = Group {
    name: "case",
    pending: Pending::Case,
    title: "hint.case.title",
    path: "`",
    rows: &[
        ch('l', NormalCmd::SwitchToLowercase, "hint.case.lower"),
        ch('u', NormalCmd::SwitchToUppercase, "hint.case.upper"),
        ch('`', NormalCmd::SwitchCase, "hint.case.switch"),
        ch('s', NormalCmd::ConvertToSimplified, "hint.case.to-s"),
        ch('t', NormalCmd::ConvertToTraditional, "hint.case.to-t"),
        ch('w', NormalCmd::ConvertToTaiwanTraditional, "hint.case.to-tw"),
        ch('f', NormalCmd::SwitchToFullwidth, "hint.case.to-full"),
        ch('h', NormalCmd::SwitchToHalfwidth, "hint.case.to-half"),
        ch('k', NormalCmd::ConvertToHongKongTraditional, "hint.case.to-hk"),
        ch('c', NormalCmd::ConvertToMainlandTraditional, "hint.case.to-c"),
        ch('g', NormalCmd::ConvertToClassicalTraditional, "hint.case.to-g"),
        ch('j', NormalCmd::ConvertToJapanese, "hint.case.to-jp"),
        note("", "hint.vi.backtick"),
    ],
    unknown: Unknown::NoSuchCombination,
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

/// `空格 m` — what to keep of the merge conflict under the cursor.
pub(super) static CONFLICT: Group = Group {
    name: "conflict",
    pending: Pending::Conflict,
    title: "hint.conflict.title",
    path: " m",
    rows: &[
        ch('o', NormalCmd::ConflictKeepOurs, "hint.conflict.ours"),
        ch('t', NormalCmd::ConflictKeepTheirs, "hint.conflict.theirs"),
        ch('b', NormalCmd::ConflictKeepBoth, "hint.conflict.both"),
    ],
    unknown: Unknown::NoSuchCombination,
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

/// **`空格 o` — 開上次的那一組**（2026-10-18 作者定）。
///
/// 三件事，三份不同的清單——這也是別家編輯器一致的形狀（VS Code 的
/// 「Open Recent」列文件夾、`⌘P` 列檔，而「重開上次的窗」是第三件；vim 的
/// `:oldfiles` 和 `:mksession` 同理）。**兩個檔的挑選器而不是一個帶開關的**，
/// 因為挑選器一開門就是插入態，數字鍵在裏面是查詢裏的字，不是鍵。
pub(super) static OPEN: Group = Group {
    name: "open",
    pending: Pending::Open,
    title: "hint.open.title",
    path: " o",
    rows: &[
        ch('f', NormalCmd::RecentFilePicker, "hint.open.recent-files"),
        ch('g', NormalCmd::GlobalRecentFilePicker, "hint.open.recent-files-global"),
        ch('p', NormalCmd::ProjectPicker, "hint.open.recent-projects"),
        ch('r', NormalCmd::ResumeSession, "hint.open.resume"),
    ],
    unknown: Unknown::NoSuchCombination,
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

/// **`空格 z` — 輸入法那一組**（2026-10-17 作者定）。
///
/// 原話：「In Yume, we have many shortcuts too with (shift + control + a
/// letter). I want them to be aligned as much as possible.」So the letter
/// under `z` is **Yume's own ⌃⇧ letter** wherever Yume has one (`m` 上屏方式,
/// `y` 輸入預測, `i` 選方案, `n` 下一個方案, `/` 注解, `3`–`9` 一頁幾條), and
/// the value under it is a letter of that value's own name.
///
/// Warning: **The letters Yume uses and yumete has no answer for stay empty**
/// — `O U D L H K J A .` and `0`. Lining the two up is only worth doing if
/// the day yumete grows one of those, the letter is already the same on both
/// sides and nobody's hand has to change.
pub(super) static YUME: Group = Group {
    name: "yume",
    pending: Pending::Yume,
    title: "hint.yume.title",
    path: " z",
    rows: &[
        ch('z', NormalCmd::YumeToggle, "hint.yume.toggle"),
        ch('m', NormalCmd::YumeCommitMode, "hint.yume.commit"),
        ch('p', NormalCmd::YumePanelMode, "hint.yume.panel"),
        ch('e', NormalCmd::YumePreeditMode, "hint.yume.preedit"),
        ch('y', NormalCmd::YumeTogglePrediction, "hint.yume.predict"),
        ch('i', NormalCmd::YumePickScheme, "hint.yume.scheme"),
        ch('n', NormalCmd::YumeNextScheme, "hint.yume.next-scheme"),
        ch('3', NormalCmd::YumeMenuSize3, "hint.yume.menu-size"),
        ch('4', NormalCmd::YumeMenuSize4, "hint.yume.menu-size"),
        ch('5', NormalCmd::YumeMenuSize5, "hint.yume.menu-size"),
        ch('6', NormalCmd::YumeMenuSize6, "hint.yume.menu-size"),
        ch('7', NormalCmd::YumeMenuSize7, "hint.yume.menu-size"),
        ch('8', NormalCmd::YumeMenuSize8, "hint.yume.menu-size"),
        ch('9', NormalCmd::YumeMenuSize9, "hint.yume.menu-size"),
        ch('/', NormalCmd::YumeNextAnnotation, "hint.yume.annotation"),
        ch('t', NormalCmd::YumeInstalledTable, "hint.yume.table"),
        ch('w', NormalCmd::YumeTableSource, "hint.yume.which"),
        ch('W', NormalCmd::YumeDataLocation, "hint.yume.where"),
    ],
    unknown: Unknown::SayKeys("hint.yume.after-z"),
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

/// The values under the three letters of `空格 z` that have any.
pub(super) static YUME_COMMIT: Group = Group {
    name: "yume_commit",
    pending: Pending::YumeValue('m'),
    title: "hint.yume.title",
    path: " zm",
    rows: &[
        ch('0', NormalCmd::YumeCommitDefault, "hint.yume.commit-default"),
        ch('d', NormalCmd::YumeCommitDelayed, "hint.yume.commit-delayed"),
        ch('u', NormalCmd::YumeCommitUnique, "hint.yume.commit-unique"),
        ch('f', NormalCmd::YumeCommitFluency, "hint.yume.commit-fluency"),
    ],
    unknown: Unknown::SayKeys("hint.yume.after-z"),
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

pub(super) static YUME_PANEL: Group = Group {
    name: "yume_panel",
    pending: Pending::YumeValue('p'),
    title: "hint.yume.title",
    path: " zp",
    rows: &[
        ch('b', NormalCmd::YumePanelBox, "hint.yume.panel-box"),
        ch('m', NormalCmd::YumePanelBottom, "hint.yume.panel-bottom"),
        ch('i', NormalCmd::YumePanelInline, "hint.yume.panel-inline"),
    ],
    unknown: Unknown::SayKeys("hint.yume.after-z"),
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

pub(super) static YUME_PREEDIT: Group = Group {
    name: "yume_preedit",
    pending: Pending::YumeValue('e'),
    title: "hint.yume.title",
    path: " ze",
    rows: &[
        ch('o', NormalCmd::YumePreeditOff, "hint.yume.preedit-off"),
        ch('c', NormalCmd::YumePreeditComposition, "hint.yume.preedit-composition"),
        ch('p', NormalCmd::YumePreeditPreview, "hint.yume.preedit-preview"),
    ],
    unknown: Unknown::SayKeys("hint.yume.after-z"),
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

/// **`空格 t` — the table group**, and what it offers **wherever the cursor
/// is**.
///
/// **The way in comes first.** The group is pressed most often by somebody
/// who is *not* in a table yet, so the six that work anywhere head every
/// list, and on a page with no table under the cursor they are the whole
/// list. `o b t` are three answers to one question — how is this table
/// drawn — from shallow to deep, and `q` steps back out of the deepest
/// (2026-09-30: 「这五个是一个系列」; `f` folded into `b` on 2026-10-21).
///
/// Then what this table can do, by where it is: a fenced block is read where
/// it lies (#216), so the keys that rewrite a file are not offered there,
/// because they are refused; a delimited file's columns are the schema's.
/// **A list that offers a key the editor would refuse is the one thing these
/// lists may not be.** The same key appears once per kind of table, each with
/// the line that kind of table says; every one of them is the table code's
/// own answer to that key, which says why a key is refused where it is.
///
/// 格子多寬 (`w`) belongs to the pane alone since 2026-10-22: 源碼 and 基本
/// draw every cell whole and may not do otherwise.
pub(super) static TABLE: Group = Group {
    name: "table",
    pending: Pending::Table,
    title: "hint.table.title",
    path: " t",
    rows: &[
        ch('o', NormalCmd::TableSourceView, "hint.table.back-to-prose"),
        ch('b', NormalCmd::TableBasicView, "hint.table.operate-it"),
        ch('t', NormalCmd::TableWindowView, "hint.table.whole-window"),
        ch('q', NormalCmd::TableLeaveWindow, "hint.table.leave-the-window"),
        ch(']', NormalCmd::GotoNextTable, "hint.table.next-or-previous"),
        ch('[', NormalCmd::GotoPrevTable, "hint.table.next-or-previous"),
        // 2026-10-02 定：命令是 `:convert-table`，鍵位摺在這一組底下。
        ch('x', NormalCmd::TableConvertMode, "hint.table.convert"),
        ch('w', NormalCmd::TableCellWidth, "hint.table.fold-wide-cells").listed(Only::Pane),
        // In a fenced block.
        ch('/', NormalCmd::TableSearchColumns, "hint.table.search-columns").listed(Only::Block),
        ch('g', NormalCmd::TableGotoCell, "hint.table.go-to-cell").listed(Only::Block),
        ch('y', NormalCmd::TableYankColumn, "hint.table.yank-or-replace-column").listed(Only::Block),
        ch('v', NormalCmd::TableReplaceColumn, "hint.table.yank-or-replace-column").listed(Only::Block),
        note("1s 1S", "hint.table.sort-by-column").listed(Only::Block),
        // In a Markdown table.
        ch('/', NormalCmd::TableSearchColumns, "hint.table.search-columns").listed(Only::Md),
        ch('g', NormalCmd::TableGotoCell, "hint.table.go-to-cell").listed(Only::Md),
        ch('r', NormalCmd::TableAddRowBelow, "hint.table.add-row").listed(Only::Md),
        ch('R', NormalCmd::TableAddRowAbove, "hint.table.add-row").listed(Only::Md),
        ch('c', NormalCmd::TableAddColumnRight, "hint.table.add-column").listed(Only::Md),
        ch('C', NormalCmd::TableAddColumnLeft, "hint.table.add-column").listed(Only::Md),
        ch('d', NormalCmd::TableDeleteRow, "hint.table.delete-row-or-column").listed(Only::Md),
        ch('D', NormalCmd::TableDeleteColumn, "hint.table.delete-row-or-column").listed(Only::Md),
        ch('j', NormalCmd::TableMoveRowDown, "hint.table.move-row").listed(Only::Md),
        ch('k', NormalCmd::TableMoveRowUp, "hint.table.move-row").listed(Only::Md),
        ch('h', NormalCmd::TableMoveColumnLeft, "hint.table.move-column").listed(Only::Md),
        ch('l', NormalCmd::TableMoveColumnRight, "hint.table.move-column").listed(Only::Md),
        ch('y', NormalCmd::TableYankColumn, "hint.table.yank-or-paste-column").listed(Only::Md),
        ch('p', NormalCmd::TablePasteColumnRight, "hint.table.yank-or-paste-column").listed(Only::Md),
        ch('P', NormalCmd::TablePasteColumnLeft, "hint.table.yank-or-paste-column").listed(Only::Md),
        ch('v', NormalCmd::TableReplaceColumn, "hint.table.replace-column").listed(Only::Md),
        note("1s 1S", "hint.table.sort-by-column").listed(Only::Md),
        ch('<', NormalCmd::TableAlignLeft, "hint.table.align-column").listed(Only::Md),
        ch('=', NormalCmd::TableAlignCenter, "hint.table.align-column").listed(Only::Md),
        ch('>', NormalCmd::TableAlignRight, "hint.table.align-column").listed(Only::Md),
        // `F`, not `f`: the lowercase letter was 完整表格 (#283), and a
        // capital is what a command that rewrites every row should have.
        ch('F', NormalCmd::TableLineUp, "hint.table.line-it-up").listed(Only::Md),
        ch('i', NormalCmd::TableRecordPanel, "hint.table.detail-panel").listed(Only::Md),
        // In a delimited file.
        ch('/', NormalCmd::TableSearchColumns, "hint.table.search-columns").listed(Only::File),
        ch('g', NormalCmd::TableGotoCell, "hint.table.go-to-cell").listed(Only::File),
        note("1s 1S", "hint.table.sort-by-column").listed(Only::File),
        ch('r', NormalCmd::TableAddRowBelow, "hint.table.add-row").listed(Only::File),
        ch('R', NormalCmd::TableAddRowAbove, "hint.table.add-row").listed(Only::File),
        ch('d', NormalCmd::TableDeleteRow, "hint.table.delete-row").listed(Only::File),
        ch('j', NormalCmd::TableMoveRowDown, "hint.table.move-row").listed(Only::File),
        ch('k', NormalCmd::TableMoveRowUp, "hint.table.move-row").listed(Only::File),
        ch('y', NormalCmd::TableYankColumn, "hint.table.yank-or-paste-column").listed(Only::File),
        ch('p', NormalCmd::TablePasteColumnRight, "hint.table.yank-or-paste-column").listed(Only::File),
        ch('P', NormalCmd::TablePasteColumnLeft, "hint.table.yank-or-paste-column").listed(Only::File),
        ch('v', NormalCmd::TableReplaceColumn, "hint.table.replace-column").listed(Only::File),
        ch('H', NormalCmd::TableFirstRowIsData, "hint.table.first-row-is-data").listed(Only::File),
        ch('i', NormalCmd::TableRecordPanel, "hint.table.detail-panel").listed(Only::File),
        // The sort keys take their columns from the count typed before them
        // (`Space t 1s`), so the menu says them as a note above.
        quiet(Key::Char('s'), NormalCmd::TableSortAscending),
        quiet(Key::Char('S'), NormalCmd::TableSortDescending),
    ],
    unknown: Unknown::Hand(Editor::table_structure),
    sticky: false,
    ctrl_is_plain: false,
    before: Some(Editor::table_group_takes),
    after: Some(|e| {
        e.sequence = None;
        e.sort_keys.clear();
    }),
};

/// `空格 t x` 底下那三種格式。**小寫是轉換，大寫是把剪貼板貼成那一種**
/// （2026-10-02 定）——和這一組的習慣一致：大寫是「另一個方向」或者「重的
/// 那一個」。名字直接寫：它們就是命令上打的那三個詞。
pub(super) static TABLE_CONVERT: Group = Group {
    name: "table_convert",
    pending: Pending::TableConvert,
    title: "hint.table.convert",
    path: " tx",
    rows: &[
        ch('p', NormalCmd::ConvertTableToPipe, "hint.table.convert-pipe"),
        ch('P', NormalCmd::PasteTableAsPipe, "hint.table.convert-pipe"),
        ch('c', NormalCmd::ConvertTableToCsv, "hint.table.convert-csv"),
        ch('C', NormalCmd::PasteTableAsCsv, "hint.table.convert-csv"),
        ch('t', NormalCmd::ConvertTableToTsv, "hint.table.convert-tsv"),
        ch('T', NormalCmd::PasteTableAsTsv, "hint.table.convert-tsv"),
    ],
    unknown: Unknown::Silent,
    sticky: false,
    ctrl_is_plain: false,
    before: None,
    after: None,
};

impl Editor {
    /// **What `Space t` takes before a key is looked up**: the column numbers
    /// and sort letters typed into it (`Space t 1a2d8as`), and the refusal
    /// when sort letters are followed by anything but the sort.
    fn table_group_takes(&mut self, key: Key) -> bool {
        if self.take_sort_key(key) || self.take_sequence_argument(key) {
            return true;
        }
        if !self.sort_keys.is_empty() && !matches!(key, Key::Char('s' | 'S') | Key::Esc) {
            self.pending = Pending::None;
            self.sequence = None;
            self.sort_keys.clear();
            self.status = say!("table.sort-wants-its-action");
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every row of every group names a command, and every command a
    /// group runs is a name a configuration can bind** (2026-10-31).
    #[test]
    fn every_group_key_is_a_bindable_command() {
        let bindable: Vec<&str> = yumete_cjk::actions::ALL
            .iter()
            .filter(|a| a.how == yumete_cjk::actions::How::Normal)
            .map(|a| a.name)
            .collect();
        for group in GROUPS {
            for bind in group.binds() {
                assert!(super::super::normal::normal_command(bind.run).is_some(), "{}: {} is no command", group.name, bind.run);
                assert!(bindable.contains(&bind.run.name()), "{}: {} is not bindable", group.name, bind.run);
            }
        }
        for (name, _, _) in super::super::group_commands::GROUP_COMMANDS {
            assert!(bindable.contains(&name.name()), "{name} is not bindable");
        }
    }

    /// **The `g` menu on a vertical page says what the keys do there**
    /// (2026-10-31). It used to be a table of its own that still turned
    /// `g h`/`g l` into `g k`/`g j`, which the keys stopped doing on
    /// 2026-10-13; drawn from the rows, it cannot.
    #[test]
    fn the_goto_menu_follows_the_page_and_the_preset() {
        let rows = |at: Context| GOTO.menu(&at);
        let across = rows(Context::default());
        let down = rows(Context { vertical: true, ..Context::default() });
        assert!(across.contains(&("h l".to_string(), "hint.goto.line-start-or-end")));
        assert!(across.contains(&("j k".to_string(), "hint.goto.by-file-line")));
        assert!(down.contains(&("h l".to_string(), "hint.goto.line-start-or-end")), "{down:?}");
        assert!(down.contains(&("j k".to_string(), "hint.goto.by-file-line-vertical")), "{down:?}");
        // Under vim, `ge` goes back to a word's end, and `gh`/`gl` are not
        // its keys.
        let vim = rows(Context { vim: true, ..Context::default() });
        assert!(vim.contains(&("e".to_string(), "action.jump-prev-word-end")), "{vim:?}");
        assert!(!vim.iter().any(|(k, _)| k == "h l"), "{vim:?}");
    }

    /// **Each kind of table lists its own keys** (2026-10-31): the rows a
    /// fenced block's table offers come first, and once hid the same keys
    /// from a Markdown table's menu.
    #[test]
    fn each_kind_of_table_lists_its_own_keys() {
        let md = Editor::table_keys(Some(Bounds::Md), false);
        for keys in ["/", "g", "y p P", "v"] {
            assert!(md.iter().any(|(k, _)| k == keys), "{keys} missing: {md:?}");
        }
        let block = Editor::table_keys(Some(Bounds::Block), false);
        assert!(block.iter().any(|(k, _)| k == "y v"), "{block:?}");
        let file = Editor::table_keys(Some(Bounds::WholeFile), false);
        assert!(file.iter().any(|(k, _)| k == "y p P"), "{file:?}");
    }

    /// The menu writes a run of digits as the run.
    #[test]
    fn a_run_of_keys_is_written_as_a_run() {
        let rows = YUME.menu(&Context::default());
        assert!(rows.contains(&("3-9".to_string(), "hint.yume.menu-size")), "{rows:?}");
        assert_eq!(keys_label(&["t".into(), "c".into(), "b".into()]), "t c b");
    }
}
