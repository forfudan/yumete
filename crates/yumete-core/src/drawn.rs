//! 虛字 — what is on the page that the file has no bytes for (Feature #248).
//!
//! The mirror of [`crate::editor::Editor::hidden_on_line`]. That one takes
//! characters the file holds **off** the page; this one puts characters the
//! file does not hold **on** it: the inline candidate being typed, the padding
//! that squares a table up, a note about the mark beside it.
//!
//! **The mirror invariant: the cursor may never sit on a character that is not
//! in the file.** Everything here is anchored *between* two of the file's own
//! characters — a run stands before the character it is anchored at — and is
//! never addressable. `l` walks the file's characters; a click on a run means
//! the character it stands before; `w` counts words the file has. This is why
//! virtual text is a property of the *measure* ([`crate::wrap::Measure`],
//! [`crate::zong::Grid`]) rather than of the renderer: a run drawn by the
//! renderer alone would put the caret, `j`, the mouse and the wrap on four
//! different pages.
//!
//! **Two views of the same page.** The renderer wants each run separately, so
//! that a note can be set in a different ink from the candidate beside it
//! ([`Run`]); the measure and the click map want one answer per anchor, which
//! is [`flat`]. They must agree on width, so `flat` concatenates in the same
//! order the renderer draws.

/// How a run is drawn — the renderer's only question about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ink {
    /// What the writer typed and has not committed: the inline candidate.
    ///
    /// First at its anchor, because it continues the word the caret is in —
    /// and the caret stands after it (see
    /// [`crate::editor::Editor::typed_on_line`]).
    Typed,
    /// Worked out from the text and standing in for nothing that was typed:
    /// the padding that squares a table up (#212).
    Padding,
    /// 「後面還有」 — the mark that stands where a cell's tail was folded
    /// away ([`crate::mdtable::FOLD_MARK`]).
    ///
    /// It was a [`Note`](Ink::Note) until 2026-09-07, and a note is set in
    /// the markup's own grey — so the one thing on the page whose whole job
    /// is to say 「this is not what the writer typed」 was drawn in the
    /// colour of what the writer typed. The mark is ASCII `>` for a reason
    /// the width table settles and the ink cannot change, so the ink is
    /// where the difference has to be said (2026-09-07).
    ///
    /// Never at the same anchor as the padding beside it: this one stands at
    /// the first character it hides, and the padding at the end of the cell.
    Fold,
    /// **A footnote's mark, set the way print sets one** (2026-09-22).
    ///
    /// `[^1]` is four characters of source in the middle of a sentence; print
    /// has written it as a small raised number for four hundred years. The
    /// source comes off the page (it is markup, and the caret standing in it
    /// shows it again, same as `**`) and this stands in its place: `⁽¹⁾`.
    ///
    /// ⚠️ **Measured, not guessed** — in 霞鶩文楷等寬 at 15px one cell is
    /// 7.50px and `⁽ ⁾ ⁰¹²³⁴⁵⁶⁷⁸⁹` are 7.50 each, so the mark is exactly three
    /// cells. `⁅ ⁆`（superscript-looking brackets）are 9.03 — **not a whole
    /// number of cells** — and `〔〕［］` are two cells each; either would push
    /// the rest of the line out of the grid, which is the mistake `●` made in
    /// the diagnostics column.
    Footnote,
    /// The editor saying something *about* the text beside it: a mark that
    /// should have been full-width, an ellipsis written with three dots.
    ///
    /// Last at its anchor, so a note is never read as part of the writing.
    Note,
    /// The space a TAB advances over, drawn as a ground rather than a glyph
    /// (#374).
    ///
    /// **No character at all**, because a marker's width is a question the
    /// terminal answers and a tab already has enough of those. What says
    /// 「this is a tab and not spaces」 is the ground under it — quiet, the
    /// rung a table's own bands sit on. Decided 2026-09-10:
    /// 「tab 不用符号，但可以用一个背景色」.
    Tab,
}

/// One run of text on the page that the file does not contain.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Run {
    /// The character of the line this run stands **before**, counting from
    /// zero — the line's own length for a run at the end of it.
    pub column: usize,
    /// What is drawn.
    pub text: String,
    /// How it is drawn.
    pub ink: Ink,
}

impl Run {
    /// A run of `text` before the character at `column`.
    pub fn new(column: usize, text: impl Into<String>, ink: Ink) -> Run {
        Run {
            column,
            text: text.into(),
            ink,
        }
    }
}

/// The page every producer's runs make together, in the order they are drawn.
///
/// Sorted by anchor, and within one anchor by ink: what was typed, then what
/// was derived from it, then what the editor has to say about it. Runs with
/// nothing in them are dropped — a producer that has nothing to say says it by
/// saying nothing, and an empty run at an anchor would still split a span.
pub fn compose(mut runs: Vec<Run>) -> Vec<Run> {
    runs.retain(|run| !run.text.is_empty());
    runs.sort_by(|a, b| a.column.cmp(&b.column).then(a.ink.cmp(&b.ink)));
    runs
}

/// The same page as one answer per anchor — what the measure and the click map
/// ask.
///
/// **One run per anchor.** Two runs standing before the same character are two
/// answers to 「what is drawn here」, and the caret, the click map and the wrap
/// would each pick their own. Concatenated in the drawing order, so the width
/// is the width that was drawn.
pub fn flat(runs: &[Run]) -> Vec<(usize, String)> {
    let mut out: Vec<(usize, String)> = Vec::with_capacity(runs.len());
    for run in runs {
        match out.last_mut() {
            Some((at, text)) if *at == run.column => text.push_str(&run.text),
            _ => out.push((run.column, run.text.clone())),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_note_is_drawn_after_what_was_typed_at_the_same_anchor() {
        let page = compose(vec![
            Run::new(3, "，", Ink::Note),
            Run::new(3, "候", Ink::Typed),
            Run::new(3, "  ", Ink::Padding),
        ]);
        let inks: Vec<Ink> = page.iter().map(|r| r.ink).collect();
        assert_eq!(inks, vec![Ink::Typed, Ink::Padding, Ink::Note]);
    }

    #[test]
    fn the_measure_is_told_one_answer_for_each_anchor() {
        let page = compose(vec![
            Run::new(5, "」", Ink::Note),
            Run::new(0, "候", Ink::Typed),
            Run::new(5, "  ", Ink::Padding),
        ]);
        assert_eq!(
            flat(&page),
            vec![(0, "候".to_string()), (5, "  」".to_string())]
        );
    }

    #[test]
    fn a_producer_with_nothing_to_say_does_not_split_the_row() {
        let page = compose(vec![Run::new(2, "", Ink::Note), Run::new(2, "候", Ink::Typed)]);
        assert_eq!(page.len(), 1);
        assert_eq!(flat(&page), vec![(2, "候".to_string())]);
    }

    #[test]
    fn an_empty_page_is_an_empty_page() {
        assert!(compose(Vec::new()).is_empty());
        assert!(flat(&[]).is_empty());
    }
}
