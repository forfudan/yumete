//! **What a language server can change, and where else a name is used**
//! (#54): helix's `gr` (references), `空格 r` (rename) and
//! `空格 a` (code actions). The questions are put down here and taken by the
//! front end, which sends them and hands back the answers, the way `gd`
//! works (#53).

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
        // A Markdown file has no server; what it has is `[[links]]`, and
        // 「where is this used」 is 「what links here」.
        if self.current_buffer().syntax() == crate::syntax::Syntax::Markdown {
            return self.show_backlinks();
        }
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

    /// **`Space h`, helix's `select_references_to_symbol_under_cursor`**:
    /// the server is asked where this symbol stands in this file.
    pub(super) fn ask_highlights(&mut self) {
        match self.caret_for_the_server() {
            Some(at) => {
                self.highlights_query = Some(at);
                self.asked_highlights_from = Some((self.current_buffer().id(), self.sel.head()));
            }
            None => self.status = say!("lsp.buffer-unsaved"),
        }
    }

    pub fn take_highlights_query(&mut self) -> Option<(PathBuf, usize, usize)> {
        self.highlights_query.take()
    }

    /// **The spans come back, and each becomes a selection**, the one the
    /// caret was on staying primary. An answer to a question asked from
    /// somewhere the caret has since left is dropped.
    pub fn show_highlights(&mut self, spans: Vec<crate::lsp::Span>) {
        if self.asked_highlights_from.take() != Some((self.current_buffer().id(), self.sel.head())) {
            return;
        }
        let head = self.sel.head();
        let rope = self.current_buffer().rope();
        let at = |(line, utf16): (usize, usize)| -> Option<usize> {
            let line = (line < rope.len_lines()).then_some(line)?;
            let text = rope.line(line).to_string();
            Some(rope.line_to_char(line) + crate::problem::char_column(&text, utf16))
        };
        let spans: Vec<(usize, usize)> = spans
            .iter()
            .filter_map(|span| Some((at(span.from)?, at(span.to)?)))
            .filter(|(from, to)| to > from)
            .collect();
        let mut ranges: Vec<crate::selection::Range> = spans.into_iter().map(|(from, to)| self.from_drawn(from, to)).collect();
        if ranges.is_empty() {
            self.status = say!("lsp.no-highlights");
            return;
        }
        ranges.sort_by_key(|r| r.span());
        let primary = ranges.iter().position(|r| r.span().0 <= head && head <= r.span().1).unwrap_or(0);
        self.sel.rebuild(ranges, primary);
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
            n => self.offer_places(&say!("picker.references", n), places),
        }
    }

    /// **`gd` `gD` `gy` `gi` found their places**: one is gone to, several
    /// are offered in a picker, as helix's `goto_impl` does (a trait's
    /// implementations are not cut to the first one).
    pub fn show_places(&mut self, places: Vec<Place>) {
        if places.len() < 2 {
            return match places.first() {
                Some(place) => self.go_to_definition(place),
                None => self.no_definition(),
            };
        }
        if self.asked_where_from.take() != Some((self.current_buffer().id(), self.sel.head())) {
            return;
        }
        let n = places.len();
        let title = match self.definition_kind {
            crate::lsp::Goto::Definition => say!("picker.definitions", n),
            crate::lsp::Goto::Declaration => say!("picker.declarations", n),
            crate::lsp::Goto::TypeDefinition => say!("picker.type-definitions", n),
            crate::lsp::Goto::Implementation => say!("picker.implementations", n),
        };
        self.offer_places(&title, places);
    }

    /// A picker over places a server named, each with its line's text.
    fn offer_places(&mut self, title: &str, places: Vec<Place>) {
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
        let picker = crate::picker::Picker::new(title, items);
        self.show_the_picker(picker);
    }

    /// **`Space j`, helix's `jumplist_picker`**: every place `C-o` can go
    /// back to, the newest first, each once. A place in a buffer with no file,
    /// or one since closed, is left out.
    pub(super) fn open_jumplist_picker(&mut self) {
        let root = self.root();
        let here = self.working_dir();
        let mut seen: Vec<(PathBuf, usize)> = Vec::new();
        let mut items = Vec::new();
        for jump in self.jumps.iter().rev() {
            let Some((index, at)) = self.jump_place(jump) else { continue };
            let open = &self.buffers[index];
            let Some(path) = open.path().map(|p| here.join(p)) else { continue };
            let rope = open.rope();
            let at = at.min(rope.len_chars());
            let line = rope.char_to_line(at);
            if seen.contains(&(path.clone(), line)) {
                continue;
            }
            seen.push((path.clone(), line));
            let column = at - rope.line_to_char(line);
            let text = rope.line(line).to_string();
            let name = path.strip_prefix(&root).unwrap_or(&path).display().to_string();
            let label = format!("{name}:{}  {}", line + 1, text.trim());
            items.push(crate::picker::Item::Place(label, path, line, column));
        }
        if items.is_empty() {
            self.status = say!("picker.no-jumps");
            return;
        }
        let picker = crate::picker::Picker::new(&say!("picker.jumplist"), items);
        self.show_the_picker(picker);
    }

    /// **`Space '`, helix's `last_picker`**: the picker closed last, as it
    /// was left.
    pub(super) fn reopen_last_picker(&mut self) {
        match self.last_picker.clone() {
            Some(picker) => {
                self.show_the_picker(picker);
            }
            None => self.status = say!("picker.none-yet"),
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

    /// **`=`, helix's `format_selections`**: the language server is asked to
    /// format the selection, one selection only, as helix asks it.
    pub(super) fn ask_format(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        if self.sel.is_plural() {
            self.status = say!("lsp.format-one-selection");
            return;
        }
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
        self.format_query = Some((path, point(start), point(end), self.indent_width(), self.tab_spaces()));
        self.asked_format_from = Some((self.current_buffer().id(), self.current_buffer().revision()));
        self.status = say!("lsp.asking-format");
    }

    pub fn take_format_query(&mut self) -> Option<super::FormatQuery> {
        self.format_query.take()
    }

    /// **`=`'s answer**: the edits, made to this file, or why not. An answer
    /// to a text that has changed since it was asked is dropped.
    pub fn formatted(&mut self, edits: Vec<TextEdit>, error: Option<String>) {
        let here = (self.current_buffer().id(), self.current_buffer().revision());
        if self.asked_format_from.take() != Some(here) {
            return;
        }
        if let Some(why) = error {
            self.status = say!("lsp.format-refused", why);
            return;
        }
        if edits.is_empty() {
            self.status = say!("lsp.format-changed-nothing");
            return;
        }
        let Some(path) = self.current_file_path() else { return };
        let edits: Vec<TextEdit> = edits.into_iter().map(|edit| TextEdit { path: path.clone(), ..edit }).collect();
        self.apply_workspace_edit(&edits);
        self.status = say!("lsp.formatted");
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
