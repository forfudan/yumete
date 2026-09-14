//! The search panel's own half of the editor — Feature #419.
//!
//! Running what the box holds, and the keys of the form it sits in. The
//! panel's *state* is [`crate::search_panel`]; drawing it is the front end's.

use super::*;
use crate::search_panel::{Case, Field, Hit, Where, AROUND, MOST};
use std::path::{Path, PathBuf};

/// How much of the line the command row shows around the highlighted hit.
///
/// The panel is a column; this row is the window. Thirty-odd characters each
/// way is a sentence, which is what tells one 「霜」 from another.
const WIDE_AROUND: usize = 36;

impl Editor {
    /// The search panel, for the front end to draw.
    pub fn search(&self) -> &crate::search_panel::Search {
        &self.search
    }

    /// The pattern `n` and `N` are walking — the panel writes it too (#419).
    pub fn last_search(&self) -> &str {
        &self.last_search
    }

    /// **Open the panel and put a pattern in it, selected** — `空格 /`, #419.
    ///
    /// What goes in the box: the selection if there is one (you marked it, the
    /// intention is on the screen), otherwise the last thing searched for. It
    /// arrives selected, so typing replaces it and `Enter` keeps it — both
    /// intentions in one key, which is how VSCode's box behaves.
    /// `:search-cd`／`-wd`／`-gd`／`:search <path>` — open it looking somewhere
    /// else (#419).
    pub(super) fn open_search_in(&mut self, scope: Where, replacing: bool) {
        // ⚠️ **A folder that is not there is said out loud.** Falling back to
        // 「this file only」 would answer a question nobody asked, and answer
        // it plausibly — a short list that looks like the truth.
        if let Where::Named(path) = &scope {
            let named = path.clone();
            if self.search_root_of(&scope).is_none() {
                self.status = say!("search.no-such-folder", named.display());
                return;
            }
        }
        self.search.scope = scope;
        // ⚠️ **Only ever turned on here.** `:search` after a `:replace` is a
        // reader saying 「just looking」, and leaving the row up would leave
        // `r` and `R` live on a panel nobody meant to change anything with.
        self.search.replacing = replacing;
        self.open_search();
    }

    pub(super) fn open_search(&mut self) {
        // ⚠️ A selection wins over the last pattern — but only one somebody
        // **made**. Every motion in this editor leaves a selection and the
        // cursor covers its own grapheme, so 「one character」 is where the
        // cursor is standing, not something marked; taking it would mean the
        // box never once opened holding the last pattern. And half a chapter
        // is a paste, not a search.
        let (from, to) = self.selection();
        let rope = self.current_buffer().rope();
        let marked: String = match to > from + 1 && to - from <= 64 {
            true => rope.slice(from..to.min(rope.len_chars())).chars().collect(),
            false => String::new(),
        };
        // ⚠️ **The box remembers what was typed, not what was compiled.**
        // `last_search` holds the pattern the engine runs — flags and all —
        // and showing `(?i)霜` to a reader who typed 霜 would be showing them
        // the plumbing. The panel's own query is that memory; `last_search` is
        // the fallback for a `/` typed on the page.
        let seed = match (!marked.is_empty() && !marked.contains('\n'), self.search.query.is_empty()) {
            (true, _) => marked,
            (false, false) => self.search.query.clone(),
            (false, true) => self.last_search.clone(),
        };
        self.search.ask(seed);
        self.show_sidebar(crate::sidebar::View::Search);
        // The form is entered where a reader would start typing.
        self.mode = Mode::Field;
        // ⚠️ **Opened onto a folder, it looks straight away** rather than
        // waiting for an `Enter` nobody knows to press: the reader just named
        // a place, and the pattern was already in the box.
        match self.search.scope.live() {
            true => self.run_search(),
            false => self.search_now(),
        }
    }

    /// What the box holds, as a pattern the engine understands.
    ///
    /// Four settings fold into one string here rather than into four branches
    /// at the point of use: 正則 off escapes the whole thing, 完整匹配 wraps it
    /// in `\b`, and 大小寫 puts the flag on the front. The engine sees one
    /// pattern and the panel is the only place that knows why.
    fn search_pattern(&self) -> String {
        let mut body = match self.search.regex {
            true => self.search.query.clone(),
            false => regex::escape(&self.search.query),
        };
        // ⚠️ **`\b` is nothing between 漢字.** There is no word boundary
        // there, so this only ever bites on the Western words in a manuscript
        // — which is what it does in VSCode too, and what the manual says.
        if self.search.whole {
            body = format!(r"\b{body}\b");
        }
        // ⚠️ **Sensitive says so out loud** (`(?-i)`), it does not just stay
        // silent. The page's own `n` runs this pattern through smart case
        // again (`compile`), which would put `(?i)` in front of a quiet one
        // and search differently from the panel that produced it. Flags apply
        // in order, so an explicit one at the front wins.
        match self.search.case {
            Case::Sensitive => format!("(?-i){body}"),
            Case::Insensitive => format!("(?i){body}"),
            // The rule the page's own `/` follows: a capital is how you ask.
            Case::Smart => match body.chars().any(char::is_uppercase) {
                true => body,
                false => format!("(?i){body}"),
            },
        }
    }

    /// **Where the search is rooted**, for a scope that is not the buffer.
    ///
    /// `None` when the answer cannot be worked out — no file open to reckon
    /// from, or a named folder that is not there.
    fn search_root(&self) -> Option<PathBuf> {
        self.search_root_of(&self.search.scope)
    }

    /// The same, for a scope that has not been adopted yet.
    fn search_root_of(&self, scope: &Where) -> Option<PathBuf> {
        match scope {
            Where::Buffer => None,
            // The file's own folder. ⚠️ Not the project: a book's drafts, its
            // notes and its exports live under one tree, and 「this folder and
            // what is under it」 is the near thing a reader means.
            Where::Folder => Some(self.here_folder()),
            Where::Workspace => std::env::current_dir().ok(),
            Where::Project => Some(self.project_root()),
            Where::Named(path) => {
                let full = match path.is_absolute() {
                    true => path.clone(),
                    false => self.here_folder().join(path),
                };
                full.is_dir().then_some(full)
            }
        }
    }

    /// The folder the file being written is in, as an absolute path.
    ///
    /// ⚠️ **Resolved against the working directory first.** A buffer opened as
    /// `a.md` has a relative path, and its `parent()` is the *empty* path —
    /// which as a root walks nothing at all, so 「this folder」 quietly found
    /// only the file already open.
    pub(super) fn here_folder(&self) -> PathBuf {
        let here = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        match self.current_buffer().path() {
            Some(path) => here.join(path).parent().map(Path::to_path_buf).unwrap_or(here),
            None => here,
        }
    }

    /// **Run what the box holds** — #419.
    ///
    /// The buffer is searched on every keystroke: it is in memory and a pass
    /// over it costs nothing worth counting. ⚠️ **Anything wider waits for
    /// `Enter`** — a hundred chapters read off the disk per letter typed is
    /// not a thing to do — and until then the panel says so rather than
    /// showing a list that answers an older question.
    pub(super) fn run_search(&mut self) {
        if !self.search.scope.live() {
            self.search.stale = true;
            return;
        }
        self.search_now();
    }

    /// Run it whatever the scope, walking the disk if that is what it takes.
    pub(super) fn search_now(&mut self) {
        self.search.broken = false;
        if !self.search.asked() {
            self.search.hits.clear();
            self.search.total = 0;
            self.search.selected = 0;
            return;
        }
        let pattern = self.search_pattern();
        let re = match regex::Regex::new(&pattern) {
            Ok(re) => re,
            Err(_) => {
                // ⚠️ The hits stay, and are drawn quiet. Typing a regular
                // expression walks through `[`, `(` and every other unfinished
                // state; emptying the list on each of them flickers, and a
                // blank list would say 「nothing found」, which is not true.
                self.search.broken = true;
                return;
            }
        };
        self.search.hits.clear();
        self.search.total = 0;
        self.search.selected = 0;
        self.search.folded.clear();
        self.search.stale = false;
        // **The pattern hands the search to the panel, and the page follows.**
        // One 「what am I looking for」 with two ways in: the highlight and
        // `n`/`N` are the same search, which is what [^415]记 `:grep` 不寫
        // `last_search` 為缺口的那條理由.
        self.last_search = pattern;
        let root = self.search_root();
        // ⚠️ **Compared as absolute paths.** A buffer opened as `a.md` and the
        // same file coming out of the walk as `/…/卷一/a.md` are one file, and
        // the guard that failed to see that searched it twice.
        let here = self
            .current_buffer()
            .path()
            .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()));
        // Where the file being written sits in the answer: under its own name
        // when the answer has names in it, and namelessly when it is the whole
        // of the answer.
        let mine = match (&root, &here) {
            (Some(root), Some(here)) => Some(
                here.strip_prefix(std::fs::canonicalize(root).as_deref().unwrap_or(root))
                    .unwrap_or(here)
                    .to_path_buf(),
            ),
            _ => None,
        };
        let mut hits = Vec::new();
        let mut total = 0usize;
        // It comes first, and it comes from memory: what is on the screen is
        // what is searched, saved or not.
        let rope = self.current_buffer().rope();
        let mut at = 0usize;
        for line in 0..rope.len_lines() {
            let text: String = rope.line(line).chars().collect();
            for (nth, m) in re.find_iter(&text).enumerate() {
                total += 1;
                if hits.len() < MOST {
                    let start = text[..m.start()].chars().count();
                    let stop = text[..m.end()].chars().count();
                    hits.push(excerpt(mine.clone(), &text, at, start, stop, line, nth));
                }
            }
            at += text.chars().count();
        }
        if let Some(root) = root {
            let mut files = Vec::new();
            crate::editor::walk(&root, &mut 0, &mut |path| files.push(path.to_path_buf()));
            for path in files {
                // Not twice: the one being written was searched from memory.
                let full = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
                if Some(&full) == here.as_ref() {
                    continue;
                }
                // **An open file is read from its buffer, not from disk.**
                // Unsaved work is work, and a search that could not see it
                // would send a reader to a line that no longer says that.
                // ⚠️ **Matched on the resolved path.** `/tmp` is a link to
                // `/private/tmp` on this platform, so the walk's path and the
                // buffer's are two spellings of one file — compared as typed,
                // a file just changed in a buffer was re-read off the disk and
                // the change looked as though it had not happened.
                let text = match self.buffers.iter().find(|b| {
                    b.path()
                        .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
                        .as_deref()
                        == Some(full.as_path())
                }) {
                    Some(buffer) => buffer.rope().to_string(),
                    None => match std::fs::read_to_string(&path) {
                        Ok(text) => text,
                        // Not text, or not readable: not this writer's prose.
                        Err(_) => continue,
                    },
                };
                let shown = path.strip_prefix(&root).unwrap_or(&path).to_path_buf();
                let mut at = 0usize;
                for (line, text) in text.split_inclusive('\n').enumerate() {
                    for (nth, m) in re.find_iter(text).enumerate() {
                        total += 1;
                        if hits.len() < MOST {
                            let start = text[..m.start()].chars().count();
                            let stop = text[..m.end()].chars().count();
                            hits.push(excerpt(
                                Some(shown.clone()),
                                text,
                                at,
                                start,
                                stop,
                                line,
                                nth,
                            ));
                        }
                    }
                    at += text.chars().count();
                }
            }
            self.search.root = Some(root);
        } else {
            self.search.root = None;
        }
        self.search.hits = hits;
        self.search.total = total;
    }

    /// Rerun the search and go to the first hit at or after the cursor.
    fn search_again(&mut self) {
        self.run_search();
        if self.search.hits.is_empty() {
            return;
        }
        let at = self.cursor;
        self.search.selected = self
            .search
            .hits
            .iter()
            .position(|h| h.at >= at)
            .unwrap_or(0);
    }

    /// One key while a field of the search panel has them (`Mode::Field`).
    pub(super) fn on_field_key(&mut self, key: Key) {
        match key {
            // **`Enter` is 「next」, and the keys stay in the box** (#419).
            // Changing the pattern is the commonest thing anybody does in a
            // search, and handing the keys away would mean coming back for
            // every letter. 「Previous」 has no key here: `Esc` out and `N`.
            Key::Enter => {
                self.search.all_selected = false;
                // **Across files, `Enter` is 「go and look」**; in this one it
                // is 「the next place」, because the looking already happened
                // as you typed.
                if !self.search.scope.live() {
                    return self.search_now();
                }
                self.repeat_search(true);
                self.search_again();
            }
            Key::Char(ch) => {
                self.search.type_char(ch);
                self.run_search();
            }
            Key::Backspace => {
                self.search.backspace();
                self.run_search();
            }
            Key::Left => {
                let to = self.search.caret.saturating_sub(1);
                self.search.move_caret(to);
            }
            Key::Right => {
                let to = self.search.caret + 1;
                self.search.move_caret(to);
            }
            Key::Home => self.search.move_caret(0),
            Key::End => {
                let to = self.search.query.chars().count();
                self.search.move_caret(to);
            }
            // The word and the line, as they are on the `:` line.
            Key::Ctrl('u') => {
                self.search.ask(String::new());
                self.run_search();
            }
            Key::Tab => self.leave_field(false),
            Key::BackTab => self.leave_field(true),
            // Out of the box, into the panel's own Normal.
            Key::Esc => self.mode = Mode::Normal,
            _ => {}
        }
    }

    /// A committed string from the IME lands in the field, not in the page.
    pub(super) fn type_into_field(&mut self, text: &str) {
        self.search.type_text(text);
        self.run_search();
    }

    /// `Tab` out of a box: the next cell.
    ///
    /// ⚠️ **Landing on another box keeps you typing.** 找什麼 and 換成什麼 sit
    /// one above the other and are filled in one after the other; having to
    /// press `i` between them would make `Tab` the wrong key for the commonest
    /// thing anybody does in this panel.
    fn leave_field(&mut self, back: bool) {
        self.search.field = self.search.field.step(back, self.search.replacing);
        self.search.all_selected = false;
        match self.search.field.takes_text() {
            true => self.search.caret = self.search.typed().chars().count(),
            false => self.mode = Mode::Normal,
        }
    }

    /// One key while the search panel has them and the box does not
    /// (`Mode::Normal`, the keys in the panel).
    pub(super) fn on_search_panel_key(&mut self, key: Key, side: crate::sidebar::Side) {
        match key {
            Key::Tab => self.search.field = self.search.field.step(false, self.search.replacing),
            Key::BackTab => self.search.field = self.search.field.step(true, self.search.replacing),
            // `hjkl` walk the form in Normal, as the author asked; in the list
            // `j`/`k` walk the hits instead, because that is what is there.
            // In the list, `h`/`l` fold a file away and open it again — the
            // same 「less of this / more of this」 the tree and the outline
            // mean by them. Elsewhere in the form they walk the cells.
            Key::Char('h') | Key::Left if self.search.field == Field::Results => {
                if !self.search.fold(true) {
                    self.search.field = self.search.field.step(true, self.search.replacing);
                }
            }
            Key::Char('l') | Key::Right if self.search.field == Field::Results => {
                self.search.fold(false);
            }
            Key::Char('l') | Key::Right => self.search.field = self.search.field.step(false, self.search.replacing),
            Key::Char('h') | Key::Left => self.search.field = self.search.field.step(true, self.search.replacing),
            Key::Char('j') | Key::Down => match self.search.field {
                Field::Results => self.search.step(true),
                _ => self.search.field = self.search.field.step(false, self.search.replacing),
            },
            Key::Char('k') | Key::Up => match self.search.field {
                Field::Results => self.search.step(false),
                _ => self.search.field = self.search.field.step(true, self.search.replacing),
            },
            // **On a switch, 空格 flips it** — a switch is the one control
            // where a reader tries the space bar. Anywhere else in the form
            // 空格 is still the menu key it is everywhere else.
            Key::Char(' ')
                if matches!(self.search.field, Field::Regex | Field::Case | Field::Whole) =>
            {
                self.flip_switch()
            }
            Key::Enter => match self.search.field {
                Field::Query | Field::Replace => self.mode = Mode::Field,
                Field::Regex | Field::Case | Field::Whole => self.flip_switch(),
                Field::Results => self.go_to_hit(),
            },
            // **`r` and `R` change things**, and only while the replace row
            // is showing — `:search` is for looking, `:replace` for changing,
            // and the panel says which it is.
            Key::Char('r') if self.search.replacing && self.search.field == Field::Results => {
                match self.search.row() {
                    Some(crate::search_panel::Row::File { path, .. }) => {
                        let done = self.replace_file(Some(&path));
                        self.after_replacing(done);
                    }
                    _ => self.replace_hit(),
                }
            }
            // ⚠️ **Only 「all of them」 asks first.** One hit and one file are
            // changes a reader is looking straight at; every file in a book is
            // not, and that is the one where a slip costs an afternoon.
            Key::Char('R') if self.search.replacing => {
                self.status = say!("search.replace-all-sure", self.search.total);
                // ⚠️ **`ReplaceAll`, not `Confirm`.** The latter is `:s …c`'s
                // per-match walker: with nothing to walk it clears itself on
                // the next key, so the question was asked and the answer went
                // nowhere.
                self.pending = Pending::ReplaceAll;
            }
            // **`F5` runs it again**, for a scope that does not run itself.
            Key::Char('F') if !self.search.scope.live() => self.search_now(),
            // `i` opens a box — this one if the keys are on one, the query
            // otherwise. The key that means 「type here」 everywhere else.
            Key::Char('i') => {
                if !self.search.field.takes_text() {
                    self.search.field = Field::Query;
                }
                self.search.caret = self.search.typed().chars().count();
                self.mode = Mode::Field;
            }
            Key::Char('g') | Key::Home if self.search.field == Field::Results => {
                self.search.selected = 0
            }
            Key::Char('G') | Key::End if self.search.field == Field::Results => {
                self.search.selected = self.search.hits.len().saturating_sub(1)
            }
            // `C-w` `q` `:` and a bare `Space` are every panel's, not this
            // one's — see `panel_key_in_common`. Tried last, so this panel's
            // own `Space` (flip the switch the keys are on) still wins.
            other => {
                self.panel_key_in_common(other, side, crate::sidebar::Layer::Top);
            }
        }
    }

    /// **The highlighted hit, with room to read it** — the command row (#419).
    ///
    /// `None` unless the keys are actually in the list: the row belongs to
    /// whatever has them, and a panel nobody is standing in has no claim on it.
    pub(super) fn hit_in_context(&self) -> Option<String> {
        if self.search.field != Field::Results || self.search.broken {
            return None;
        }
        let (side, layer) = self.panel_focus()?;
        if layer != crate::sidebar::Layer::Top
            || self.panel(side).map(|p| p.view()) != Some(crate::sidebar::View::Search)
        {
            return None;
        }
        let hit = self.search.here()?;
        // As many characters as a window is wide, centred on the match — far
        // more than the column can hold, which is the whole point.
        let rope = self.current_buffer().rope();
        let line: String = rope.line(hit.line).chars().filter(|c| *c != '\n').collect();
        let chars: Vec<char> = line.chars().collect();
        let at = hit.at.saturating_sub(rope.line_to_char(hit.line));
        let from = at.saturating_sub(WIDE_AROUND);
        let to = (at + WIDE_AROUND).min(chars.len());
        let mut text = String::new();
        if from > 0 {
            text.push('…');
        }
        text.extend(&chars[from..to]);
        if to < chars.len() {
            text.push('…');
        }
        Some(say!("search.in-context", hit.line + 1, text))
    }

    /// Flip whichever switch the keys are on, and search again.
    fn flip_switch(&mut self) {
        match self.search.field {
            Field::Regex => self.search.regex = !self.search.regex,
            Field::Case => self.search.case = self.search.case.next(),
            Field::Whole => self.search.whole = !self.search.whole,
            _ => return,
        }
        self.run_search();
    }

    // ---- Changing what was found (#419 三) --------------------------------
    //
    // **The safety is the order, and it is the whole design.** There is no
    // project-wide substitute anybody can type blind: the pattern is the one
    // already in the box, its hits are already listed, and every change is
    // made **in a buffer** — not a byte reaches the disk until `:write-all`.
    // So `u` takes any one file back, `gn` walks them, and the moment a person
    // says yes is a moment they choose.

    /// `r` on a hit: change **that one**.
    fn replace_hit(&mut self) {
        let Some(hit) = self.search.here().cloned() else {
            return;
        };
        let Some(re) = self.search_regex() else { return };
        let with = self.search.replace.clone();
        match self.buffer_of(&hit) {
            Some(index) => {
                let done = self.with_buffer(index, |ed| ed.swap_one(&re, &with, hit.line, hit.nth));
                match done {
                    true => self.after_replacing(1),
                    // The line has fewer matches than it had when it was read:
                    // somebody has edited it since. Saying so beats changing
                    // whatever is in that position now.
                    false => self.status = say!("search.moved-on"),
                }
            }
            None => self.status = say!("search.moved-on"),
        }
    }

    /// `r` on a file header, and each step of `R`: change **every hit in one
    /// file**.
    fn replace_file(&mut self, rel: Option<&Path>) -> usize {
        let Some(re) = self.search_regex() else { return 0 };
        let with = self.search.replace.clone();
        let Some(index) = self.buffer_for(rel) else {
            return 0;
        };
        self.with_buffer(index, |ed| ed.swap_all(&re, &with))
    }

    /// `R`: change every hit there is, once the reader has said yes.
    pub(super) fn replace_all_found(&mut self) {
        let mut files: Vec<Option<PathBuf>> = Vec::new();
        for hit in &self.search.hits {
            if !files.contains(&hit.file) {
                files.push(hit.file.clone());
            }
        }
        let mut done = 0usize;
        for file in files {
            done += self.replace_file(file.as_deref());
        }
        self.after_replacing(done);
    }

    /// Change **one match on one line** of the buffer being worked on.
    ///
    /// Found again by running the pattern over the line as it stands, rather
    /// than by the offsets the hit carries: those were counted when the file
    /// was read, and the first change in a file moves every one after it.
    /// `false` when the line no longer holds that many matches.
    fn swap_one(&mut self, re: &regex::Regex, with: &str, line: usize, nth: usize) -> bool {
        let rope = self.current_buffer().rope();
        if line >= rope.len_lines() {
            return false;
        }
        let text: String = rope.line(line).chars().collect();
        // `captures_iter` rather than `find_iter`, so `$1` in the replacement
        // works the way it does in `:s`.
        let Some(caps) = re.captures_iter(&text).nth(nth) else {
            return false;
        };
        let Some(m) = caps.get(0) else { return false };
        let at = rope.line_to_char(line);
        let from = at + text[..m.start()].chars().count();
        let to = at + text[..m.end()].chars().count();
        let mut grown = String::new();
        caps.expand(with, &mut grown);
        let mut rebuilt = rope.to_string();
        let (b0, b1) = (rope.char_to_byte(from), rope.char_to_byte(to));
        rebuilt.replace_range(b0..b1, &grown);
        self.write_whole(rebuilt)
    }

    /// Change **every match** in the buffer being worked on; how many.
    fn swap_all(&mut self, re: &regex::Regex, with: &str) -> usize {
        let rope = self.current_buffer().rope();
        let text = rope.to_string();
        let count = re.find_iter(&text).count();
        if count == 0 {
            return 0;
        }
        let rebuilt = re.replace_all(&text, with).into_owned();
        match self.write_whole(rebuilt) {
            true => count,
            false => 0,
        }
    }

    /// Put a rewritten document back, with the guards a `:s` gets.
    fn write_whole(&mut self, rebuilt: String) -> bool {
        // ⚠️ The grid guard, for the same reason `:s` has it: a substitution
        // that changes how many cells a row has turns a 拆分表 into rubbish,
        // and it cannot be seen happening in a file nobody is looking at.
        if let Some(why) = self.substitution_breaks_the_grid(&rebuilt) {
            self.status = why;
            return false;
        }
        // One snapshot per file, so `u` in that file takes the whole of this
        // back — not one match at a time.
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(0..len, &rebuilt));
        if !self.applied(done) {
            return false;
        }
        self.clamp_cursor();
        self.anchor = self.cursor;
        self.refresh_goal_column();
        true
    }

    /// Which buffer a hit is in, opening the file if it is not open yet.
    fn buffer_of(&mut self, hit: &Hit) -> Option<usize> {
        self.buffer_for(hit.file.as_deref())
    }

    /// The same, from a path relative to the search's root.
    fn buffer_for(&mut self, rel: Option<&Path>) -> Option<usize> {
        let Some(rel) = rel else {
            return Some(self.current);
        };
        let full = match &self.search.root {
            Some(root) => root.join(rel),
            None => rel.to_path_buf(),
        };
        let full = std::fs::canonicalize(&full).unwrap_or(full);
        if let Some(i) = self.buffers.iter().position(|b| {
            b.path()
                .map(|p| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf()))
                .as_deref()
                == Some(full.as_path())
        }) {
            return Some(i);
        }
        // **Opened as a buffer, not rewritten on disk.** That is the whole of
        // why a change across a book is safe here.
        match self.open_file(&full) {
            Ok(()) => Some(self.current),
            Err(err) => {
                self.status = say!("buffer.cannot-open", full.display(), err);
                None
            }
        }
    }

    /// The pattern the panel is running, compiled — `None` if it will not.
    fn search_regex(&self) -> Option<regex::Regex> {
        regex::Regex::new(&self.search_pattern()).ok()
    }

    /// Look again and say how it went. The offsets are all stale now.
    fn after_replacing(&mut self, done: usize) {
        let where_ = self.search.selected;
        match self.search.scope.live() {
            true => self.run_search(),
            false => self.search_now(),
        }
        // Stay where the eye was, or at the end if the list got shorter.
        self.search.selected = where_.min(self.search.rows().len().saturating_sub(1));
        self.status = match done {
            0 => say!("search.replaced-none"),
            n => say!("search.replaced", n),
        };
    }

    /// `Enter` on a row: fold a file, or go to a hit and hand the keys back.
    fn go_to_hit(&mut self) {
        // On a file header, `Enter` is what `h`/`l` are: open or shut.
        if let Some(crate::search_panel::Row::File { path, folded, .. }) = self.search.row() {
            let _ = self.search.fold(!folded);
            let _ = path;
            return;
        }
        let Some(hit) = self.search.here().cloned() else {
            return;
        };
        self.remember_jump();
        // ⚠️ **A hit carries a file name even when it is in the file being
        // written** (it has to, or the tree could not group it), so 「another
        // file」 is a question about the path, not about whether there is one.
        let mine = self
            .current_buffer()
            .path()
            .and_then(|p| std::fs::canonicalize(p).ok());
        let elsewhere = match (&hit.file, &self.search.root) {
            (None, _) => false,
            (Some(rel), Some(root)) => std::fs::canonicalize(root.join(rel)).ok() != mine,
            (Some(_), None) => true,
        };
        // Another file has to be opened first — and if it cannot be, say so
        // rather than walking the cursor to that line of the wrong file.
        if elsewhere {
            let rel = hit.file.clone().unwrap_or_default();
            let full = match &self.search.root {
                Some(root) => root.join(&rel),
                None => rel,
            };
            if let Err(err) = self.open_file(&full) {
                self.status = say!("buffer.cannot-open", full.display(), err);
                return;
            }
        }
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();
        // ⚠️ **A hit in another file is placed by line, not by the offset.**
        // Those offsets were counted in the text as it was read; the buffer
        // just opened may have been edited since, and a stale offset would put
        // the cursor in the middle of a word somewhere else.
        let (at, end) = match elsewhere {
            true => {
                let line = hit.line.min(rope.len_lines().saturating_sub(1));
                let at = rope.line_to_char(line);
                (at, at)
            }
            false => (hit.at.min(len), hit.end.min(len)),
        };
        self.anchor = at;
        self.cursor = motion::prev_grapheme(self.current_buffer().rope(), end.max(at)).max(at);
        self.extend = false;
        self.refresh_goal_column();
        self.panel_focus = None;
    }
}

/// A few characters either side of one match, and where the match is in them.
fn excerpt(
    file: Option<PathBuf>,
    text: &str,
    line_at: usize,
    start: usize,
    stop: usize,
    line: usize,
    nth: usize,
) -> Hit {
    let chars: Vec<char> = text.chars().collect();
    let from = start.saturating_sub(AROUND);
    let to = (stop + AROUND).min(chars.len());
    let mut excerpt = String::new();
    if from > 0 {
        excerpt.push('…');
    }
    let lead = excerpt.chars().count();
    excerpt.extend(chars[from..to].iter().filter(|c| **c != '\n'));
    if to < chars.len() {
        excerpt.push('…');
    }
    Hit {
        file,
        line,
        nth,
        at: line_at + start,
        end: line_at + stop,
        mark: lead + (start - from)..lead + (stop - from),
        excerpt,
    }
}
