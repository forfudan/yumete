//! **A heading's section, moved past its siblings**: org-mode's
//! `M-up`/`M-down` (`org-move-subtree-up`/`-down`) on the flavour's
//! `A-k`/`A-j`.
//!
//! A section is the heading line and everything under it, its own
//! sub-sections included, up to the next heading at its level or above. Its
//! siblings are the sections of the same level under the same parent: a
//! section never leaves its parent, and the text above the first heading is
//! no one's section, so it never moves. Promoting and demoting are not here.
//!
//! The move is one replace over the section and the siblings it passes, so
//! `u` takes it back in one step; folds, the cursor and the outline's folded
//! rows are carried through it by where each character went, since a replace
//! alone would squeeze every one of them to its start.

use super::*;

/// What one move rearranges: the text `a..c` read as two blocks, `a..b` and
/// `b..c`, written back as the second then the first.
#[derive(Debug, Clone, Copy)]
struct Swap {
    a: usize,
    b: usize,
    c: usize,
    /// The second block is the file's last line and has no line break: it
    /// takes the first block's, so the two do not run together and the file
    /// still ends without one.
    lends: usize,
}

impl Swap {
    /// Where a character of the text before the move is after it.
    fn to(self, at: usize) -> usize {
        let Swap { a, b, c, lends } = self;
        match at {
            _ if at < a => at,
            _ if at < b => a + (c - b) + lends + (at - a),
            _ if at < c => a + (at - b),
            // The end of the text is the end of the last block, which moved.
            _ if at == c && lends > 0 => a + (c - b),
            _ => at,
        }
    }
}

impl Editor {
    /// The headings a section can be moved by, as `(line, depth)`: the
    /// outline's, without a Typst file's `#include` rows (those are lines of
    /// the section they stand in, not sections of their own). None in code.
    fn section_heads(&self) -> Vec<(usize, usize)> {
        if !self.prose_here() {
            return Vec::new();
        }
        let rope = self.current_buffer().rope();
        self.outline()
            .into_iter()
            .filter(|&(line, ..)| !rope.line(line).to_string().trim_start().starts_with("#include"))
            .map(|(line, depth, _)| (line, depth))
            .collect()
    }

    /// Whether `A-j`/`A-k` mean the section here: one bare cursor, no select
    /// mode, standing on a heading's line.
    pub(super) fn bare_on_a_heading(&self) -> bool {
        if self.extend || self.vim_lines || self.sel.is_plural() {
            return false;
        }
        let rope = self.current_buffer().rope();
        let (start, end) = self.selection();
        if motion::prev_grapheme(rope, end) > start {
            return false;
        }
        let line = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        self.section_heads().iter().any(|&(at, _)| at == line)
    }

    /// **The section headed on `line`, moved past `count` sibling sections**
    /// down or up. Answers the line its heading is on after the move, or
    /// `None` when nothing moved (and the status line says why).
    ///
    /// A count larger than the siblings there are moves as far as they go.
    pub(super) fn move_section(&mut self, line: usize, count: usize, down: bool) -> Option<usize> {
        let heads = self.section_heads();
        let Some(i) = heads.iter().position(|&(at, _)| at == line) else {
            self.status = say!("edit.section-not-a-heading");
            return None;
        };
        let depth = heads[i].1;
        // A section's end: the next heading at its depth or above, as a
        // heading index, or the end of the text.
        let end_of = |k: usize| heads[k + 1..].iter().position(|&(_, d)| d <= depth).map(|j| k + 1 + j);
        // The neighbours: the heading that ends a section is the next sibling
        // when it is at the same depth, and the parent's next when it is
        // shallower; the nearest heading above at this depth or shallower is
        // the previous sibling or the parent.
        let mut far = i;
        let mut moved = 0;
        while moved < count.max(1) {
            let next = match down {
                true => end_of(far),
                false => heads[..far].iter().rposition(|&(_, d)| d <= depth),
            };
            match next {
                Some(k) if heads[k].1 == depth => {
                    far = k;
                    moved += 1;
                }
                _ => break,
            }
        }
        if moved == 0 {
            self.status = match down {
                true => say!("edit.section-last"),
                false => say!("edit.section-first"),
            };
            return None;
        }
        let rope = self.current_buffer().rope();
        let start = |k: usize| rope.line_to_char(heads[k].0);
        let end = |k: usize| end_of(k).map_or(rope.len_chars(), start);
        let (a, b, c) = match down {
            true => (start(i), end(i), end(far)),
            false => (start(far), start(i), end(i)),
        };
        let first = rope.slice(a..b).to_string();
        let mut second = rope.slice(b..c).to_string();
        let mut first = first.as_str();
        let mut lends = 0;
        if !second.ends_with('\n') {
            let ending = if first.ends_with("\r\n") { "\r\n" } else { "\n" };
            first = &first[..first.len() - ending.len()];
            second.push_str(ending);
            lends = ending.chars().count();
        }
        let swap = Swap { a, b, c, lends };
        let text = format!("{second}{first}");
        let was = rope.clone();
        let folds = self.current_buffer().fold_map();
        self.snapshot();
        if !self.overwrite(a, c, &text) {
            return None;
        }
        self.current_buffer_mut().put_folds_through(folds, &|at| swap.to(at));
        let now = self.current_buffer().rope().clone();
        let line_to = |l: usize| now.char_to_line(swap.to(was.line_to_char(l)).min(now.len_chars()));
        if let Some(side) = self.showing(crate::sidebar::View::Outline) {
            if let Some(panel) = self.panel_mut(side) {
                panel.remap_folded_lines(&line_to);
            }
        }
        self.sel.map(|r| crate::selection::Range::new(swap.to(r.anchor), swap.to(r.head)));
        self.fold_generation += 1;
        self.refresh_goal_column();
        Some(line_to(line))
    }

    /// `A-j`/`A-k` in the outline sidebar: the highlighted heading's
    /// section moved, and the highlight kept on it. A row from an included
    /// chapter is another file's, and is left alone.
    pub(super) fn move_section_from_outline(&mut self, side: crate::sidebar::Side, down: bool) {
        let Some(row) = self.panel(side).and_then(|p| p.rows().get(p.selected()).cloned()) else {
            return;
        };
        if !row.path.as_os_str().is_empty() {
            self.status = say!("edit.section-included");
            return;
        }
        let Some(line) = self.move_section(row.depth, 1, down) else {
            return;
        };
        self.refresh_sidebar();
        if let Some(panel) = self.panel_mut(side) {
            if let Some(i) = panel.rows().iter().position(|r| r.path.as_os_str().is_empty() && r.depth == line) {
                panel.select(i);
            }
        }
    }
}

/// The sort key of one heading: a class first (titles with no number go
/// after the numbered ones under `-k number`), then the number, the
/// syllables, and the title itself, which settles same-sounding titles by
/// code point.
type SectionKey = (u8, u64, Vec<(String, u8)>, String);

impl Editor {
    /// **The headings the markup marks**, as `(line, level, title)`: read by
    /// what the file says even under `:render off`, never guessed from
    /// unmarked prose, and without Typst's `#include` lines.
    fn marked_heads(&self) -> Vec<(usize, usize, String)> {
        let buffer = self.current_buffer();
        let rope = buffer.rope();
        super::render::marked_outline(rope, buffer.syntax(), &|line| self.scanned_block(line))
            .into_iter()
            .filter(|&(line, ..)| !rope.line(line).to_string().trim_start().starts_with("#include"))
            .collect()
    }

    /// The lines of the text without their breaks, the empty line after a
    /// last line break not counted.
    fn plain_lines(&self) -> Vec<String> {
        let rope = self.current_buffer().rope();
        let mut lines: Vec<String> =
            rope.lines().map(|l| l.to_string().trim_end_matches(['\n', '\r']).to_string()).collect();
        if lines.len() > 1 && lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines
    }

    /// **A title read aloud**, syllable by syllable, toned: each word read as
    /// `:ruby-auto` reads it (為了 is wèi le), and without the Yume tables the
    /// built-in search table's first reading, toneless. Anything not 漢字 is
    /// itself, case folded.
    fn spoken(&self, text: &str) -> Vec<(String, u8)> {
        let chars: Vec<char> = text.chars().collect();
        let mut read: Vec<Option<(String, u8)>> = vec![None; chars.len()];
        if self.reader.available() {
            for (ws, we) in self.segmenter.segment(text) {
                if we > chars.len() || !chars[ws..we].iter().any(|&c| yumete_cjk::word::is_han(c)) {
                    continue;
                }
                let word: String = chars[ws..we].iter().collect();
                if let Some(readings) = self.reader.read(&word).filter(|r| r.len() == we - ws) {
                    for (k, reading) in readings.iter().enumerate() {
                        if yumete_cjk::word::is_han(chars[ws + k]) {
                            read[ws + k] = Some(crate::headings::toned(reading));
                        }
                    }
                }
            }
        }
        chars
            .iter()
            .zip(read)
            .filter(|(c, _)| !c.is_whitespace())
            .map(|(&c, read)| {
                read.unwrap_or_else(|| match yumete_cjk::word::is_han(c) {
                    true => (crate::pinyin::readings(c).next().map_or_else(|| c.to_string(), str::to_string), 0),
                    false => (c.to_lowercase().collect(), 0),
                })
            })
            .collect()
    }

    /// `:sort-sections` — **sibling sections ordered by their headings**,
    /// each carrying its body and every section under it (#1068). The
    /// whole file, the range written in front or the selected lines, or with
    /// `--under` the sections under the one the cursor is in (its parent's,
    /// when the cursor is in a section with none). One undo step;
    /// the cursor, folds and the outline's folded rows go with their lines.
    ///
    /// Two checks guard the text: every line is still there as often as
    /// before, and every heading still has the same chain of parents. A sort
    /// that fails either is taken back and said.
    pub(super) fn sort_sections(&mut self, order: crate::command::SectionOrder, rows: Option<crate::command::Rows>) {
        use crate::command::SectionSort;
        if self.refuse_readonly() {
            return;
        }
        let syntax = self.current_buffer().syntax();
        let typst = syntax == crate::syntax::Syntax::Typst;
        if !matches!(syntax, crate::syntax::Syntax::Markdown | crate::syntax::Syntax::Typst) {
            self.status = say!("sections.sort-markup-only");
            return;
        }
        let lines = self.plain_lines();
        let all = self.marked_heads();
        let end_of = |i: usize| {
            all[i + 1..].iter().find(|(_, level, _)| *level <= all[i].1).map_or(lines.len(), |(line, ..)| *line)
        };
        let rope = self.current_buffer().rope().clone();
        let cursor = rope.char_to_line(self.sel.head().min(rope.len_chars()));
        let (from, to) = if order.under {
            // The innermost section around the cursor that has sections of
            // its own: in a last-level section, its parent's.
            let has_children = |i: usize| all.get(i + 1).is_some_and(|(line, ..)| *line < end_of(i));
            match (0..all.len()).rev().find(|&i| all[i].0 <= cursor && cursor < end_of(i) && has_children(i)) {
                Some(i) => (all[i].0 + 1, end_of(i)),
                None => {
                    self.status = say!("sections.sort-not-in-a-section");
                    return;
                }
            }
        } else {
            match rows {
                Some(rows) => match self.substitution_rows(rows) {
                    Ok(super::Chosen::Span(a, b)) => (a, b + 1),
                    Ok(super::Chosen::These(these)) => {
                        (these.iter().copied().min().unwrap_or(0), these.iter().copied().max().map_or(0, |m| m + 1))
                    }
                    Err(why) => {
                        self.status = why;
                        return;
                    }
                },
                None if self.has_selection() => {
                    let (start, end) = self.selection();
                    (rope.char_to_line(start), rope.char_to_line(end.saturating_sub(1).max(start)) + 1)
                }
                None => (0, lines.len()),
            }
        };
        let to = to.min(lines.len());
        let heads: Vec<crate::headings::Head> = all
            .iter()
            .filter(|(line, ..)| (from..to).contains(line))
            .map(|&(line, level, _)| crate::headings::Head { line, level })
            .collect();
        if heads.is_empty() {
            self.status = say!("sections.sort-no-headings");
            return;
        }
        let keys: Vec<SectionKey> = all
            .iter()
            .filter(|(line, ..)| (from..to).contains(line))
            .map(|(_, _, title)| {
                let text = crate::headings::visible_title(title, typst);
                match order.key {
                    SectionSort::Unicode => (0, 0, Vec::new(), text),
                    SectionSort::Number => match crate::numerals::first_number(&text) {
                        Some(n) => (0, n, Vec::new(), text),
                        None => (1, 0, Vec::new(), text),
                    },
                    SectionSort::Pinyin => (0, 0, self.spoken(&text), text),
                }
            })
            .collect();
        let arranged = crate::headings::arrange(from, to, &heads, &keys, order.levels, order.reverse);
        if arranged.moved == 0 {
            self.status = say!("sections.sort-already", arranged.sorted);
            return;
        }
        let mut before: Vec<&String> = lines[from..to].iter().collect();
        let mut after: Vec<&String> = arranged.order.iter().map(|&l| &lines[l]).collect();
        before.sort();
        after.sort();
        if before != after {
            self.status = say!("sections.sort-check-failed");
            return;
        }
        let chains = |heads: &[(usize, usize, String)]| {
            crate::headings::chains(
                &heads.iter().map(|(_, level, title)| (*level, crate::headings::visible_title(title, typst))).collect::<Vec<_>>(),
            )
        };
        let chains_before = chains(&all);
        // The text replaced: lines `from..to`, with the break after the last
        // one when it has one.
        let start = rope.line_to_char(from);
        let end = match to < rope.len_lines() {
            true => rope.line_to_char(to),
            false => rope.len_chars(),
        };
        let was: String = rope.slice(start..end).chars().collect();
        let ending = self.current_buffer().ending().to_string();
        let mut text = arranged.order.iter().map(|&l| lines[l].as_str()).collect::<Vec<_>>().join(&ending);
        if was.ends_with('\n') {
            text.push_str(&ending);
        }
        // Where each old line lands: its new index, and the character its
        // new line starts at.
        let mut lands = vec![0usize; to - from];
        let mut starts = Vec::with_capacity(to - from);
        let mut at = start;
        for (new, &old) in arranged.order.iter().enumerate() {
            lands[old - from] = new;
            starts.push(at);
            at += lines[old].chars().count() + ending.chars().count();
        }
        let shift = text.chars().count() as isize - was.chars().count() as isize;
        let move_to = |c: usize| -> usize {
            if c < start {
                return c;
            }
            if c >= end {
                return (c as isize + shift).max(0) as usize;
            }
            let line = rope.char_to_line(c);
            let column = (c - rope.line_to_char(line)).min(lines[line].chars().count());
            starts[lands[line - from]] + column
        };
        let folds = self.current_buffer().fold_map();
        self.snapshot();
        if !self.overwrite(start, end, &text) {
            return;
        }
        if chains(&self.marked_heads()) != chains_before {
            self.undo();
            self.status = say!("sections.sort-check-failed");
            return;
        }
        self.current_buffer_mut().put_folds_through(folds, &move_to);
        let line_to = |l: usize| match (from..to).contains(&l) {
            true => from + lands[l - from],
            false => l,
        };
        if let Some(side) = self.showing(crate::sidebar::View::Outline) {
            if let Some(panel) = self.panel_mut(side) {
                panel.remap_folded_lines(&line_to);
            }
        }
        self.sel.map(|r| crate::selection::Range::new(move_to(r.anchor), move_to(r.head)));
        self.fold_generation += 1;
        self.refresh_goal_column();
        self.status = say!("sections.sorted", arranged.sorted, arranged.moved);
    }

    /// `:heading-numbering` and `:heading-numbering-off` — **a Markdown
    /// file's headings numbered by the patterns it names** (#1069), or the
    /// numbers taken off. Typst numbers its headings itself, with
    /// `#set heading(numbering: …)`, so a Typst file is told so and left.
    pub(super) fn heading_numbering(&mut self, on: bool) {
        if self.refuse_readonly() {
            return;
        }
        match self.current_buffer().syntax() {
            crate::syntax::Syntax::Markdown => {}
            crate::syntax::Syntax::Typst => {
                self.status = say!("sections.numbering-typst");
                return;
            }
            _ => {
                self.status = say!("sections.numbering-markdown-only");
                return;
            }
        }
        let lines = self.plain_lines();
        let heads: Vec<crate::headings::Numbered> = self
            .marked_heads()
            .into_iter()
            .map(|(line, level, _)| crate::headings::Numbered {
                line,
                level,
                omitted: crate::headings::omits(&lines[line])
                    || line.checked_sub(1).is_some_and(|above| {
                        let above = lines[above].trim();
                        above == "<!-- omit from toc -->" || above == "<!-- omit in toc -->"
                    }),
            })
            .collect();
        let Some(first) = heads.first().map(|h| h.line) else {
            self.status = say!("sections.numbering-no-headings");
            return;
        };
        let numbering = match crate::headings::numbering_of(&lines, first) {
            Ok(Some(numbering)) => numbering,
            Ok(None) => {
                crate::headings::Numbering { levels: crate::headings::Numbering::default_levels(), traditional: false }
            }
            Err(crate::headings::SpecError::NotAList) => {
                self.status = say!("sections.numbering-not-a-list");
                return;
            }
        };
        let done = crate::headings::renumber(&lines, &heads, &numbering, on);
        let doubled = done.doubled.iter().map(|l| (l + 1).to_string()).collect::<Vec<_>>().join("、");
        let (Some(&(first, _)), Some(&(last, _))) = (done.lines.first(), done.lines.last()) else {
            self.status = match (on, doubled.is_empty()) {
                (true, true) => say!("sections.numbering-current"),
                (true, false) => say!("sections.numbering-doubled", 0, doubled),
                (false, _) => say!("sections.numbering-none-to-take"),
            };
            return;
        };
        let rope = self.current_buffer().rope().clone();
        let ending = self.current_buffer().ending().to_string();
        let mut changed: std::collections::HashMap<usize, &str> = std::collections::HashMap::new();
        for (line, text) in &done.lines {
            changed.insert(*line, text.as_str());
        }
        let text =
            (first..=last).map(|l| changed.get(&l).copied().unwrap_or(&lines[l])).collect::<Vec<_>>().join(&ending);
        let start = rope.line_to_char(first);
        let end = rope.line_to_char(last) + lines[last].chars().count();
        // A character keeps its place before what changed on its line and
        // moves with the text after it.
        let mut lengths = Vec::with_capacity(last + 1 - first);
        let mut at = start;
        for (l, old) in lines.iter().enumerate().take(last + 1).skip(first) {
            let new = changed.get(&l).copied().unwrap_or(old);
            let same = old.chars().zip(new.chars()).take_while(|(a, b)| a == b).count();
            lengths.push((at, same, new.chars().count() as isize - old.chars().count() as isize));
            at += new.chars().count() + ending.chars().count();
        }
        let shift = text.chars().count() as isize - (end - start) as isize;
        let move_to = |c: usize| -> usize {
            if c < start {
                return c;
            }
            if c > end {
                return (c as isize + shift).max(0) as usize;
            }
            let line = rope.char_to_line(c);
            let column = c - rope.line_to_char(line);
            let (begins, same, grew) = lengths[line - first];
            match column < same {
                true => begins + column,
                false => (begins as isize + column as isize + grew).max(begins as isize) as usize,
            }
        };
        let folds = self.current_buffer().fold_map();
        self.snapshot();
        if !self.overwrite(start, end, &text) {
            return;
        }
        self.current_buffer_mut().put_folds_through(folds, &move_to);
        self.sel.map(|r| crate::selection::Range::new(move_to(r.anchor), move_to(r.head)));
        self.refresh_goal_column();
        self.status = match (on, doubled.is_empty()) {
            (true, true) => say!("sections.numbered", done.lines.len()),
            (true, false) => say!("sections.numbering-doubled", done.lines.len(), doubled),
            (false, _) => say!("sections.unnumbered", done.lines.len()),
        };
    }
}
