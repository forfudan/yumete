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
}
