//! Counts, repetition, and the Helix tutorial verbs (#296).
//!
//! `3w`, `.`, `%`, `x`, `J`, `r`, `>`, `<`, `C-a` — the verbs a Helix hand
//! reaches for, plus the count that prefixes them.

use super::*;

impl Editor {
    // ---- Counts, repetition, and the Helix tutorial verbs -----------------

    /// Enter Insert mode, starting a fresh recording for `.` to replay.
    pub(super) fn enter_insert(&mut self) {
        // A locked buffer does not get an Insert mode to type into
        // (Feature #213). Refusing here rather than at each keystroke is the
        // difference between 「只讀」 once and a status line that says it forty
        // times while the writer works out that nothing is going in.
        if self.refuse_readonly() {
            return;
        }
        // 延伸模式 is left at the door. It is a *mode* kept outside `Mode`, so
        // every operation has had to remember to clear it and some did not —
        // `v i X Esc` came back to Normal still extending, and the next `j`
        // grew a selection instead of moving.
        self.extend = false;
        self.insert_recording.clear();
        self.mode = Mode::Insert;
    }

    /// Take the pending count prefix, defaulting to one.
    pub(super) fn take_count(&mut self) -> usize {
        self.count.take().unwrap_or(1).max(1)
    }

    /// Run `action` `n` times — how a count prefix is applied to a motion or an
    /// edit. Stops early once the action stops moving the cursor, so `999j` at
    /// the end of the buffer costs one step rather than a thousand.
    pub(super) fn repeat(&mut self, n: usize, mut action: impl FnMut(&mut Self)) {
        // **A count is one command.** The first pass announces the undo point
        // the whole run goes back to; the rest are the same command still
        // running, so their announcements are suppressed (#323). Without this
        // `100p` took a hundred presses of `u` to take back one keystroke.
        let mut group: Option<bool> = None;
        for _ in 0..n {
            let (before, anchor) = (self.cursor, self.anchor);
            let revision = self.current_buffer().char_count();
            action(self);
            if self.cursor == before
                && self.anchor == anchor
                && self.current_buffer().char_count() == revision
            {
                break;
            }
            group.get_or_insert_with(|| self.current_buffer_mut().begin_undo_group());
        }
        if let Some(was) = group {
            self.current_buffer_mut().end_undo_group(was);
        }
    }

    /// The ceiling on a count that **writes**.
    ///
    /// A motion may be handed a million and cost almost nothing: it walks to
    /// the end of the buffer and [`Editor::repeat`] breaks. An edit has no such
    /// floor — every pass really does its work, so `1000000p` is a million
    /// pastes, a million undo snapshots, and a buffer that grows until the
    /// process dies (#318). Ten thousand is far above any count a hand types
    /// on purpose and far below the count that hangs the editor.
    pub(super) const WRITING_MAX: usize = 10_000;

    /// [`Editor::repeat`], for an action that changes the text.
    ///
    /// Clips the count to [`Editor::WRITING_MAX`] and **says so** — a silently
    /// shortened `50000p` would look like a paste that lost half its work.
    pub(super) fn repeat_writing(&mut self, n: usize, action: impl FnMut(&mut Self)) {
        self.repeat(n.min(Self::WRITING_MAX), action);
        // After, not before: the notice is the one thing the writer has to
        // read, and an action that sets its own status would bury it.
        if n > Self::WRITING_MAX {
            self.status = say!("count.writing-ceiling", Self::WRITING_MAX);
        }
    }

    /// Select the whole buffer (Helix `%`).
    pub(super) fn select_all(&mut self) {
        let rope = self.current_buffer().rope();
        // On the last grapheme, not one past it: the selection now covers the
        // grapheme the cursor is on.
        let last = motion::prev_grapheme(rope, rope.len_chars());
        self.anchor = 0;
        self.cursor = last;
        self.goal_column = 0;
    }

    /// Grow the selection outward to whole lines (Helix `X`).
    pub(super) fn extend_to_line_bounds(&mut self) {
        let (start, end) = self.selection();
        let rope = self.current_buffer().rope();
        let first = rope.char_to_line(start);
        let last = rope.char_to_line(end.saturating_sub(1).max(start));
        let head = rope.line_to_char(first);
        let tail = if last + 1 < rope.len_lines() {
            rope.line_to_char(last + 1)
        } else {
            rope.len_chars()
        };
        let tail = motion::prev_grapheme(rope, tail).max(head);
        self.anchor = head;
        self.cursor = tail;
    }

    /// Join the line below onto this one (Helix `J`).
    ///
    /// Helix always inserts a space; yumete does not put one between two
    /// full-width characters, because in CJK prose a line break carries no
    /// space and joining two 漢字 with one would insert text nobody ever
    /// typed. Between Latin words the space is kept.
    pub(super) fn join_lines(&mut self) {
        if self.refuse_readonly() {
            return;
        }
        let rope = self.current_buffer().rope();
        // The *selection's* first line, not the cursor's: `x` parks the cursor
        // on the line after the one it selected, so joining from the cursor
        // joined the wrong pair — and after `xxx` joined nothing at all.
        let (start, _) = self.selection();
        let line = rope.char_to_line(start);
        if line >= motion::last_line(rope) {
            return;
        }
        let end = motion::line_end(rope, start);
        // Swallow the break and any indentation that follows it.
        let mut next = end + 1;
        let len = rope.len_chars();
        while next < len && matches!(rope.char(next), ' ' | '\t' | '\u{3000}') {
            next += 1;
        }
        let before = (end > 0).then(|| rope.char(end - 1));
        let after = (next < len).then(|| rope.char(next));
        let glue = match (before, after) {
            (Some(a), Some(b)) if is_wide(a) && is_wide(b) => "",
            (None, _) | (_, None) => "",
            _ => " ",
        };
        self.snapshot();
        let done = self.current_buffer_mut().replace(end..next, glue);
        if !self.applied(done) {
            return;
        }
        self.cursor = end;
        self.anchor = end;
        self.clamp_cursor();
    }

    /// Rewrite every character of the selection through `f` (`~`, `` ` ``).
    pub(super) fn map_selection(&mut self, f: impl Fn(char) -> String) {
        let (start, selected) = self.selection();
        let collapsed = selected == start;
        let end = if collapsed {
            motion::right(self.current_buffer().rope(), start).max(start + 1)
        } else {
            selected
        };
        let end = end.min(self.current_buffer().char_count());
        if start >= end {
            return;
        }
        // **A case change may be more than one character** (#325). `ﬁ`
        // upper-cases to `FI`, `ß` to `SS`, and taking only the first of them
        // — which is what a `char -> char` signature forces — deleted the
        // rest, silently: `ﬁ` came back `F` and `ß` came back `S`. Ligatures
        // pasted out of a PDF and German `ß` are ordinary in an English
        // manuscript.
        let text: String = self
            .current_buffer()
            .rope()
            .slice(start..end)
            .chars()
            .map(f)
            .collect();
        self.snapshot();
        if !self.overwrite(start, end, &text) {
            return;
        }
        // …so the selection is measured off what was written, not off what
        // was there.
        let end = start + text.chars().count();
        // The head sits on the selection's last grapheme, not one past it: the
        // selection covers the cursor's own grapheme, so a head at `end` would
        // put the *next* character inside the highlight — and the next edit
        // would take one more than the highlight showed.
        self.anchor = start;
        self.cursor = motion::prev_grapheme(self.current_buffer().rope(), end).max(start);
    }

    /// Overwrite every character of the selection with `c` (Helix `r`).
    ///
    /// The selection keeps its length — this writes over the text rather than
    /// replacing it with one character — so `r` on a selected word turns the
    /// whole word into that character, one for one.
    pub(super) fn replace_chars(&mut self, c: char) {
        let (start, end) = self.selection();
        let end = end.min(self.current_buffer().char_count());
        if start >= end {
            return;
        }
        // Over the *characters*, not over the range: a line ending is not a
        // character you meant to write over. `x` selects a line including its
        // newline, so `x r Z` used to run the line into the next one — a lost
        // paragraph, silently, from two keys that mean "blank this out".
        // **One grapheme in, one out** (#324). Over `chars()` this wrote a
        // `Z` per *code point*, so `r Z` on a ZWJ family emoji laid down five
        // of them, and on a decomposed か (か + U+3099) two. `h` and `l` walk
        // by grapheme, so what a writer selected was one glyph and what they
        // got back was however many code points happened to be inside it —
        // and NFD Japanese and emoji are ordinary in a mixed manuscript.
        let slice = self.current_buffer().rope().slice(start..end).to_string();
        let text: String = yumete_cjk::graphemes(&slice)
            .map(|had| match had.starts_with(['\n', '\r']) {
                // A line ending is not a character anybody meant to write
                // over: `x` selects a line including its newline, and `x r Z`
                // used to run the line into the next one.
                true => had.to_string(),
                false => c.to_string(),
            })
            .collect();
        self.snapshot();
        if !self.overwrite(start, end, &text) {
            return;
        }
        // The selection is what it was: `r` writes over the text without moving
        // through it, so `r` then `l` steps one character, not two.
        let head = motion::prev_grapheme(self.current_buffer().rope(), end).max(start);
        self.anchor = start;
        self.cursor = head;
        self.clamp_cursor();
    }

    /// Replace the selection with already-composed text — `r` running the IME.
    ///
    /// One character keeps [`Self::replace_chars`]'s promise and fills the
    /// whole selection: 「錢塘江」 `r` ■ is ■■■, because that is what `r` has
    /// always meant and one 字 is what `r` has always taken.
    ///
    /// More than one character is a different intent. 「錢」 `r` 春天 is 春天 —
    /// there is no way to fill a three-character selection with a two-character
    /// word, so the selection is replaced instead of written over. A trailing
    /// line ending is still not part of it: `x r` must not run two paragraphs
    /// together, whichever length the reader committed.
    pub(super) fn replace_str(&mut self, text: &str) {
        let mut chars = text.chars();
        let (first, second) = (chars.next(), chars.next());
        if second.is_none() {
            if let Some(c) = first {
                self.replace_chars(c);
            }
            return;
        }
        let (start, mut end) = self.selection();
        end = end.min(self.current_buffer().char_count());
        let rope = self.current_buffer().rope();
        while end > start && matches!(rope.char(end - 1), '\n' | '\r') {
            end -= 1;
        }
        if start >= end {
            return;
        }
        self.snapshot();
        if !self.overwrite(start, end, text) {
            return;
        }
        // The new text is the selection, the way `c` leaves what it inserted:
        // the reader looked at a word and now looks at the word that took its
        // place.
        let end = start + text.chars().count();
        self.anchor = start;
        self.cursor = motion::prev_grapheme(self.current_buffer().rope(), end).max(start);
        self.clamp_cursor();
    }

    /// Swap which end of the selection the cursor sits on (Helix `A-;`).
    ///
    /// Only the cursor moves; the selection is the same range. It is how you
    /// extend a selection from the other end without starting it again.
    pub(super) fn flip_selection(&mut self) {
        std::mem::swap(&mut self.anchor, &mut self.cursor);
        self.refresh_goal_column();
    }

    /// Replace the selection with the yank register (Helix `R`).
    pub(super) fn replace_with_register(&mut self) {
        // Read before `recall`, which *takes* it: asking afterwards always
        // answered `None`, so the reader who named a register was told 「nothing
        // has been yanked」 about a register they had just named.
        let named = self.pending_register;
        let text = self.recall();
        if text.is_empty() {
            // A key that does nothing and says nothing is indistinguishable
            // from a key that is broken — and the reader whose `y` went to a
            // named register is the one most likely to press this.
            self.status = match named {
                Some(name) => say!("register.empty", name),
                None => say!("register.nothing-yanked"),
            };
            return;
        }
        let (start, end) = self.selection();
        self.snapshot();
        if !self.overwrite(start, end.max(start), &text) {
            return;
        }
        self.anchor = start;
        self.cursor = start + text.chars().count();
        self.clamp_cursor();
    }
    /// Indent (`>`) or unindent (`<`) every line the selection touches.
    pub(super) fn indent(&mut self, add: bool) {
        let rope = self.current_buffer().rope();
        let (start, end) = self.selection();
        let first = rope.char_to_line(start);
        let last = rope.char_to_line(end.saturating_sub(1).max(start));
        let pad = " ".repeat(self.indent_width);
        self.snapshot();
        // Bottom-up, so earlier edits do not shift the lines still to come.
        for line in (first..=last).rev() {
            let at = self.current_buffer().rope().line_to_char(line);
            if add {
                self.edit_insert(at, &pad);
            } else {
                let rope = self.current_buffer().rope();
                let len = rope.len_chars();
                let mut n = 0;
                while n < self.indent_width && at + n < len && rope.char(at + n) == ' ' {
                    n += 1;
                }
                if n > 0 {
                    self.edit_remove(at..at + n);
                }
            }
        }
        self.clamp_cursor();
    }

    /// Add `delta` to the number at or after the cursor on its line
    /// (Helix `C-a` / `C-x`).
    pub(super) fn bump_number(&mut self, delta: i64) {
        if self.refuse_readonly() {
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor);
        let line_start = rope.line_to_char(line);
        let text = rope.line(line).to_string();
        let chars: Vec<char> = text.chars().collect();
        let col = self.cursor - line_start;

        // The number under the cursor, else the next one along the line.
        let Some(mut start) = (col..chars.len())
            .find(|&i| chars[i].is_ascii_digit())
            .map(|i| {
                let mut s = i;
                while s > 0 && chars[s - 1].is_ascii_digit() {
                    s -= 1;
                }
                s
            })
        else {
            return;
        };
        let mut end = start;
        while end < chars.len() && chars[end].is_ascii_digit() {
            end += 1;
        }
        let negative = start > 0 && chars[start - 1] == '-';
        if negative {
            start -= 1;
        }
        let digits: String = chars[start..end].iter().collect();
        let Ok(value) = digits.parse::<i64>() else {
            return;
        };
        // Keep zero padding: `007` steps to `008`, not `8`.
        let width = digits.trim_start_matches('-').len();
        let next = value.saturating_add(delta);
        let text = if digits.trim_start_matches('-').starts_with('0') && width > 1 {
            format!(
                "{}{:0width$}",
                if next < 0 { "-" } else { "" },
                next.abs(),
                width = width
            )
        } else {
            next.to_string()
        };

        self.snapshot();
        let done = self
            .current_buffer_mut()
            .replace(line_start + start..line_start + end, &text);
        if !self.applied(done) {
            return;
        }
        self.cursor = line_start + start;
        self.anchor = self.cursor;
        self.clamp_cursor();
    }
}
