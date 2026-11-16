//! **The formula the cursor is in** — what the float under the
//! cursor draws, in a terminal that can show a picture.

use super::Editor;
use crate::markdown::{Block, Maths};

impl Editor {
    /// The formula the cursor stands in, if it stands in one: a one-line
    /// `$…$` or `$$…$$` (Typst's `$…$`), or anywhere inside a display-maths
    /// block that runs over several lines.
    ///
    /// Only where the maths is coloured — a Markdown or Typst file with its
    /// markup shown — so the float answers for exactly what the page draws as
    /// maths, and never for a price.
    pub fn maths_here(&self) -> Option<Maths> {
        let typst = match self.current_buffer().syntax() {
            crate::syntax::Syntax::Markdown => false,
            crate::syntax::Syntax::Typst => true,
            _ => return None,
        };
        if !self.markup_visible() {
            return None;
        }
        let line = self.cursor_line();
        let column = self.cursor_column();
        let rope = self.current_buffer().rope();
        let text_of = |i: usize| rope.line(i).to_string().trim_end_matches(['\n', '\r']).to_string();
        match self.block_of(line) {
            Block::Math { .. } => {
                let mut top = line;
                while !matches!(self.block_of(top), Block::Math { first: true, .. }) && top > 0 {
                    top -= 1;
                }
                let mut bottom = line;
                while !matches!(self.block_of(bottom), Block::Math { last: true, .. })
                    && bottom + 1 < rope.len_lines()
                {
                    bottom += 1;
                }
                let lines: Vec<String> = (top..=bottom).map(text_of).collect();
                // Not before the opening delimiter, nor past the closing one:
                // `where $` is writing up to its `$`.
                let here = &lines[line - top];
                let runs = crate::markdown::maths_block_spans(here, line == top, line == bottom, typst);
                let first = runs.iter().filter(|s| s.construct == crate::markdown::MATHS_BLOCK_CONSTRUCT).map(|s| s.start).min();
                let end = runs.iter().filter(|s| s.construct == crate::markdown::MATHS_BLOCK_CONSTRUCT).map(|s| s.end).max();
                let inside = match (first, end) {
                    (Some(a), Some(b)) => a <= column && (column < b || line != bottom),
                    _ => line != top && line != bottom,
                };
                if !inside {
                    return None;
                }
                let source = crate::markdown::maths_block_source(&lines, typst);
                // Typst sets a formula on its own when there is space inside
                // both of its ends, and a line break is space; Markdown's `$$`
                // always is.
                let display = !typst
                    || source.starts_with(char::is_whitespace) && source.ends_with(char::is_whitespace);
                Some(Maths { source: source.trim().to_string(), latex: !typst, display })
            }
            block if block.is_literal() || matches!(block, Block::Comment { .. }) => None,
            block => crate::markdown::maths_at(&text_of(line), &self.markup_line_in(line, block), column, typst),
        }
        .filter(|m| !m.source.trim().is_empty())
    }

    /// **Which formula that is**: the buffer and the line it starts on, so
    /// the float can tell a formula being typed from another one, and does not
    /// show the last picture made wherever that was.
    pub fn maths_anchor(&self) -> (u64, usize) {
        let mut line = self.cursor_line();
        while line > 0 && matches!(self.block_of(line), Block::Math { first: false, .. }) {
            line -= 1;
        }
        (self.current_buffer().id(), line)
    }
}

/// **A display-maths block the page may draw as its picture**: lines
/// `first..=last` of the buffer, and the formula they hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MathsInPlace {
    pub first: usize,
    pub last: usize,
    pub maths: Maths,
}

impl Editor {
    /// **The display-maths blocks from line `from` through `to` that the page
    /// may draw as their typeset picture** — a block that runs into the range
    /// from either end counts whole.
    ///
    /// Only under `:render full`, in a Markdown or Typst buffer, and only a
    /// block whose delimiters stand on lines of their own (`$$` above and
    /// below the formula, Typst's `$` likewise): a delimiter sharing its line
    /// with writing would take that writing into the picture. A block the
    /// cursor or the selection touches is left out, for the same reason
    /// `:render full` shows the markup there: that is where the source is
    /// being worked on. Whether a picture can be shown at all is the
    /// front end's to know; this says only which blocks would be pictures.
    pub fn maths_in_place(&self, from: usize, to: usize) -> Vec<MathsInPlace> {
        let typst = match self.current_buffer().syntax() {
            crate::syntax::Syntax::Markdown => false,
            crate::syntax::Syntax::Typst => true,
            _ => return Vec::new(),
        };
        if !self.wysiwyg() {
            return Vec::new();
        }
        let rope = self.current_buffer().rope();
        let lines = rope.len_lines();
        if lines == 0 {
            return Vec::new();
        }
        let to = to.min(lines - 1);
        let text_of = |i: usize| rope.line(i).to_string().trim_end_matches(['\n', '\r']).to_string();
        let delimiter = if typst { "$" } else { "$$" };
        let mut line = from.min(to);
        while line > 0 && matches!(self.block_of(line), Block::Math { first: false, .. }) {
            line -= 1;
        }
        let mut out = Vec::new();
        while line <= to {
            if !matches!(self.block_of(line), Block::Math { first: true, .. }) {
                line += 1;
                continue;
            }
            let first = line;
            let mut last = first;
            while last + 1 < lines && !matches!(self.block_of(last), Block::Math { last: true, .. }) {
                last += 1;
            }
            line = last + 1;
            if last == first || text_of(first).trim() != delimiter || text_of(last).trim() != delimiter {
                continue;
            }
            if (first..=last).any(|l| self.selected_columns(l).is_some()) {
                continue;
            }
            let body: Vec<String> = (first..=last).map(text_of).collect();
            let source = crate::markdown::maths_block_source(&body, typst);
            if source.trim().is_empty() {
                continue;
            }
            out.push(MathsInPlace {
                first,
                last,
                maths: Maths { source: source.trim().to_string(), latex: !typst, display: true },
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::editor::Editor;
    use crate::input::Key;
    use crate::markdown::Maths;
    use crate::syntax::Syntax;

    /// A buffer of `syntax` holding `text`, the cursor on line `line`,
    /// column `column`.
    fn at(text: &str, syntax: Syntax, line: usize, column: usize) -> Editor {
        let mut ed = Editor::new();
        ed.current_buffer_mut().set_syntax(Syntax::Text);
        ed.set_auto_pairs(crate::editor::AutoPairs::Off);
        ed.on_key(Key::Char('i'));
        for c in text.chars() {
            ed.on_key(if c == '\n' { Key::Enter } else { Key::Char(c) });
        }
        ed.on_key(Key::Esc);
        ed.set_syntax(syntax);
        for c in "gg0".chars() {
            ed.on_key(Key::Char(c));
        }
        for _ in 0..line {
            ed.on_key(Key::Char('j'));
        }
        ed.on_key(Key::Char('0'));
        for _ in 0..column {
            ed.on_key(Key::Char('l'));
        }
        assert_eq!((ed.cursor_line(), ed.cursor_column()), (line, column), "the cursor is where the test put it");
        ed
    }

    /// **Anywhere inside a display-maths block is inside the formula**, and
    /// the formula is the whole block.
    #[test]
    fn the_whole_block_is_the_formula_under_the_cursor() {
        let doc = "前文\n$$\nx = 1 \\\\\ny = 2\n$$\n後文";
        let block = Maths { source: "x = 1 \\\\\ny = 2".into(), latex: true, display: true };
        for line in 1..=4 {
            assert_eq!(at(doc, Syntax::Markdown, line, 0).maths_here(), Some(block.clone()), "line {line}");
        }
        assert_eq!(at(doc, Syntax::Markdown, 0, 0).maths_here(), None);
        assert_eq!(at(doc, Syntax::Markdown, 5, 0).maths_here(), None);
        // A plain text file has no maths, whatever it says.
        assert_eq!(at(doc, Syntax::Text, 2, 0).maths_here(), None);
    }

    /// Typst's block, and the writing on its delimiters' lines.
    #[test]
    fn a_typst_block_is_maths_from_its_dollar_on() {
        let doc = "面積 $\n  pi r^2\n$ 平方米";
        let block = Maths { source: "pi r^2".into(), latex: false, display: true };
        assert_eq!(at(doc, Syntax::Typst, 0, 0).maths_here(), None, "before the `$`");
        assert_eq!(at(doc, Syntax::Typst, 0, 3).maths_here(), Some(block.clone()));
        assert_eq!(at(doc, Syntax::Typst, 1, 3).maths_here(), Some(block.clone()));
        assert_eq!(at(doc, Syntax::Typst, 2, 0).maths_here(), Some(block));
        assert_eq!(at(doc, Syntax::Typst, 2, 2).maths_here(), None, "after the closing `$`");
        // The one-line form.
        assert_eq!(
            at("式 $x^2$ 呢", Syntax::Typst, 0, 3).maths_here(),
            Some(Maths { source: "x^2".into(), latex: false, display: false })
        );
    }

    /// The lines of each block drawn in place, from the whole of `ed`.
    fn in_place(ed: &Editor) -> Vec<(usize, usize, String)> {
        ed.maths_in_place(0, usize::MAX).into_iter().map(|b| (b.first, b.last, b.maths.source)).collect()
    }

    /// **Which blocks are pictures**: a display block with its delimiters on
    /// lines of their own, under `:render full`, and the cursor elsewhere.
    #[test]
    fn a_block_on_lines_of_its_own_is_drawn_in_place() {
        let doc = "前文\n$$\na^2 + b^2 = c^2\n$$\n中\n$$ x = 1\n$$\n中\n式 $$y$$ 呢\n$$\n\\frac{1}{2}\n\\\\ z\n$$\n後文";
        let mut ed = at(doc, Syntax::Markdown, 0, 0);
        assert_eq!(in_place(&ed), Vec::new(), "only under :render full");
        ed.set_render(crate::editor::Render::Full);
        assert_eq!(
            in_place(&ed),
            vec![(1, 3, "a^2 + b^2 = c^2".to_string()), (9, 12, "\\frac{1}{2}\n\\\\ z".to_string())],
            "a `$$` sharing its line with the formula, and a one-line `$$y$$`, stay source"
        );
        assert!(ed.maths_in_place(1, 3)[0].maths.display && ed.maths_in_place(1, 3)[0].maths.latex);
        // A range that reaches into a block from either end takes it whole.
        assert_eq!(ed.maths_in_place(2, 2).len(), 1);
        assert_eq!(ed.maths_in_place(11, 13)[0].first, 9);
        assert_eq!(ed.maths_in_place(4, 8), Vec::new());
        // A text file has no maths in it.
        ed.set_syntax(Syntax::Text);
        assert_eq!(in_place(&ed), Vec::new());
    }

    /// Typst's display block: a `$` alone above and below the formula.
    #[test]
    fn a_typst_block_on_lines_of_its_own_is_drawn_in_place() {
        let doc = "前\n$\n  sum_(k=1)^n k\n$\n面積 $\n  pi r^2\n$ 平方米";
        let mut ed = at(doc, Syntax::Typst, 0, 0);
        ed.set_render(crate::editor::Render::Full);
        let blocks = ed.maths_in_place(0, 10);
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        assert_eq!((blocks[0].first, blocks[0].last), (1, 3));
        assert_eq!(blocks[0].maths, Maths { source: "sum_(k=1)^n k".into(), latex: false, display: true });
    }

    /// **The cursor or a selection in a block shows its source**: `j`
    /// landing in it, a search landing in it, a selection reaching into it.
    #[test]
    fn the_cursor_or_a_selection_in_a_block_opens_it() {
        let doc = "前文\n$$\nx + y\n$$\n後文";
        let mut ed = at(doc, Syntax::Markdown, 0, 0);
        ed.set_render(crate::editor::Render::Full);
        assert_eq!(in_place(&ed).len(), 1);
        ed.on_key(Key::Char('j'));
        assert_eq!(in_place(&ed), Vec::new(), "`j` onto the opening line");
        ed.on_key(Key::Char('j'));
        assert_eq!(in_place(&ed), Vec::new(), "the formula's line");
        for c in "jj".chars() {
            ed.on_key(Key::Char(c));
        }
        assert_eq!(in_place(&ed).len(), 1, "out of it again");
        // A search hit inside it.
        for c in "gg/y\n".chars() {
            ed.on_key(if c == '\n' { Key::Enter } else { Key::Char(c) });
        }
        assert_eq!(ed.cursor_line(), 2);
        assert_eq!(in_place(&ed), Vec::new(), "the search landed in it");
        // A selection from the line below reaching up into it.
        ed.on_key(Key::Esc);
        for c in "Gvk".chars() {
            ed.on_key(Key::Char(c));
        }
        assert_eq!(in_place(&ed), Vec::new(), "the selection touches its closing line");
        ed.on_key(Key::Esc);
        ed.on_key(Key::Char('G'));
        assert_eq!(in_place(&ed).len(), 1);
    }
}
