//! Search and substitute (#14 / #15).

use super::*;

impl Editor {
    // ---- Search (Feature #14) ---------------------------------------------

    /// Compile a search or substitution pattern, remembering the last one.
    ///
    /// Patterns are **regular expressions**, as they are in vi and Helix: half
    /// the work of revising a manuscript is a pattern rather than a string —
    /// 「行首的『他說』」, 「連續兩個以上的驚嘆號」, 「每個。後面斷行」. The
    /// cost is that `.` `*` `(` mean something; `\.` is a full stop.
    ///
    /// `n` and `N` ask for the same pattern over and over, so the compiled form
    /// is kept until the pattern changes.
    pub(super) fn compile(&self, pattern: &str) -> Result<Regex, String> {
        let pattern = &self.smart_cased(pattern);
        // **Keyed on the pattern that is actually compiled**, not on what was
        // typed: `(?i)` is part of it, so `/todo` and `/TODO` are two entries
        // and never hand each other their answer.
        if let Some((cached, re)) = self.compiled.borrow().as_ref() {
            if cached == pattern {
                return Ok(re.clone());
            }
        }
        match Regex::new(pattern) {
            Ok(re) => {
                *self.compiled.borrow_mut() = Some((pattern.to_string(), re.clone()));
                Ok(re)
            }
            // The writer needs to know *which* part of their pattern is wrong,
            // and regex's own message says so; its multi-line form does not fit
            // a status line.
            Err(err) => Err(format!(
                "bad pattern: {}",
                err.to_string().lines().last().unwrap_or("").trim()
            )),
        }
    }

    /// Smart case (#301): a pattern with no capital in it ignores case.
    ///
    /// Typing a capital is how you ask for the case to matter — nothing else
    /// has to be turned on or off, and the whole rule is one line to explain.
    /// `(?-i)` in front of the pattern is the way to say 「lower case, and mean
    /// it」; `[editor] smart_case = false` turns the rule off for good.
    fn smart_cased(&self, pattern: &str) -> String {
        match self.smart_case && !pattern.chars().any(char::is_uppercase) {
            true => format!("(?i){pattern}"),
            false => pattern.to_string(),
        }
    }

    /// Turn smart case off (or back on) — `[editor] smart_case`.
    pub fn set_smart_case(&mut self, on: bool) {
        self.smart_case = on;
        // The cache is keyed on the compiled pattern, and the rule that builds
        // it has just changed underneath it.
        *self.compiled.borrow_mut() = None;
    }

    /// Search for [`Self::last_search`] in `forward` direction and move there.
    /// The lines a search may land in: the table's own rows while the grid
    /// holds the pane, and the whole buffer otherwise (#354).
    fn search_rows(&self) -> std::ops::Range<usize> {
        let all = 0..self.current_buffer().rope().len_lines();
        if !self.table.as_ref().is_some_and(|view| view.takes_the_pane()) {
            return all;
        }
        match self.table_row_span() {
            Some((first, last)) => first..last + 1,
            None => all,
        }
    }

    pub(super) fn repeat_search(&mut self, forward: bool) {
        // `n` with nothing to repeat used to do nothing and say nothing, which
        // reads as a key that is broken rather than one with no answer yet.
        if self.last_search.is_empty() {
            self.status = say!("find.nothing-searched-yet");
            return;
        }
        // Jumping back after a search is the whole reason `C-o` exists: you
        // look something up, and you want to be back where you were writing.
        self.remember_jump();
        let pattern = self.last_search.clone();
        let re = match self.compile(&pattern) {
            Ok(re) => re,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();

        // Line by line, not over the whole buffer: a pattern cannot contain a
        // newline (Enter submits the prompt), so a match never straddles a line
        // break, and materialising the document for every `n` costs an 800 KB
        // copy on a novel.
        // **Where a hit is allowed to be** (#354). With the grid holding the
        // pane the cursor is held inside the table (`tables.rs`), so a hit
        // outside it is one `n` can never reach — the reader watches the
        // search find something and the cursor refuse to go, and `n` stops
        // going round. The rows of the table are the whole document as far as
        // this search is concerned.
        let within = self.search_rows();
        let scoped = within != (0..rope.len_lines());
        let found = if forward {
            search_forward(rope, &re, (self.cursor + 1).min(len), within)
        } else {
            search_backward(rope, &re, self.cursor, within)
        };

        match found {
            // The match itself becomes the selection. Every motion leaves one —
            // that is the first thing the manual says about this editor — and a
            // search that only moved the cursor made `/` the one motion after
            // which `d` did something other than what the screen showed.
            Some((pos, end)) => {
                // On the match's last grapheme, not one past it — the selection
                // covers the cursor's own grapheme.
                let end = end.min(len);
                let head = motion::prev_grapheme(rope, end).max(pos);
                self.anchor = pos;
                self.cursor = head;
                self.extend = false;
                self.refresh_goal_column();
            }
            None if scoped => self.status = say!("find.not-in-table", pattern),
            None => self.status = say!("find.not-found", pattern),
        }
    }

    // ---- Substitute (Feature #15) -----------------------------------------

    /// Replace `pattern` with `replacement` on the cursor's line, or on every
    /// line when `whole_file`; `global` replaces every match on a line.
    pub(super) fn substitute(&mut self, how: Substitution<'_>) {
        if self.refuse_readonly() {
            return;
        }
        let Substitution {
            pattern,
            replacement,
            global,
            ignore_case,
            count_only,
            reshape,
            rows,
        } = how;
        if pattern.is_empty() {
            self.status = say!("find.empty-pattern");
            return;
        }
        // `i` is the regex engine's own flag, so it is written into the
        // pattern rather than reimplemented here.
        let cased = match ignore_case {
            true => format!("(?i){pattern}"),
            false => pattern.to_string(),
        };
        let re = match self.compile(&cased) {
            Ok(re) => re,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        let replacement = unescape_replacement(replacement);

        let text = self.current_buffer().text();
        let chosen = self.substitution_rows(rows);
        let mut count = 0usize;
        let mut rebuilt = String::with_capacity(text.len());

        for (idx, line) in text.split_inclusive('\n').enumerate() {
            if chosen.has(idx) {
                let (new_line, n) = replace_in_line(line, &re, &replacement, global);
                count += n;
                rebuilt.push_str(&new_line);
            } else {
                rebuilt.push_str(line);
            }
        }

        // A substitution rewrites whole lines, so the cell guard cannot judge it
        // character by character. What it can check is the thing the guard
        // exists to protect: that no row gained or lost a cell.
        if count > 0 && !reshape {
            if let Some(why) = self.substitution_breaks_the_grid(&rebuilt) {
                self.status = why;
                return;
            }
        }
        // `n` in vi means "count, and change nothing". It used to substitute.
        if count_only {
            self.status = say!("find.substitute-nothing-changed", count);
            return;
        }
        if count > 0 {
            self.snapshot();
            let len = self.current_buffer().char_count();
            let done = self.without_cell_guard(|e| e.current_buffer_mut().replace(0..len, &rebuilt));
            if !self.applied(done) {
                return;
            }
            self.clamp_cursor();
            self.anchor = self.cursor;
            self.refresh_goal_column();
        }
        self.status = say!("find.substitute-changed", count);
    }

    /// The lines a `:s` range names, as a question a line number can be put to.
    fn substitution_rows(&self, rows: crate::command::Rows) -> Chosen {
        use crate::command::{Bound, Rows};
        let rope = self.current_buffer().rope();
        let last_line = motion::last_line(rope);
        let resolve = |b: Bound| match b {
            Bound::Line(n) => n.saturating_sub(1).min(last_line),
            Bound::Cursor => rope.char_to_line(self.caret().min(rope.len_chars())),
            Bound::Last => last_line,
        };
        match rows {
            Rows::All => Chosen::Span(0, last_line),
            Rows::Span(a, b) => {
                let (a, b) = (resolve(a), resolve(b));
                Chosen::Span(a.min(b), a.max(b))
            }
            // `1,5,9` — these and no others. Written in any order, and a line
            // named twice is still one line.
            Rows::List(bounds) => Chosen::These(bounds.into_iter().map(resolve).collect()),
            // No range written: the lines the *selection* covers. Reading the
            // cursor's line instead meant that after `x` — which leaves the
            // cursor on the line below the one it selected — `:s` edited a
            // line the writer had not selected and could not see was selected.
            Rows::Selection => {
                let (start, end) = self.selection();
                let first = rope.char_to_line(start);
                let last = if end > start {
                    rope.char_to_line(end.saturating_sub(1))
                } else {
                    first
                };
                Chosen::Span(first, last)
            }
        }
    }
}

/// The lines a `:s` will touch, already resolved to 0-based line numbers.
enum Chosen {
    /// Everything from the first to the last, inclusive.
    Span(usize, usize),
    /// Exactly these, in whatever order they were written.
    These(Vec<usize>),
}

impl Chosen {
    /// Is this line one of them?
    fn has(&self, line: usize) -> bool {
        match self {
            Self::Span(first, last) => line >= *first && line <= *last,
            Self::These(lines) => lines.contains(&line),
        }
    }
}
