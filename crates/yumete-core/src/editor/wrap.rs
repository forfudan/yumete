//! Soft wrap, and the shape of the page it wraps into (#77).

use super::*;

impl Editor {
    // ---- Soft wrap (Feature #77) ------------------------------------------

    /// Whether long paragraphs wrap onto further screen rows.
    pub fn soft_wrap(&self) -> bool {
        self.soft_wrap
    }

    /// Set whether long paragraphs wrap, returning the new state.
    ///
    /// With it off a paragraph wider than the terminal runs off the right edge
    /// and the rest cannot be reached with the eye — which is why it is on by
    /// default in an editor for prose.
    pub fn set_soft_wrap(&mut self, on: bool) -> bool {
        self.soft_wrap = on;
        self.soft_wrap
    }

    /// Tell the editor how much room the renderer has, so `j` and `k` walk the
    /// same rows the reader sees. The renderer calls this once per frame.
    ///
    /// A measure the writer has set wins, but only downwards: `:view-wrap 50` on a
    /// 40-column terminal still has to wrap at 40, because rows that do not fit
    /// cannot be read.
    pub fn set_wrap_width(&mut self, available: usize) {
        let width = self.measure.map_or(available, |m| m.min(available));
        self.wrap_width = Some(width.max(crate::wrap::MIN_WRAP_WIDTH));
    }

    /// The gap between 縱, if the writer has set one for this session.
    pub fn zong_gap(&self) -> Option<usize> {
        // Packed, there is none; otherwise whatever was asked for.
        self.dense.then_some(0).or(self.zong_gap)
    }

    /// Pack the page as tight as a terminal can, or let it breathe again.
    ///
    /// Four things at once, because they are one thing: **how much of the
    /// window is writing**. The gap between 縱 goes, the reading column goes,
    /// the margin 句讀 hang in goes, and the 稿紙 ticks go — after which a 縱 is
    /// two cells wide, which is exactly one 漢字 and the narrowest a terminal
    /// can draw one.
    ///
    /// What it cannot do is make the 字 itself narrower: a terminal cell is a
    /// fixed size that the terminal decides, and 90%-wide cells are a setting
    /// in the terminal, not here.
    pub fn set_dense(&mut self, on: bool) {
        // A view, not a change of settings. Packing the page *suppresses* the
        // readings, the hung 句讀 and the ticks; it does not turn them off,
        // because they are choices about the book and this is a choice about
        // the window. So `:view-dense off` needs nothing remembered — what was
        // configured was never touched, and simply applies again.
        self.dense = on;
        // 橫排 has no columns to pack, so `:view-dense` means the other axis there:
        // the row of air above every row.
        if self.layout == Layout::Horizontal {
            self.loose_rows = !on;
        }
        self.status = match (on, self.layout) {
            (true, Layout::Vertical) => say!("layout.dense-on"),
            (false, Layout::Vertical) => say!("layout.dense-off"),
            (true, Layout::Horizontal) => say!("layout.tight"),
            (false, Layout::Horizontal) => say!("layout.loose"),
        };
    }

    /// Whether the page is packed tight.
    pub fn dense(&self) -> bool {
        self.dense
    }

    /// One 句 to a 縱 (`:view-sentence`, Feature #237).
    ///
    /// **A view, and the point is that it is one.** The manual has taught
    /// `:%s/。/。\n/g` for reading a draft back one sentence at a time since the
    /// first version — a substitution that edits the manuscript in order to
    /// read it, and has to be undone before anybody can write again. This is
    /// that, without touching the file: the page breaks a 縱 at the end of every
    /// 句 as well as at the measure, so a long sentence still wraps rather than
    /// running off the foot of the page.
    ///
    /// The boundaries are `motion::sentence_starts`', which is what `(` and `)`
    /// jump between — one answer, so the cursor cannot walk to a place the page
    /// does not break at.
    pub fn set_sentences(&mut self, on: bool) {
        self.sentences = on;
        self.status = match on {
            true => say!("sentence.on"),
            false => say!("sentence.off"),
        };
    }

    /// Whether every 句 opens a 縱 of its own.
    pub fn sentences(&self) -> bool {
        self.sentences
    }

    /// Whether the horizontal page keeps a row of air above every row (疏排).
    pub fn loose_rows(&self) -> bool {
        self.loose_rows
    }

    /// The measure the writer set, if any.
    pub fn measure(&self) -> Option<usize> {
        self.measure
    }

    /// Set the measure — `None` for "as wide as the window".
    ///
    /// Bounded below by the width a row can wrap at, since a measure narrower
    /// than that would fold a single wide character onto its own row forever.
    pub fn set_measure(&mut self, measure: Option<usize>) {
        self.measure = measure.map(|m| m.clamp(crate::wrap::MIN_WRAP_WIDTH, 400));
        if let Some(m) = self.measure {
            if self.layout == Layout::Vertical {
                self.set_zong_length(m);
            }
        }
        self.status = match self.measure {
            Some(m) => say!("layout.measure-columns", m),
            None => say!("layout.measure-window"),
        };
    }

    /// The width horizontal motion should wrap at, or `None` when the buffer is
    /// drawn as unwrapped logical lines.
    pub fn wrap_width(&self) -> Option<usize> {
        if self.soft_wrap && self.layout == Layout::Horizontal {
            self.wrap_width
        } else {
            None
        }
    }

    /// Tell the editor how much fits on screen, for the page motions.
    pub fn set_page(&mut self, lines: usize, columns: usize) {
        self.page_lines = lines.max(1);
        self.page_columns = columns.max(1);
    }

    /// Set how many columns `>` adds and `<` removes.
    pub fn set_indent_width(&mut self, width: usize) {
        self.indent_width = width.max(1);
    }

    /// The count typed so far (`3` of a pending `3w`), for the status line.
    pub fn pending_count(&self) -> Option<usize> {
        self.count
    }

    /// Take one key of a sequence's numeric argument, if that is what it is.
    ///
    /// Returns whether the key was swallowed — in which case the sequence stays
    /// open, waiting for its verb.
    pub(super) fn take_sequence_argument(&mut self, key: Key) -> bool {
        let Key::Char(c) = key else {
            return false;
        };
        if let Some(digit) = c.to_digit(10) {
            let at = self.sequence.get_or_insert_with(Sequence::started).last();
            *at = at.saturating_mul(10).saturating_add(digit as usize).min(1_000_000);
            return true;
        }
        // **`-` is a range and `,` is a list** (§5.7), and a sequence is one or
        // the other. Both only ever follow a number, so neither key is taken
        // away from anything: `t-` and `t,` are still whatever they were.
        let joint = match c {
            '-' => Joint::Span,
            ',' => Joint::List,
            _ => return false,
        };
        if let Some(sequence) = self.sequence.as_mut() {
            // A span has exactly two ends, so a second `-` is not part of it;
            // a list goes on as long as commas do.
            let room = match sequence.joint {
                None => true,
                Some(Joint::List) => joint == Joint::List,
                Some(Joint::Span) => false,
            };
            if room {
                sequence.joint = Some(joint);
                sequence.numbers.push(0);
                return true;
            }
        }
        false
    }

    /// Take one `<column><a|d>` of a sort, if that is what this key is.
    ///
    /// `t1a2d8as` — 「對第一列升序，第二列降序，第八列升序，最後的 s 發出動作指
    /// 令」. Returns whether the key was swallowed, in which case the sequence
    /// stays open for the next column.
    ///
    /// Only after a plain number, so `t d` is still 「delete this row」 and
    /// `t2-5d` — a *span*, which no sort key is — is still whatever it was.
    pub(super) fn take_sort_key(&mut self, key: Key) -> bool {
        let Key::Char(c @ ('a' | 'd')) = key else {
            return false;
        };
        let Some(column) = self.sequence.as_ref().and_then(Sequence::one) else {
            return false;
        };
        self.sort_keys.push((column, c == 'd'));
        self.sequence = None;
        true
    }

    /// The sequence's argument as a span, if it was given one.
    pub(super) fn sequence_span(&self) -> Option<(usize, usize)> {
        self.sequence.as_ref()?.span()
    }

    /// Every column the sequence names, 1-based — `t3`, `t2-10`, `t1,5,9`.
    pub(super) fn sequence_columns(&self) -> Option<Vec<usize>> {
        Some(self.sequence.as_ref()?.columns())
    }

    /// **The command as far as it has been typed** — `3`, `3-5`, `g`, `2t`.
    ///
    /// A modal editor asks you to type a command a key at a time and then says
    /// nothing about what you have typed: press `3` and the editor looks
    /// exactly as it did, so `30d` and `3d` are told apart by memory alone.
    /// vi has answered this since 1976 (`showcmd`, in the bottom right) and so
    /// does Helix; this is that string, and the front end draws it in both
    /// places yumete draws it — the status line's right edge, and beside the
    /// caret, where the eyes already are.
    ///
    /// Empty when nothing is pending, which is most of the time.
    pub fn typed_so_far(&self) -> String {
        let word = match self.pending {
            Pending::None => "",
            Pending::Goto => "g",
            Pending::Space => "␣",
            Pending::Find(FindKind::Forward) => "f",
            Pending::Find(FindKind::Backward) => "F",
            Pending::Replace => "r",
            Pending::Register => "\"",
            Pending::Match => "m",
            Pending::MatchPair { around: false } => "mi",
            Pending::MatchPair { around: true } => "ma",
            Pending::Surround => "ms",
            Pending::SurroundFrom | Pending::SurroundTo(_) => "mr",
            Pending::Table => "t",
            Pending::Hop { forward: true } => "]",
            Pending::Hop { forward: false } => "[",
            Pending::Conflict => "␣c",
            Pending::Case => "`",
            Pending::Mark => "M",
            Pending::Recall => "'",
        };
        // **In the order it was typed.** Inside a sequence the number comes
        // *after* the prefix — `g3` is on its way to `g3d` — and outside one it
        // comes before the key it multiplies, which is `3w`.
        let mut out = String::new();
        if self.pending != Pending::None {
            out.push_str(word);
            // The columns a sort has already been told about, so `t1a2d` reads
            // back as `t1a2d` and not as `t2d` — the whole point of the
            // grammar is that it is typed a column at a time.
            for &(column, down) in &self.sort_keys {
                out.push_str(&column.to_string());
                out.push(match down {
                    true => 'd',
                    false => 'a',
                });
            }
            match self.sequence.as_ref() {
                Some(sequence) => out.push_str(&sequence.spelled()),
                // A count typed the other way round is still part of what was
                // typed: `3gd` says `3g` here, not `g`. **In the order it was
                // typed** — `2-5g`, not `-52g`, which is what two inserts at
                // index 0 produced.
                None => {
                    let mut before = String::new();
                    if let Some(n) = self.operator_count {
                        before.push_str(&n.to_string());
                    }
                    if let Some((_, to)) = self.column_span {
                        before.push('-');
                        before.push_str(&to.to_string());
                    }
                    out.insert_str(0, &before);
                }
            }
            return out;
        }
        if let Some(n) = self.count {
            out.push_str(&n.to_string());
        }
        if let Some(to) = self.count_to {
            out.push('-');
            if let Some(n) = to {
                out.push_str(&n.to_string());
            }
        }
        out
    }
}
