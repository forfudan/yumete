//! Merge conflicts: walking them, and keeping one side (#249, #296).
//!
//! It sat in the middle of the table code, between 「go to the next `|` table」
//! and 「type in a cell」, because that is where it happened to be written — one
//! `// ----` marker away from anything about it. Moved out on 2026-09-08 as
//! `editor.rs` was taken apart; the code is untouched.

use super::*;

impl Editor {
    /// `]c` / `[c` — the next merge conflict, and the one before (#249).
    ///
    /// Lands on the `<<<<<<<` line, which is where the reader has to start
    /// reading anyway. Does **not** wrap: a file's conflicts are a list to be
    /// worked through from the top, and a wrap turns 「that was the last one」
    /// into a silent trip back to the first.
    pub(super) fn go_to_conflict(&mut self, forward: bool) {
        let found = self.conflicts();
        if found.is_empty() {
            self.status = say!("conflict.none");
            return;
        }
        let here = self.cursor_line();
        // Out of the one the cursor stands in first, in whichever direction —
        // otherwise 「next」 lands on the same conflict's own foot marker.
        let standing = self.conflict_at(here);
        let next = match forward {
            true => {
                let from = standing.map(|c| c.foot).unwrap_or(here);
                found.iter().find(|c| c.head > from)
            }
            false => {
                let from = standing.map(|c| c.head).unwrap_or(here);
                found.iter().rev().find(|c| c.foot < from)
            }
        };
        let Some(conflict) = next else {
            self.status = match forward {
                true => say!("conflict.no-next"),
                false => say!("conflict.no-previous"),
            };
            return;
        };
        let line = conflict.head;
        self.remember_jump();
        self.goto_line(line + 1);
        self.status = say!("conflict.at-line", line + 1);
    }

    /// Keep one side of the conflict the cursor stands in, markers and all the
    /// rest thrown away (#249).
    ///
    /// **The markers go too.** Resolving a conflict is exactly the act of
    /// taking the furniture off the page: a file that still has seven angle
    /// brackets in it is one git will not let anybody commit, so a 「resolution」
    /// that left them behind would be a lie the editor told.
    pub(super) fn resolve_conflict(&mut self, keep: crate::conflict::Keep) {
        if self.refuse_readonly() {
            return;
        }
        let Some(conflict) = self.conflict_at(self.cursor_line()) else {
            self.status = say!("conflict.not-in-one");
            return;
        };
        let kept = conflict.kept(keep);
        if kept.is_empty() && keep == crate::conflict::Keep::Base {
            self.status = say!("conflict.no-ancestor");
            return;
        }
        let rope = self.current_buffer().rope();
        let lines: Vec<String> = kept
            .into_iter()
            .flatten()
            .map(|line| {
                rope.line(line)
                    .to_string()
                    .trim_end_matches(['\n', '\r'])
                    .to_string()
            })
            .collect();
        self.snapshot();
        // **Both sides may lose.** `Keep::Base` on a file merged without the
        // common ancestor written down, or either side of an empty half, ends
        // with nothing at all — and `replace_lines` with no lines is how a
        // conflict that was only ever an addition gets taken back out.
        self.replace_lines(conflict.head, conflict.foot, &lines);
        self.goto_line(conflict.head + 1);
        self.status = say!("conflict.kept", lines.len());
    }


    // ---- Where the conflicts are (Feature #249) ---------------------------

    /// Every merge conflict in this buffer, in the order they are written
    /// (Feature #249).
    ///
    /// Not gated on whether the markup is drawn: `:render off` says how the
    /// page is *coloured*, and `]c` is a motion. A file with seven angle
    /// brackets in it is in a state the writer needs to get out of either way.
    pub fn conflicts(&self) -> Vec<crate::conflict::Conflict> {
        self.scan_blocks();
        let key = (
            self.current_buffer().id(),
            self.current_buffer().revision(),
        );
        match self.block_cache.borrow().as_ref() {
            Some((cached, _, found)) if *cached == key => found.clone(),
            _ => Vec::new(),
        }
    }

    /// The conflict `line` stands in, markers included.
    pub fn conflict_at(&self, line: usize) -> Option<crate::conflict::Conflict> {
        self.conflicts()
            .into_iter()
            .find(|c| c.lines().contains(&line))
    }

    /// `:check merge` — every conflict in this file, as a buffer to walk (#249).
    ///
    /// The same `路徑:行:` shape `:grep` writes, so `gf` follows a row back to
    /// the line it names and every motion works in the list. **This file
    /// only**: a merge conflict is a state a file is in, and the file the
    /// writer is looking at is the one they are about to resolve.
    pub(super) fn list_conflicts(&mut self) {
        let found = self.conflicts();
        if found.is_empty() {
            self.status = say!("conflict.none");
            return;
        }
        let shown = self
            .current_buffer()
            .path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| self.current_buffer().display_name());
        let mut listing = String::new();
        for c in &found {
            // Wordless on purpose: the two labels are the branches git wrote
            // into the file, and they say more than any sentence here could.
            listing.push_str(&format!(
                "{shown}:{}: {} ⇄ {}\n",
                c.head + 1,
                c.ours,
                c.theirs
            ));
        }
        let count = found.len();
        let mut buffer = Buffer::from_text(&listing);
        buffer.name_as("[conflicts]");
        self.add_buffer(buffer);
        self.set_cursor(0);
        self.status = say!("conflict.some", count);
    }
}
