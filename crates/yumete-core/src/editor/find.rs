//! The search panel's own half of the editor — Feature #419.
//!
//! Running what the box holds, and the keys of the form it sits in. The
//! panel's *state* is [`crate::search_panel`]; drawing it is the front end's.

use super::*;
use crate::search_panel::{Case, Field, Hit, AROUND, MOST};

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
        self.run_search();
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

    /// **Run what the box holds over the buffer being written** — #419 一.
    ///
    /// Called on every keystroke in the box: this buffer is in memory and a
    /// pass over it costs nothing worth counting. Searching other files is
    /// [^419]'s second sitting and will not be able to afford this.
    pub(super) fn run_search(&mut self) {
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
        // **The pattern hands the search to the panel, and the page follows.**
        // One 「what am I looking for」 with two ways in: the highlight and
        // `n`/`N` are the same search, which is what [^415]记 `:grep` 不寫
        // `last_search` 為缺口的那條理由.
        self.last_search = pattern;
        let rope = self.current_buffer().rope();
        let mut hits = Vec::new();
        let mut total = 0usize;
        let mut at = 0usize;
        for line in 0..rope.len_lines() {
            let text: String = rope.line(line).chars().collect();
            for m in re.find_iter(&text) {
                total += 1;
                if hits.len() >= MOST {
                    continue;
                }
                let start = text[..m.start()].chars().count();
                let stop = text[..m.end()].chars().count();
                hits.push(excerpt(&text, at, start, stop, line));
            }
            at += text.chars().count();
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

    /// `Tab` out of the box: the next cell, and back to the panel's Normal.
    fn leave_field(&mut self, back: bool) {
        self.search.field = self.search.field.step(back);
        self.mode = Mode::Normal;
    }

    /// One key while the search panel has them and the box does not
    /// (`Mode::Normal`, the keys in the panel).
    pub(super) fn on_search_panel_key(&mut self, key: Key, side: crate::sidebar::Side) {
        match key {
            Key::Tab => self.search.field = self.search.field.step(false),
            Key::BackTab => self.search.field = self.search.field.step(true),
            // `hjkl` walk the form in Normal, as the author asked; in the list
            // `j`/`k` walk the hits instead, because that is what is there.
            Key::Char('l') | Key::Right if self.search.field != Field::Results => {
                self.search.field = self.search.field.step(false)
            }
            Key::Char('h') | Key::Left => self.search.field = self.search.field.step(true),
            Key::Char('j') | Key::Down => match self.search.field {
                Field::Results => self.search.step(true),
                _ => self.search.field = self.search.field.step(false),
            },
            Key::Char('k') | Key::Up => match self.search.field {
                Field::Results => self.search.step(false),
                _ => self.search.field = self.search.field.step(true),
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
                Field::Query => self.mode = Mode::Field,
                Field::Regex | Field::Case | Field::Whole => self.flip_switch(),
                Field::Results => self.go_to_hit(),
            },
            // `i` opens the box, wherever you are in the form — the key that
            // means 「type here」 everywhere else in this editor.
            Key::Char('i') => {
                self.search.field = Field::Query;
                self.mode = Mode::Field;
            }
            Key::Char('g') | Key::Home if self.search.field == Field::Results => {
                self.search.selected = 0
            }
            Key::Char('G') | Key::End if self.search.field == Field::Results => {
                self.search.selected = self.search.hits.len().saturating_sub(1)
            }
            Key::Ctrl('w') => self.cycle_region(),
            Key::Char('q') => self.close_panel(side),
            Key::Char(':') => {
                self.mode = Mode::Command;
                self.command_line.clear();
                self.command_caret = 0;
            }
            Key::Char(' ') => self.pending = Pending::Space,
            _ => {}
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

    /// `Enter` on a hit: go there, and hand the keys back to the page.
    fn go_to_hit(&mut self) {
        let Some(hit) = self.search.here().cloned() else {
            return;
        };
        self.remember_jump();
        let len = self.current_buffer().rope().len_chars();
        self.anchor = hit.at.min(len);
        self.cursor = motion::prev_grapheme(self.current_buffer().rope(), hit.end.min(len))
            .max(hit.at.min(len));
        self.extend = false;
        self.refresh_goal_column();
        self.panel_focus = None;
    }
}

/// A few characters either side of one match, and where the match is in them.
fn excerpt(text: &str, line_at: usize, start: usize, stop: usize, line: usize) -> Hit {
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
        line,
        at: line_at + start,
        end: line_at + stop,
        mark: lead + (start - from)..lead + (stop - from),
        excerpt,
    }
}
