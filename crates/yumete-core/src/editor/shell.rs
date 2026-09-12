//! The shell, the pipe, and writing markdown by name (#296).
//!
//! Three things the editor cannot do by itself: run a command, hand a
//! selection to one and take the answer back, and — the odd one out that has
//! always sat here — insert a heading, a rule or a note by name.

use super::*;

impl Editor {
    // ---- The command row (Feature #122, #302) ------------------------------

    /// A command line the front end should run.
    pub fn take_shell_request(&mut self) -> Option<Shell> {
        self.shell_request.take()
    }

    /// Put what a command said in place of the text it was given.
    ///
    /// One edit, so one `u` takes it back — which matters more here than
    /// anywhere else, because the text that went in is gone and only the
    /// command knows how to make it again.
    pub fn provide_pipe_output(&mut self, output: &str) {
        let (start, end) = self.selection();
        // A filter ends its output with a newline whether or not what it was
        // given had one. Keeping it where the selection did not have one pushes
        // the rest of the paragraph down a line every time; dropping it where
        // the selection *did* have one runs two lines together. So it follows
        // what was there.
        let had = self
            .current_buffer()
            .rope()
            .slice(start..end)
            .to_string()
            .ends_with('\n');
        let text = match had {
            true => output,
            false => output.strip_suffix('\n').unwrap_or(output),
        };
        // A filter over whole rows is the *advertised* use — the manual's own
        // example is `LC_ALL=C sort` over a table — and the grid used to refuse
        // it, after spawning the command and reading its output. What matters
        // is not that no delimiter moved, it is that **every row that comes
        // back has a row's shape**; `sort -u` dropping a duplicate row is a
        // table operation, not damage.
        let rows = self.pipe_covers_whole_rows(start, end);
        if rows {
            if let Some(why) = self.rows_break_the_grid(text) {
                self.status = why;
                return;
            }
        }
        self.snapshot();
        let done = if rows {
            self.without_cell_guard(|e| e.overwrite(start, end, text))
        } else {
            self.overwrite(start, end, text)
        };
        if !done {
            return;
        }
        self.anchor = start;
        self.cursor = (start + text.chars().count()).saturating_sub(1).max(start);
        self.clamp_cursor();
        self.status = say!("edit.pipe-replaced-characters", text.chars().count());
    }

    /// Whether a range covers whole rows of the grid — line start to line end.
    fn pipe_covers_whole_rows(&self, start: usize, end: usize) -> bool {
        if !self.table_here() {
            return false;
        }
        let rope = self.current_buffer().rope();
        let end = end.min(rope.len_chars());
        start == motion::line_start(rope, start)
            && (end == rope.len_chars()
                || end == motion::line_end(rope, end.saturating_sub(1))
                || rope.char(end.saturating_sub(1)) == '\n')
    }

    /// Whether any line of `text` would not be a row of this grid.
    fn rows_break_the_grid(&self, text: &str) -> Option<String> {
        let view = self.table.as_ref()?;
        // **The table the cursor is in, not the one it was entered in** (#283):
        // a document holds as many tables as somebody typed, and filtering the
        // five-column one through `sort` while the view still carried the
        // two-column one's schema refused every row it was handed.
        let want = self.table_column_count_at(self.cursor_line());
        for (i, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let got = view.cells(line).len();
            if got != want {
                return Some(say!("table.filter-changed-shape", i + 1, got, want));
            }
        }
        None
    }

    /// Put what a command said into a buffer of its own.
    ///
    /// A buffer rather than a message: the answer to `wc -w` is a number and
    /// would fit anywhere, but the answer to `git log` is two hundred lines,
    /// and a writer wants to search it, yank from it and keep it while they go
    /// on writing. It is the same place `:grep` puts its answers.
    pub fn provide_shell_output(&mut self, line: &str, output: &str) {
        let text = if output.trim().is_empty() {
            format!("$ {line}\n（沒有輸出）\n")
        } else {
            format!("$ {line}\n{output}")
        };
        let mut buffer = crate::Buffer::from_text(&text);
        buffer.name_as(&format!("!{line}"));
        self.add_buffer(buffer);
        self.status = say!("shell.done", line);
    }

    /// **`:markdown …`** — write a piece of Markdown at the cursor.
    ///
    /// The things a manuscript keeps needing and nobody wants to type: a
    /// footnote with the next free number *and* its note at the foot, an inline
    /// note, a table of a given size. The cursor is left where the typing goes,
    /// in Insert, because that is the next thing that happens every time.
    pub(super) fn write_markdown(&mut self, bit: crate::command::MarkdownBit) {
        use crate::command::MarkdownBit;
        match bit {
            MarkdownBit::Footnote => {
                // A footnote is Markdown. In a Typst book `[^1]` is four
                // characters of nothing, and in a plain manuscript it is four
                // characters the reader did not ask for.
                if self.current_buffer().syntax() != crate::syntax::Syntax::Markdown {
                    self.status = say!("note.not-markdown");
                    return;
                }
                // **The next free number**, from the file itself: a footnote
                // whose number is already taken is a footnote pointing at
                // somebody else's note.
                let text = self.current_buffer().text();
                let taken: Vec<usize> = crate::markdown::footnote_numbers(&text);
                let n = (1..).find(|n| !taken.contains(n)).unwrap_or(1);
                let tag = format!("[^{n}]");
                self.snapshot();
                let at = self.cursor;
                if !self.edit_insert(at, &tag) {
                    return;
                }
                self.set_cursor(at + tag.chars().count());
                // …and the note it points at, written and stood in. `gd` does
                // exactly this when it cannot find a note; this is the same
                // path, asked for rather than stumbled into.
                self.definition_preview = false;
                let rope = self.current_buffer().rope();
                let end = rope.len_chars();
                let text = self.current_buffer().text();
                let lead = match text.ends_with("\n\n") {
                    true => String::new(),
                    false => match text.ends_with('\n') {
                        true => "\n".to_string(),
                        false => "\n\n".to_string(),
                    },
                };
                let note = format!("{lead}{tag}: ");
                self.write_the_note(end, &note, &tag);
                self.mode = Mode::Insert;
            }
            MarkdownBit::InlineNote => {
                let at = self.cursor;
                self.snapshot();
                if !self.edit_insert(at, "^[]") {
                    return;
                }
                // Between the brackets, where the note goes.
                self.set_cursor(at + 2);
                self.mode = Mode::Insert;
                self.status = say!("md.inline-note-inserted");
            }
        }
    }
}
