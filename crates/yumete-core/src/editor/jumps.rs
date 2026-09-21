//! Marks and the jump list (#45).

use super::*;

impl Editor {
    // ---- Marks (Feature #45) ----------------------------------------------

    /// Name this place by a letter (`M a`).
    ///
    /// The jump list remembers where you *came from*; a mark remembers where
    /// you meant to come back **to** — the scene you are rewriting, the note
    /// at the end of the file, the chapter you keep checking against. `M` and
    /// `'` rather than vi's `m` and `'`, because `m` here opens match mode.
    pub(super) fn set_mark(&mut self, name: char) {
        if !name.is_alphanumeric() {
            self.status = say!("goto.mark-name-one-character");
            return;
        }
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(self.cursor.min(rope.len_chars()));
        let spot = match self.current_buffer().path() {
            Some(path) => Spot::InFile(path.to_path_buf(), line),
            None => Spot::InBuffer(self.current_buffer().id(), self.cursor),
        };
        self.marks.insert(name, spot);
        self.status = say!("goto.mark-set", name, self.current_buffer().display_name(), line + 1);
    }

    /// Go back to the place a letter names (`' a`).
    pub(super) fn go_to_mark(&mut self, name: char) {
        let Some(spot) = self.marks.get(&name).cloned() else {
            self.status = say!("goto.no-such-mark", name);
            return;
        };
        self.remember_jump();
        match spot {
            Spot::InFile(path, line) => {
                if self.current_buffer().path() != Some(path.as_path()) {
                    if let Err(err) = self.open_file(&path) {
                        self.status = say!("buffer.cannot-open", path.display(), err);
                        return;
                    }
                }
                self.move_to_line(line + 1);
                self.status = say!("goto.mark-in-another-file", name, self.current_buffer().display_name(), line + 1);
            }
            Spot::InBuffer(id, pos) => {
                let Some(i) = self.buffer_with(id) else {
                    self.status = say!("goto.mark-buffer-closed", name);
                    return;
                };
                self.show_buffer(i);
                self.set_cursor(pos.min(self.current_buffer().rope().len_chars()));
                self.status = say!("goto.mark-in-this-file", name);
            }
        }
    }

    // ---- The jump list (Feature #45) ---------------------------------------

    /// Note where the cursor is, before a jump takes it somewhere far.
    ///
    /// Every far motion in this editor goes through `goto_line` — `gg`, `G`,
    /// `:1200`, `:toc`, a heading in the outline, a `#include` followed with
    /// `gf`, a component followed with `Enter` in a table — so one call here
    /// gives all of them a way back. `C-o` walks back through them, `C-i`
    /// forward again, as they do in vi and in Helix.
    ///
    /// Before this, a table's `Enter` was a one-way door: following 螭 → 虫
    /// and coming back meant remembering 螭 and searching for it. The footnote
    /// panel had its own private way back; this is that idea, generalised.
    pub(super) fn remember_jump(&mut self) {
        // **This move was a jump**, which is what the page needs to know to
        // decide where to put the cursor: a jump lands in the middle, because
        // a search hit `scrolloff` from an edge shows nothing on one side of
        // the thing that was looked for. Cleared on the next key, so it
        // describes the move that just happened and nothing after it.
        self.jumped = true;
        let here = (self.current_buffer().id(), self.cursor);
        // Walking away from a place already noted adds nothing.
        if self.jumps.last() == Some(&here) {
            return;
        }
        // A new jump ends the forward history, as it does everywhere else.
        self.jumps.truncate(self.jump_at);
        self.jumps.push(here);
        // Bounded: a session of a thousand jumps does not need a thousandth of
        // them, and the oldest is the one nobody comes back to.
        if self.jumps.len() > JUMPS {
            self.jumps.remove(0);
        }
        self.jump_at = self.jumps.len();
    }

    /// Go back to where a jump came from (`C-o`), or forward again (`C-i`).
    pub(super) fn walk_jumps(&mut self, back: bool) {
        if back {
            if self.jump_at == 0 {
                self.status = say!("goto.no-earlier-jump");
                return;
            }
            // Stepping back for the first time has to note where we are, or
            // `C-i` would have nowhere to return to.
            if self.jump_at == self.jumps.len() {
                let here = (self.current_buffer().id(), self.cursor);
                if self.jumps.last() != Some(&here) {
                    self.jumps.push(here);
                }
            }
            self.jump_at -= 1;
        } else {
            if self.jump_at + 1 >= self.jumps.len() {
                self.status = say!("goto.no-later-jump");
                return;
            }
            self.jump_at += 1;
        }
        let (id, cursor) = self.jumps[self.jump_at];
        // A buffer that has since been closed leaves its jumps behind rather
        // than sending you to whichever file took its place in the list.
        match self.buffer_with(id) {
            Some(i) if i != self.current => self.show_buffer(i),
            Some(_) => {}
            None => {
                self.status = say!("goto.that-file-is-closed");
                return;
            }
        }
        let len = self.current_buffer().rope().len_chars();
        self.move_head(cursor.min(len));
        self.status = say!("goto.jump-list-position", self.jump_at + 1, self.jumps.len());
    }

    /// Find `target` on the current line (`f`/`t`/`F`/`T`), moving the head and
    /// selecting the jumped-over range (unless already extending).
    pub(super) fn find_char(&mut self, kind: FindKind, target: char) {
        // **The searching is a motion, the saying is the editor's** (B1,
        // 2026-09-20). What `f` covers is a value now — so an operator can be
        // handed it without anybody replaying the key — while 「there is no
        // such character on this line」 stays here, where the status line is.
        // ⚠️ **`t`／`T` are till again** (2026-09-21). They were retired when
        // `t` became the table group, on the reasoning that a verb-last editor
        // puts till 「one keystroke away from find and no more」. Two things
        // changed: the vim preset puts the verb *first* (`dt,`), and helix's
        // own `t` is `find_till_char` (`keymap/default.rs:14`) — so the key
        // was costing both hands their muscle memory to save one keystroke in
        // the one group that could afford to be a keystroke longer. The table
        // group is `空格 t` now.
        let span = self.run_motion(crate::motion::Motion::Find {
            forward: kind.forward(),
            target,
            till: kind.till(),
        });
        if span == crate::motion::Span::Missed {
            self.status = say!("find.no-such-character-on-this-line", target);
            return;
        }
        self.take_span(span);
    }
}
