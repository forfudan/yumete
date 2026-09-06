//! Merge conflicts as something the page can see (Feature #249).
//!
//! What `git merge` leaves in a file is not prose, and it is not markup — it
//! is a **block**, in exactly the sense the [`crate::markdown::BlockScanner`]
//! already means: seven `<` opened it three paragraphs ago and every line
//! until the seven `>` belongs to it. So it is read the same way, from the top
//! and by each line's opening, and the page can then say which side of the
//! quarrel a line is on before the reader has counted a single angle bracket.
//!
//! The shapes, which are git's and are not negotiable:
//!
//! ```text
//! <<<<<<< HEAD          ← the head marker, and whose side it opens
//! 我方寫的
//! ||||||| merged common ancestors   ← only in `diff3` style
//! 兩邊都改之前的樣子
//! =======               ← the split
//! 他方寫的
//! >>>>>>> feature/枝     ← the foot, and whose side that was
//! ```
//!
//! Nothing here parses a *diff*: a conflict is text a tool wrote into the
//! file, and the file is the only thing that knows about it. That is what
//! makes this cheap enough to run down a document on every edit.

use crate::Rope;

/// How long a marker is. Git writes seven, always.
const RUN: usize = 7;

/// Which line of a conflict this is — or which side of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    /// `<<<<<<< 我方`
    Head,
    /// `||||||| 共同祖先` — only `diff3` and `zdiff3` write this one.
    Base,
    /// `=======`
    Split,
    /// `>>>>>>> 他方`
    Foot,
}

impl Marker {
    /// The character the marker is a run of.
    fn glyph(self) -> char {
        match self {
            Marker::Head => '<',
            Marker::Base => '|',
            Marker::Split => '=',
            Marker::Foot => '>',
        }
    }
}

/// Which side of a conflict a line belongs to.
///
/// The markers themselves are on no side — they are the furniture, and they
/// are what [`Conflict::kept`] throws away, whichever side wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Between the head and the base-or-split: what is already here.
    Ours,
    /// Between `|||||||` and `=======`: what both sides started from.
    Base,
    /// Between the split and the foot: what is arriving.
    Theirs,
}

/// Which side of a conflict to keep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
    Ours,
    Theirs,
    /// Both, in the order they are written — ours first.
    Both,
    /// What the two of them started from, which is a way of saying 「都不要」.
    Base,
}

/// Whether a line's opening is one of the four markers, and which.
///
/// A marker is a run of exactly seven of its character followed by a space or
/// by the end of the line. **Exactly** seven, because a Markdown quote of a
/// conflict — or a row of `=======` under a heading, which is Setext and is
/// common — is not one, and eight `=` is a rule.
pub fn marker(line: &str) -> Option<Marker> {
    let text = line.trim_end_matches(['\n', '\r']);
    for kind in [Marker::Head, Marker::Base, Marker::Split, Marker::Foot] {
        let glyph = kind.glyph();
        let run = text.chars().take_while(|&c| c == glyph).count();
        if run == RUN && matches!(text.chars().nth(RUN), None | Some(' ')) {
            return Some(kind);
        }
    }
    None
}

/// The label after a marker — the branch, the commit, whoever it was.
pub fn label(line: &str) -> String {
    line.trim_end_matches(['\n', '\r'])
        .chars()
        .skip(RUN)
        .collect::<String>()
        .trim()
        .to_string()
}

/// One conflict, in line numbers.
///
/// Every field is a line of the buffer, and the ranges are half-open in the
/// usual way: `ours` is the writing between the head marker and whichever
/// marker ends it, markers excluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conflict {
    /// The `<<<<<<<` line.
    pub head: usize,
    /// The `|||||||` line, when the file was written in `diff3` style.
    pub base_mark: Option<usize>,
    /// The `=======` line.
    pub split: usize,
    /// The `>>>>>>>` line.
    pub foot: usize,
    /// Whose side the head named — `HEAD`, usually.
    pub ours: String,
    /// Whose side the foot named — the branch being merged in.
    pub theirs: String,
}

impl Conflict {
    /// The lines of our side, markers excluded.
    pub fn ours_lines(&self) -> std::ops::Range<usize> {
        self.head + 1..self.base_mark.unwrap_or(self.split)
    }

    /// The lines of the common ancestor, when the file has one.
    pub fn base_lines(&self) -> Option<std::ops::Range<usize>> {
        self.base_mark.map(|at| at + 1..self.split)
    }

    /// The lines of their side, markers excluded.
    pub fn theirs_lines(&self) -> std::ops::Range<usize> {
        self.split + 1..self.foot
    }

    /// Every line the conflict occupies, both markers included.
    pub fn lines(&self) -> std::ops::Range<usize> {
        self.head..self.foot + 1
    }

    /// Which side `line` is on, or [`None`] when it is a marker or is outside.
    pub fn side_of(&self, line: usize) -> Option<Side> {
        if self.ours_lines().contains(&line) {
            return Some(Side::Ours);
        }
        if self.base_lines().is_some_and(|r| r.contains(&line)) {
            return Some(Side::Base);
        }
        if self.theirs_lines().contains(&line) {
            return Some(Side::Theirs);
        }
        None
    }

    /// Which lines survive `keep`, in the order they are written.
    ///
    /// The markers are not among them — resolving a conflict is exactly the
    /// act of taking the furniture away, and a 「resolution」 that left the
    /// angle brackets behind would be a conflict git still refuses to commit.
    pub fn kept(&self, keep: Keep) -> Vec<std::ops::Range<usize>> {
        match keep {
            Keep::Ours => vec![self.ours_lines()],
            Keep::Theirs => vec![self.theirs_lines()],
            Keep::Both => vec![self.ours_lines(), self.theirs_lines()],
            // With no `|||||||` in the file there is nothing to go back to,
            // and 「都不要」 is the honest answer.
            Keep::Base => self.base_lines().into_iter().collect(),
        }
    }
}

/// Every conflict in the document, in the order they are written.
///
/// Only each line's opening is read — eight characters settle it — so this
/// walks a 600 KB manuscript without materialising a single paragraph. What
/// the markers add up to is [`assemble`]'s answer.
pub fn scan(rope: &Rope) -> Vec<Conflict> {
    let lines = rope.len_lines();
    let mut marks = Vec::new();
    for line in 0..lines {
        let start = rope.line_to_char(line);
        let end = match line + 1 < lines {
            true => rope.line_to_char(line + 1),
            false => rope.len_chars(),
        };
        let head: String = rope.chars_at(start).take((end - start).min(PREFIX)).collect();
        if let Some(kind) = marker(&head) {
            marks.push((line, kind, label(&head)));
        }
    }
    assemble(marks)
}

/// How much of a line says whether it is a marker, and whose.
///
/// Seven brackets, a space, and as much of a branch name as anyone wants to
/// read in a status line.
pub const PREFIX: usize = 64;

/// The conflicts a document's marker lines add up to.
///
/// Taken apart from [`scan`] so the one walk down the document that the block
/// scan already makes can hand its markers straight here, rather than the page
/// reading every line twice to answer two questions about the same characters.
///
/// **A conflict that never closes is not one.** A head with no foot below it
/// is dropped rather than run to the end of the file: `<<<<<<<` appears in
/// prose about merges — this module's own documentation opens with one — and
/// swallowing the rest of a document because of a line in a code fence is
/// worse than seeing nothing.
pub fn assemble(marks: impl IntoIterator<Item = (usize, Marker, String)>) -> Vec<Conflict> {
    let mut out = Vec::new();
    let mut open: Option<Conflict> = None;
    for (line, kind, label) in marks {
        match (kind, open.as_mut()) {
            // A second `<<<<<<<` before the first has closed is not a nested
            // conflict — there is no such thing — it is the real start, and
            // what came before it was prose that looked like a marker.
            (Marker::Head, _) => {
                open = Some(Conflict {
                    head: line,
                    base_mark: None,
                    split: line,
                    foot: line,
                    ours: label,
                    theirs: String::new(),
                });
            }
            (Marker::Base, Some(c)) if c.base_mark.is_none() && c.split == c.head => {
                c.base_mark = Some(line);
            }
            (Marker::Split, Some(c)) if c.split == c.head => c.split = line,
            (Marker::Foot, Some(c)) if c.split > c.head => {
                c.foot = line;
                c.theirs = label;
                out.push(open.take().expect("just borrowed"));
            }
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rope(text: &str) -> Rope {
        Rope::from_str(text)
    }

    const PLAIN: &str = "\
上文
<<<<<<< HEAD
我方寫的
=======
他方寫的
>>>>>>> feature/枝
下文
";

    const DIFF3: &str = "\
<<<<<<< HEAD
我方寫的
||||||| merged common ancestors
原本的樣子
=======
他方寫的
>>>>>>> theirs
";

    #[test]
    fn a_marker_is_exactly_seven_of_its_character() {
        assert_eq!(marker("<<<<<<< HEAD"), Some(Marker::Head));
        assert_eq!(marker("======="), Some(Marker::Split));
        assert_eq!(marker(">>>>>>> topic\n"), Some(Marker::Foot));
        assert_eq!(marker("||||||| base"), Some(Marker::Base));
        // Six is not a marker, and neither is eight: a Setext heading is
        // underlined with a row of `=` as long as the heading, and a rule is
        // as long as the writer felt like.
        assert_eq!(marker("======"), None);
        assert_eq!(marker("========"), None);
        // Nor is a marker with something jammed against it.
        assert_eq!(marker("<<<<<<<HEAD"), None);
        assert_eq!(marker("  ======="), None);
    }

    #[test]
    fn a_conflict_is_read_from_its_two_markers() {
        let found = scan(&rope(PLAIN));
        assert_eq!(found.len(), 1);
        let c = &found[0];
        assert_eq!((c.head, c.split, c.foot), (1, 3, 5));
        assert_eq!(c.base_mark, None);
        assert_eq!(c.ours, "HEAD");
        assert_eq!(c.theirs, "feature/枝");
        assert_eq!(c.ours_lines(), 2..3);
        assert_eq!(c.theirs_lines(), 4..5);
        assert_eq!(c.lines(), 1..6);
    }

    #[test]
    fn the_common_ancestor_is_a_third_side() {
        let found = scan(&rope(DIFF3));
        assert_eq!(found.len(), 1);
        let c = &found[0];
        assert_eq!(c.base_mark, Some(2));
        assert_eq!(c.ours_lines(), 1..2);
        assert_eq!(c.base_lines(), Some(3..4));
        assert_eq!(c.theirs_lines(), 5..6);
        assert_eq!(c.side_of(1), Some(Side::Ours));
        assert_eq!(c.side_of(3), Some(Side::Base));
        assert_eq!(c.side_of(5), Some(Side::Theirs));
        // The markers are on nobody's side.
        assert_eq!(c.side_of(0), None);
        assert_eq!(c.side_of(4), None);
    }

    #[test]
    fn a_conflict_that_never_closes_is_not_one() {
        // Prose about merges — a paragraph of this manual is exactly this.
        let text = "說明：衝突長這樣\n<<<<<<< HEAD\n然後就沒有然後了\n";
        assert!(scan(&rope(text)).is_empty());
        // A head and a split with no foot is still not one.
        let text = "<<<<<<< HEAD\n甲\n=======\n乙\n";
        assert!(scan(&rope(text)).is_empty());
    }

    #[test]
    fn two_conflicts_in_one_file_are_two() {
        let text = format!("{PLAIN}{PLAIN}");
        let found = scan(&rope(&text));
        assert_eq!(found.len(), 2);
        assert_eq!(found[1].head, 8);
    }

    #[test]
    fn keeping_a_side_keeps_no_markers() {
        let c = &scan(&rope(DIFF3))[0];
        assert_eq!(c.kept(Keep::Ours), vec![1..2]);
        assert_eq!(c.kept(Keep::Theirs), vec![5..6]);
        assert_eq!(c.kept(Keep::Both), vec![1..2, 5..6]);
        assert_eq!(c.kept(Keep::Base), vec![3..4]);
        // With no ancestor written down, 「回到原樣」 is 「兩邊都不要」.
        let plain = &scan(&rope(PLAIN))[0];
        assert!(plain.kept(Keep::Base).is_empty());
    }
}
