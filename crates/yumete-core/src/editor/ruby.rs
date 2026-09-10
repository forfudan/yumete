//! Ruby mode — 注音／拼音 above the line (#65).

use super::*;

impl Editor {
    // ---- Ruby mode (Feature #65) ------------------------------------------

    /// The dialect this buffer is written in, from its file extension.
    fn file_dialect(&self) -> Dialect {
        self.current_buffer()
            .path()
            .and_then(|p| p.extension())
            .and_then(|e| e.to_str())
            .and_then(Dialect::for_extension)
            .unwrap_or(Dialect::Html)
    }

    /// `:ruby-auto` — write the readings in, by word (Feature #234).
    ///
    /// **Why a word and not a character.** 了 is `le` in 為了 and `liǎo` in
    /// 了解, and every tool that annotates 拼音 one character at a time gets one
    /// of those wrong — Word's 拼音指南 included, which is why nobody uses it
    /// twice. The reading comes from the 讀音表 by whole word, so the polyphone
    /// is settled by the word it is in rather than by a coin toss over a
    /// dictionary entry; see [`yumete_cjk::Reader`].
    ///
    /// **What it cannot settle: a polyphone whose readings differ only in
    /// tone.** The 讀音表 is toneless — 認為 is stored `ren wei` — so 為 `wéi`
    /// and 為 `wèi` ask it the same question and it gives the same answer;
    /// 難, 好, 教 and 中 are the same shape. Those take the common reading and
    /// may need a hand. 了, 行, 和 and 長 differ in spelling and are settled.
    ///
    /// **`rare` is the one people actually want.** A novel with a reading over
    /// every character is a textbook, not a novel; a novel with a reading over
    /// the handful nobody knows is a novel a reader can finish. So `:ruby-auto
    /// rare` keeps only the words holding a character that **no** standard in
    /// current use carries — 通用規範, 通規繁, 臺灣, 香港 — and keeps the
    /// *word*, because one character of a two-character word read alone is
    /// worse typography than either extreme.
    ///
    /// The markup is mono-ruby ([`crate::ruby::SPLIT`]): one group per
    /// character, which is how CJK ruby is set and what [`crate::zong`] already
    /// spaces the 縱 out for.
    pub(super) fn auto_ruby(&mut self, rare: bool) {
        if self.refuse_readonly() {
            return;
        }
        if !self.reader.available() {
            self.status = say!("ruby.auto-no-readings");
            return;
        }
        let dialect = self.file_dialect();
        // A standing selection is the region; without one it is the whole file,
        // which is the pass a manuscript is actually given. `u` takes all of it
        // back in one step either way.
        let (from, to) = if self.anchor == self.cursor {
            (0, self.current_buffer().char_count())
        } else {
            self.selection()
        };
        let rebuilt = {
            let rope = self.current_buffer().rope();
            let first = rope.char_to_line(from);
            let last = rope.char_to_line(to.saturating_sub(1).max(from));
            let mut edits: Vec<(usize, usize, String)> = Vec::new();
            for line in first..=last.min(rope.len_lines().saturating_sub(1)) {
                let line_start = rope.line_to_char(line);
                let chars: Vec<char> = crate::zong::line_chars(rope, line);
                // Whatever is already annotated stays as it is: a reader who
                // corrected one reading by hand does not get it overwritten by
                // the command that offered to help.
                let groups = crate::ruby::all_groups(&chars);
                // The segmenter itself, not [`Self::segment_line`]: that one
                // drops words the reader can already see the edges of, which is
                // right for the overlay and wrong here — 字 sitting alone after
                // a `</ruby>` is exactly a word this pass has to annotate.
                let text: String = chars.iter().collect();
                for (ws, we) in self.segmenter.segment(&text) {
                    let (start, end) = (line_start + ws, line_start + we);
                    if start < from || end > to || we > chars.len() {
                        continue;
                    }
                    if groups.iter().any(|g| ws < g.end && g.start < we) {
                        continue;
                    }
                    let word: String = chars[ws..we].iter().collect();
                    if word.is_empty() || !word.chars().all(is_han) {
                        continue;
                    }
                    if rare && !word.chars().any(|c| self.reader.is_rare(c) == Some(true)) {
                        continue;
                    }
                    let Some(readings) = self.reader.read(&word) else {
                        continue;
                    };
                    // A reading that does not cover the word is not a reading —
                    // writing it would put syllables over the wrong characters.
                    if readings.len() != we - ws {
                        continue;
                    }
                    let split = crate::ruby::SPLIT.to_string();
                    let markup =
                        crate::ruby::markup(&chars[ws..we], &readings.join(&split), dialect);
                    edits.push((start, end, markup));
                }
            }
            if edits.is_empty() {
                None
            } else {
                let mut out = String::with_capacity(rope.len_chars());
                let mut at = 0usize;
                for (start, end, markup) in &edits {
                    out.push_str(&rope.slice(at..*start).to_string());
                    out.push_str(markup);
                    at = *end;
                }
                out.push_str(&rope.slice(at..rope.len_chars()).to_string());
                Some((edits.len(), out))
            }
        };
        let Some((n, rebuilt)) = rebuilt else {
            self.status = say!("ruby.auto-none");
            return;
        };
        // The same rule `:replace` and `:format ruby` keep: a 拆分表 whose cells
        // hold readings must not gain a field because a command rewrote it.
        if let Some(why) = self.substitution_breaks_the_grid(&rebuilt) {
            self.status = why;
            return;
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.current_buffer_mut().replace(0..len, &rebuilt);
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
        self.status = say!("ruby.auto-added", n);
    }

    /// Rewrite every reading in the buffer into one dialect (`:format-ruby-…`).
    pub(super) fn format_ruby(&mut self, dialect: Dialect) {
        if self.refuse_readonly() {
            return;
        }
        let text = self.current_buffer().text();
        let Some(formatted) = crate::ruby::reformat(&text, dialect) else {
            self.status = say!("ruby.already-in-that-form", dialect.name());
            return;
        };
        // The same rule `:replace` keeps, in the sibling that rewrites just as
        // much text: `#ruby("永", "ㄩㄥˇ")` carries a comma, so reformatting a
        // 拆分表 whose cells hold readings gave every one of those rows an
        // extra field — silently, in one keystroke, across the whole file.
        if let Some(why) = self.substitution_breaks_the_grid(&formatted) {
            self.status = why;
            return;
        }
        self.snapshot();
        let len = self.current_buffer().char_count();
        let done = self.current_buffer_mut().replace(0..len, &formatted);
        if !self.applied(done) {
            return;
        }
        self.clamp_cursor();
        self.status = say!("ruby.rewritten-as", dialect.name());
    }

    /// Step Tab's completion through the matching commands, writing each onto
    /// the command line in turn.
    ///
    /// Only the command *word* completes: once there is a space the rest is an
    /// argument, and a file name is not something this list knows about.
    pub(super) fn cycle_completion(&mut self, step: isize) {
        let prefix = match &self.completion {
            Some((prefix, _)) => prefix.clone(),
            None => self.command_line.clone(),
        };
        let matches = command::complete(&prefix);
        if matches.is_empty() {
            return;
        }
        let n = matches.len() as isize;
        let next = match &self.completion {
            Some((_, i)) => (*i as isize + step).rem_euclid(n),
            // The first Tab lands on the first match going forward, and on the
            // last going back.
            None if step > 0 => 0,
            None => n - 1,
        } as usize;
        // Replace the word being completed, not the whole line: `:yume sch`
        // has to become `:yume-scheme`, not `scheme`.
        let (start, _) = command::complete_at(&prefix);
        if matches[next].name.is_empty() {
            return;
        }
        // A word promoted out of its parent's list writes the parent too:
        // picking `scheme` out of what `:yume` takes leaves `:yume-scheme`.
        let chosen = matches[next].written();
        self.command_line = format!("{}{chosen}", &prefix[..start.min(prefix.len())]);
        self.command_caret = self.command_line.chars().count();
        self.completion = Some((prefix, next));
    }

    /// Open Ruby mode on whatever the cursor is pointing at.
    ///
    /// Inside an existing group, the current reading is loaded so it can be
    /// corrected rather than retyped — and cleared and submitted to take the
    /// annotation off again. Over a selection, the reading typed here wraps it.
    pub(super) fn enter_ruby_mode(&mut self) {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor);
        let line_start = rope.line_to_char(line);
        let chars: Vec<char> = crate::zong::line_chars(rope, line);
        let col = self.cursor - line_start;

        if let Some(group) = crate::ruby::group_at(&chars, col) {
            self.command_line = group.reading_text(&chars).iter().collect();
            self.command_caret = self.command_line.chars().count();
            self.ruby_target = Some(RubyTarget::Existing {
                span: (line_start + group.start, line_start + group.end),
                base: (line_start + group.base.0, line_start + group.base.1),
            });
            self.mode = Mode::Ruby;
            return;
        }

        // With no selection this annotates the character under the cursor,
        // which is in the selection like any other; only an empty buffer has
        // nothing to annotate.
        let (start, end) = self.selection();
        if end <= start {
            self.status = say!("ruby.put-cursor-in-a-reading");
            return;
        }
        self.command_line.clear();
        self.command_caret = self.command_line.chars().count();
        self.ruby_target = Some(RubyTarget::New { span: (start, end) });
        self.mode = Mode::Ruby;
    }

    pub(super) fn on_ruby_key(&mut self, key: Key) {
        match key {
            Key::Esc => {
                self.command_line.clear();
                self.command_caret = self.command_line.chars().count();
                self.ruby_target = None;
                self.mode = Mode::Normal;
            }
            // Backspacing to empty does *not* leave: an empty reading is a
            // meaningful thing to submit here — it is how an annotation is
            // taken off — so it has to be reachable. Esc is the way out.
            Key::Backspace if self.command_line.is_empty() => {}
            Key::Enter => {
                let reading = std::mem::take(&mut self.command_line);
                let target = self.ruby_target.take();
                self.mode = Mode::Normal;
                self.command_caret = 0;
                if let Some(target) = target {
                    self.apply_reading(target, &reading);
                }
            }
            // **The same prompt keys as everywhere else.** A reading is typed
            // text like a command or a search, and this mode had no caret at
            // all: no `Left`, no `Home`, no `C-a`, no `C-w` — a typo in the
            // middle of a reading meant deleting back to it.
            other => self.edit_prompt(other),
        }
    }

    /// Write `reading` onto `target`, or strip the markup when it is empty.
    fn apply_reading(&mut self, target: RubyTarget, reading: &str) {
        if self.refuse_readonly() {
            return;
        }
        let (span, base) = match target {
            RubyTarget::Existing { span, base } => (span, base),
            RubyTarget::New { span } => (span, span),
        };
        let rope = self.current_buffer().rope();
        if base.1 > rope.len_chars() || span.1 > rope.len_chars() {
            return;
        }
        let base_chars: Vec<char> = rope.slice(base.0..base.1).chars().collect();
        // An empty reading is how an annotation is removed: what is left is the
        // base, with the markup gone.
        let text = if reading.is_empty() {
            base_chars.iter().collect()
        } else {
            crate::ruby::markup(&base_chars, reading, self.ruby.writer())
        };

        // A reading is text, and text going into a row obeys the row's rule:
        // `a,b` typed as a reading used to be written straight into the rope,
        // past every gate, and the row it was on gained a field. Asked of the
        // *file*, not of `:table`, since nobody turns table mode on to annotate
        // a character.
        if let Some(why) = self.replacement_reshapes_the_grid(span, &text) {
            self.status = why;
            return;
        }
        self.snapshot();
        let done = self.current_buffer_mut().replace(span.0..span.1, &text);
        if !self.applied(done) {
            return;
        }
        self.anchor = span.0;
        self.cursor = span.0;
        self.clamp_cursor();
        self.status = if reading.is_empty() {
            say!("ruby.removed")
        } else {
            say!("ruby.set", reading)
        };
    }

    /// Replay the text typed during the last Insert session (Helix `.`).
    /// Do the last change again (`.`).
    ///
    /// The whole change, not only a typing session: `r`, `~`, `d`, `c…Esc`,
    /// `ms(`, a paste. Which makes `n.n.n.` — search, fix, search, fix — work,
    /// and that is the loop a manuscript is proofread in.
    ///
    /// It plays the *keys* back rather than re-running a remembered operation,
    /// so every command is repeatable the day it is written and none of them
    /// has to be taught about `.` — the price being that the keys act on where
    /// the cursor is *now*, which is exactly what a person pressing `.` means.
    pub(super) fn repeat_edit(&mut self) {
        if self.last_edit_keys.is_empty() {
            self.status = say!("edit.nothing-to-repeat");
            return;
        }
        // …and a guard, the way `replay_macro` has one: whatever the recorder
        // manages to record, a repeat may never repeat itself.
        if self.repeating_edit {
            return;
        }
        let keys = self.last_edit_keys.clone();
        self.repeating_edit = true;
        for key in keys {
            self.on_key(key);
        }
        // An IME `r` left `Pending::Replace` armed — its answer came from the
        // candidate panel, not from a key, so there is nothing in `keys` to
        // supply it. Supply it here, or `.` would leave `r` waiting and eat
        // whatever the reader pressed next.
        if self.pending == Pending::Replace && !self.last_replacement.is_empty() {
            self.pending = Pending::None;
            let text = self.last_replacement.clone();
            self.replace_str(&text);
        }
        self.repeating_edit = false;
    }
}
