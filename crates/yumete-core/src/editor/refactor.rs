//! **What a language server can change, and where else a name is used**
//! (#54, 2026-10-29): helix's `gr` (references), `空格 r` (rename) and
//! `空格 a` (code actions). The questions are put down here and taken by the
//! front end, which sends them and hands back the answers, the way `gd` has
//! worked since #53.

use super::*;
use crate::lsp::{Action, Place, TextEdit};

impl Editor {
    /// Where the caret is, as the wire wants it: the file, the line, and the
    /// column in UTF-16 code units. `None` for a buffer with no path, which no
    /// server can be asked about. Whether a server serves this kind of file is
    /// the front end's to say (prose has one when the config names one).
    fn caret_for_the_server(&self) -> Option<(PathBuf, usize, usize)> {
        let path = self.current_buffer().path()?.to_path_buf();
        let rope = self.current_buffer().rope();
        let head = self.sel.head().min(rope.len_chars());
        let line = rope.char_to_line(head);
        let text = rope.line(line).to_string();
        Some((path, line, crate::problem::utf16_column(&text, head - rope.line_to_char(line))))
    }

    /// **`gr`: where else is this used?**
    pub(super) fn ask_references(&mut self) {
        match self.caret_for_the_server() {
            Some(at) => {
                self.references_query = Some(at);
                self.asked_refs_from = Some((self.current_buffer().id(), self.sel.head()));
                self.status = say!("lsp.asking-references");
            }
            None => self.status = say!("lsp.buffer-unsaved"),
        }
    }

    pub fn take_references_query(&mut self) -> Option<(PathBuf, usize, usize)> {
        self.references_query.take()
    }

    /// **The places come back**: one is gone to, several are offered in a
    /// picker, as helix offers them. An answer to a question asked from
    /// somewhere the caret has since left is dropped, as `gd`'s is.
    pub fn show_references(&mut self, places: Vec<Place>) {
        if self.asked_refs_from.take() != Some((self.current_buffer().id(), self.sel.head())) {
            return;
        }
        match places.len() {
            0 => self.status = say!("lsp.no-references"),
            1 => {
                self.asked_where_from = Some((self.current_buffer().id(), self.sel.head()));
                self.go_to_definition(&places[0]);
            }
            n => {
                let root = self.root();
                let items = places
                    .into_iter()
                    .map(|place| {
                        let text = self.line_of_file(&place.path, place.line).unwrap_or_default();
                        let column = crate::problem::char_column(&text, place.utf16_column);
                        let name = place.path.strip_prefix(&root).unwrap_or(&place.path).display().to_string();
                        let label = format!("{name}:{}  {}", place.line + 1, text.trim());
                        crate::picker::Item::Place(label, place.path, place.line, column)
                    })
                    .collect();
                let picker = crate::picker::Picker::new(&say!("picker.references", n), items);
                self.show_the_picker(picker);
            }
        }
    }

    /// One line of a file: the open buffer's when it is open, the disk's when
    /// it is not.
    pub(super) fn line_of_file(&self, path: &Path, line: usize) -> Option<String> {
        match self.buffers.iter().find(|b| b.path() == Some(path)) {
            Some(open) => {
                let rope = open.rope();
                (line < rope.len_lines()).then(|| crate::motion::line_text(rope, line))
            }
            None => std::fs::read_to_string(path).ok()?.lines().nth(line).map(str::to_string),
        }
    }

    /// **Go to a place a picker offered** (a reference): the file, the line,
    /// the character.
    pub(super) fn go_to_place(&mut self, path: &Path, line: usize, column: usize) {
        self.remember_jump();
        if self.current_buffer().path() != Some(path) {
            if let Err(err) = self.open_file(path) {
                self.status = say!("buffer.cannot-open", path.display(), err);
                return;
            }
        }
        let rope = self.current_buffer().rope();
        let line = line.min(crate::motion::last_line(rope));
        let at = rope.line_to_char(line) + column;
        self.set_cursor(at.min(crate::motion::line_end(rope, rope.line_to_char(line))));
    }

    /// **`空格 r`: the command line, with the name to change already in it.**
    /// The rename is a command (`:lsp-rename <新名>`), so it can be typed too.
    pub(super) fn start_a_rename(&mut self) {
        if self.caret_for_the_server().is_none() {
            self.status = say!("lsp.buffer-unsaved");
            return;
        }
        let word = self.name_under_the_caret();
        self.open_the_command_line(&format!("lsp-rename {word}"));
    }

    /// The identifier the caret is on: letters, digits and `_` around it.
    fn name_under_the_caret(&self) -> String {
        let rope = self.current_buffer().rope();
        let head = self.sel.head().min(rope.len_chars());
        let line = rope.char_to_line(head);
        let start = rope.line_to_char(line);
        let chars: Vec<char> = crate::motion::line_text(rope, line).chars().collect();
        let at = (head - start).min(chars.len());
        let part = |c: &char| c.is_alphanumeric() || *c == '_';
        let from = chars[..at].iter().rev().take_while(|c| part(c)).count();
        let to = chars[at..].iter().take_while(|c| part(c)).count();
        chars[at - from..at + to].iter().collect()
    }

    /// **`:lsp-rename <新名>`**: put the rename down for the front end.
    pub(super) fn ask_rename(&mut self, new_name: &str) {
        let new_name = new_name.trim();
        if new_name.is_empty() {
            self.status = say!("lsp.rename-needs-a-name");
            return;
        }
        match self.caret_for_the_server() {
            Some((path, line, column)) => {
                self.rename_query = Some((path, line, column, new_name.to_string()));
                self.status = say!("lsp.asking-rename");
            }
            None => self.status = say!("lsp.buffer-unsaved"),
        }
    }

    pub fn take_rename_query(&mut self) -> Option<(PathBuf, usize, usize, String)> {
        self.rename_query.take()
    }

    /// **The rename's answer**: the edits, or the server's refusal.
    pub fn renamed(&mut self, edits: Vec<TextEdit>, error: Option<String>) {
        if let Some(why) = error {
            self.status = say!("lsp.rename-refused", why);
            return;
        }
        if edits.is_empty() {
            self.status = say!("lsp.rename-changed-nothing");
            return;
        }
        let (files, places) = self.apply_workspace_edit(&edits);
        self.status = say!("lsp.renamed", places, files);
    }

    /// **`空格 a`: what can be done here?** Over the selection, or the
    /// character under the caret.
    pub(super) fn ask_actions(&mut self) {
        let Some((path, ..)) = self.caret_for_the_server() else {
            self.status = say!("lsp.buffer-unsaved");
            return;
        };
        let rope = self.current_buffer().rope();
        let (start, end) = self.selection();
        let point = |at: usize| {
            let at = at.min(rope.len_chars());
            let line = rope.char_to_line(at);
            let text = rope.line(line).to_string();
            (line, crate::problem::utf16_column(&text, at - rope.line_to_char(line)))
        };
        self.actions_query = Some((path, point(start), point(end)));
        self.status = say!("lsp.asking-actions");
    }

    pub fn take_actions_query(&mut self) -> Option<super::ActionsQuery> {
        self.actions_query.take()
    }

    /// **What the server offers**, in a picker of titles.
    pub fn show_actions(&mut self, actions: Vec<Action>) {
        if actions.is_empty() {
            self.status = say!("lsp.no-actions");
            return;
        }
        let items = actions
            .iter()
            .enumerate()
            .map(|(i, action)| crate::picker::Item::Action(i, action.title.clone()))
            .collect();
        self.offered_actions = actions;
        let picker = crate::picker::Picker::new(&say!("picker.actions"), items);
        self.show_the_picker(picker);
    }

    /// **Do the action picked**: its edit, then its command, which goes to
    /// the server (and usually comes back as an edit to apply).
    pub(super) fn take_action(&mut self, which: usize) {
        let Some(action) = self.offered_actions.get(which).cloned() else {
            return;
        };
        if !action.edits.is_empty() {
            let (files, places) = self.apply_workspace_edit(&action.edits);
            self.status = say!("lsp.action-done", action.title, places, files);
        }
        if let Some((command, arguments)) = action.command {
            self.command_query = Some((action.server, command, arguments));
        }
    }

    /// The server, the command and its arguments.
    pub fn take_command_query(&mut self) -> Option<(String, String, String)> {
        self.command_query.take()
    }

    /// **A server's edit, applied**: every file it names, the open ones in
    /// place and the rest opened, each edit at its UTF-16 position on that
    /// file's line. One undo point per file. The caret goes back where it
    /// was. Answers how many files and how many places changed.
    pub fn apply_workspace_edit(&mut self, edits: &[TextEdit]) -> (usize, usize) {
        let mut by_file: std::collections::BTreeMap<PathBuf, Vec<&TextEdit>> = Default::default();
        for edit in edits {
            by_file.entry(edit.path.clone()).or_default().push(edit);
        }
        let home = self.current_buffer().path().map(Path::to_path_buf);
        let home_index = self.current;
        let (mut files, mut places) = (0, 0);
        for (path, mut edits) in by_file {
            if self.current_buffer().path() != Some(path.as_path()) {
                if let Err(err) = self.open_file(&path) {
                    self.status = say!("buffer.cannot-open", path.display(), err);
                    continue;
                }
            }
            // From the end of the file back, so the positions not yet used
            // still point at the text they were counted against.
            edits.sort_by_key(|edit| std::cmp::Reverse(edit.from));
            self.snapshot();
            let mut changed = 0;
            for edit in edits {
                let rope = self.current_buffer().rope();
                let at = |(line, column): (usize, usize)| -> usize {
                    if line >= rope.len_lines() {
                        return rope.len_chars();
                    }
                    let text = rope.line(line).to_string();
                    rope.line_to_char(line) + crate::problem::char_column(&text, column)
                };
                let (from, to) = (at(edit.from), at(edit.to).max(at(edit.from)));
                let done = self.current_buffer_mut().replace(from..to, &edit.text);
                if self.applied(done) {
                    changed += 1;
                }
            }
            if changed > 0 {
                files += 1;
                places += changed;
            }
        }
        // Back to the file the caret was in.
        let back = home
            .and_then(|p| self.buffers.iter().position(|b| b.path() == Some(p.as_path())))
            .unwrap_or(home_index);
        self.show_buffer(back);
        self.clamp_cursor();
        (files, places)
    }
}
