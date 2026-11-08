//! **A prompt's, a picker's and a sidebar's keys, each a named command**
//! (2026-11-01, step 9 of the keymap plan). helix hard-codes these keys and
//! nobody can rebind them; here each place has a default table, and the
//! reader's own (`[keys.prompt]`, `[keys.picker]`, `[keys.sidebar]`) is read
//! before it.
//!
//! A picker takes the prompt's keys too, for its query, and its own win
//! where both name a key: helix's rule ("Prompt keybinds also work in
//! pickers, except where they conflict with picker keybinds"). The `::`
//! palette is a picker over the commands, so it reads both tables the same
//! way.
//!
//! The names are listed in `yumete_cjk::actions::Place`; what each does is
//! said where the place's keys are answered (`prompt.rs`, `sidebar.rs`).

use super::*;
use yumete_cjk::actions::PlaceCmd;
use crate::input::Key;
use yumete_cjk::actions::Place;

/// A prompt's default keys: `:`, `/`, `::`, a reading, a picker's query.
pub(super) const PROMPT_KEYS: &[(Key, PlaceCmd)] = &[
    (Key::Esc, PlaceCmd::NormalMode),
    (Key::Enter, PlaceCmd::Confirm),
    (Key::Tab, PlaceCmd::CompletionNext),
    (Key::BackTab, PlaceCmd::CompletionPrev),
    (Key::Up, PlaceCmd::HistoryPrev),
    (Key::Ctrl('p'), PlaceCmd::HistoryPrev),
    (Key::Down, PlaceCmd::HistoryNext),
    (Key::Ctrl('n'), PlaceCmd::HistoryNext),
    (Key::Left, PlaceCmd::MoveCharLeft),
    (Key::Right, PlaceCmd::MoveCharRight),
    (Key::Home, PlaceCmd::GotoLineStart),
    (Key::Ctrl('a'), PlaceCmd::GotoLineStart),
    (Key::End, PlaceCmd::GotoLineEnd),
    (Key::Ctrl('e'), PlaceCmd::GotoLineEnd),
    (Key::Backspace, PlaceCmd::DeleteCharBackward),
    (Key::Delete, PlaceCmd::DeleteCharForward),
    (Key::Ctrl('w'), PlaceCmd::DeleteWordBackward),
    (Key::Ctrl('u'), PlaceCmd::KillToLineStart),
    (Key::Alt('j'), PlaceCmd::ToggleWideSearch),
];

/// A picker's default keys, read before the prompt's (helix's `Tab`, `C-n`
/// and `Down` for the next entry).
pub(super) const PICKER_KEYS: &[(Key, PlaceCmd)] = &[
    (Key::Tab, PlaceCmd::NextEntry),
    (Key::Ctrl('n'), PlaceCmd::NextEntry),
    (Key::Down, PlaceCmd::NextEntry),
    (Key::BackTab, PlaceCmd::PrevEntry),
    (Key::Ctrl('p'), PlaceCmd::PrevEntry),
    (Key::Up, PlaceCmd::PrevEntry),
    (Key::PageDown, PlaceCmd::PageDown),
    (Key::PageUp, PlaceCmd::PageUp),
    (Key::Alt('h'), PlaceCmd::CycleHiddenFiles),
];

/// A panel box's default keys, read before the prompt's: the boxes of a
/// form are walked by `Tab` and the arrows, which on the `:` line complete
/// and walk the history.
pub(super) const FIELD_KEYS: &[(Key, PlaceCmd)] = &[
    (Key::Tab, PlaceCmd::NextField),
    (Key::BackTab, PlaceCmd::PrevField),
    (Key::Down, PlaceCmd::MoveLineDown),
    (Key::Up, PlaceCmd::MoveLineUp),
];

/// A sidebar's default keys: the text's letters for moving, and what every
/// panel answers.
pub(super) const SIDEBAR_KEYS: &[(Key, PlaceCmd)] = &[
    (Key::Char('j'), PlaceCmd::MoveLineDown),
    (Key::Down, PlaceCmd::MoveLineDown),
    (Key::Char('k'), PlaceCmd::MoveLineUp),
    (Key::Up, PlaceCmd::MoveLineUp),
    (Key::Char('J'), PlaceCmd::PageDown),
    (Key::PageDown, PlaceCmd::PageDown),
    (Key::Char('K'), PlaceCmd::PageUp),
    (Key::PageUp, PlaceCmd::PageUp),
    (Key::Char('g'), PlaceCmd::GotoFileStart),
    (Key::Home, PlaceCmd::GotoFileStart),
    (Key::Char('G'), PlaceCmd::GotoLastLine),
    (Key::End, PlaceCmd::GotoLastLine),
    (Key::Char('h'), PlaceCmd::CollapseEntry),
    (Key::Left, PlaceCmd::CollapseEntry),
    (Key::Char('l'), PlaceCmd::ExpandEntry),
    (Key::Right, PlaceCmd::ExpandEntry),
    (Key::Enter, PlaceCmd::OpenEntry),
    (Key::Char('v'), PlaceCmd::ToggleLinksGraph),
    (Key::Char('u'), PlaceCmd::RestoreChange),
    (Key::Char('y'), PlaceCmd::YankOldText),
    (Key::Char('R'), PlaceCmd::RefreshSidebar),
    (Key::Char('w'), PlaceCmd::CycleSidebarWidth),
    (Key::Tab, PlaceCmd::NextSidebarView),
    (Key::BackTab, PlaceCmd::PrevSidebarView),
    (Key::Esc, PlaceCmd::NormalMode),
    (Key::Char('q'), PlaceCmd::LeaveSidebar),
    (Key::Char(':'), PlaceCmd::CommandMode),
    (Key::Char(' '), PlaceCmd::SpaceMode),
    (Key::Ctrl('w'), PlaceCmd::WindowMode),
];

/// The default table of a place.
pub(super) fn place_keys(place: Place) -> &'static [(Key, PlaceCmd)] {
    match place {
        Place::Prompt => PROMPT_KEYS,
        Place::Picker => PICKER_KEYS,
        Place::Field => FIELD_KEYS,
        Place::Sidebar => SIDEBAR_KEYS,
    }
}

/// A place's command for the docs: its name, its help, and its keys.
pub type PlaceRow = (&'static str, &'static str, Vec<String>);

impl Editor {
    /// **What `key` runs in `place`**: the reader's line for it, else the
    /// default; in a picker, its own two tables before the prompt's two.
    /// `None` when nothing names the key, which in a prompt types it.
    ///
    /// Keys a binding plays (`"@hello"`) are not read again, so a binding
    /// that names its own key cannot call itself.
    pub(super) fn commands_in(&self, place: Place, key: Key) -> Option<Vec<String>> {
        let order = place.chain();
        let typed = super::keys::key_internal(key);
        let read = !self.expanding_alias && !typed.is_empty();
        order.iter().find_map(|&one| {
            let mine = || self.place_bindings[one as usize].get(&typed).cloned();
            let inherited = || self.inherited(one, &typed).map(|name| vec![name]);
            let default = || place_keys(one).iter().find(|(k, _)| *k == key).map(|(_, cmd)| vec![cmd.name().to_string()]);
            read.then(mine).flatten().or_else(|| read.then(inherited).flatten()).or_else(default)
        })
    }

    /// **A line of the reader's own that follows them into `place`**
    /// (`[keys] inherit`, 2026-11-01): a `[keys.normal]` line into a
    /// sidebar, a `[keys.insert]` one into a prompt, when it is one key
    /// naming one command the place has. Read after the place's own table
    /// and before its defaults, so a Colemak reader's `n =
    /// "move_line_down"` walks a sidebar too; the defaults of Normal mode
    /// never come over (its `J` joins, a sidebar's pages). `no_op` stays
    /// where it was written: unbinding a key in the text leaves the
    /// sidebar's alone.
    fn inherited(&self, place: Place, typed: &str) -> Option<String> {
        self.lines_that_follow(place).into_iter().find(|(key, _)| key == typed).map(|(_, name)| name)
    }

    /// Every line of the reader's that follows them into `place`: its key,
    /// internal spelling, and the command.
    fn lines_that_follow(&self, place: Place) -> Vec<(String, String)> {
        // `[keys.normal]` is kept as written; `[keys.insert]` in the
        // internal spelling already.
        let normal = self.normal_lines_in_force();
        let from: Vec<(String, &Vec<String>)> = match place {
            _ if !self.place_inherits => return Vec::new(),
            Place::Sidebar => normal.iter().map(|(k, v)| (k.clone(), v)).collect(),
            Place::Prompt => self.insert_aliases.iter().map(|(k, v)| (k.clone(), v)).collect(),
            Place::Picker | Place::Field => return Vec::new(),
        };
        // helix's names for `j` and `k` are the visual ones; in a list a row
        // is a row, so they mean the place's own line moves (found in review,
        // 2026-11-02: a Colemak `n = "move_visual_line_down"` did nothing in
        // the file tree).
        fn in_a_list(name: &str) -> &str {
            match name {
                "move_visual_line_down" => "move_line_down",
                "move_visual_line_up" => "move_line_up",
                other => other,
            }
        }
        from.into_iter()
            .filter(|(key, _)| key.chars().filter(|&c| c != yumete_cjk::actions::ALT).count() == 1)
            .filter_map(|(key, run)| match run.as_slice() {
                [one] if one != "no_op" => {
                    let name = in_a_list(one);
                    place.actions().iter().any(|a| a.name == name).then(|| (key, name.to_string()))
                }
                _ => None,
            })
            .collect()
    }

    /// `[keys] inherit`.
    pub fn set_place_inheritance(&mut self, on: bool) {
        self.place_inherits = on;
    }

    /// **Run what a place's key names**, in order: a command line, keys
    /// played back into the same place, or one of the place's commands,
    /// which `run` carries out.
    pub(super) fn run_in_place(&mut self, place: Place, commands: &[String], mut run: impl FnMut(&mut Editor, PlaceCmd)) {
        for one in commands {
            match place.bound(one) {
                Some(yumete_cjk::actions::Bound::Command(line)) => self.run_bound_command(line),
                Some(yumete_cjk::actions::Bound::Keys(keys)) => self.play_in_place(&yumete_cjk::actions::written(keys)),
                // `no_op` is a name every place has and no command.
                Some(yumete_cjk::actions::Bound::Named(action)) => {
                    if let Some(cmd) = PlaceCmd::from_name(action.name) {
                        run(self, cmd);
                    }
                }
                None => {}
            }
            if self.quit_from_binding {
                break;
            }
        }
    }

    /// Keys a place's binding plays, each answered by wherever the keys are
    /// when it arrives (an `Enter` that closes the prompt hands the rest to
    /// the text), and none read against the reader's own tables again.
    fn play_in_place(&mut self, keys: &str) {
        let outer = self.expanding_alias;
        self.expanding_alias = true;
        let mut chars = keys.chars();
        while let Some(c) = chars.next() {
            let key = match c {
                yumete_cjk::actions::ALT => match chars.next() {
                    Some(next) => Key::Alt(next),
                    None => break,
                },
                c => super::keys::pressed(c),
            };
            match self.mode {
                Mode::Command => self.on_command_key(key),
                Mode::Lookfor => self.on_lookfor_key(key),
                Mode::Search => self.on_search_key(key),
                Mode::Ruby => self.on_ruby_key(key),
                Mode::Picker => self.on_picker_key(key),
                Mode::Normal if self.sidebar_focused() => self.on_sidebar_key(key),
                Mode::Normal => self.on_normal_key(key),
                Mode::Insert => self.insert_typed(key),
                Mode::Field => self.on_field_key(key),
                Mode::Value => self.on_value_key(key),
            }
        }
        self.expanding_alias = outer;
    }

    /// Install a place's lines (`[keys.prompt]`, `[keys.picker]`,
    /// `[keys.sidebar]`).
    pub fn set_place_bindings<C: crate::editor::BindingCommands>(&mut self, place: Place, lines: HashMap<String, C>) {
        self.place_bindings[place as usize] = lines
            .into_iter()
            .map(|(k, v)| (yumete_cjk::actions::written(&k), v.into_commands()))
            .collect();
    }

    /// **The keys that run `name` in `place` now**, internal spelling: the
    /// defaults nothing took over, then the reader's own lines and the ones
    /// that followed them in, as [`Self::commands_in`] reads them.
    pub(super) fn keys_in_place(&self, place: Place, name: &str) -> Vec<String> {
        let mine = &self.place_bindings[place as usize];
        let taken = |key: &String| mine.contains_key(key) || self.inherited(place, key).is_some();
        let mut keys: Vec<String> = place_keys(place)
            .iter()
            .filter(|(_, one)| one.name() == name)
            .map(|(key, _)| super::keys::key_internal(*key))
            .filter(|key| !taken(key))
            .collect();
        let mut theirs: Vec<String> = mine
            .iter()
            .filter(|(_, run)| run.len() == 1 && run[0] == name)
            .map(|(key, _)| key.clone())
            .chain(
                self.lines_that_follow(place)
                    .into_iter()
                    .filter(|(key, one)| one == name && !mine.contains_key(key))
                    .map(|(key, _)| key),
            )
            .collect();
        theirs.sort();
        keys.extend(theirs);
        keys
    }

    /// **The keys a hint row names for some commands of `place`, as they
    /// are now** (2026-11-01): each key of `spec` that still runs its
    /// command, in the order the row has always written them, then the
    /// reader's own keys for those commands. `None` when no key is left,
    /// and the row is not drawn: a key that is not bound is not named.
    pub(super) fn place_hint(&self, place: Place, spec: &[(Key, PlaceCmd)]) -> Option<String> {
        let runs = |key: Key, cmd: PlaceCmd| self.commands_in(place, key).is_some_and(|c| c.len() == 1 && c[0] == cmd.name());
        let mut shown: Vec<String> = spec.iter().filter(|(k, n)| runs(*k, *n)).map(|(k, _)| hint_key(*k)).collect();
        let mut names: Vec<PlaceCmd> = spec.iter().map(|(_, n)| *n).collect();
        names.dedup();
        // A picker's and a field's keys are their own and the prompt's. The
        // spec says which of a command's default keys to show; any other key
        // that runs it is the reader's, even one that is a default key of
        // another command (found in review, 2026-11-02: a sidebar's `j` bound
        // to go up was left off the hint).
        let chain = place.chain();
        let default = |key: &Key, name: PlaceCmd| {
            chain.iter().any(|&p| place_keys(p).iter().any(|(d, n)| d == key && *n == name))
        };
        for &from in chain {
            for &name in &names {
                for typed in self.keys_in_place(from, name.name()) {
                    let Some(key) = super::keys::pressed_key(&typed).filter(|k| !default(k, name)) else { continue };
                    let one = hint_key(key);
                    if runs(key, name) && !shown.contains(&one) {
                        shown.push(one);
                    }
                }
            }
        }
        (!shown.is_empty()).then(|| shown.join(" "))
    }

    /// The keys that run `name` in `place` now, as a hint writes them:
    /// its default keys still bound to it, then the reader's own.
    pub fn place_keys_written(&self, place: Place, name: PlaceCmd) -> Option<String> {
        let spec: Vec<(Key, PlaceCmd)> = place_keys(place).iter().filter(|(_, one)| *one == name).copied().collect();
        if !spec.is_empty() {
            return self.place_hint(place, &spec);
        }
        let keys: Vec<String> = self
            .keys_in_place(place, name.name())
            .iter()
            .filter_map(|typed| super::keys::pressed_key(typed))
            .map(hint_key)
            .collect();
        (!keys.is_empty()).then(|| keys.join(" "))
    }

    /// **Each place's commands with their default keys**, for the site
    /// (`ye --docgen`): the place, then its commands in the order its list
    /// gives, each with its help and the keys that run it.
    pub fn place_commands_for_docs() -> Vec<(Place, Vec<PlaceRow>)> {
        Place::ALL
            .into_iter()
            .map(|place| {
                let rows = place
                    .actions()
                    .iter()
                    .map(|action| {
                        let keys = place_keys(place)
                            .iter()
                            .filter(|(_, cmd)| cmd.name() == action.name)
                            .map(|(key, _)| super::keys::key_internal(*key))
                            .collect();
                        (action.name, action.help, keys)
                    })
                    .collect();
                (place, rows)
            })
            .collect()
    }
}

/// **A run of keys as the hint row writes it**, internal spelling in:
/// `gg`, `␣L`, `A-.`.
pub(super) fn hint_run(run: &str) -> String {
    let mut out = String::new();
    let mut chars = run.chars();
    while let Some(c) = chars.next() {
        match c {
            yumete_cjk::actions::ALT => {
                if let Some(next) = chars.next() {
                    out.push_str(&hint_key(Key::Alt(next)));
                }
            }
            c => out.push_str(&hint_key(super::keys::pressed(c))),
        }
    }
    out
}

/// **A key as the hint row writes it**: `↑`, `S-Tab`, `␣`, `C-w`.
pub(super) fn hint_key(key: Key) -> String {
    match key {
        Key::Char(' ') => "\u{2423}".to_string(),
        Key::Char(c) => c.to_string(),
        Key::Up => "↑".to_string(),
        Key::Down => "↓".to_string(),
        Key::Left => "←".to_string(),
        Key::Right => "→".to_string(),
        Key::Tab => "Tab".to_string(),
        Key::BackTab => "S-Tab".to_string(),
        Key::Enter => "Enter".to_string(),
        Key::Esc => "Esc".to_string(),
        Key::Backspace => "⌫".to_string(),
        Key::Delete => "Del".to_string(),
        Key::Home => "Home".to_string(),
        Key::End => "End".to_string(),
        Key::PageUp => "PgUp".to_string(),
        Key::PageDown => "PgDn".to_string(),
        Key::Ctrl(c) => format!("C-{c}"),
        Key::Alt(c) => format!("A-{c}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Every name in a place's default table is one of its commands**, so
    /// the table and the list the config reads cannot drift apart.
    #[test]
    fn every_default_key_runs_a_command_of_its_place() {
        for place in Place::ALL {
            for (key, name) in place_keys(place) {
                assert!(place.action(name.name()).is_some(), "{key:?} runs `{name:?}`, which {} does not have", place.name());
            }
        }
    }

    /// And every command a place lists has a default key or is one the
    /// reader binds (helix's `first_entry`/`last_entry`, which `Home`/`End`
    /// are not, since here they move the caret).
    #[test]
    fn a_place_command_without_a_key_is_named_here() {
        let unbound = [PlaceCmd::FirstEntry, PlaceCmd::LastEntry];
        for place in Place::ALL {
            for action in place.actions() {
                let keyed = place_keys(place).iter().any(|(_, name)| name.name() == action.name)
                    || place.chain()[1..].iter().any(|&p| place_keys(p).iter().any(|(_, n)| n.name() == action.name));
                let left_unbound = unbound.iter().any(|cmd| cmd.name() == action.name);
                assert!(keyed || left_unbound, "`{}` has no key in {}", action.name, place.name());
            }
        }
    }
}
