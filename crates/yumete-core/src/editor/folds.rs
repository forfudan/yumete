//! **Folds on the page**: which lines a window shows as one
//! row, and vim's keys for them. The bookkeeping is [`crate::fold`]; this
//! is where the folds a window has meet the page.
//!
//! A closed fold is drawn through the paths a table already uses: the lines
//! after its first are off the page ([`Editor::line_is_folded`]), and its
//! first line is one row ([`Editor::table_row_at`]) with its text off the
//! page and the fold's own line drawn at its end ([`Editor::fold_line`]).
//! So the caret, `j`, the wrap and the mouse all read the same page.
//!
//! What lands inside a closed fold opens it, as vim's `foldopen` does for
//! search, marks, `%`, undo and the horizontal moves, and as Insert always
//! does; only vim's jumps (`G`, `gg`, `:N`, `C-o`, `C-i`) leave it closed
//! and put the cursor on the fold's line (measured in nvim).

use super::*;
use crate::fold::FoldSet;

/// What a fold is: one made with `zf`, or a heading's section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FoldOf {
    Manual(u64),
    /// The heading's line.
    Heading(usize),
}

/// One fold, as lines.
#[derive(Debug, Clone)]
pub(super) struct FoldSpan {
    pub(super) first: usize,
    pub(super) last: usize,
    pub(super) closed: bool,
    pub(super) of: FoldOf,
    /// 1 for a fold no other holds.
    pub(super) depth: usize,
}

impl FoldSpan {
    fn holds(&self, line: usize) -> bool {
        self.first <= line && line <= self.last
    }
}

/// The folds of the window and buffer in view, worked out once per change.
#[derive(Debug, Default)]
pub(super) struct FoldView {
    /// The syntax too: `:syntax` changes which lines are headings without
    /// an edit.
    key: (u64, u64, u64, u64, bool, crate::syntax::Syntax),
    /// Outermost first: by first line, the longer of two that start on one
    /// line before the shorter.
    pub(super) spans: Vec<FoldSpan>,
    /// The closed folds no closed fold holds: what the page leaves out.
    shut: Vec<(usize, usize)>,
}

impl Editor {
    /// The folds this window has in this buffer, worked out when the buffer,
    /// the window or a fold changed.
    pub(super) fn fold_view(&self) -> std::cell::Ref<'_, FoldView> {
        let buffer = self.current_buffer();
        let headings = self.flavor_on(yumete_cjk::FlavorItem::HeadingFolds) && self.lists_here();
        let key = (buffer.id(), buffer.revision(), self.folds_window(), self.fold_generation, headings, buffer.syntax());
        // **The outline waits for Normal mode**: while typing, a view that
        // differs only by the edit stands, so a key typed does not rebuild the
        // book's outline. Not when a fold is closed: lines typed above it
        // would move what it hides.
        let stale = self.fold_view.borrow().as_ref().is_none_or(|view| {
            let only_typed = view.key.0 == key.0
                && (view.key.2, view.key.3, view.key.4, view.key.5) == (key.2, key.3, key.4, key.5)
                && self.mode == crate::input::Mode::Insert
                && view.shut.is_empty();
            view.key != key && !only_typed
        });
        if stale {
            let view = self.work_out_folds(key, headings);
            *self.fold_view.borrow_mut() = Some(view);
        }
        std::cell::Ref::map(self.fold_view.borrow(), |view| view.as_ref().expect("filled above"))
    }

    /// The window whose folds are asked about: the one being drawn while
    /// another half of a split is, the live one otherwise.
    fn folds_window(&self) -> crate::window::WindowId {
        self.viewing.get().map_or(self.live, |v| v.window)
    }

    fn work_out_folds(&self, key: (u64, u64, u64, u64, bool, crate::syntax::Syntax), headings: bool) -> FoldView {
        let buffer = self.current_buffer();
        let rope = buffer.rope();
        let len = rope.len_chars();
        let line_of = |at: usize| rope.char_to_line(at.min(len));
        let set = buffer.folds(key.2);
        let mut spans: Vec<FoldSpan> = Vec::new();
        if headings {
            for (line, _, start, end) in self.prose_sections() {
                let last = line_of(end.saturating_sub(1).max(start));
                let closed = set.is_some_and(|s| s.closed_headings.iter().any(|(at, text)| heading_is(rope, line, *at, text)));
                spans.push(FoldSpan { first: line, last, closed, of: FoldOf::Heading(line), depth: 0 });
            }
        }
        if let Some(set) = set {
            for fold in &set.manual {
                spans.push(FoldSpan {
                    first: line_of(fold.from),
                    last: line_of(fold.to),
                    closed: fold.closed,
                    of: FoldOf::Manual(fold.id),
                    depth: 0,
                });
            }
        }
        spans.sort_by_key(|s| (s.first, std::cmp::Reverse(s.last)));
        // Depth: how many hold it, itself included.
        let mut open: Vec<usize> = Vec::new();
        for i in 0..spans.len() {
            while open.last().is_some_and(|&j| spans[j].last < spans[i].first) {
                open.pop();
            }
            spans[i].depth = open.len() + 1;
            open.push(i);
        }
        let disabled = set.is_some_and(|s| s.disabled);
        let mut shut: Vec<(usize, usize)> = Vec::new();
        if !disabled {
            for span in spans.iter().filter(|s| s.closed && s.last > s.first) {
                if shut.last().is_some_and(|&(_, last)| span.first <= last) {
                    continue;
                }
                shut.push((span.first, span.last));
            }
        }
        FoldView { key, spans, shut }
    }

    /// The closed fold `line` is in, when one is: its first and last line.
    pub(super) fn shut_around(&self, line: usize) -> Option<(usize, usize)> {
        let view = self.fold_view();
        let at = view.shut.partition_point(|&(first, _)| first <= line);
        at.checked_sub(1).map(|i| view.shut[i]).filter(|&(_, last)| line <= last)
    }

    /// Whether `line` is inside a closed fold and not its first line, so not
    /// on the page.
    pub fn fold_hides(&self, line: usize) -> bool {
        self.shut_around(line).is_some_and(|(first, _)| line > first)
    }

    /// Whether `line` is the first line of a closed fold: drawn as the
    /// fold's own line.
    pub fn fold_head(&self, line: usize) -> bool {
        self.shut_around(line).is_some_and(|(first, _)| line == first)
    }

    /// A closed fold's first line, all of it off the page: the span that
    /// says so, or `None` for any other line.
    pub(super) fn fold_head_hides(&self, line: usize) -> Option<Vec<(usize, usize)>> {
        self.fold_head(line).then(|| vec![(0, crate::zong::line_chars(self.current_buffer().rope(), line).len())])
    }

    /// The width a fold's line fills, as the front end last said (the text
    /// area's columns), or a 縱's length on a vertical page.
    pub fn note_fold_width(&self, columns: usize) {
        self.fold_width.set(columns);
    }

    /// **What a closed fold's line says**: in code vim's own,
    /// `+--  5 lines: fn main() {···`; in a text file the heading or the
    /// first words and how many 字 are folded, `## 第三節 ···· 4,213 字`, as
    /// the status line counts them.
    pub fn fold_line(&self, line: usize) -> Option<String> {
        let (first, last) = self.shut_around(line).filter(|&(first, _)| first == line)?;
        let (depth, heading) = self
            .fold_view()
            .spans
            .iter()
            .find(|s| (s.first, s.last) == (first, last))
            .map_or((1, false), |s| (s.depth, matches!(s.of, FoldOf::Heading(..))));
        let rope = self.current_buffer().rope();
        let text: String = rope
            .line(first)
            .to_string()
            .trim_end()
            .trim_start()
            .replace('\t', " ");
        let width = self.fold_width.get().max(8);
        // A 縱 holds one character a row, whatever its width, and a dot
        // half a cell wide would be a row each: the vertical page measures
        // in characters and fills with ︙.
        let vertical = self.layout() == Layout::Vertical;
        // Rows down a 縱, packed as the page packs them: `104` is one row
        // there, and counted as three it would cut the heading short.
        let size = |text: &str| match vertical {
            true => crate::zong::slot_count(text, self.tatechuyoko),
            false => yumete_cjk::str_width(text),
        };
        let dot = if vertical { "︙" } else { "·" };
        // The run stands after the line's hidden text, which the 縱 starts
        // a row down for (measured on the page).
        let width = match vertical {
            true => width.saturating_sub(1).max(4),
            false => width,
        };
        let fill = |head: String, tail: String| -> String {
            let used = size(&head) + size(&tail);
            let room = width.saturating_sub(used + 2);
            match tail.is_empty() {
                true => format!("{head}{}", dot.repeat(room + 2)),
                false => format!("{head} {} {tail}", dot.repeat(room)),
            }
        };
        let cut = |text: &str, room: usize| -> String {
            let mut out = String::new();
            let mut used = 0;
            for g in yumete_cjk::graphemes(text) {
                let w = size(g);
                if used + w > room {
                    out.push('…');
                    break;
                }
                used += w;
                out.push_str(g);
            }
            out
        };
        if self.writes_code() {
            let head = format!("+-{} {:>2} lines: ", "-".repeat(depth), last - first + 1);
            let room = width.saturating_sub(size(&head) + 1);
            return Some(fill(format!("{head}{}", cut(&text, room)), String::new()));
        }
        let mut units = crate::units::Units::default();
        let mut buf = String::new();
        // A heading's fold counts what is under it, as the outline does: the
        // heading itself is on the line.
        for l in first + usize::from(heading)..=last.min(rope.len_lines().saturating_sub(1)) {
            buf.clear();
            buf.extend(rope.line(l).chunks());
            self.units_in_line(&buf, &mut units);
        }
        let count = match units.total() {
            1 => say!("ui.count-one", 1),
            n => say!("ui.count", n),
        };
        let room = width.saturating_sub(size(&count) + 4);
        Some(fill(cut(&text, room), count))
    }

    // ---- What lands in a fold -------------------------------------------

    /// **After every key**: a cursor that the key put inside a closed fold
    /// opens it, unless the key was one of vim's jumps, which leave it
    /// closed and the cursor on its line (the place is kept for `zo`).
    pub(super) fn settle_folds(&mut self) {
        let keep_closed = std::mem::take(&mut self.fold_stays_closed);
        // A hidden place outlives no move off the fold's line.
        if self.fold_return.is_some() && self.hidden_place().is_none() {
            self.fold_return = None;
        }
        // Typing opens the folds every cursor is in, not the primary's
        // alone: text a closed fold hides is not typed into unseen.
        if self.mode == Mode::Insert && self.sel.is_plural() {
            let rope = self.current_buffer().rope();
            let len = rope.len_chars();
            let lines: Vec<usize> = self.sel.secondaries().map(|r| rope.char_to_line(r.head.min(len))).collect();
            for line in lines {
                if self.shut_around(line).is_some() {
                    self.open_folds_at(line, usize::MAX);
                }
            }
        }
        let line = self.cursor_line();
        let Some((first, _)) = self.shut_around(line) else { return };
        // Typing on a closed fold's line opens it, as vim's Insert does.
        if line == first && self.mode != Mode::Insert {
            return;
        }
        if keep_closed && self.mode != Mode::Insert {
            let at = self.sel.head();
            let rope = self.current_buffer().rope();
            let to = rope.line_to_char(first);
            self.fold_return = Some((self.current_buffer().id(), first, at));
            self.set_cursor(to);
            return;
        }
        self.open_folds_at(line, usize::MAX);
    }

    /// Open the closed folds holding `line`, outermost first, `levels` of
    /// them (vim's `zo` with a count; `zv` and `zO` open them all).
    pub(super) fn open_folds_at(&mut self, line: usize, levels: usize) {
        let closed: Vec<FoldOf> = self
            .fold_view()
            .spans
            .iter()
            .filter(|s| s.holds(line) && s.closed)
            .map(|s| s.of.clone())
            .collect();
        for of in closed.into_iter().take(levels) {
            self.set_fold_closed(&of, false);
        }
        self.cursor_back_into_the_fold();
    }

    /// **Before a Normal command under vim**: a sideways move on a closed
    /// fold opens it and moves on its first line ('foldopen' has `hor`:
    /// `l` `$` `f` …), and a word motion from one starts past it, at the
    /// end of its last line or the start of its first, as vim's
    /// `fwd_word` does (measured in nvim).
    ///
    /// The edits at the cursor that do not take a closed fold whole act
    /// where a hidden cursor really is: `~` `J` `gJ` change its own line and
    /// leave the fold closed, `a` and `A` type there (as in nvim).
    pub(super) fn before_a_motion(&mut self, cmd: yumete_cjk::actions::NormalCmd) {
        use yumete_cjk::actions::NormalCmd as C;
        if !self.vim_keymap() || self.pending != Pending::None || self.mode == Mode::Insert {
            return;
        }
        let line = self.cursor_line();
        let Some((first, last)) = self.shut_around(line) else { return };
        match cmd {
            C::MoveCharLeft
            | C::MoveCharRight
            | C::GotoLineStart
            | C::GotoLineEnd
            | C::VimGotoLineEnd
            | C::GotoFirstNonwhitespace
            | C::VimGotoFirstNonwhitespace
            | C::GotoColumn
            | C::FindNextChar
            | C::FindPrevChar
            | C::FindTillChar
            | C::TillPrevChar
            | C::RepeatFindChar
            | C::RepeatFindCharReverse => {
                let at = self.hidden_place();
                self.fold_return = None;
                self.open_folds_at(line, usize::MAX);
                if let Some(at) = at {
                    self.sel.set_head(at);
                    self.sel.set_anchor(at);
                }
            }
            C::JumpNextWordStart | C::JumpNextLongWordStart | C::JumpNextWordEnd | C::JumpNextLongWordEnd => {
                let rope = self.current_buffer().rope();
                let at = crate::motion::line_last(rope, rope.line_to_char(last));
                self.sel.set_head(at);
                self.sel.set_anchor(at);
            }
            C::JumpPrevWordStart | C::JumpPrevLongWordStart => {
                let at = self.current_buffer().rope().line_to_char(first);
                self.sel.set_head(at);
                self.sel.set_anchor(at);
            }
            C::VimSwitchCase | C::VimJoinSelections => self.stand_where_hidden(true),
            C::VimAppendMode | C::VimInsertAtLineEnd => self.stand_where_hidden(false),
            _ => {}
        }
    }

    /// **The cursor to the line a closed fold hides it on**, for an edit
    /// made there; `closed` keeps the fold closed over it after.
    pub(super) fn stand_where_hidden(&mut self, closed: bool) {
        let Some(at) = self.hidden_place() else { return };
        self.fold_return = None;
        self.sel.set_head(at);
        self.sel.set_anchor(at);
        self.fold_stays_closed |= closed;
    }

    /// **Where the cursor stands when a closed fold hides it**: vim keeps it
    /// on its own line inside the fold, drawn on the fold's (`:3` in a fold
    /// of lines 2–4 reads Ln 3, as in nvim), and the fold keys act
    /// there. `None` when the cursor is where it is drawn.
    pub fn hidden_place(&self) -> Option<usize> {
        let (buffer, first, at) = self.fold_return?;
        let rope = self.current_buffer().rope();
        let at = at.min(rope.len_chars());
        let drawn = rope.line_to_char(first.min(rope.len_lines().saturating_sub(1)));
        (buffer == self.current_buffer().id() && self.sel.head() == drawn && self.fold_hides(rope.char_to_line(at)))
            .then_some(at)
    }

    /// The cursor's line as the fold keys read it: the hidden one, when a
    /// closed fold hides it.
    fn fold_cursor_line(&self) -> usize {
        match self.hidden_place() {
            Some(at) => self.current_buffer().rope().char_to_line(at),
            None => self.cursor_line(),
        }
    }

    /// Open the closed folds that hide `line` and leave one that starts on
    /// it closed: the outline goes to a heading and shows it, and its mark
    /// already says whether the section is closed.
    pub(super) fn open_folds_over(&mut self, line: usize) {
        let hiding: Vec<FoldOf> = self
            .fold_view()
            .spans
            .iter()
            .filter(|s| s.holds(line) && s.first < line && s.closed)
            .map(|s| s.of.clone())
            .collect();
        for of in hiding {
            self.set_fold_closed(&of, false);
        }
    }

    /// The place `G` left inside a fold, back when that fold opens with the
    /// cursor still on its line.
    fn cursor_back_into_the_fold(&mut self) {
        let Some((buffer, first, at)) = self.fold_return else { return };
        if buffer != self.current_buffer().id() || self.cursor_line() != first {
            self.fold_return = None;
            return;
        }
        let line = self.current_buffer().rope().char_to_line(at.min(self.current_buffer().rope().len_chars()));
        if !self.fold_hides(line) {
            self.fold_return = None;
            self.set_cursor(at.min(self.current_buffer().rope().len_chars()));
        }
    }

    /// Open or close one fold.
    fn set_fold_closed(&mut self, of: &FoldOf, closed: bool) {
        let window = self.live;
        match of {
            FoldOf::Manual(id) => {
                let set: &mut FoldSet = self.current_buffer_mut().folds_mut(window);
                if let Some(fold) = set.manual.iter_mut().find(|f| f.id == *id) {
                    fold.closed = closed;
                }
            }
            FoldOf::Heading(line) => {
                let rope = self.current_buffer().rope().clone();
                let (at, text) = heading_place(&rope, *line);
                let set: &mut FoldSet = self.current_buffer_mut().folds_mut(window);
                // A place whose heading went (deleted, rewritten) is let go.
                set.closed_headings.retain(|(a, t)| {
                    !(*a == at && *t == text) && heading_is(&rope, rope.char_to_line((*a).min(rope.len_chars())), *a, t)
                });
                if closed {
                    set.closed_headings.push((at, text));
                }
            }
        }
        self.fold_generation += 1;
    }

    /// The folds holding the cursor's line, outermost first.
    fn folds_here(&self) -> Vec<FoldSpan> {
        let line = self.fold_cursor_line();
        self.fold_view().spans.iter().filter(|s| s.holds(line)).cloned().collect()
    }

    // ---- vim's keys -------------------------------------------------------

    /// `zo` (`count` levels) and `zO` (all).
    pub(super) fn fold_open(&mut self, count: usize, all: bool) {
        let line = self.fold_cursor_line();
        if self.folds_here().is_empty() {
            return self.no_fold_here();
        }
        self.open_folds_at(line, if all { usize::MAX } else { count.max(1) });
    }

    /// `zc` (`count` levels, the innermost open one first) and `zC` (all).
    pub(super) fn fold_close(&mut self, count: usize, all: bool) {
        let here = self.folds_here();
        if here.is_empty() {
            return self.no_fold_here();
        }
        let open: Vec<FoldOf> = here.iter().rev().filter(|s| !s.closed).map(|s| s.of.clone()).collect();
        let take = if all { usize::MAX } else { count.max(1) };
        for of in open.into_iter().take(take) {
            self.set_fold_closed(&of, true);
        }
        self.fold_stays_closed = true;
        self.uncover_disabled();
    }

    /// `za` / `zA`: open the closed fold the cursor is on, or close the
    /// open one it is in.
    pub(super) fn fold_toggle(&mut self, count: usize, all: bool) {
        match self.shut_around(self.fold_cursor_line()).is_some() {
            true => self.fold_open(count, all),
            false => self.fold_close(count, all),
        }
    }

    /// `zv`: open just enough to see the cursor's line.
    pub(super) fn fold_view_cursor(&mut self) {
        let line = self.fold_cursor_line();
        self.open_folds_at(line, usize::MAX);
    }

    /// `zR` opens every fold, `zM` closes every one.
    pub(super) fn fold_all(&mut self, closed: bool) {
        let hidden = self.hidden_place();
        let all: Vec<FoldOf> = self.fold_view().spans.iter().map(|s| s.of.clone()).collect();
        for of in &all {
            self.set_fold_closed(of, closed);
        }
        let deepest = self.fold_view().spans.iter().map(|s| s.depth).max().unwrap_or(0);
        self.keep_fold_level(if closed { 0 } else { deepest });
        if closed {
            self.fold_stays_closed = true;
            self.uncover_disabled();
        }
        self.back_where_hidden(hidden);
    }

    /// **A cursor a closed fold hid stands on its own line** once a key has
    /// opened the folds over it (`zR`, `zr`), as `zo` leaves it and vim's
    /// does — not on the fold's first line.
    fn back_where_hidden(&mut self, hidden: Option<usize>) {
        let Some(at) = hidden else { return };
        let line = self.current_buffer().rope().char_to_line(at.min(self.current_buffer().rope().len_chars()));
        if !self.fold_hides(line) {
            self.fold_return = None;
            self.sel.set_head(at);
            self.sel.set_anchor(at);
        }
    }

    /// **`zm` (`less` false) and `zr`**: one level more folded, or less,
    /// `count` levels. The level is where the folds stand now — the depth
    /// of the shallowest closed fold, less one, or the deepest when none is
    /// closed — and every fold deeper than the new level closes and every
    /// other opens, as vim's 'foldlevel' does. `zm` with the headings open
    /// down to `##` is the chapter's outline.
    pub(super) fn fold_level(&mut self, less: bool, count: usize) {
        let kept = self.current_buffer().folds(self.live).and_then(|s| s.level);
        let (now, deepest) = {
            let view = self.fold_view();
            let deepest = view.spans.iter().map(|s| s.depth).max().unwrap_or(0);
            let shallowest_closed = view.spans.iter().filter(|s| s.closed).map(|s| s.depth).min();
            // The level `zm` `zr` last set, as vim keeps 'foldlevel'; before
            // any, where the folds stand.
            (kept.unwrap_or_else(|| shallowest_closed.map_or(deepest, |d| d - 1)).min(deepest), deepest)
        };
        if deepest == 0 {
            return self.no_fold_here();
        }
        let level = match less {
            true => (now + count.max(1)).min(deepest),
            false => now.saturating_sub(count.max(1)),
        };
        let hidden = self.hidden_place();
        self.apply_fold_level(level);
        self.back_where_hidden(hidden);
        self.keep_fold_level(level);
        self.fold_stays_closed = true;
        self.status = say!("fold.level", level);
    }

    /// Every fold deeper than `level` closed, every other open.
    fn apply_fold_level(&mut self, level: usize) {
        let all: Vec<(FoldOf, usize)> = self.fold_view().spans.iter().map(|s| (s.of.clone(), s.depth)).collect();
        for (of, depth) in all {
            self.set_fold_closed(&of, depth > level);
        }
    }

    fn keep_fold_level(&mut self, level: usize) {
        let window = self.live;
        self.current_buffer_mut().folds_mut(window).level = Some(level);
    }

    /// **`zx` (`view`) and `zX`**: the folds opened and closed by hand put
    /// back to the level, folding turned on; `zx` then opens what the cursor
    /// is in, as `zv` (`nv_zet`).
    pub(super) fn reapply_fold_level(&mut self, view: bool) {
        let window = self.live;
        let (level, deepest) = {
            let v = self.fold_view();
            (self.current_buffer().folds(window).and_then(|s| s.level), v.spans.iter().map(|s| s.depth).max().unwrap_or(0))
        };
        if deepest == 0 {
            return self.no_fold_here();
        }
        if self.current_buffer().folds(window).is_some_and(|s| s.disabled) {
            self.current_buffer_mut().folds_mut(window).disabled = false;
            self.fold_generation += 1;
        }
        // Unset, vim's 'foldlevel' is 0: every fold closes.
        let unset = if self.vim_keymap() { 0 } else { deepest };
        self.apply_fold_level(level.unwrap_or(unset));
        match view {
            true => self.fold_view_cursor(),
            false => self.fold_stays_closed = true,
        }
    }

    /// `zn` (`Some(false)`), `zN` (`Some(true)`), `zi` (`None`, flips): folds
    /// off and back on, each kept as it was.
    pub(super) fn fold_enable(&mut self, on: Option<bool>) {
        let window = self.live;
        let set = self.current_buffer_mut().folds_mut(window);
        set.disabled = match on {
            Some(on) => !on,
            None => !set.disabled,
        };
        self.fold_stays_closed = true;
        self.fold_generation += 1;
    }

    /// Closing a fold turns folding back on, as vim's `zc` sets
    /// 'foldenable'.
    fn uncover_disabled(&mut self) {
        let window = self.live;
        if self.current_buffer().folds(window).is_some_and(|s| s.disabled) {
            self.current_buffer_mut().folds_mut(window).disabled = false;
            self.fold_generation += 1;
        }
    }

    /// `:view-fold` and helix's `zf`: a closed fold over the selection's
    /// lines.
    pub(super) fn fold_selection(&mut self) {
        let (from, to) = self.selection();
        let rope = self.current_buffer().rope();
        let (first, last) = (rope.char_to_line(from), rope.char_to_line(to.saturating_sub(1).max(from)));
        self.extend = false;
        self.fold_lines(first, last);
    }

    /// `zf{motion}`, Visual `zf`, `zF` and `:fold`: a closed fold over lines
    /// `first..=last`.
    pub(super) fn fold_lines(&mut self, first: usize, last: usize) {
        let window = self.live;
        let rope = self.current_buffer().rope();
        let last = last.min(rope.len_lines().saturating_sub(1));
        let from = rope.line_to_char(first.min(last));
        let to = rope.line_to_char(last) + crate::zong::line_chars(rope, last).len();
        self.current_buffer_mut().folds_mut(window).add(from, to);
        self.fold_generation += 1;
        self.uncover_disabled();
        self.fold_stays_closed = true;
        let to = self.current_buffer().rope().line_to_char(first.min(last));
        self.set_cursor(to);
    }

    /// `zd` (the innermost fold made with `zf` here), `zD` (every one
    /// here) and `zE` (every one in the window).
    pub(super) fn fold_delete(&mut self, which: FoldDelete) {
        let here = self.folds_here();
        let ids: Vec<u64> = match which {
            FoldDelete::Innermost => here.iter().rev().filter_map(|s| match s.of {
                FoldOf::Manual(id) => Some(id),
                _ => None,
            }).take(1).collect(),
            FoldDelete::Here => here.iter().filter_map(|s| match s.of {
                FoldOf::Manual(id) => Some(id),
                _ => None,
            }).collect(),
            FoldDelete::All => Vec::new(),
        };
        let window = self.live;
        match which {
            FoldDelete::All => {
                let set = self.current_buffer_mut().folds_mut(window);
                let ids: Vec<u64> = set.manual.iter().map(|f| f.id).collect();
                set.deleted.extend(ids);
                set.manual.clear();
            }
            _ if ids.is_empty() => {
                return match here.is_empty() {
                    true => self.no_fold_here(),
                    false => self.status = say!("fold.not-made-with-zf"),
                };
            }
            _ => {
                let set = self.current_buffer_mut().folds_mut(window);
                set.deleted.extend(ids.iter().copied());
                set.manual.retain(|f| !ids.contains(&f.id));
            }
        }
        self.fold_generation += 1;
    }

    /// `zj` (down to the next fold's start) and `zk` (up to the previous
    /// one's end), `count` folds on. A closed fold is one fold; those
    /// inside it are not on the page.
    pub(super) fn fold_step(&mut self, down: bool, count: usize) {
        let mut line = self.cursor_line();
        for _ in 0..count.max(1) {
            let next = {
                let view = self.fold_view();
                let seen = |s: &&FoldSpan| !view.shut.iter().any(|&(f, l)| f < s.first && s.first <= l);
                match down {
                    true => view.spans.iter().filter(seen).map(|s| s.first).filter(|&f| f > line).min(),
                    false => view.spans.iter().filter(seen).map(|s| s.last).filter(|&l| l < line).max(),
                }
            };
            match next {
                Some(to) => line = to,
                None => break,
            }
        }
        if line != self.cursor_line() {
            let at = self.current_buffer().rope().line_to_char(line);
            self.set_cursor(at);
        }
    }

    /// `[z` and `]z`: the start or end of the open fold the cursor is in;
    /// already there, of the one holding that.
    pub(super) fn fold_edge(&mut self, end: bool) {
        let line = self.cursor_line();
        let here: Vec<FoldSpan> = self.folds_here().into_iter().filter(|s| !s.closed).collect();
        let target = here.iter().rev().map(|s| if end { s.last } else { s.first }).find(|&l| l != line);
        if let Some(to) = target {
            let at = self.current_buffer().rope().line_to_char(to);
            self.set_cursor(at);
        }
    }

    /// The primary selection widened to the whole lines of the closed folds
    /// at its ends, line break and all. `false` when it reaches into none.
    pub(super) fn select_over_closed_folds(&mut self) -> bool {
        let (from, to) = self.selection();
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();
        let a = rope.char_to_line(from.min(len));
        let b = rope.char_to_line(to.saturating_sub(1).max(from).min(len));
        let (fa, fb) = (self.shut_around(a), self.shut_around(b));
        if fa.is_none() && fb.is_none() {
            return false;
        }
        let first = fa.map_or(a, |(first, _)| first);
        let last = fb.map_or(b, |(_, last)| last);
        let start = rope.line_to_char(first);
        let end = rope.line_to_char((last + 1).min(rope.len_lines()));
        let head = crate::motion::prev_grapheme(rope, end).max(start);
        self.sel.set_anchor(start);
        self.sel.set_head(head);
        true
    }

    fn no_fold_here(&mut self) {
        self.status = say!("fold.none-here");
    }

    /// A window split from this one: the new window `to` has its folds.
    pub(super) fn fold_copy_to(&mut self, from: u64, to: u64) {
        self.current_buffer_mut().copy_folds(from, to);
        self.fold_generation += 1;
    }
}

/// Which manual folds `zd` `zD` `zE` take.
#[derive(Debug, Clone, Copy)]
pub(super) enum FoldDelete {
    Innermost,
    Here,
    All,
}

/// Where a heading's line starts, and its text: how a closed heading is
/// kept ([`crate::fold::FoldSet::closed_headings`]).
pub(super) fn heading_place(rope: &crate::Rope, line: usize) -> (usize, String) {
    let line = line.min(rope.len_lines().saturating_sub(1));
    (rope.line_to_char(line), rope.line(line).to_string().trim_end().to_string())
}

/// Whether the heading on `line` is the one kept as `at` and `text`.
fn heading_is(rope: &crate::Rope, line: usize, at: usize, text: &str) -> bool {
    let (start, now) = heading_place(rope, line);
    start == at && now == text
}
