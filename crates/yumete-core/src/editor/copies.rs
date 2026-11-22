//! **The selection's other copies on the screen** (`[editor]
//! selection_highlight`): every place the visible lines hold the text that is
//! selected, for the front end to lay the search panel's faint ground under.
//!
//! The rules are VS Code's `editor.selectionHighlight`, with the one change a
//! page of 漢字 needs:
//!
//! - **What is selected, not how it came to be.** A drag, `v`, helix's `w`:
//!   any selection wider than the cursor's own character. A bare cursor is
//!   not one, so moving about never lights anything.
//! - **A substring, never a word.** 中国 lights the 中国 of 中国人. Regex
//!   `\b` (what helix's `*` adds) sees no edge between two 漢字, so a selected
//!   word at the start of a line would miss every copy in the middle of a
//!   sentence; and the text is taken literally, `.` as a full stop.
//! - **Nothing for** blanks only, a selection over a line break, or one
//!   longer than [`LONGEST`] characters: none of those is a thing being
//!   looked for, and the last two are where the cost would be.
//! - **The visible lines only.** The front end hands over what it is drawing,
//!   so the work is a page of text however long the file.
//! - **Not where a selection already is**: a copy that overlaps any selection
//!   is left out, so the ground never hides what an edit would take.

use super::*;

/// The longest selection looked for, in characters (VS Code's cap): past it a
/// selection is a passage being moved, not a word being looked for.
pub const LONGEST: usize = 200;

/// **Whether case tells two copies apart** (`[editor]
/// selection_highlight_case`). Latin letters only: 漢字 have no case, and a
/// full-width `Ａ` folds with `ａ` as `A` does with `a`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CopyCase {
    /// `Text` lights `Text` and not `text`.
    #[default]
    Respect,
    /// `Text` lights `text` and `TEXT` too.
    Ignore,
    /// Each copy judged where it sits: respected in code — a code file, a
    /// Markdown fence, a Typst raw block or code body — and ignored in
    /// writing, which is prose and a code file's comments. A code file whose
    /// language has no grammar (or whose colours are off) has no comments to
    /// find, so all of it is code.
    ByPlace,
}

impl CopyCase {
    /// The word the config writes: `respect`, `ignore` or `by_place`.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "respect" => Some(CopyCase::Respect),
            "ignore" => Some(CopyCase::Ignore),
            "by_place" => Some(CopyCase::ByPlace),
            _ => None,
        }
    }
}

/// **How the copies are matched**: the case rule, and whether simplified and
/// traditional forms count as one character (`[editor]
/// selection_highlight_glyphs`, by the search panel's table but not its
/// switch).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CopyRule {
    pub case: CopyCase,
    pub glyphs: bool,
}

/// A Latin letter, the only kind case folding touches: Basic Latin, the
/// Latin-1 and Extended blocks, and the full-width forms a CJK keyboard
/// types.
fn latin_letter(c: char) -> bool {
    c.is_alphabetic()
        && matches!(c as u32, 0x41..=0x5A | 0x61..=0x7A | 0xC0..=0x24F | 0x1E00..=0x1EFF | 0xFF21..=0xFF3A | 0xFF41..=0xFF5A)
}

/// **The text as a pattern that matches it literally**, each character
/// widened to its forms where the rule says so: its simplified and
/// traditional shapes ([`crate::glyphs::shapes`], the panel's asymmetric
/// table: 发 lights 髮, 發 does not), or its two cases.
fn pattern(text: &str, fold_case: bool, glyphs: bool) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        let shapes = match glyphs {
            true => crate::glyphs::shapes(c),
            false => "",
        };
        if !shapes.is_empty() {
            out.push('[');
            out.push_str(&regex::escape(shapes));
            out.push(']');
            continue;
        }
        let mut forms = vec![c];
        if fold_case && latin_letter(c) {
            let others: [Vec<char>; 2] = [c.to_lowercase().collect(), c.to_uppercase().collect()];
            for other in others {
                // `ß` upper-cases to `SS`: two characters are not a form of one.
                if let [one] = other[..] {
                    if !forms.contains(&one) {
                        forms.push(one);
                    }
                }
            }
        }
        match forms[..] {
            [only] => out.push_str(&regex::escape(&only.to_string())),
            _ => {
                out.push('[');
                out.push_str(&regex::escape(&forms.iter().collect::<String>()));
                out.push(']');
            }
        }
    }
    out
}

/// Every match of `finder` in `text`, which starts at char `from`, as char
/// spans. Matches come in order, so the byte-to-char count walks once.
fn spans_in(finder: &regex::Regex, text: &str, from: usize, out: &mut Vec<(usize, usize)>) {
    let (mut byte, mut char_at) = (0usize, from);
    for found in finder.find_iter(text) {
        char_at += text[byte..found.start()].chars().count();
        let end = char_at + found.as_str().chars().count();
        out.push((char_at, end));
        byte = found.end();
        char_at = end;
    }
}

impl Editor {
    /// **The selected text, if it is one to look for**: something is
    /// selected, it is on one line, it is not all blanks, and it is no longer
    /// than [`LONGEST`]. With several selections, the primary one's.
    fn copy_needle(&self) -> Option<String> {
        if !self.has_selection() {
            return None;
        }
        let (from, to) = self.selection();
        let rope = self.current_buffer().rope();
        let to = to.min(rope.len_chars());
        if to <= from || to - from > LONGEST {
            return None;
        }
        let text = rope.slice(from..to).to_string();
        if text.contains(['\n', '\r']) || text.chars().all(char::is_whitespace) {
            return None;
        }
        Some(text)
    }

    /// **Whether the spot `at` is code** for [`CopyCase::ByPlace`]: in a code
    /// file, anywhere but a comment; elsewhere, inside a fence, a raw block
    /// or a Typst code body. Read off what the file says, not what is drawn,
    /// so `:render off` does not turn a fence into prose.
    fn copy_in_code(&self, at: usize, comments: &mut Option<(usize, Vec<(usize, usize)>)>) -> bool {
        let rope = self.current_buffer().rope();
        let line = rope.char_to_line(at.min(rope.len_chars()));
        if !self.writes_code() {
            return matches!(self.scanned_block(line), crate::markdown::Block::Code { .. });
        }
        let column = at - rope.line_to_char(line);
        let spans = match comments {
            Some((held, spans)) if *held == line => spans,
            _ => {
                use crate::markdown::Kind;
                let spans = self
                    .markup_runs(line)
                    .into_iter()
                    .filter(|run| matches!(run.kind, Kind::Comment | Kind::Token(crate::code::Token::Comment)))
                    .map(|run| (run.start, run.end))
                    .collect();
                &mut comments.insert((line, spans)).1
            }
        };
        !spans.iter().any(|&(a, b)| a <= column && column < b)
    }

    /// **Where the selected text is found again in `spans`** (char ranges
    /// the front end is drawing), as char spans in order — none of them
    /// overlapping a selection. Empty when nothing is selected or the
    /// selection is not one to look for (see [`Editor::copy_needle`]).
    ///
    /// Each span is searched on its own, so the caller hands over the rows it
    /// draws, already cut to what fits the window: a line of a megabyte with
    /// wrapping off costs the width of the window, not the line.
    pub fn selection_copies(&self, spans: &[(usize, usize)], rule: CopyRule) -> Vec<(usize, usize)> {
        let Some(needle) = self.copy_needle() else {
            return Vec::new();
        };
        let exact = match rule.case {
            CopyCase::Ignore => None,
            _ => regex::Regex::new(&pattern(&needle, false, rule.glyphs)).ok(),
        };
        let folded = match rule.case {
            CopyCase::Respect => None,
            _ => regex::Regex::new(&pattern(&needle, true, rule.glyphs)).ok(),
        };
        let rope = self.current_buffer().rope();
        let len = rope.len_chars();
        let mut found: Vec<(usize, usize)> = Vec::new();
        let mut folded_found: Vec<(usize, usize)> = Vec::new();
        for &(from, to) in spans {
            let (from, to) = (from.min(len), to.min(len));
            if from >= to {
                continue;
            }
            let text = rope.slice(from..to).to_string();
            if let Some(finder) = &exact {
                spans_in(finder, &text, from, &mut found);
            }
            if let Some(finder) = &folded {
                spans_in(finder, &text, from, &mut folded_found);
            }
        }
        // By place, each copy is the case rule of where it starts: the exact
        // matches stand in code, the folded ones in writing.
        if rule.case == CopyCase::ByPlace {
            let mut comments = None;
            found.retain(|&(at, _)| self.copy_in_code(at, &mut comments));
            folded_found.retain(|&(at, _)| !self.copy_in_code(at, &mut comments));
        }
        found.append(&mut folded_found);
        found.sort_unstable();
        found.dedup();
        let mut selections = self.secondary_selections();
        selections.push(self.selection());
        let mut out: Vec<(usize, usize)> = Vec::with_capacity(found.len());
        for (a, b) in found {
            // A copy that overlaps one already kept: the two rules found the
            // same place at a place where code meets writing.
            if out.last().is_some_and(|&(_, end)| a < end) {
                continue;
            }
            if selections.iter().any(|&(from, to)| a < to && from < b) {
                continue;
            }
            out.push((a, b));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selection::Range;

    /// `text` in a fresh buffer read as `syntax`, with `from..to` selected
    /// (the head on the last character, as helix's selections stand).
    fn selecting(text: &str, syntax: crate::syntax::Syntax, from: usize, to: usize) -> Editor {
        let mut ed = Editor::new();
        ed.current_buffer_mut().replace(0..0, text).unwrap();
        ed.current_buffer_mut().set_syntax_by_hand(syntax);
        ed.sel = crate::selection::Selections::one(Range::new(from, to - 1));
        ed
    }

    /// Plain text with `from..to` selected.
    fn plain(text: &str, from: usize, to: usize) -> Editor {
        selecting(text, crate::syntax::Syntax::Text, from, to)
    }

    /// Where `what` stands in `text` the `nth` time, as a char span.
    fn spot(text: &str, what: &str, nth: usize) -> (usize, usize) {
        let byte = text.match_indices(what).nth(nth).expect("there").0;
        let at = text[..byte].chars().count();
        (at, at + what.chars().count())
    }

    /// Everything, with the rule given.
    fn copies(ed: &Editor, case: CopyCase, glyphs: bool) -> Vec<(usize, usize)> {
        let len = ed.current_buffer().rope().len_chars();
        ed.selection_copies(&[(0, len)], CopyRule { case, glyphs })
    }

    /// **The same text anywhere, as a substring**: 中国 lights the 中国 of
    /// 中国人 and the 中国 between two other 漢字, and `cat` the `cat` of
    /// `concatenate`. The selection itself is not one of them.
    #[test]
    fn a_copy_is_a_substring_in_chinese_and_latin_alike() {
        let text = "我们中国人。\n他也在中国，中国好。\n";
        let (a, b) = spot(text, "中国", 0);
        let ed = plain(text, a, b);
        assert_eq!(copies(&ed, CopyCase::Respect, false), vec![spot(text, "中国", 1), spot(text, "中国", 2)]);

        let text = "cat concatenate cat\n";
        let ed = plain(text, 0, 3);
        assert_eq!(copies(&ed, CopyCase::Respect, false), vec![spot(text, "cat", 1), spot(text, "cat", 2)]);
    }

    /// **The text is taken literally**: a selected `a.b` does not light
    /// `axb`.
    #[test]
    fn the_selection_is_not_a_pattern() {
        let text = "a.b axb a.b (x)+ (x)+\n";
        let ed = plain(text, 0, 3);
        assert_eq!(copies(&ed, CopyCase::Respect, false), vec![spot(text, "a.b", 1)]);
        let (a, b) = spot(text, "(x)+", 0);
        let ed = plain(text, a, b);
        assert_eq!(copies(&ed, CopyCase::Respect, false), vec![spot(text, "(x)+", 1)]);
    }

    /// **Nothing for a cursor, blanks, a line break, or more than 200
    /// characters.**
    #[test]
    fn some_selections_are_not_looked_for() {
        // A bare cursor is not a selection.
        let mut ed = plain("word word\n", 0, 4);
        ed.sel = crate::selection::Selections::at(0);
        assert!(copies(&ed, CopyCase::Respect, false).is_empty(), "a cursor");
        // Blanks only.
        let ed = plain("a  b  c\n", 1, 3);
        assert!(copies(&ed, CopyCase::Respect, false).is_empty(), "blanks");
        // Over a line break, or ending on one.
        let ed = plain("foo\nfoo\nfoo\n", 0, 5);
        assert!(copies(&ed, CopyCase::Respect, false).is_empty(), "two lines");
        let ed = plain("foo\nfoo\nfoo\n", 0, 4);
        assert!(copies(&ed, CopyCase::Respect, false).is_empty(), "a line and its break");
        // 200 characters are looked for; 201 are not.
        let text = format!("{} {}\n", "字".repeat(201), "字".repeat(201));
        let ed = plain(&text, 0, 200);
        assert_eq!(copies(&ed, CopyCase::Respect, false), vec![(202, 402)], "200");
        let ed = plain(&text, 0, 201);
        assert!(copies(&ed, CopyCase::Respect, false).is_empty(), "201");
    }

    /// **Case**: respected, ignored, or by place — and only Latin letters
    /// fold, the full-width ones with them.
    #[test]
    fn case_is_respected_or_ignored() {
        let text = "Rain rain RAIN Ｒａｉｎ ｒａｉｎ Rain\n";
        let ed = plain(text, 0, 4);
        assert_eq!(copies(&ed, CopyCase::Respect, false), vec![spot(text, "Rain", 1)]);
        assert_eq!(
            copies(&ed, CopyCase::Ignore, false),
            vec![spot(text, "rain", 0), spot(text, "RAIN", 0), spot(text, "Rain", 1)],
        );
        let (a, b) = spot(text, "Ｒａｉｎ", 0);
        let ed = plain(text, a, b);
        assert_eq!(copies(&ed, CopyCase::Ignore, false), vec![spot(text, "ｒａｉｎ", 0)], "full width");
    }

    /// **By place in Markdown**: case is told apart inside a fence and not
    /// in the prose around it.
    #[test]
    fn by_place_a_fence_is_code_and_prose_is_not() {
        let text = "Name and name in prose.\n\n```rust\nlet name = Name::new();\n```\n\nname again.\n";
        let ed = selecting(text, crate::syntax::Syntax::Markdown, 0, 4);
        assert_eq!(
            copies(&ed, CopyCase::ByPlace, false),
            vec![spot(text, "name", 0), spot(text, "Name", 1), spot(text, "name", 2)],
            "the fence's `name` is another word; the prose's are the same one",
        );
        assert_eq!(copies(&ed, CopyCase::Respect, false), vec![spot(text, "Name", 1)]);
        assert_eq!(copies(&ed, CopyCase::Ignore, false).len(), 4);

        // A Typst raw block is code as a fence is.
        let text = "Name and name.\n\n```\nlet name = Name;\n```\n";
        let ed = selecting(text, crate::syntax::Syntax::Typst, 0, 4);
        assert_eq!(copies(&ed, CopyCase::ByPlace, false), vec![spot(text, "name", 0), spot(text, "Name", 1)]);
    }

    /// **By place in a code file**: case is told apart in the code and not in
    /// its comments.
    #[test]
    fn by_place_a_comment_is_writing() {
        let text = "// Total counts the total.\nlet total = Total::new(); // total again\n";
        let rust = crate::syntax::Syntax::Code(crate::code::Language::Rust);
        let (a, b) = spot(text, "Total", 0);
        let ed = selecting(text, rust, a, b);
        assert_eq!(
            copies(&ed, CopyCase::ByPlace, false),
            vec![spot(text, "total", 0), spot(text, "Total", 1), spot(text, "total", 2)],
            "the comments' `total` lit, the variable not",
        );
        assert_eq!(copies(&ed, CopyCase::Respect, false), vec![spot(text, "Total", 1)]);
    }

    /// **Simplified and traditional as one character**, only when asked.
    #[test]
    fn glyphs_fold_only_when_asked() {
        let text = "書店旁有书店，也有書店。\n";
        let ed = plain(text, 0, 2);
        assert_eq!(copies(&ed, CopyCase::Respect, false), vec![spot(text, "書店", 1)]);
        assert_eq!(copies(&ed, CopyCase::Respect, true), vec![spot(text, "书店", 0), spot(text, "書店", 1)]);
    }

    /// **Only the spans handed over are searched**: what is not drawn is not
    /// looked at.
    #[test]
    fn only_the_drawn_spans_are_searched() {
        let text = "alpha one\nalpha two\nalpha three\n";
        let ed = plain(text, 0, 5);
        let rule = CopyRule::default();
        let third = spot(text, "alpha three", 0);
        assert_eq!(ed.selection_copies(&[(third.0, third.1)], rule), vec![spot(text, "alpha", 2)]);
        assert_eq!(ed.selection_copies(&[(0, 10)], rule), Vec::new(), "the selection's own line");
        assert_eq!(ed.selection_copies(&[(0, text.len())], rule).len(), 2);
    }

    /// **Never where a selection is**: a copy another cursor stands on is
    /// left to that selection's own ground.
    #[test]
    fn a_copy_under_a_selection_is_left_out() {
        let text = "ab ab ab\n";
        let mut ed = plain(text, 0, 2);
        ed.sel.rebuild(vec![Range::new(0, 1), Range::new(3, 4)], 0);
        assert_eq!(copies(&ed, CopyCase::Respect, false), vec![(6, 8)]);
    }

    #[test]
    fn the_case_words_parse() {
        for (word, case) in [("respect", CopyCase::Respect), ("ignore", CopyCase::Ignore), ("by_place", CopyCase::ByPlace)] {
            assert_eq!(CopyCase::parse(word), Some(case));
        }
        assert_eq!(CopyCase::parse("smart"), None);
    }
}
