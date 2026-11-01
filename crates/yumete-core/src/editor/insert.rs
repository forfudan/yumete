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
use yumete_cjk::actions::NormalCmd;
use crate::input::Key;
use crate::motion;

/// Where an Insert-mode key holds.
type InsertWhen = Option<fn(&Editor) -> bool>;

/// Insert mode's default keymap: a key, the command it runs, and where it
/// holds (the vim preset's `C-o`).
pub(super) const INSERT_KEYS: &[(Key, NormalCmd, InsertWhen)] = &[
    (Key::Esc, NormalCmd::NormalMode, None),
    (Key::Enter, NormalCmd::InsertNewline, None),
    (Key::Ctrl('j'), NormalCmd::InsertNewline, None),
    (Key::Backspace, NormalCmd::DeleteCharBackward, None),
    (Key::Ctrl('h'), NormalCmd::DeleteCharBackward, None),
    (Key::Delete, NormalCmd::DeleteCharForward, None),
    (Key::Ctrl('d'), NormalCmd::DeleteCharForward, None),
    (Key::Alt('d'), NormalCmd::DeleteWordForward, None),
    (Key::Ctrl('w'), NormalCmd::DeleteWordBackward, None),
    (Key::Ctrl('u'), NormalCmd::VimKillToLineStart, Some(|e| e.vim_keymap())),
    (Key::Ctrl('k'), NormalCmd::VimKillToLineEnd, Some(|e| e.vim_keymap())),
    (Key::Ctrl('u'), NormalCmd::KillToLineStart, None),
    (Key::Ctrl('k'), NormalCmd::KillToLineEnd, None),
    (Key::Ctrl('r'), NormalCmd::InsertRegister, None),
    (Key::Ctrl('o'), NormalCmd::RunOneNormalCommand, Some(|e| e.vim_keymap())),
    (Key::Left, NormalCmd::MoveCharLeft, None),
    (Key::Right, NormalCmd::MoveCharRight, None),
    (Key::Up, NormalCmd::MoveVisualLineUp, None),
    (Key::Down, NormalCmd::MoveVisualLineDown, None),
    (Key::PageUp, NormalCmd::PageUp, None),
    (Key::PageDown, NormalCmd::PageDown, None),
    (Key::Home, NormalCmd::GotoLineStart, None),
    (Key::Ctrl('a'), NormalCmd::GotoLineStart, None),
    (Key::End, NormalCmd::GotoLineEndNewline, None),
    (Key::Ctrl('e'), NormalCmd::GotoLineEndNewline, None),
    // vim's `C-g u` (helix's `C-s`, which a terminal takes for XOFF).
    (Key::Ctrl('g'), NormalCmd::CommitUndoCheckpoint, None),
    (Key::Tab, NormalCmd::InsertTab, None),
    (Key::BackTab, NormalCmd::InsertOtherTab, None),
];

/// **The commands only Insert mode's keys run.** Bound in Normal mode they
/// do the same thing there (`insert_newline` writes a line break).
pub(super) const INSERT_COMMANDS: &[NormalCommand] = &[
    (NormalCmd::InsertNewline, |e, _| e.insert_newline(), Key::Enter),
    (NormalCmd::DeleteCharForward, |e, _| e.delete_at_cursor(), Key::Delete),
    (NormalCmd::DeleteWordForward, |e, _| e.delete_word_after_cursor(), Key::Alt('d')),
    (NormalCmd::DeleteWordBackward, |e, _| e.delete_word_before_cursor(), Key::Ctrl('w')),
    (NormalCmd::KillToLineStart, |e, _| e.acting(false, |e| e.delete_to_line_start()), Key::Ctrl('u')),
    (NormalCmd::KillToLineEnd, |e, _| e.acting(false, |e| e.delete_to_line_end()), Key::Ctrl('k')),
    // vim's: `C-u` back to where this Insert began, `C-k` never the break.
    (NormalCmd::VimKillToLineStart, |e, _| e.acting(true, |e| e.delete_to_line_start()), Key::Ctrl('u')),
    (NormalCmd::VimKillToLineEnd, |e, _| e.acting(true, |e| e.delete_to_line_end()), Key::Ctrl('k')),
    (NormalCmd::InsertRegister, |e, _| e.pending = Pending::Register, Key::Ctrl('r')),
    // vim's `i_CTRL-O`: one Normal-mode command, then back.
    (NormalCmd::RunOneNormalCommand, |e, _| {
        e.one_normal_key = Some(Mode::Insert);
        e.mode = Mode::Normal;
    }, Key::Ctrl('o')),
    (NormalCmd::GotoLineEndNewline, |e, _| {
        let pos = motion::line_end(e.current_buffer().rope(), e.sel.head());
        e.set_cursor(pos);
    }, Key::End),
    (NormalCmd::CommitUndoCheckpoint, |e, _| {
        e.snapshot();
        e.pending = Pending::UndoBreak;
        e.set_status(say!("edit.undo-break"));
    }, Key::Ctrl('g')),
    (NormalCmd::InsertTab, |e, _| e.insert_indent(true), Key::Tab),
    (NormalCmd::InsertOtherTab, |e, _| e.insert_indent(false), Key::BackTab),
];

impl Editor {
    /// **The command `key` runs in Insert mode**, by the default keymap.
    pub(super) fn insert_command_for(&self, key: Key) -> Option<&'static str> {
        INSERT_KEYS
            .iter()
            .find(|(k, _, when)| *k == key && when.is_none_or(|holds| holds(self)))
            .map(|(_, name, _)| name.name())
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
