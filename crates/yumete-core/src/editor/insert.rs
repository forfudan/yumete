//! **Insert mode's keys, each a named command** (2026-10-31, step 6 of the
//! keymap plan; helix's `insert` keymap, `keymap/default.rs`).
//!
//! [`INSERT_KEYS`] says which command each key runs; a key it does not name
//! types itself. Before it come the layers that depend on where the caret
//! is (a completion list open, a table cell, a vertical page), as Normal
//! mode's table and vertical layers come before its keymap, and before
//! those the reader's own `[keys.insert]` ([`Editor::insert_binding`]).
//!
//! A command shared with Normal mode (`normal_mode`, `delete_char_backward`,
//! `move_char_left`) is one command, as in helix, and does in Insert mode
//! what its key always did there.

use super::normal::NormalCommand;
use super::*;
use crate::input::Key;
use crate::motion;

/// Where an Insert-mode key holds.
type InsertWhen = Option<fn(&Editor) -> bool>;

/// Insert mode's default keymap: a key, the command it runs, and where it
/// holds (the vim preset's `C-o`).
pub(super) const INSERT_KEYS: &[(Key, &str, InsertWhen)] = &[
    (Key::Esc, "normal_mode", None),
    (Key::Enter, "insert_newline", None),
    (Key::Ctrl('j'), "insert_newline", None),
    (Key::Backspace, "delete_char_backward", None),
    (Key::Ctrl('h'), "delete_char_backward", None),
    (Key::Delete, "delete_char_forward", None),
    (Key::Ctrl('d'), "delete_char_forward", None),
    (Key::Alt('d'), "delete_word_forward", None),
    (Key::Ctrl('w'), "delete_word_backward", None),
    (Key::Ctrl('u'), "kill_to_line_start", None),
    (Key::Ctrl('k'), "kill_to_line_end", None),
    (Key::Ctrl('r'), "insert_register", None),
    (Key::Ctrl('o'), "run_one_normal_command", Some(|e| e.key_preset == yumete_cjk::KeyPreset::Vim)),
    (Key::Left, "move_char_left", None),
    (Key::Right, "move_char_right", None),
    (Key::Up, "move_visual_line_up", None),
    (Key::Down, "move_visual_line_down", None),
    (Key::PageUp, "page_up", None),
    (Key::PageDown, "page_down", None),
    (Key::Home, "goto_line_start", None),
    (Key::Ctrl('a'), "goto_line_start", None),
    (Key::End, "goto_line_end_newline", None),
    (Key::Ctrl('e'), "goto_line_end_newline", None),
    // vim's `C-g u` (helix's `C-s`, which a terminal takes for XOFF).
    (Key::Ctrl('g'), "commit_undo_checkpoint", None),
    (Key::Tab, "insert_tab", None),
    (Key::BackTab, "insert_other_tab", None),
];

/// **The commands only Insert mode's keys run.** Bound in Normal mode they
/// do the same thing there (`insert_newline` writes a line break).
pub(super) const INSERT_COMMANDS: &[NormalCommand] = &[
    ("insert_newline", |e, _| e.insert_newline(), Key::Enter),
    ("delete_char_forward", |e, _| e.delete_at_cursor(), Key::Delete),
    ("delete_word_forward", |e, _| e.delete_word_after_cursor(), Key::Alt('d')),
    ("delete_word_backward", |e, _| e.delete_word_before_cursor(), Key::Ctrl('w')),
    ("kill_to_line_start", |e, _| e.delete_to_line_start(), Key::Ctrl('u')),
    ("kill_to_line_end", |e, _| e.delete_to_line_end(), Key::Ctrl('k')),
    ("insert_register", |e, _| e.pending = Pending::Register, Key::Ctrl('r')),
    // vim's `i_CTRL-O`: one Normal-mode command, then back.
    ("run_one_normal_command", |e, _| {
        e.one_normal_key = Some(Mode::Insert);
        e.mode = Mode::Normal;
    }, Key::Ctrl('o')),
    ("goto_line_end_newline", |e, _| {
        let pos = motion::line_end(e.current_buffer().rope(), e.sel.head());
        e.set_cursor(pos);
    }, Key::End),
    ("commit_undo_checkpoint", |e, _| {
        e.snapshot();
        e.pending = Pending::UndoBreak;
        e.set_status(say!("edit.undo-break"));
    }, Key::Ctrl('g')),
    ("insert_tab", |e, _| e.insert_indent(true), Key::Tab),
    ("insert_other_tab", |e, _| e.insert_indent(false), Key::BackTab),
];

impl Editor {
    /// **The command `key` runs in Insert mode**, by the default keymap.
    pub(super) fn insert_command_for(&self, key: Key) -> Option<&'static str> {
        INSERT_KEYS
            .iter()
            .find(|(k, _, when)| *k == key && when.is_none_or(|holds| holds(self)))
            .map(|(_, name, _)| *name)
    }

    /// **The reader's `[keys.insert]`, asked first** (2026-10-31): `true`
    /// when it took the key. A key that begins a longer binding is held, as
    /// `j` of `jk` is, and typed after all when the next key does not finish
    /// one.
    pub(super) fn insert_binding(&mut self, key: Key) -> bool {
        if self.insert_aliases.is_empty() || self.expanding_alias {
            return false;
        }
        let typed = match key {
            Key::Char(c) => Some(c.to_string()),
            Key::Alt(c) => Some(format!("{}{c}", yumete_cjk::actions::ALT)),
            key => super::keys::typed_as(key).map(|c| c.to_string()),
        };
        let earlier = std::mem::take(&mut self.insert_held);
        let Some(typed) = typed else {
            self.type_held(&earlier);
            return false;
        };
        let held = format!("{earlier}{typed}");
        if self.insert_aliases.keys().any(|k| k.len() > held.len() && k.starts_with(held.as_str())) {
            self.insert_held = held;
            return true;
        }
        if let Some(commands) = self.insert_aliases.get(&held).cloned() {
            self.run_binding(&commands);
            return true;
        }
        if earlier.is_empty() {
            return false;
        }
        // What was held is typed, and the key that did not finish it may
        // begin a binding of its own.
        self.type_held(&earlier);
        self.insert_binding(key)
    }

    /// Type keys a binding held and did not use, as if pressed then.
    fn type_held(&mut self, held: &str) {
        let mut chars = held.chars();
        while let Some(c) = chars.next() {
            let key = match c {
                yumete_cjk::actions::ALT => match chars.next() {
                    Some(next) => Key::Alt(next),
                    None => break,
                },
                c => super::keys::pressed(c),
            };
            self.insert_typed(key);
        }
    }

    /// Install Insert mode's lines (`[keys.insert]`).
    pub fn set_insert_aliases<C: crate::editor::BindingCommands>(&mut self, aliases: HashMap<String, C>) {
        self.insert_aliases = aliases
            .into_iter()
            .map(|(k, v)| (yumete_cjk::actions::written(&k), v.into_commands()))
            .collect();
        self.insert_held.clear();
    }
}
