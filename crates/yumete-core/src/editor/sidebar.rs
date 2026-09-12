//! The sidebar, the pickers, and the clipboard (#94).
//!
//! One column down the side showing files, buffers, the outline or the
//! dictionary; the pickers that open over it; and the copy/paste that the
//! front end has to be asked for, because a terminal cannot read the
//! clipboard by itself.

use super::*;

impl Editor {
    // ---- The file sidebar (Feature #94) ------------------------------------

    /// What `Space e` and `Space o` do — one rule for both, so neither is the
    /// odd one out.
    ///
    /// A key that names a view answers three different intentions depending on
    /// what is already showing, and all three are what a reader means by
    /// pressing it:
    ///
    /// - closed → open it on that view, with the keys.
    /// - open on **another** view → switch to that view and take the keys. The
    ///   key means "show me the outline", not "toggle the sidebar".
    /// - open on **that** view, unfocused → take the keys back.
    /// - open on that view, focused → put it away. Pressing the same key twice
    ///   undoes it, which is the one thing every toggle must do.
    pub(super) fn show_sidebar(&mut self, view: crate::sidebar::View) {
        match self.sidebar.as_mut() {
            Some(sidebar) if sidebar.view() == view && self.sidebar_focus => {
                self.sidebar = None;
                self.sidebar_focus = false;
            }
            Some(sidebar) => {
                sidebar.show(view);
                self.sidebar_focus = true;
                self.refresh_sidebar();
            }
            None => {
                let root = self.project_root();
                self.open_sidebar_showing(&root, view);
            }
        }
    }

    /// Show the sidebar rooted at `root` and give it the keys.
    pub fn open_sidebar_at(&mut self, root: &Path) {
        self.open_sidebar_showing(root, crate::sidebar::View::Explorer);
    }

    /// Show the sidebar rooted at `root`, opened on `view`.
    pub fn open_sidebar_showing(&mut self, root: &Path, view: crate::sidebar::View) {
        let mut sidebar = crate::sidebar::Sidebar::new(root);
        sidebar.show(view);
        // Open on the file being written, so the tree says where you are rather
        // than making you find yourself in it.
        if let Some(path) = self.current_buffer().path() {
            if let Ok(full) = std::fs::canonicalize(path) {
                sidebar.reveal(&full);
            }
        }
        self.sidebar = Some(sidebar);
        self.sidebar_focus = true;
        self.refresh_sidebar();
    }

    /// Fill the sidebar with whatever its current view shows.
    ///
    /// The tree builds its own rows from the file system; the other two are the
    /// editor's own knowledge, so they are pushed in from here.
    /// Refreshed when it is asked for, not on every keystroke.
    ///
    /// Building the outline walks the document, and doing that per key is the
    /// trap this editor has fallen into three times. Headings do not change
    /// while a sentence is being typed, so the views are rebuilt when the
    /// sidebar is opened, focused, switched, or the file under it changes —
    /// every moment a reader is about to look at it.
    pub(super) fn refresh_sidebar(&mut self) {
        use crate::sidebar::{Row, View};
        let Some(view) = self.sidebar.as_ref().map(|s| s.view()) else {
            return;
        };
        let rows = match view {
            View::Explorer => {
                if let Some(sidebar) = self.sidebar.as_mut() {
                    sidebar.rebuild();
                }
                return;
            }
            // `depth` carries the index the row stands for — the buffer's, or
            // the line's — since a flat list has no depth to spend.
            View::Buffers => self
                .buffers
                .iter()
                .enumerate()
                .map(|(i, b)| Row {
                    path: b.path().map(Path::to_path_buf).unwrap_or_default(),
                    name: format!(
                        "{}{}",
                        b.display_name(),
                        if b.is_modified() { " +" } else { "" }
                    ),
                    depth: i,
                    is_dir: false,
                    expanded: i == self.current,
                })
                .collect(),
            View::Outline if self.current_buffer().syntax() == crate::syntax::Syntax::Typst => {
                self.included_outline()
            }
            View::Outline => self
                .outline()
                .into_iter()
                .map(|(line, level, title)| Row {
                    path: PathBuf::new(),
                    name: format!("{}{title}", "  ".repeat(level.saturating_sub(1))),
                    depth: line,
                    is_dir: false,
                    expanded: false,
                })
                .collect(),
            View::Dictionary => self.dictionary_rows(),
        };
        if let Some(sidebar) = self.sidebar.as_mut() {
            sidebar.set_rows(rows);
        }
    }

    /// The 字典 panel's rows — Feature #215.
    ///
    /// The names are padded to the widest of them so the values line up down a
    /// column, and the padding is counted in **columns** rather than characters
    /// (`拆分` is two characters and four columns wide).
    ///
    /// A row with no value is a heading — the character itself at the top, and
    /// the 陸／臺／港 label above each block when the 拆分表 has more than one
    /// answer. `is_dir` is what the sidebar draws headings with; the flat views
    /// already spend the tree's fields on what they have instead of what a tree
    /// has, and this is that.
    fn dictionary_rows(&self) -> Vec<crate::sidebar::Row> {
        use crate::sidebar::Row;
        let heading = |name: String| Row {
            path: PathBuf::new(),
            name,
            depth: 0,
            is_dir: true,
            expanded: false,
        };
        let Some((ch, answer)) = self.dictionary.as_ref() else {
            return Vec::new();
        };
        let mut rows = vec![heading(ch.to_string())];
        let Some(fields) = answer else {
            return rows;
        };
        if fields.is_empty() {
            rows.push(Row {
                path: PathBuf::new(),
                name: say!("ui.not-in-the-table"),
                depth: 0,
                is_dir: false,
                expanded: false,
            });
            return rows;
        }
        let width = fields
            .iter()
            .filter(|(_, value)| !value.is_empty())
            .map(|(name, _)| yumete_cjk::str_width(name))
            .max()
            .unwrap_or(0);
        for (name, value) in fields {
            if value.is_empty() {
                rows.push(heading(name.clone()));
                continue;
            }
            let pad = " ".repeat(width.saturating_sub(yumete_cjk::str_width(name)));
            rows.push(Row {
                path: PathBuf::new(),
                name: format!("{name}{pad}  {value}"),
                depth: 0,
                is_dir: false,
                expanded: false,
            });
        }
        rows
    }

    /// Look this character up in the 拆分表 — `Space d`, and `Tab` on a
    /// candidate (#215).
    ///
    /// The editor does not hold the table: yume does, and only the front end
    /// has it. So the character is parked here and the panel is opened empty;
    /// the answer arrives on the next pass through the loop, one frame later,
    /// which is not long enough for a reader to see the gap.
    pub fn look_up(&mut self, ch: char, focus: bool) {
        self.dictionary_query = Some(ch);
        // Asked, unanswered: what is showing until the answer arrives is the
        // character alone, which is not the same panel as 「查不到」.
        self.dictionary = Some((ch, None));
        match self.sidebar.as_mut() {
            Some(sidebar) => sidebar.show(crate::sidebar::View::Dictionary),
            None => {
                let root = self.project_root();
                let mut sidebar = crate::sidebar::Sidebar::new(&root);
                sidebar.show(crate::sidebar::View::Dictionary);
                self.sidebar = Some(sidebar);
            }
        }
        // Asked from the page, the keys go with the question. Asked while a
        // word is being typed, they must not — the reader is mid-word, and the
        // panel is only there to be glanced at.
        self.sidebar_focus = focus;
        self.refresh_sidebar();
    }

    /// The character `Space d` or `Tab` asked about, for the front end to
    /// answer once (#215).
    pub fn take_dictionary_query(&mut self) -> Option<char> {
        self.dictionary_query.take()
    }

    /// The answer to [`Editor::take_dictionary_query`].
    ///
    /// Dropped if the reader has since asked about a different character —
    /// the answer to last frame's question must not overwrite this frame's.
    pub fn set_dictionary(&mut self, ch: char, fields: Vec<(String, String)>) {
        if self.dictionary.as_ref().is_some_and(|(at, _)| *at != ch) {
            return;
        }
        self.dictionary = Some((ch, Some(fields)));
        self.refresh_sidebar();
    }

    /// The character the 字典 panel is about, and the answer if one has come.
    pub fn dictionary(&self) -> Option<(char, Option<&[(String, String)]>)> {
        self.dictionary
            .as_ref()
            .map(|(ch, answer)| (*ch, answer.as_deref()))
    }

    /// The sidebar, for the front end to draw.
    pub fn sidebar(&self) -> Option<&crate::sidebar::Sidebar> {
        self.sidebar.as_ref()
    }

    /// Whether the keys are going to the sidebar.
    pub fn sidebar_focused(&self) -> bool {
        self.sidebar_focus && self.sidebar.is_some()
    }

    /// What the sidebar's keys are, for the status line to say while it has
    /// them.
    ///
    /// A pane that takes the keys has to say how to give them back, in the
    /// place a reader already looks for what is going on.
    pub fn sidebar_keys() -> String {
        say!("hint.sidebar.keys")
    }

    /// How many rows `J`/`K` move in a list — a screenful of a sidebar, near
    /// enough. The sidebar does not know how tall it is drawn (the front end
    /// does), and a list moves by a *fixed* amount for the same reason `J`
    /// moves by half a page in the text: the eye keeps its place.
    const PAGE_IN_A_LIST: usize = 12;

    /// Run one key while the sidebar has the keys.
    ///
    /// The same letters that move in the text move here — `j`/`k` down and up,
    /// `l` into, `h` out of — so there is nothing new to learn; only what they
    /// move through is different.
    pub(super) fn on_sidebar_key(&mut self, key: Key) {
        let Some(sidebar) = self.sidebar.as_mut() else {
            self.sidebar_focus = false;
            return;
        };
        match key {
            Key::Char('j') | Key::Down => sidebar.step(true),
            Key::Char('k') | Key::Up => sidebar.step(false),
            // **A list pages by the same keys the page does.** `J`/`K` are
            // half a page in the text; a 700-chapter outline is the one list
            // where walking it by `j` is not walking, and `PageDown` is not on
            // every keyboard a novelist owns.
            Key::Char('J') | Key::PageDown => {
                for _ in 0..Self::PAGE_IN_A_LIST {
                    sidebar.step(true);
                }
            }
            Key::Char('K') | Key::PageUp => {
                for _ in 0..Self::PAGE_IN_A_LIST {
                    sidebar.step(false);
                }
            }
            // …and the ends, spelled as they are in the text.
            Key::Char('g') | Key::Home => sidebar.go_to_end(false),
            Key::Char('G') | Key::End => sidebar.go_to_end(true),
            // The views are built when they are opened, not on every key, so
            // `R` is how a writer who has just added a file or a chapter says
            // to look again.
            Key::Char('R') => self.refresh_sidebar(),
            // A chapter's whole name does not fit in a column narrow enough to
            // be worth keeping open, so `w` trades the columns for the name
            // and back.
            Key::Char('w') => {
                let wide = sidebar.toggle_width();
                self.status = if wide {
                    say!("sidebar.wide")
                } else {
                    say!("sidebar.narrow")
                };
            }
            Key::Char('h') | Key::Left => sidebar.collapse(),
            Key::Char('l') | Key::Right | Key::Enter => {
                let chosen = sidebar.activate();
                match chosen {
                    Some(crate::sidebar::Chosen::File(path)) => {
                        if let Err(err) = self.open_file(&path) {
                            self.status = say!("buffer.cannot-open", path.display(), err);
                        }
                        // Entering a file means going to write in it.
                        self.sidebar_focus = false;
                        self.refresh_sidebar();
                    }
                    Some(crate::sidebar::Chosen::Buffer(i)) => {
                        self.show_buffer(i);
                        self.sidebar_focus = false;
                        self.refresh_sidebar();
                    }
                    Some(crate::sidebar::Chosen::Line(line)) => {
                        self.goto_line(line + 1);
                        self.sidebar_focus = false;
                    }
                    Some(crate::sidebar::Chosen::FileLine(path, line)) => {
                        match self.open_included_file(&path) {
                            Ok(()) => self.goto_line(line + 1),
                            Err(err) => {
                                self.status = say!("buffer.cannot-open", path.display(), err)
                            }
                        }
                        self.sidebar_focus = false;
                        self.refresh_sidebar();
                    }
                    None => {}
                }
            }
            // Tab walks the three views: the project, what is open in it, and
            // the chapter on screen.
            Key::Tab => {
                sidebar.cycle();
                self.refresh_sidebar();
            }
            Key::BackTab => {
                sidebar.cycle();
                sidebar.cycle();
                self.refresh_sidebar();
            }
            // Esc hands the keys back but leaves the tree up; `q` puts it away.
            Key::Esc | Key::Ctrl('w') => self.sidebar_focus = false,
            Key::Char('q') => {
                self.sidebar = None;
                self.sidebar_focus = false;
            }
            // Space still opens the menu, so `Space e` closes the sidebar from
            // inside it exactly as it opened it.
            Key::Char(' ') => self.pending = Pending::Space,
            _ => {}
        }
    }

    /// Open a picker over the files of the project (`Space f`).
    pub(super) fn open_file_picker(&mut self) {
        let root = self.project_root();
        let mut items = Vec::new();
        walk(&root, &mut 0, &mut |path| {
            if items.len() < PICKER_LIMIT {
                let shown = path.strip_prefix(&root).unwrap_or(path);
                items.push(crate::picker::Item::File(shown.display().to_string()));
            }
        });
        if items.is_empty() {
            self.status = say!("picker.no-files-here");
            return;
        }
        self.grep_root = Some(root);
        self.picker = Some(crate::picker::Picker::new(&say!("picker.files"), items));
        self.mode = Mode::Picker;
    }

    /// Open a picker over the buffers already open (`Space b`).
    pub(super) fn open_buffer_picker(&mut self) {
        let items = self
            .buffers
            .iter()
            .enumerate()
            .map(|(i, b)| crate::picker::Item::Buffer(i, b.display_name()))
            .collect();
        self.picker = Some(crate::picker::Picker::new(&say!("picker.buffers"), items));
        self.mode = Mode::Picker;
    }

    /// Whether `Space` is waiting for the key that says what to do — which is
    /// when the which-key menu is drawn.
    pub fn space_pending(&self) -> bool {
        matches!(self.pending, Pending::Space)
    }

    /// The open picker, for the front end to draw.
    pub fn picker(&self) -> Option<&crate::picker::Picker> {
        self.picker.as_ref()
    }

    /// Run one key while a picker is open.
    pub(super) fn on_picker_key(&mut self, key: Key) {
        let Some(picker) = self.picker.as_mut() else {
            self.mode = Mode::Normal;
            return;
        };
        match key {
            Key::Esc => self.close_picker(),
            // Backspace past the start of the query closes it, the way it
            // leaves the `:` line: the query is the only thing to go back over.
            Key::Backspace => {
                if !picker.backspace() {
                    self.close_picker();
                }
            }
            Key::Down | Key::Tab | Key::Ctrl('n') => picker.step(true),
            Key::Up | Key::BackTab | Key::Ctrl('p') => picker.step(false),
            Key::PageDown => {
                for _ in 0..10 {
                    picker.step(true);
                }
            }
            Key::PageUp => {
                for _ in 0..10 {
                    picker.step(false);
                }
            }
            Key::Enter => {
                let chosen = picker.chosen();
                self.close_picker();
                match chosen {
                    Some(crate::picker::Item::File(path)) => {
                        let full = match &self.grep_root {
                            Some(root) => root.join(&path),
                            None => PathBuf::from(&path),
                        };
                        if let Err(err) = self.open_file(&full) {
                            self.status = say!("buffer.cannot-open", path, err);
                        }
                    }
                    Some(crate::picker::Item::Buffer(i, _)) => self.show_buffer(i),
                    // The picker belongs to whichever key opened it, so
                    // choosing from it lands the way that key lands.
                    Some(crate::picker::Item::Row(line, _)) => {
                        let preview = self.definition_preview;
                        self.land_on_row(line, preview)
                    }
                    Some(crate::picker::Item::Paste(Some(which), _)) => {
                        self.paste_from_menu(which)
                    }
                    // The system clipboard is the front end's to read.
                    Some(crate::picker::Item::Paste(None, _)) => self.clipboard_paste(true),
                    None => self.status = say!("picker.nothing-matched"),
                }
            }
            Key::Char(c) => picker.push(c),
            // A query is typed text, and typed text is edited in the middle.
            Key::Delete => picker.delete(),
            Key::Left => picker.move_caret(crate::picker::Caret::Left),
            Key::Right => picker.move_caret(crate::picker::Caret::Right),
            Key::Home | Key::Ctrl('a') => picker.move_caret(crate::picker::Caret::Start),
            Key::End | Key::Ctrl('e') => picker.move_caret(crate::picker::Caret::End),
            Key::Ctrl('u') => picker.clear_before_caret(),
            _ => {}
        }
    }

    /// Shut the picker and go back to Normal.
    fn close_picker(&mut self) {
        self.picker = None;
        self.mode = Mode::Normal;
    }

    /// Insert text that arrived from outside — the system clipboard, by way of
    /// the terminal's bracketed paste (Feature #108).
    ///
    /// It is *writing*, whatever mode the editor is in. Without this a paste is
    /// a stream of keystrokes, and in Normal mode every character of the pasted
    /// paragraph runs as a command: that is not a paste going wrong so much as
    /// the editor running a macro nobody wrote.
    pub fn paste_text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        // **A spreadsheet's clipboard becomes rows** (Feature #226). Excel,
        // Numbers, LibreOffice and a browser table all put tab-separated lines
        // on the clipboard, and until now the cell refused every one of them
        // for holding a tab — the writer got 「格子裏不能有 Tab」 for the one
        // paste a table editor exists to accept. `t p` had already learned to
        // read a block out of the register; this is the same block arriving by
        // the other door, and it lands the same way.
        if (self.mode == Mode::Insert || self.mode == Mode::Normal) && self.table_here() {
            if let Some(grid) = sniff_grid(text.trim_end_matches(['\n', '\r'])) {
                self.paste_grid(grid);
                return;
            }
        }
        // **Judged before anything happens**, in Insert as well as in Normal:
        // the Insert branch used to hand the text to `insert_str`, which
        // silently drops what a cell refuses, and then say 「貼了 8 個字」 about
        // a paste that had not happened. It also spent an undo point on it.
        if self.mode == Mode::Insert || self.mode == Mode::Normal {
            if let Some(why) = self.cell_refuses_text(text) {
                self.status = why;
                return;
            }
        }
        self.snapshot();
        match self.mode {
            // In Insert it lands where the caret is, like anything typed.
            Mode::Insert => self.insert_str(text),
            // In Normal it replaces the selection, which is what `p` over a
            // selection does — and what a writer means by pasting over
            // something they have just picked out.
            Mode::Normal => {
                // **Both halves judged before either runs**, the way `r` and
                // `R` already do it: pasting a comma into a cell used to
                // delete what was selected and *then* refuse the paste, so the
                // cell came back short and the message only talked about the
                // refusal.
                self.delete_selection();
                let at = self.cursor;
                if !self.edit_insert(at, text) {
                    return;
                }
                let rope = self.current_buffer().rope();
                let end = at + text.chars().count();
                let head = motion::prev_grapheme(rope, end).max(at);
                self.anchor = at;
                self.cursor = head;
                self.refresh_goal_column();
            }
            // A prompt takes it as typing, minus the line breaks that would
            // submit it — asked of the mode rather than listed here (#351).
            // The picker is a prompt too, but its query has a store of its
            // own and nothing here reaches it.
            mode => {
                if mode.types_into_command_line() {
                    for c in text.chars().filter(|c| !c.is_control()) {
                        self.command_line.push(c);
                    }
                    self.completion = None;
                }
            }
        }
        self.status = say!("edit.pasted-characters", text.chars().count());
    }

    /// Put the selection on the system clipboard (`Space y`).
    ///
    /// Through OSC 52, the terminal's own copy escape: it needs no library, and
    /// it is the only way that works over ssh and inside tmux, which is where a
    /// terminal editor is often run from. The terminal may refuse — many do by
    /// default — so this says what it asked for rather than claiming success.
    pub(super) fn copy_to_clipboard(&mut self) {
        let (start, end) = self.selection();
        let text = self.current_buffer().rope().slice(start..end).to_string();
        if text.is_empty() {
            self.status = say!("edit.nothing-selected");
            return;
        }
        // Into the editor's own register too: having copied something, `p` is
        // the next thing a hand reaches for.
        self.store(text.clone());
        let n = text.chars().count();
        self.clipboard_request = Some(text);
        self.status = say!("edit.copied-to-clipboard", n);
    }

    /// Put the cursor at char index `pos`, starting a selection there
    /// (Feature #109).
    pub fn point_at(&mut self, pos: usize) {
        let pos = pos.min(self.current_buffer().char_count());
        self.extend = false;
        self.anchor = pos;
        self.cursor = pos;
        self.refresh_goal_column();
        // **The mouse leaves a guessed block too** (#275). Walking out of one
        // with `j` drops it at the end of `on_key`; clicking out of one never
        // went through `on_key`, so the mode stayed on until the next
        // keystroke — the cursor was in the paragraph and `hjkl` were still
        // walking cells.
        self.forget_a_guessed_table();
        self.find_the_table_here();
    }

    /// Drag the selection's head to char index `pos`, keeping its anchor.
    pub fn drag_to(&mut self, pos: usize) {
        self.cursor = pos.min(self.current_buffer().char_count());
        self.refresh_goal_column();
    }

    /// Take a pending clipboard copy, for the front end to send to the terminal.
    pub fn take_clipboard_request(&mut self) -> Option<String> {
        self.clipboard_request.take()
    }

    /// Ask for the system clipboard, to be pasted after (or before) the
    /// selection once the front end has fetched it.
    pub(super) fn clipboard_paste(&mut self, after: bool) {
        self.clipboard_read = Some(after);
    }

    /// Take a pending clipboard read; `true` means paste after.
    pub fn take_clipboard_read(&mut self) -> Option<bool> {
        self.clipboard_read.take()
    }

    /// Hand over what the system clipboard held, and paste it.
    pub fn provide_clipboard(&mut self, text: &str, after: bool) {
        if text.is_empty() {
            self.status = say!("edit.clipboard-empty");
            return;
        }
        self.snapshot();
        // Whole lines go back as whole lines, and the selection is replaced
        // when there is one — the same rules `p` follows, because this is `p`
        // with the text coming from somewhere else.
        self.store(text.to_string());
        self.paste(after);
    }

    /// Move to the first non-blank character of line `n`, counting from 1 and
    /// clamped to the end of the buffer (`10gg`, `:10`, `:goto 10`).
    pub(super) fn goto_line(&mut self, n: usize) {
        self.remember_jump();
        self.move_to_line(n);
    }

    /// The same, without noting a jump.
    ///
    /// For the callers that have already noted one — a mark, `:table-jump` — where a
    /// second note would be of the place *after* the file switch, and `C-o`
    /// would then take you to the file you had just arrived in.
    pub(super) fn move_to_line(&mut self, n: usize) {
        let rope = self.current_buffer().rope();
        let last = motion::last_line(rope);
        let line = n.saturating_sub(1).min(last);
        let at = rope.line_to_char(line);
        let pos = motion::line_first_non_blank(rope, at);
        self.move_head(pos);
    }
}
