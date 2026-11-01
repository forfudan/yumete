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
use crate::input::Key;

/// **Where a row holds.** `Always` and the presets are the common ones; the
/// rest each answer one group's question.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Only {
    Always,
    Vim,
    /// `g h` / `g l`: helix's keys, which the vim preset reaches only from a
    /// binding (`0` and `$` are its own).
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
    pub run: &'static str,
    /// The menu line; empty for a key that is not on the menu (the arrow
    /// keys beside `hjkl`, the `C-` spellings in the window group).
    pub says: &'static str,
    /// Where the key does this.
    pub when: Only,
    /// Where the menu offers it, when the key does it.
    pub listed: Only,
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

    const fn listed(mut self, only: Only) -> Self {
        match &mut self {
            Row::Bind(bind) => bind.listed = only,
            Row::Note { listed, .. } => *listed = only,
        }
        self
    }
}

const fn key(key: Key, run: &'static str, says: &'static str) -> Row {
    Row::Bind(Bind { key, run, says, when: Only::Always, listed: Only::Always })
}

const fn ch(c: char, run: &'static str, says: &'static str) -> Row {
    key(Key::Char(c), run, says)
}

/// A key that is not on the menu.
const fn quiet(key_: Key, run: &'static str) -> Row {
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
        self.binds().find(|bind| bind.key == key && at.holds(bind.when))
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
                    if bind.says.is_empty() || !at.holds(bind.when) || !at.holds(bind.listed) || taken(bind.key) {
                        continue;
                    }
                    // Only a row the key reaches, once: a key with a vim row
                    // and a helix row is listed as what it does here, and a
                    // key offered in several kinds of table as that kind's.
                    let hidden = self.rows[..nth].iter().any(|earlier| match earlier {
                        Row::Bind(b) if b.key == bind.key && at.holds(b.when) => {
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
    if commands.iter().all(|one| one == "no_op") {
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
}

impl Context {
    pub(super) fn holds(&self, only: Only) -> bool {
        match only {
            Only::Always => true,
            Only::Vim => self.vim,
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

    /// The context the generated keymap page is written for: helix's keys,
    /// a page laid out across, a code file (so every bracket key is listed).
    pub(super) fn for_docs() -> Self {
        Context { code: true, ..Context::default() }
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
            vim: self.key_preset == yumete_cjk::KeyPreset::Vim,
            vertical: self.layout() == crate::zong::Layout::Vertical,
            alias: self.expanding_alias,
            code: self.writes_code(),
            extend: self.extend,
            pane: self.the_pane_is_on(),
            table,
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
                if let Some(&(_, run, _)) = super::normal::normal_command(bind.run) {
                    let operator_count = self.operator_count;
                    let count = operator_count.unwrap_or(1).max(1);
                    run(self, Cx { key, count, operator_count, continuing_zong });
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
        (crate::messages::say(group.title, &[]), rows)
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

    /// **Every key menu, for the generated keymap page** (`ye --docgen`):
    /// each menu's title message, the keys that open it, and its rows as
    /// (keys, message), read off the same rows the which-key panel draws.
    /// Helix's keys on a page laid out across; the table group lists every
    /// key it has in some kind of table, the way in first.
    pub fn menus_for_docs() -> Vec<super::keys::DocMenu> {
        let at = Context::for_docs();
        let mut out = Vec::new();
        for group in GROUPS {
            // `Z` is `z` that stays open, and the three value lists under
            // `Space z` are one line each on that menu.
            if matches!(group.name, "sticky_view" | "yume_commit" | "yume_panel" | "yume_preedit") {
                continue;
            }
            let rows = match group.name {
                "table" => {
                    let mut all: Vec<(String, &'static str)> = Vec::new();
                    for (inside, pane) in [(Some(Bounds::Md), true), (Some(Bounds::WholeFile), false), (Some(Bounds::Block), false)] {
                        for (keys, what) in Self::table_keys(inside, pane) {
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
            out.push((group.title, written_path(group.path), rows));
        }
        out
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
            Only::GLetter | Only::HelixVertical => !vim,
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
        use super::normal::{HELIX_NORMAL, SELECT_NORMAL, VIM_NORMAL};
        let one = |key: Key| vec![super::keys::key_internal(key)];
        let mut out = Vec::new();
        for action in yumete_cjk::actions::ALL {
            if action.how != yumete_cjk::actions::How::Normal {
                continue;
            }
            let name = action.name;
            let mut helix: Vec<Vec<String>> = Vec::new();
            let mut vim: Vec<Vec<String>> = Vec::new();
            for (key, n) in HELIX_NORMAL.iter().chain(SELECT_NORMAL) {
                if *n != name {
                    continue;
                }
                helix.push(one(*key));
                // The vim preset keeps a helix key it has no entry of its own for.
                if !VIM_NORMAL.iter().any(|(k, _, _)| k == key) {
                    vim.push(one(*key));
                }
            }
            for (key, n, _) in VIM_NORMAL {
                if *n == name {
                    vim.push(one(*key));
                }
            }
            for group in GROUPS {
                // Keys off the menu too (`z j`, the sort keys a column
                // number comes before): a key the editor has is listed.
                for bind in group.binds().filter(|b| b.run == name) {
                    for (is_vim, keys) in [(false, &mut helix), (true, &mut vim)] {
                        let Some(mut path) = group_opened_by(group, is_vim).filter(|_| bind.when.under(is_vim)) else {
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
                .filter(|(_, n, _)| *n == name)
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

/// A group's keys as the docs write them: `Space t x`, `C-w n`.
fn written_path(path: &str) -> String {
    let mut out: Vec<String> = Vec::new();
    for c in path.chars() {
        out.push(match c {
            ' ' => "Space".to_string(),
            '\u{17}' => "C-w".to_string(),
            c => c.to_string(),
        });
    }
    out.join(" ")
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
        ch('g', "extend_to_file_start", "hint.goto.start-of-file").when(Only::Selecting),
        ch('g', "goto_file_start", "hint.goto.start-of-file"),
        note("<N>g", "hint.goto.line-n"),
        ch('e', "jump_prev_word_end", "action.jump-prev-word-end").when(Only::Vim),
        ch('E', "jump_prev_long_word_end", "action.jump-prev-long-word-end").when(Only::Vim),
        ch('e', "extend_to_file_end", "hint.goto.end-of-file").when(Only::Selecting),
        ch('e', "goto_file_end", "hint.goto.end-of-file"),
        ch('h', "goto_line_start", "hint.goto.line-start-or-end").when(Only::GLetter),
        ch('l', "goto_line_end", "hint.goto.line-start-or-end").when(Only::GLetter),
        quiet(Key::Up, "goto_line_start").when(Only::HelixVertical),
        quiet(Key::Down, "goto_line_end").when(Only::HelixVertical),
        ch('s', "goto_first_nonwhitespace", "hint.goto.first-non-blank"),
        ch('f', "open_here", "hint.goto.open-this-file"),
        ch('L', "follow_link", "hint.goto.follow-link"),
        ch('w', "extend_to_word", "hint.goto.jump-labels").when(Only::Selecting),
        ch('w', "goto_word", "hint.goto.jump-labels"),
        ch('x', "goto_word_by_letters", "hint.goto.seek-written"),
        ch('z', "goto_char_by_pinyin", "hint.goto.seek-reading"),
        ch('a', "goto_last_accessed_file", "hint.goto.last-file"),
        ch('n', "search_next", "hint.goto.next-match").when(Only::Vim),
        ch('N', "search_prev", "hint.goto.next-match").when(Only::Vim),
        ch('n', "next_buffer", "hint.goto.next-or-previous-buffer"),
        ch('p', "prev_buffer", "hint.goto.next-or-previous-buffer"),
        ch('d', "goto_definition", "hint.goto.follow-note"),
        ch('D', "goto_definition_other_window", "hint.goto.follow-note-elsewhere"),
        ch('r', "goto_reference", "hint.goto.references"),
        ch('/', "search_selection", "hint.goto.search-word"),
        ch('?', "search_selection_other_window", "hint.goto.search-word-elsewhere"),
        ch('j', "move_next_zong", "hint.goto.by-file-line-vertical").when(Only::VimVertical),
        ch('k', "move_prev_zong", "hint.goto.by-file-line-vertical").when(Only::VimVertical),
        ch('j', "move_visual_line_down", "action.move-visual-line-down").when(Only::Vim),
        ch('k', "move_visual_line_up", "action.move-visual-line-up").when(Only::Vim),
        ch('j', "move_line_down", "hint.goto.by-file-line-vertical").when(Only::HelixVertical),
        ch('k', "move_line_up", "hint.goto.by-file-line-vertical").when(Only::HelixVertical),
        ch('j', "extend_line_down", "hint.goto.by-file-line").when(Only::Selecting),
        ch('k', "extend_line_up", "hint.goto.by-file-line").when(Only::Selecting),
        ch('j', "move_line_down", "hint.goto.by-file-line"),
        ch('k', "move_line_up", "hint.goto.by-file-line"),
        quiet(Key::Down, "move_visual_line_down").when(Only::FlatVim),
        quiet(Key::Up, "move_visual_line_up").when(Only::FlatVim),
        quiet(Key::Down, "move_line_down").when(Only::Flat),
        quiet(Key::Up, "move_line_up").when(Only::Flat),
        ch('.', "goto_last_modification", "hint.goto.last-change"),
        ch('t', "goto_window_top", "hint.goto.screen-top-middle-bottom"),
        ch('c', "goto_window_center", "hint.goto.screen-top-middle-bottom"),
        ch('b', "goto_window_bottom", "hint.goto.screen-top-middle-bottom"),
        // vim's own: none of these is a helix command.
        ch('v', "reselect_last_selection", "hint.goto.reselect").when(Only::Vim),
        ch('i', "insert_at_last_insert", "hint.goto.insert-again").when(Only::Vim),
        ch(';', "goto_older_change", "hint.goto.walk-changes").when(Only::Vim),
        ch(',', "goto_newer_change", "hint.goto.walk-changes").when(Only::Vim),
        ch('u', "operator_lowercase", "hint.goto.case-operators").when(Only::Vim),
        ch('U', "operator_uppercase", "hint.goto.case-operators").when(Only::Vim),
        ch('~', "operator_switch_case", "hint.goto.case-operators").when(Only::Vim),
        ch('q', "operator_fill", "hint.goto.fill").when(Only::Vim),
        ch('J', "join_selections_no_space", "hint.goto.join-without-a-space").when(Only::Vim),
        ch('`', "case_mode", "action.case-mode").when(Only::Vim),
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
        ch('f', "file_picker", "hint.space.open-file"),
        ch('F', "file_picker_in_current_directory", "hint.space.open-file-here"),
        ch('e', "file_explorer", "hint.space.explorer"),
        ch('E', "file_explorer_in_current_buffer_directory", "hint.space.explorer-here"),
        ch('b', "buffer_picker", "hint.space.switch-buffer"),
        ch('g', "changed_file_picker", "hint.space.changed-files"),
        ch('y', "copy_to_clipboard", "hint.space.copy-to-clipboard"),
        ch('p', "paste_from_clipboard", "hint.space.paste-from-clipboard"),
        ch('P', "paste_clipboard_before", "hint.space.paste-before"),
        ch('R', "replace_selections_with_clipboard", "hint.space.replace-from-clipboard"),
        ch('s', "symbol_picker", "hint.space.symbols"),
        ch('S', "workspace_symbol_picker", "hint.space.project-symbols"),
        ch('n', "dictionary", "hint.space.dictionary"),
        ch('N', "dictionary_panel", "hint.space.dictionary-panel"),
        ch('k', "hover", "hint.space.what-is-this"),
        ch('K', "hover_panel", "hint.space.what-is-this-panel"),
        ch('d', "diagnostics_picker", "hint.space.diagnostics"),
        ch('D', "workspace_diagnostics_picker", "hint.space.project-diagnostics"),
        ch('l', "show_diagnostic", "hint.space.problems"),
        ch('L', "show_diagnostic_panel", "hint.space.problems-panel"),
        ch('x', "show_change", "hint.space.change"),
        ch('X', "show_change_panel", "hint.space.change-panel"),
        ch('r', "rename_symbol", "hint.space.rename"),
        ch('a', "code_action", "hint.space.actions"),
        ch('i', "show_record", "hint.space.record"),
        ch('I', "show_record_panel", "hint.space.record-panel"),
        ch('w', "window_mode", "hint.region.title"),
        ch('t', "table_mode", "hint.space.table"),
        ch('"', "paste_menu", "menu.paste.title"),
        ch('c', "comment_line", "hint.space.comment-line"),
        ch('C', "comment_block", "hint.space.comment-block"),
        ch('m', "conflict_mode", "hint.conflict.title"),
        ch('o', "open_recent_mode", "hint.space.open-recent"),
        ch('z', "yume_mode", "hint.space.yume"),
        ch('/', "search_panel", "hint.space.advanced-search"),
        ch('?', "command_menu", "hint.space.all-commands"),
        ch(' ', "page_down", "hint.space.page-down"),
        key(Key::Backspace, "page_up", "hint.space.page-up"),
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
        ch('m', "goto_next_conflict", "hint.hop.conflict"),
        ch('g', "goto_next_change", "hint.hop.change"),
        ch('d', "goto_next_diag", "hint.hop.problem"),
        ch('D', "goto_last_diag", "hint.hop.problem"),
        ch('f', "goto_next_function", "hint.hop.function").listed(Only::Code),
        ch('t', "goto_next_class", "hint.hop.class").listed(Only::Code),
        ch('c', "goto_next_comment", "hint.hop.comment").listed(Only::Code),
        // Warning: **段落那一行借的是 `:keymap-actions` 的名字**：同一件事在兩張
        // 表上，寫成兩句話只會讓它們慢慢說岔。
        ch('p', "goto_next_paragraph", "action.goto-next-paragraph"),
        ch(' ', "add_newline_below", "hint.hop.blank-below"),
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
        ch('m', "goto_prev_conflict", "hint.hop.conflict"),
        ch('g', "goto_prev_change", "hint.hop.change"),
        ch('d', "goto_prev_diag", "hint.hop.problem"),
        ch('D', "goto_first_diag", "hint.hop.problem"),
        ch('f', "goto_prev_function", "hint.hop.function").listed(Only::Code),
        ch('t', "goto_prev_class", "hint.hop.class").listed(Only::Code),
        ch('c', "goto_prev_comment", "hint.hop.comment").listed(Only::Code),
        ch('p', "goto_prev_paragraph", "action.goto-prev-paragraph"),
        ch(' ', "add_newline_above", "hint.hop.blank-above"),
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
    ch('z', "align_view_center", "hint.aim.middle"),
    quiet(Key::Char('c'), "align_view_center"),
    ch('t', "align_view_top", "hint.aim.top"),
    ch('b', "align_view_bottom", "hint.aim.bottom"),
    quiet(Key::Char('j'), "scroll_down"),
    quiet(Key::Down, "scroll_down"),
    quiet(Key::Char('k'), "scroll_up"),
    quiet(Key::Up, "scroll_up"),
    quiet(Key::Ctrl('d'), "half_page_down"),
    quiet(Key::Char(' '), "half_page_down"),
    quiet(Key::Ctrl('u'), "half_page_up"),
    quiet(Key::Backspace, "half_page_up"),
    quiet(Key::Ctrl('f'), "page_down"),
    quiet(Key::PageDown, "page_down"),
    quiet(Key::Ctrl('b'), "page_up"),
    quiet(Key::PageUp, "page_up"),
    quiet(Key::Char('/'), "search"),
    quiet(Key::Char('?'), "rsearch"),
    quiet(Key::Char('n'), "search_next"),
    quiet(Key::Char('N'), "search_prev"),
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
        ch('m', "match_brackets", "hint.match.pair"),
        ch('i', "select_textobject_inner", "hint.match.inside"),
        ch('a', "select_textobject_around", "hint.match.around"),
        ch('s', "surround_add", "hint.match.surround"),
        ch('d', "surround_delete", "hint.match.take-off"),
        ch('r', "surround_replace", "hint.change"),
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
        ch('w', "next_region", "hint.region.next"),
        ch('h', "jump_view_left", "hint.region.go"),
        ch('j', "jump_view_down", "hint.region.go"),
        ch('k', "jump_view_up", "hint.region.go"),
        ch('l', "jump_view_right", "hint.region.go"),
        quiet(Key::Left, "jump_view_left"),
        quiet(Key::Down, "jump_view_down"),
        quiet(Key::Up, "jump_view_up"),
        quiet(Key::Right, "jump_view_right"),
        ch('e', "toggle_left_sidebar", "hint.region.toggle-bar"),
        ch('i', "toggle_right_sidebar", "hint.region.toggle-bar"),
        ch('E', "goto_left_sidebar", "hint.region.into-bar"),
        ch('I', "goto_right_sidebar", "hint.region.into-bar"),
        ch('s', "hsplit", "hint.region.split"),
        ch('v', "vsplit", "hint.region.split-beside"),
        // vim's `C-w n` is the blank window below; helix's is a group.
        ch('n', "hsplit_new", "hint.region.new").when(Only::Vim),
        ch('n', "window_new_mode", "hint.region.new"),
        ch('q', "wclose", "hint.region.close"),
        ch('o', "wonly", "hint.region.only"),
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
        ch('s', "hsplit_new", "hint.region.new-below"),
        ch('v', "vsplit_new", "hint.region.new-beside"),
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
        ch('l', "switch_to_lowercase", "hint.case.lower"),
        ch('u', "switch_to_uppercase", "hint.case.upper"),
        ch('`', "switch_case", "hint.case.switch"),
        ch('s', "convert_to_simplified", "hint.case.to-s"),
        ch('t', "convert_to_traditional", "hint.case.to-t"),
        ch('w', "convert_to_taiwan_traditional", "hint.case.to-tw"),
        ch('f', "switch_to_fullwidth", "hint.case.to-full"),
        ch('h', "switch_to_halfwidth", "hint.case.to-half"),
        ch('k', "convert_to_hong_kong_traditional", "hint.case.to-hk"),
        ch('c', "convert_to_mainland_traditional", "hint.case.to-c"),
        ch('g', "convert_to_classical_traditional", "hint.case.to-g"),
        ch('j', "convert_to_japanese", "hint.case.to-jp"),
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
        ch('o', "conflict_keep_ours", "hint.conflict.ours"),
        ch('t', "conflict_keep_theirs", "hint.conflict.theirs"),
        ch('b', "conflict_keep_both", "hint.conflict.both"),
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
        ch('f', "recent_file_picker", "hint.open.recent-files"),
        ch('g', "global_recent_file_picker", "hint.open.recent-files-global"),
        ch('p', "project_picker", "hint.open.recent-projects"),
        ch('r', "resume_session", "hint.open.resume"),
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
        ch('z', "yume_toggle", "hint.yume.toggle"),
        ch('m', "yume_commit_mode", "hint.yume.commit"),
        ch('p', "yume_panel_mode", "hint.yume.panel"),
        ch('e', "yume_preedit_mode", "hint.yume.preedit"),
        ch('y', "yume_toggle_prediction", "hint.yume.predict"),
        ch('i', "yume_pick_scheme", "hint.yume.scheme"),
        ch('n', "yume_next_scheme", "hint.yume.next-scheme"),
        ch('3', "yume_menu_size_3", "hint.yume.menu-size"),
        ch('4', "yume_menu_size_4", "hint.yume.menu-size"),
        ch('5', "yume_menu_size_5", "hint.yume.menu-size"),
        ch('6', "yume_menu_size_6", "hint.yume.menu-size"),
        ch('7', "yume_menu_size_7", "hint.yume.menu-size"),
        ch('8', "yume_menu_size_8", "hint.yume.menu-size"),
        ch('9', "yume_menu_size_9", "hint.yume.menu-size"),
        ch('/', "yume_toggle_chaifen", "hint.yume.chaifen"),
        ch('t', "yume_installed_table", "hint.yume.table"),
        ch('w', "yume_table_source", "hint.yume.which"),
        ch('W', "yume_data_location", "hint.yume.where"),
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
        ch('0', "yume_commit_default", "hint.yume.commit-default"),
        ch('d', "yume_commit_delayed", "hint.yume.commit-delayed"),
        ch('u', "yume_commit_unique", "hint.yume.commit-unique"),
        ch('f', "yume_commit_fluency", "hint.yume.commit-fluency"),
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
        ch('b', "yume_panel_box", "hint.yume.panel-box"),
        ch('m', "yume_panel_bottom", "hint.yume.panel-bottom"),
        ch('i', "yume_panel_inline", "hint.yume.panel-inline"),
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
        ch('o', "yume_preedit_off", "hint.yume.preedit-off"),
        ch('c', "yume_preedit_composition", "hint.yume.preedit-composition"),
        ch('p', "yume_preedit_preview", "hint.yume.preedit-preview"),
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
        ch('o', "table_source_view", "hint.table.back-to-prose"),
        ch('b', "table_basic_view", "hint.table.operate-it"),
        ch('t', "table_window_view", "hint.table.whole-window"),
        ch('q', "table_leave_window", "hint.table.leave-the-window"),
        ch(']', "goto_next_table", "hint.table.next-or-previous"),
        ch('[', "goto_prev_table", "hint.table.next-or-previous"),
        // 2026-10-02 定：命令是 `:convert-table`，鍵位摺在這一組底下。
        ch('x', "table_convert_mode", "hint.table.convert"),
        ch('w', "table_cell_width", "hint.table.fold-wide-cells").listed(Only::Pane),
        // In a fenced block.
        ch('/', "table_search_columns", "hint.table.search-columns").listed(Only::Block),
        ch('?', "table_search_columns_other_window", "hint.table.search-columns-elsewhere").listed(Only::Block),
        ch('g', "table_goto_cell", "hint.table.go-to-cell").listed(Only::Block),
        ch('y', "table_yank_column", "hint.table.yank-or-replace-column").listed(Only::Block),
        ch('v', "table_replace_column", "hint.table.yank-or-replace-column").listed(Only::Block),
        note("1s 1S", "hint.table.sort-by-column").listed(Only::Block),
        // In a Markdown table.
        ch('/', "table_search_columns", "hint.table.search-columns").listed(Only::Md),
        ch('?', "table_search_columns_other_window", "hint.table.search-columns-elsewhere").listed(Only::Md),
        ch('g', "table_goto_cell", "hint.table.go-to-cell").listed(Only::Md),
        ch('r', "table_add_row_below", "hint.table.add-row").listed(Only::Md),
        ch('R', "table_add_row_above", "hint.table.add-row").listed(Only::Md),
        ch('c', "table_add_column_right", "hint.table.add-column").listed(Only::Md),
        ch('C', "table_add_column_left", "hint.table.add-column").listed(Only::Md),
        ch('d', "table_delete_row", "hint.table.delete-row-or-column").listed(Only::Md),
        ch('D', "table_delete_column", "hint.table.delete-row-or-column").listed(Only::Md),
        ch('j', "table_move_row_down", "hint.table.move-row").listed(Only::Md),
        ch('k', "table_move_row_up", "hint.table.move-row").listed(Only::Md),
        ch('h', "table_move_column_left", "hint.table.move-column").listed(Only::Md),
        ch('l', "table_move_column_right", "hint.table.move-column").listed(Only::Md),
        ch('y', "table_yank_column", "hint.table.yank-or-paste-column").listed(Only::Md),
        ch('p', "table_paste_column_right", "hint.table.yank-or-paste-column").listed(Only::Md),
        ch('P', "table_paste_column_left", "hint.table.yank-or-paste-column").listed(Only::Md),
        ch('v', "table_replace_column", "hint.table.replace-column").listed(Only::Md),
        note("1s 1S", "hint.table.sort-by-column").listed(Only::Md),
        ch('<', "table_align_left", "hint.table.align-column").listed(Only::Md),
        ch('=', "table_align_center", "hint.table.align-column").listed(Only::Md),
        ch('>', "table_align_right", "hint.table.align-column").listed(Only::Md),
        // `F`, not `f`: the lowercase letter was 完整表格 (#283), and a
        // capital is what a command that rewrites every row should have.
        ch('F', "table_line_up", "hint.table.line-it-up").listed(Only::Md),
        ch('i', "table_record_panel", "hint.table.detail-panel").listed(Only::Md),
        // In a delimited file.
        ch('/', "table_search_columns", "hint.table.search-columns").listed(Only::File),
        ch('?', "table_search_columns_other_window", "hint.table.search-columns-elsewhere").listed(Only::File),
        ch('g', "table_goto_cell", "hint.table.go-to-cell").listed(Only::File),
        note("1s 1S", "hint.table.sort-by-column").listed(Only::File),
        ch('r', "table_add_row_below", "hint.table.add-row").listed(Only::File),
        ch('R', "table_add_row_above", "hint.table.add-row").listed(Only::File),
        ch('d', "table_delete_row", "hint.table.delete-row").listed(Only::File),
        ch('j', "table_move_row_down", "hint.table.move-row").listed(Only::File),
        ch('k', "table_move_row_up", "hint.table.move-row").listed(Only::File),
        ch('y', "table_yank_column", "hint.table.yank-or-paste-column").listed(Only::File),
        ch('p', "table_paste_column_right", "hint.table.yank-or-paste-column").listed(Only::File),
        ch('P', "table_paste_column_left", "hint.table.yank-or-paste-column").listed(Only::File),
        ch('v', "table_replace_column", "hint.table.replace-column").listed(Only::File),
        ch('H', "table_first_row_is_data", "hint.table.first-row-is-data").listed(Only::File),
        ch('i', "table_record_panel", "hint.table.detail-panel").listed(Only::File),
        // The sort keys take their columns from the count typed before them
        // (`Space t 1s`), so the menu says them as a note above.
        quiet(Key::Char('s'), "table_sort_ascending"),
        quiet(Key::Char('S'), "table_sort_descending"),
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
        ch('p', "convert_table_to_pipe", "hint.table.convert-pipe"),
        ch('P', "paste_table_as_pipe", "hint.table.convert-pipe"),
        ch('c', "convert_table_to_csv", "hint.table.convert-csv"),
        ch('C', "paste_table_as_csv", "hint.table.convert-csv"),
        ch('t', "convert_table_to_tsv", "hint.table.convert-tsv"),
        ch('T', "paste_table_as_tsv", "hint.table.convert-tsv"),
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
                assert!(bindable.contains(&bind.run), "{}: {} is not bindable", group.name, bind.run);
            }
        }
        for (name, _, _) in super::super::group_commands::GROUP_COMMANDS {
            assert!(bindable.contains(name), "{name} is not bindable");
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
        for keys in ["/", "?", "g", "y p P", "v"] {
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
